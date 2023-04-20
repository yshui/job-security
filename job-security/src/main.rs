use std::ffi::OsString;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use daemonize::{Daemonize, Outcome};

#[derive(Subcommand, Debug, PartialEq, Eq)]
enum Command {
    /// Start as daemon
    Daemon {
        #[clap(short, long)]
        log_file: Option<String>,
    },
    /// Resume a stopped job, and/or bring the backgrounded job to the
    /// foreground. Attempting to `continue` a job that is already running
    /// in the foreground is an error.
    Continue {
        /// Detach after resuming. If not specified, the resumed job will be
        /// brought to the foreground. If the job was already not stopped,
        /// specifying this will make `continue` a no-op.
        #[clap(short, long)]
        detached: bool,

        /// The id of the job to resume/foreground. If not specified, the
        /// last backgrounded/stopped job will be resumed.
        id: Option<u32>,

        /// Fetch the output that has generated while the job was running in the
        /// background. (A stopped job will not generate any output)
        #[clap(long, short = 'o')]
        fetch_output: bool,
    },
    /// Start a new job
    Run {
        cmd:  String,
        args: Vec<String>,

        /// Detach after starting the job. If not specified, the job will run in
        /// the foreground
        #[clap(short, long)]
        detached: bool,
    },
    /// List all jobs
    List,
    /// Stop the server
    KillServer,
}

enum Shell {
    Bash,
    Zsh,
    Nushell,
    Unknown,
}

impl Shell {
    fn wrap_command(&self, cmd: Vec<String>) -> Vec<OsString> {
        match self {
            Shell::Bash => ["bash", "-c", &cmd.join(" ")]
                .map(Into::into)
                .into_iter()
                .collect(),
            Shell::Zsh => ["zsh", "-c", &cmd.join(" ")]
                .map(Into::into)
                .into_iter()
                .collect(),
            Shell::Nushell | Shell::Unknown => cmd.into_iter().map(Into::into).collect(),
        }
    }
}

fn detect_current_shell() -> Shell {
    if std::env::var("NU_VERSION").ok().is_some() {
        return Shell::Nushell
    }
    let shell = std::env::var("SHELL").unwrap();
    let basename = std::path::Path::new(&shell)
        .file_name()
        .unwrap()
        .to_str()
        .unwrap();
    match basename {
        "bash" => Shell::Bash,
        "zsh" => Shell::Zsh,
        _ => Shell::Unknown,
    }
}

#[derive(Parser, Debug)]
struct Opts {
    #[command(subcommand)]
    command: Command,
}

fn start_server() -> std::io::Result<()> {
    let server = server::Server::new(vec!["__runner".into()]).unwrap();

    match Daemonize::new().working_directory("/").execute() {
        Outcome::Child(child) => {
            let _ = child.unwrap();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(server.run()).unwrap();
            std::process::exit(0);
        },
        Outcome::Parent(parent) => {
            let _ = parent.unwrap();
            std::mem::forget(server);
            Ok(())
        },
    }
}

mod ui;
use protocol::{ExitStatus, ProcessState};
use ui::Tui;

fn main() -> std::io::Result<()> {
    let command = Opts::command()
        .subcommand(clap::Command::new("__runner").hide(true))
        .mut_subcommand("run", |c| {
            c.mut_arg("args", |a| a.allow_hyphen_values(true).last(true))
        })
        .mut_subcommand("continue", |c| {
            c.visible_alias("fg").visible_alias("resume")
        })
        .infer_subcommands(true);
    let matches = command.get_matches();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    if matches.subcommand_matches("__runner").is_some() {
        let runner = server::Runner::default();
        unsafe { runtime.block_on(runner.run()) };
        return Ok(())
    }
    let opts = Opts::from_arg_matches(&matches).unwrap();
    let subscriber_builder = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env());
    if let Command::Daemon { log_file } = &opts.command {
        let server = server::Server::new(vec!["__runner".into()]).unwrap();
        if let Some(log_file) = log_file {
            let log_file = std::fs::OpenOptions::new()
                .read(false)
                .write(true)
                .create(true)
                .truncate(false)
                .open(log_file)?;
            subscriber_builder.with_writer(log_file).init();
        } else {
            subscriber_builder.init();
        }
        runtime.block_on(server.run()).unwrap();
        return Ok(())
    }

    subscriber_builder.init();
    let start_server: Option<fn() -> std::io::Result<()>> =
        if let Command::Run { .. } = &opts.command {
            Some(start_server)
        } else {
            None
        };
    let client = match client::Client::new(start_server) {
        Ok(client) => client,
        Err(e)
            if e.kind() == std::io::ErrorKind::NotFound ||
                e.kind() == std::io::ErrorKind::ConnectionRefused =>
        {
            eprintln!("Daemon is not running");
            return Ok(())
        },
        Err(e) => return Err(e),
    };

    let status = match opts.command {
        Command::Run {
            cmd,
            args,
            detached,
        } => {
            let shell = detect_current_shell();
            let cmd = std::iter::once(cmd).chain(args).collect();
            let cmd = shell.wrap_command(cmd);
            runtime.block_on(client.run(
                &mut Tui,
                protocol::Request::Start {
                    command: cmd,
                    env:     std::env::vars_os().collect(),
                    pwd:     std::env::current_dir().unwrap(),
                    cols:    0,
                    rows:    0,
                },
                detached,
            ))?
        },
        Command::Continue {
            id,
            detached,
            fetch_output,
        } => runtime.block_on(client.run(
            &mut Tui,
            protocol::Request::Resume {
                id,
                with_output: fetch_output,
            },
            detached,
        ))?,
        Command::KillServer =>
            runtime.block_on(client.run(&mut Tui, protocol::Request::Quit, true))?,
        Command::List =>
            runtime.block_on(client.run(&mut Tui, protocol::Request::ListProcesses, false))?,
        _ => unreachable!(),
    };
    if let Some(status) = status {
        match status {
            ProcessState::Stopped => {
                println!("Job stopped");
            },
            ProcessState::Terminated(status) => match status {
                ExitStatus::Exited(code) if code == 0 => println!("Job completed successfully"),
                ExitStatus::Exited(code) => println!("Job completed with code {}", code),
                ExitStatus::Signaled(signal) => println!(
                    "Job was terminated by signal {}",
                    client::signal_to_string(signal)?
                ),
            },
            ProcessState::Running => {},
        }
    }
    Ok(())
}
