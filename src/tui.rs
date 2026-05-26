//! Terminal UI for parallel sample execution.

use std::{
    io::{self, IsTerminal, Write},
    sync::{Arc, Mutex},
};

use crate::{AgentStreamEvent, AgentStreamObserver, SampleObserverFactory, SamplePlan};

const DEFAULT_TERMINAL_WIDTH: usize = 120;
const DEFAULT_TERMINAL_HEIGHT: usize = 24;
const HEADER_ROWS: usize = 3;
const MIN_COLUMN_WIDTH: usize = 16;
const MIN_SAMPLE_COUNT: usize = 1;
const DEFAULT_EVENT_ROWS: usize = 12;
const ELLIPSIS_WIDTH: usize = 3;
const INLINE_HEADER_ROWS: usize = 2;
const ANSI_CLEAR: &str = "\x1b[2J";
const ANSI_HOME: &str = "\x1b[H";
const ANSI_HIDE_CURSOR: &str = "\x1b[?25l";
const ANSI_SHOW_CURSOR: &str = "\x1b[?25h";
const ANSI_ENTER_ALT: &str = "\x1b[?1049h";
const ANSI_LEAVE_ALT: &str = "\x1b[?1049l";
const ANSI_BLUE: &str = "\x1b[34m";
const ANSI_GREEN: &str = "\x1b[32m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_RESET: &str = "\x1b[0m";
const PERFECT_SCORE: i64 = 10_000;

/// Renders sample tool-call streams as one terminal column per sample.
#[derive(Clone, Debug)]
pub struct SampleTui {
    state: Arc<Mutex<SampleTuiState>>,
}

impl SampleTui {
    /// Creates a terminal UI for a fixed sample count.
    #[must_use]
    pub fn new(sample_count: usize) -> Self {
        Self::for_task("task", 1, sample_count)
    }

    /// Creates a terminal UI for one task run.
    #[must_use]
    pub fn for_task(task_id: &str, run_index: usize, sample_count: usize) -> Self {
        let mode = SampleTuiMode::for_terminal(sample_count, io::stdout().is_terminal());
        let state = SampleTuiState::new(task_id.to_owned(), run_index, sample_count, mode);
        let tui = Self {
            state: Arc::new(Mutex::new(state)),
        };
        tui.start();
        tui
    }

    /// Returns an observer for one sample column.
    #[must_use]
    pub fn observer(&self, sample_id: usize) -> SampleTuiObserver {
        SampleTuiObserver {
            sample_id,
            state: Arc::clone(&self.state),
        }
    }

    /// Marks a sample column as complete.
    ///
    /// # Errors
    ///
    /// Returns an error when terminal output cannot be written.
    pub fn finish_sample(&self, sample_id: usize, accepted: bool, score: i64) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_error| io::Error::other("sample TUI lock poisoned"))?;
        state.finish_sample(sample_id, accepted, score)
    }

    /// Restores terminal state after the sample run.
    ///
    /// # Errors
    ///
    /// Returns an error when terminal output cannot be written.
    pub fn finish(&self) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_error| io::Error::other("sample TUI lock poisoned"))?;
        state.finish()
    }

    fn start(&self) {
        if let Ok(mut state) = self.state.lock() {
            let _ = state.start();
        }
    }
}

/// Per-sample observer that writes tool calls into one TUI column.
#[derive(Clone, Debug)]
pub struct SampleTuiObserver {
    sample_id: usize,
    state: Arc<Mutex<SampleTuiState>>,
}

impl AgentStreamObserver for SampleTuiObserver {
    fn observe(&mut self, event: &AgentStreamEvent) {
        if let Ok(mut state) = self.state.lock() {
            let line = event.to_string();
            let _ = state.push(self.sample_id, &line);
        }
    }
}

impl SampleObserverFactory for SampleTui {
    fn observer_for(&self, sample: &SamplePlan) -> Box<dyn AgentStreamObserver + Send> {
        Box::new(self.observer(sample.id().as_usize()))
    }
}

#[derive(Clone, Debug)]
struct SampleTuiState {
    task_id: String,
    run_index: usize,
    sample_count: usize,
    mode: SampleTuiMode,
    active: bool,
    columns: Vec<Vec<String>>,
    statuses: Vec<SampleTuiStatus>,
}

impl SampleTuiState {
    fn new(task_id: String, run_index: usize, sample_count: usize, mode: SampleTuiMode) -> Self {
        let sample_count = usize::max(MIN_SAMPLE_COUNT, sample_count);
        Self {
            task_id,
            run_index,
            sample_count,
            mode,
            active: false,
            columns: vec![Vec::new(); sample_count],
            statuses: vec![SampleTuiStatus::Running; sample_count],
        }
    }

    fn start(&mut self) -> io::Result<()> {
        if self.mode != SampleTuiMode::Columns || self.active {
            return Ok(());
        }

        self.active = true;
        let mut stdout = io::stdout().lock();
        write!(stdout, "{ANSI_ENTER_ALT}{ANSI_HIDE_CURSOR}")?;
        stdout.flush()?;
        drop(stdout);
        self.render()
    }

    fn push(&mut self, sample_id: usize, line: &str) -> io::Result<()> {
        if sample_id == 0 || sample_id > self.sample_count {
            return Ok(());
        }

        let Some(column) = self.columns.get_mut(sample_id - 1) else {
            return Ok(());
        };
        column.push(line.to_owned());

        if column.len() > event_rows() {
            let overflow = column.len() - event_rows();
            drop(column.drain(0..overflow));
        }

        match self.mode {
            #[cfg(test)]
            SampleTuiMode::Disabled => Ok(()),
            SampleTuiMode::Inline => self.write_inline(sample_id, line),
            SampleTuiMode::Columns => self.render(),
        }
    }

    fn finish(&mut self) -> io::Result<()> {
        if self.mode != SampleTuiMode::Columns || !self.active {
            return Ok(());
        }

        self.active = false;
        let mut stdout = io::stdout().lock();
        write!(
            stdout,
            "{ANSI_SHOW_CURSOR}{ANSI_CLEAR}{ANSI_HOME}{ANSI_LEAVE_ALT}"
        )?;
        stdout.flush()
    }

    fn finish_sample(&mut self, sample_id: usize, accepted: bool, score: i64) -> io::Result<()> {
        if sample_id == 0 || sample_id > self.sample_count {
            return Ok(());
        }
        let Some(status) = self.statuses.get_mut(sample_id - 1) else {
            return Ok(());
        };
        *status = SampleTuiStatus::from_score(accepted, score);
        self.render()
    }

    fn render(&self) -> io::Result<()> {
        if self.mode != SampleTuiMode::Columns || !self.active {
            return Ok(());
        }

        let width = terminal_width();
        let column_width = usize::max(MIN_COLUMN_WIDTH, width / self.sample_count);
        let rows = event_rows();
        let mut stdout = io::stdout().lock();
        write!(stdout, "{ANSI_CLEAR}{ANSI_HOME}")?;

        for row in 0..rows + HEADER_ROWS {
            for column_index in 0..self.sample_count {
                let cell = self.cell(column_index, row);
                let fitted = fit_cell(&cell, column_width);
                let styled = self.styled_cell(column_index, row, &fitted);
                write!(stdout, "{styled}")?;
            }
            writeln!(stdout)?;
        }

        stdout.flush()
    }

    fn write_inline(&self, sample_id: usize, line: &str) -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        if self.sample_count > MIN_SAMPLE_COUNT {
            writeln!(stdout, "sample {sample_id} {line}")?;
        } else {
            writeln!(stdout, "{line}")?;
        }
        stdout.flush()
    }

    fn cell(&self, column_index: usize, row: usize) -> String {
        if self.mode != SampleTuiMode::Columns {
            if row == 0 {
                return format!("sample {}", column_index + 1);
            }

            if row == 1 {
                return "-".repeat(MIN_COLUMN_WIDTH);
            }

            let Some(column) = self.columns.get(column_index) else {
                return String::new();
            };
            let event_index = row - INLINE_HEADER_ROWS;
            return column.get(event_index).cloned().unwrap_or_default();
        }

        if row == 0 {
            return column_border();
        }

        if row == 1 {
            return format!(
                "|{}|",
                fit_inner(
                    &format!(
                        "#{} R{}/S{} {}",
                        self.task_id,
                        self.run_index,
                        column_index + 1,
                        self.status(column_index).label()
                    ),
                    MIN_COLUMN_WIDTH.saturating_sub(2),
                )
            );
        }

        if row == INLINE_HEADER_ROWS {
            return column_border();
        }

        let Some(column) = self.columns.get(column_index) else {
            return String::new();
        };
        let event_index = row - HEADER_ROWS;
        column
            .get(event_index)
            .map_or_else(empty_event_cell, |line| event_cell(line))
    }

    fn status(&self, column_index: usize) -> SampleTuiStatus {
        self.statuses
            .get(column_index)
            .copied()
            .unwrap_or(SampleTuiStatus::Running)
    }

    fn styled_cell(&self, column_index: usize, row: usize, cell: &str) -> String {
        if self.mode != SampleTuiMode::Columns || row >= HEADER_ROWS {
            return cell.to_owned();
        }
        let status = self.status(column_index);
        format!("{}{}{}", status.color(), cell, ANSI_RESET)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SampleTuiStatus {
    Running,
    Improved(i64),
    Regressed(i64),
    Perfect(i64),
}

impl SampleTuiStatus {
    const fn from_score(accepted: bool, score: i64) -> Self {
        if accepted && score >= PERFECT_SCORE {
            Self::Perfect(score)
        } else if accepted {
            Self::Improved(score)
        } else {
            Self::Regressed(score)
        }
    }

    fn label(self) -> String {
        match self {
            Self::Running => "...".to_owned(),
            Self::Improved(score) => format!("✓ {score}"),
            Self::Regressed(score) => format!("✗ {score}"),
            Self::Perfect(score) => format!("✅ {score}"),
        }
    }

    const fn color(self) -> &'static str {
        match self {
            Self::Running => ANSI_BLUE,
            Self::Improved(_) | Self::Perfect(_) => ANSI_GREEN,
            Self::Regressed(_) => ANSI_RED,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SampleTuiMode {
    #[cfg(test)]
    Disabled,
    Inline,
    Columns,
}

impl SampleTuiMode {
    fn for_terminal(sample_count: usize, terminal: bool) -> Self {
        if sample_count <= MIN_SAMPLE_COUNT {
            Self::Inline
        } else if terminal {
            Self::Columns
        } else {
            Self::Inline
        }
    }
}

impl Drop for SampleTuiState {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn fit_cell(value: &str, width: usize) -> String {
    let content_width = width.saturating_sub(1);
    let mut output = if value.chars().count() > content_width && content_width > ELLIPSIS_WIDTH {
        let mut output = value
            .chars()
            .take(content_width - ELLIPSIS_WIDTH)
            .collect::<String>();
        output.push_str("...");
        output
    } else {
        value.chars().take(content_width).collect::<String>()
    };

    while output.chars().count() < content_width {
        output.push(' ');
    }
    output.push(' ');
    output
}

fn fit_inner(value: &str, width: usize) -> String {
    let mut output = if value.chars().count() > width && width > ELLIPSIS_WIDTH {
        let mut output = value
            .chars()
            .take(width - ELLIPSIS_WIDTH)
            .collect::<String>();
        output.push_str("...");
        output
    } else {
        value.chars().take(width).collect::<String>()
    };

    while output.chars().count() < width {
        output.push(' ');
    }
    output
}

fn column_border() -> String {
    format!("+{}+", "-".repeat(MIN_COLUMN_WIDTH.saturating_sub(2)))
}

fn event_cell(line: &str) -> String {
    format!("|{}|", fit_inner(line, MIN_COLUMN_WIDTH.saturating_sub(2)))
}

fn empty_event_cell() -> String {
    event_cell("")
}

fn terminal_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_TERMINAL_WIDTH)
}

fn event_rows() -> usize {
    std::env::var("LINES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map_or(DEFAULT_EVENT_ROWS, |rows| {
            rows.saturating_sub(HEADER_ROWS)
                .min(DEFAULT_TERMINAL_HEIGHT)
        })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{
        HEADER_ROWS, SampleTui, SampleTuiMode, SampleTuiState, SampleTuiStatus, event_rows,
        fit_cell, fit_inner,
    };
    use crate::{
        AgentStreamEvent, AgentStreamObserver, AgentStreamToolCallEvent, SampleObserverFactory,
        SamplePlanner, SsdConfig, TaskKind,
    };

    #[test]
    fn fits_cells_to_column_width() {
        assert_eq!(fit_cell("abc", 6), "abc   ");
        assert_eq!(fit_cell("abcdef", 6), "ab... ");
        assert_eq!(fit_inner("abcdef", 5), "ab...");
    }

    #[test]
    fn disabled_tui_accepts_events_without_rendering() -> std::io::Result<()> {
        let mut state = SampleTuiState::new("123".to_owned(), 1, 2, SampleTuiMode::Disabled);

        state.start()?;
        state.push(1, "[Bash: cargo test]")?;
        state.push(0, "[Bash: ignored]")?;
        state.push(3, "[Bash: ignored]")?;
        state.finish()?;
        let mut inconsistent = SampleTuiState::new("123".to_owned(), 1, 1, SampleTuiMode::Disabled);
        inconsistent.columns.clear();
        inconsistent.push(1, "[Bash: ignored]")?;

        assert_eq!(state.cell(0, 2), "[Bash: cargo test]");
        assert!(state.cell(1, 2).is_empty());
        assert!(state.cell(9, 2).is_empty());

        Ok(())
    }

    #[test]
    fn sample_tui_uses_at_least_one_column() {
        let state = SampleTuiState::new("123".to_owned(), 1, 0, SampleTuiMode::Disabled);

        assert_eq!(state.sample_count, 1);
        assert_eq!(state.columns.len(), 1);
    }

    #[test]
    fn single_sample_uses_inline_stdout_mode() -> std::io::Result<()> {
        assert_eq!(SampleTuiMode::for_terminal(1, true), SampleTuiMode::Inline);
        assert_eq!(SampleTuiMode::for_terminal(1, false), SampleTuiMode::Inline);
        assert_eq!(SampleTuiMode::for_terminal(2, true), SampleTuiMode::Columns);
        assert_eq!(SampleTuiMode::for_terminal(2, false), SampleTuiMode::Inline);

        let mut state = SampleTuiState::new("123".to_owned(), 1, 1, SampleTuiMode::Inline);
        state.start()?;
        state.push(1, "[Bash: cargo test]")?;
        state.finish()?;

        assert!(!state.active);
        assert_eq!(state.cell(0, 2), "[Bash: cargo test]");

        Ok(())
    }

    #[test]
    fn sample_tui_keeps_only_visible_event_rows() -> std::io::Result<()> {
        let mut state = SampleTuiState::new("123".to_owned(), 1, 1, SampleTuiMode::Disabled);

        for index in 0..event_rows() + HEADER_ROWS {
            state.push(1, &format!("[Bash: event {index}]"))?;
        }

        assert_eq!(state.columns.first().map(Vec::len), Some(event_rows()));

        Ok(())
    }

    #[test]
    fn enabled_tui_renders_and_finishes() -> std::io::Result<()> {
        let mut state = SampleTuiState::new("123".to_owned(), 1, 2, SampleTuiMode::Columns);

        state.start()?;
        state.push(1, "[Bash: cargo test]")?;
        state.push(2, "[Read: src/main.rs]")?;
        state.finish_sample(1, true, 10_000)?;
        state.finish_sample(2, false, 3_250)?;
        state.finish()?;
        state.finish()?;

        assert_eq!(state.cell(0, 0), "+--------------+");
        assert!(state.cell(0, 1).starts_with("|#123 R1/S1"));
        assert!(state.cell(0, 1).ends_with('|'));
        assert!(state.cell(1, 1).starts_with("|#123 R1/S2"));
        assert!(state.cell(1, 1).ends_with('|'));
        assert_eq!(state.status(0), SampleTuiStatus::Perfect(10_000));
        assert_eq!(state.status(1), SampleTuiStatus::Regressed(3_250));
        assert!(
            state
                .styled_cell(0, 0, &state.cell(0, 0))
                .starts_with(super::ANSI_GREEN)
        );

        Ok(())
    }

    #[test]
    fn sample_tui_observer_records_tool_call() -> Result<(), Box<dyn std::error::Error>> {
        let tui = SampleTui {
            state: Arc::new(Mutex::new(SampleTuiState::new(
                "123".to_owned(),
                1,
                1,
                SampleTuiMode::Disabled,
            ))),
        };
        let mut observer = tui.observer(1);
        let event = AgentStreamEvent::ToolCall(AgentStreamToolCallEvent::new(
            "Bash".to_owned(),
            "cargo test".to_owned(),
        ));

        observer.observe(&event);

        let rendered_line = {
            let state = tui
                .state
                .lock()
                .map_err(|_error| std::io::Error::other("sample TUI state should lock"))?;
            state.cell(0, 2)
        };
        assert_eq!(rendered_line, "[Bash: cargo test]");

        Ok(())
    }

    #[test]
    fn sample_tui_implements_sample_observer_factory() -> Result<(), Box<dyn std::error::Error>> {
        let tui = SampleTui {
            state: Arc::new(Mutex::new(SampleTuiState::new(
                "123".to_owned(),
                1,
                1,
                SampleTuiMode::Disabled,
            ))),
        };
        let config = SsdConfig::new(1, TaskKind::Bugfix)?;
        let prompt = crate::InnerPrompt::new("prompt".to_owned());
        let planner = SamplePlanner::new(config);
        let plan = planner.plan(&prompt);
        let sample = plan
            .samples()
            .first()
            .ok_or_else(|| std::io::Error::other("expected one planned sample"))?;
        let mut observer = tui.observer_for(sample);
        let event = AgentStreamEvent::ToolCall(AgentStreamToolCallEvent::new(
            "Bash".to_owned(),
            "cargo check".to_owned(),
        ));

        observer.observe(&event);

        let rendered_line = {
            let state = tui
                .state
                .lock()
                .map_err(|_error| std::io::Error::other("sample TUI state should lock"))?;
            state.cell(0, 2)
        };
        assert_eq!(rendered_line, "[Bash: cargo check]");

        Ok(())
    }

    #[test]
    fn sample_tui_new_and_finish_are_safe_without_terminal() -> std::io::Result<()> {
        let tui = SampleTui::new(1);
        let mut observer = tui.observer(1);
        let event = AgentStreamEvent::ToolCall(AgentStreamToolCallEvent::new(
            "Bash".to_owned(),
            "cargo check".to_owned(),
        ));

        observer.observe(&event);
        tui.finish()
    }
}
