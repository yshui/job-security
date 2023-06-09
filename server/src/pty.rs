#![allow(dead_code)]
use std::{
    fs::File,
    os::{
        fd::OwnedFd,
        unix::prelude::{AsRawFd, FromRawFd, RawFd},
    },
    thread,
    time::{self, Duration},
};

pub use nix::{
    errno,
    sys::{signal::Signal, wait::WaitStatus},
    Error,
};
use nix::{
    fcntl::{open, OFlag},
    ioctl_write_ptr_bad,
    libc::{self, winsize, STDIN_FILENO, STDOUT_FILENO},
    pty::{grantpt, posix_openpt, unlockpt, PtyMaster},
    sys::{stat::Mode, termios},
    unistd::{close, dup, isatty, setsid},
    Result,
};
use termios::SpecialCharacterIndices;
use tokio::process::Command;

const DEFAULT_TERM_COLS: u16 = 80;
const DEFAULT_TERM_ROWS: u16 = 24;

const DEFAULT_VEOF_CHAR: u8 = 0x4; // ^D
const DEFAULT_INTR_CHAR: u8 = 0x3; // ^C

const DEFAULT_TERMINATE_DELAY: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub struct PtyProcess {
    master:          Master,
    eof_char:        u8,
    intr_char:       u8,
    terminate_delay: Duration,
}

impl PtyProcess {
    /// Spawns a child process and create a [PtyProcess].
    ///
    /// ```no_run
    ///   # use std::process::Command;
    ///   # use ptyprocess::PtyProcess;
    /// let proc = PtyProcess::spawn(Command::new("bash"));
    /// ```
    pub fn spawn(
        mut command: Command,
        rows: u16,
        cols: u16,
    ) -> std::io::Result<(Self, tokio::process::Child)> {
        let master = Master::open()?;
        master.grant_slave_access()?;
        master.unlock_slave()?;
        let slave_fd = master.get_slave_fd()?;
        command
            .stdin(slave_fd.try_clone()?)
            .stdout(slave_fd.try_clone()?)
            .stderr(slave_fd);
        let pts_name = master.get_slave_name()?;

        unsafe {
            command.pre_exec(move || -> std::io::Result<()> {
                make_controlling_tty(&pts_name)?;

                set_echo(STDIN_FILENO, false)?;
                set_term_size(STDIN_FILENO, cols, rows)?;
                Ok(())
            });
        }
        Ok((
            PtyProcess {
                master,
                eof_char: get_eof_char(),
                intr_char: get_intr_char(),
                terminate_delay: DEFAULT_TERMINATE_DELAY,
            },
            command.spawn()?,
        ))
    }

    pub fn get_raw_handle(&self) -> Result<File> {
        self.master.get_file_handle()
    }

    /// Get a end of file character if set or a default.
    pub fn get_eof_char(&self) -> u8 {
        self.eof_char
    }

    /// Get a interapt character if set or a default.
    pub fn get_intr_char(&self) -> u8 {
        self.intr_char
    }

    /// Get window size of a terminal.
    ///
    /// Default size is 80x24.
    pub fn get_window_size(&self) -> Result<(u16, u16)> {
        get_term_size(self.master.as_raw_fd())
    }

    /// Sets a terminal size.
    pub fn set_window_size(&self, cols: u16, rows: u16) -> Result<()> {
        set_term_size(self.master.as_raw_fd(), cols, rows)
    }

    /// The function returns true if an echo setting is setup.
    pub fn get_echo(&self) -> Result<bool> {
        termios::tcgetattr(self.master.as_raw_fd())
            .map(|flags| flags.local_flags.contains(termios::LocalFlags::ECHO))
    }

    /// Sets a echo setting for a terminal
    pub fn set_echo(&mut self, on: bool, timeout: Option<Duration>) -> Result<bool> {
        set_echo(self.master.as_raw_fd(), on)?;
        self.wait_echo(on, timeout)
    }

    /// Returns true if a underline `fd` connected with a TTY.
    pub fn isatty(&self) -> Result<bool> {
        isatty(self.master.as_raw_fd())
    }

    /// Set the pty process's terminate approach delay.
    pub fn set_terminate_delay(&mut self, terminate_approach_delay: Duration) {
        self.terminate_delay = terminate_approach_delay;
    }

    fn wait_echo(&self, on: bool, timeout: Option<Duration>) -> Result<bool> {
        let now = time::Instant::now();
        while timeout.is_none() || now.elapsed() < timeout.unwrap() {
            if on == self.get_echo()? {
                return Ok(true)
            }

            thread::sleep(Duration::from_millis(100));
        }

        Ok(false)
    }
}

fn set_term_size(fd: i32, cols: u16, rows: u16) -> Result<()> {
    ioctl_write_ptr_bad!(_set_window_size, libc::TIOCSWINSZ, winsize);

    let size = winsize {
        ws_row:    rows,
        ws_col:    cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    let _ = unsafe { _set_window_size(fd, &size) }?;

    Ok(())
}

fn get_term_size(fd: i32) -> Result<(u16, u16)> {
    nix::ioctl_read_bad!(_get_window_size, libc::TIOCGWINSZ, winsize);

    let mut size = winsize {
        ws_col:    0,
        ws_row:    0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    let _ = unsafe { _get_window_size(fd, &mut size) }?;

    Ok((size.ws_col, size.ws_row))
}

#[derive(Debug)]
struct Master {
    fd: PtyMaster,
}

impl Master {
    fn open() -> Result<Self> {
        let master_fd = posix_openpt(OFlag::O_RDWR)?;
        // Set close-on-exec flag
        nix::fcntl::fcntl(
            master_fd.as_raw_fd(),
            nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
        )?;
        Ok(Self { fd: master_fd })
    }

    fn grant_slave_access(&self) -> Result<()> {
        grantpt(&self.fd)
    }

    fn unlock_slave(&self) -> Result<()> {
        unlockpt(&self.fd)
    }

    fn get_slave_name(&self) -> Result<String> {
        get_slave_name(&self.fd)
    }

    fn get_slave_fd(&self) -> Result<OwnedFd> {
        let slave_name = self.get_slave_name()?;
        let slave_fd = open(
            slave_name.as_str(),
            OFlag::O_RDWR | OFlag::O_NOCTTY,
            Mode::empty(),
        )?;
        Ok(unsafe { OwnedFd::from_raw_fd(slave_fd) })
    }

    fn get_file_handle(&self) -> Result<File> {
        let fd = dup(self.as_raw_fd())?;
        let file = unsafe { File::from_raw_fd(fd) };

        Ok(file)
    }
}

impl AsRawFd for Master {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

fn get_slave_name(fd: &PtyMaster) -> Result<String> {
    nix::pty::ptsname_r(fd)
}

fn set_echo(fd: RawFd, on: bool) -> Result<()> {
    // Set echo off
    // Even though there may be something left behind https://stackoverflow.com/a/59034084
    let mut flags = termios::tcgetattr(fd)?;
    match on {
        true => flags.local_flags |= termios::LocalFlags::ECHO,
        false => flags.local_flags &= !termios::LocalFlags::ECHO,
    }

    termios::tcsetattr(fd, termios::SetArg::TCSANOW, &flags)?;
    Ok(())
}

pub fn set_raw(fd: RawFd) -> Result<()> {
    let mut flags = termios::tcgetattr(fd)?;

    termios::cfmakeraw(&mut flags);
    termios::tcsetattr(fd, termios::SetArg::TCSANOW, &flags)?;
    Ok(())
}

fn get_this_term_char(char: SpecialCharacterIndices) -> Option<u8> {
    for &fd in &[STDIN_FILENO, STDOUT_FILENO] {
        if let Ok(char) = get_term_char(fd, char) {
            return Some(char)
        }
    }

    None
}

fn get_intr_char() -> u8 {
    get_this_term_char(SpecialCharacterIndices::VINTR).unwrap_or(DEFAULT_INTR_CHAR)
}

fn get_eof_char() -> u8 {
    get_this_term_char(SpecialCharacterIndices::VEOF).unwrap_or(DEFAULT_VEOF_CHAR)
}

fn get_term_char(fd: RawFd, char: SpecialCharacterIndices) -> Result<u8> {
    let flags = termios::tcgetattr(fd)?;
    let b = flags.control_chars[char as usize];
    Ok(b)
}

fn make_controlling_tty(pts_name: &str) -> Result<()> {
    // https://github.com/pexpect/ptyprocess/blob/c69450d50fbd7e8270785a0552484182f486092f/ptyprocess/_fork_pty.py

    // Disconnect from controlling tty, if any
    //
    // it may be a simmilar call to ioctl TIOCNOTTY
    // https://man7.org/linux/man-pages/man4/tty_ioctl.4.html
    let fd = open("/dev/tty", OFlag::O_RDWR | OFlag::O_NOCTTY, Mode::empty());
    match fd {
        Ok(fd) => {
            close(fd)?;
        },
        Err(Error::ENXIO) => {
            // Sometimes we get ENXIO right here which 'probably' means
            // that we has been already disconnected from controlling tty.
            // Specifically it was discovered on ubuntu-latest Github CI
            // platform.
        },
        Err(err) => return Err(err),
    }

    // setsid() will remove the controlling tty. Also the ioctl TIOCNOTTY does this.
    // https://www.win.tue.nl/~aeb/linux/lk/lk-10.html
    setsid()?;

    // Verify we are disconnected from controlling tty by attempting to open
    // it again.  We expect that OSError of ENXIO should always be raised.
    let fd = open("/dev/tty", OFlag::O_RDWR | OFlag::O_NOCTTY, Mode::empty());
    match fd {
        Err(Error::ENXIO) => {}, // ok
        Ok(fd) => {
            close(fd)?;
            return Err(Error::ENOTSUP)
        },
        Err(_) => return Err(Error::ENOTSUP),
    }

    // Verify we can open child pty.
    let fd = open(pts_name, OFlag::O_RDWR, Mode::empty())?;
    close(fd)?;

    // Verify we now have a controlling tty.
    let fd = open("/dev/tty", OFlag::O_WRONLY, Mode::empty())?;
    close(fd)?;
    Ok(())
}
