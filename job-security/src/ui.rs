use std::ops::Deref;

use protocol::ExitStatus;
use tabled::{
    grid::{
        color::AnsiColor,
        config::{
            AlignmentHorizontal, ColorMap, ColoredConfig, Entity, Indent, Sides, SpannedConfig,
        },
        dimension::{CompleteDimension, Estimate},
        records::vec_records::VecRecords,
    },
    settings::{
        object::{Object, Rows},
        TableOption,
    },
};
use termcolor::{Buffer, Color, ColorSpec, WriteColor};
pub(crate) struct ColoredString {
    color:  ColorSpec,
    string: String,
}

impl ColoredString {
    fn new(color: &ColorSpec, string: impl ToString) -> Self {
        Self {
            color:  color.clone(),
            string: string.to_string(),
        }
    }

    fn color(&self) -> AnsiColor<'static> {
        let mut prefix = Buffer::ansi();
        prefix.set_color(&self.color).unwrap();
        let mut suffix = Buffer::ansi();
        suffix.reset().unwrap();
        let prefix = String::from_utf8(prefix.into_inner()).unwrap();
        let suffix = String::from_utf8(suffix.into_inner()).unwrap();
        AnsiColor::new(prefix.into(), suffix.into())
    }
}

#[derive(Debug)]
struct PrettyProcessState(protocol::ProcessState);

#[derive(Debug)]
enum FgBg {
    Foreground,
    Background,
    Irrelevant,
}

impl From<&'_ str> for ColoredString {
    fn from(s: &str) -> Self {
        ColoredString::new(&ColorSpec::new(), s)
    }
}

impl From<&'_ FgBg> for ColoredString {
    fn from(i: &FgBg) -> Self {
        use FgBg::*;
        match i {
            Foreground => ColoredString::new(ColorSpec::new().set_bold(true), "Foreground"),
            Background => ColoredString::new(&ColorSpec::new(), "Background"),
            Irrelevant => ColoredString::new(&ColorSpec::new(), ""),
        }
    }
}

impl From<&'_ PrettyProcessState> for ColoredString {
    fn from(i: &PrettyProcessState) -> Self {
        use protocol::ProcessState;
        match i.0 {
            ProcessState::Running => ColoredString::new(
                ColorSpec::new()
                    .set_fg(Some(Color::Green))
                    .set_intense(true)
                    .set_bold(true),
                "Running",
            ),
            ProcessState::Stopped => ColoredString::new(
                ColorSpec::new()
                    .set_fg(Some(Color::Yellow))
                    .set_intense(true)
                    .set_bold(true),
                "Stopped",
            ),
            ProcessState::Terminated(ExitStatus::Signaled(signal)) => ColoredString::new(
                ColorSpec::new().set_fg(Some(Color::Red)).set_bold(true),
                format!(
                    "Killed - {}",
                    client::signal_to_string(signal).unwrap_or("UNKNOWN")
                ),
            ),
            ProcessState::Terminated(ExitStatus::Exited(exit_code)) =>
                if exit_code == 0 {
                    ColoredString::new(ColorSpec::new().set_fg(Some(Color::Green)), "Finished")
                } else {
                    ColoredString::new(
                        ColorSpec::new().set_fg(Some(Color::Red)).set_bold(true),
                        format!("Finished with error - {}", exit_code),
                    )
                },
        }
    }
}

impl AsRef<str> for ColoredString {
    fn as_ref(&self) -> &str {
        &self.string
    }
}

struct PrettyProcess {
    id:      usize,
    command: String,
    state:   PrettyProcessState,
    fgbg:    FgBg,
    pid:     u32,
}

struct PrettyProcessIter<'a> {
    inner: &'a PrettyProcess,
    field: usize,
}

impl Iterator for PrettyProcessIter<'_> {
    type Item = ColoredString;

    fn next(&mut self) -> Option<Self::Item> {
        let result = match self.field {
            0 => self.inner.id.to_string().as_str().into(),
            1 => self.inner.command.as_str().into(),
            2 => (&self.inner.state).into(),
            3 => (&self.inner.fgbg).into(),
            4 => self.inner.pid.to_string().as_str().into(),
            _ => return None,
        };
        self.field += 1;
        Some(result)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = 5 - self.field;
        (remaining, Some(remaining))
    }
}

impl<'a> IntoIterator for &'a PrettyProcess {
    type IntoIter = PrettyProcessIter<'a>;
    type Item = ColoredString;

    fn into_iter(self) -> Self::IntoIter {
        PrettyProcessIter {
            inner: self,
            field: 0,
        }
    }
}

struct PrettyProcesses(Vec<PrettyProcess>);
pub(crate) type Table = tabled::grid::Grid<
    VecRecords<ColoredString>,
    CompleteDimension<'static>,
    SpannedConfig,
    ColorMap,
>;
impl PrettyProcesses {
    fn records(&self) -> Vec<Vec<ColoredString>> {
        let mut ret = Vec::new();
        let mut bright_white = ColorSpec::new();
        bright_white.set_fg(Some(Color::White)).set_bold(true);
        ret.push(vec![
            ColoredString::new(&bright_white, "ID"),
            ColoredString::new(&bright_white, "Command"),
            ColoredString::new(&bright_white, "State"),
            ColoredString::new(&bright_white, "Fg/Bg"),
            ColoredString::new(&bright_white, "PID"),
        ]);
        for p in &self.0 {
            ret.push(p.into_iter().collect());
        }
        ret
    }

    pub(crate) fn grid(&self) -> Table {
        let mut config = ColoredConfig::new(SpannedConfig::default());
        let records = self.records();
        for (r, row) in records.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                config.set_color((r, c).into(), cell.color());
            }
        }
        let mut records = VecRecords::new(records);

        let mut dimension = CompleteDimension::default();
        config.set_alignment_horizontal(Entity::Global, AlignmentHorizontal::Right);
        for entity in Rows::new(0..=0).cells(&records) {
            config.set_alignment_horizontal(entity, AlignmentHorizontal::Center);
        }
        config.set_padding(Entity::Global, Sides {
            left: Indent::spaced(1),
            right: Indent::spaced(1),
            ..Default::default()
        });
        tabled::settings::Style::rounded().change(&mut records, &mut config, &mut dimension);
        dimension.estimate(&records, &config);

        tabled::grid::Grid::new(
            records,
            dimension,
            config.deref().clone(),
            config.get_colors().clone(),
        )
    }
}

pub(crate) struct Tui;
impl client::UserInterface for Tui {
    fn print_processes(&mut self, processes: &[protocol::Process]) {
        if processes.is_empty() {
            println!("No jobs found");
            return
        }
        let pretty_processes = PrettyProcesses(
            processes
                .iter()
                .map(|p| PrettyProcess {
                    id:      p.id as usize,
                    pid:     p.pid,
                    state:   PrettyProcessState(p.state),
                    fgbg:    if p.state == protocol::ProcessState::Running {
                        if p.connected {
                            FgBg::Foreground
                        } else {
                            FgBg::Background
                        }
                    } else {
                        FgBg::Irrelevant
                    },
                    command: p.command.to_string_lossy().to_string(),
                })
                .collect(),
        );
        println!("{}", pretty_processes.grid().to_string());
    }
}
