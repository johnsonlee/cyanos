//! Agent command runner.

use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use super::{
    AgentRuntimeError, AgentStreamObserver, AgentStreamParser, CommandSpec, NoopAgentStreamObserver,
};
use crate::runtime::process_group::{ChildProcessGroup, CommandProcessGroupExt};

const DEFAULT_AGENT_TIMEOUT_SECONDS: u64 = 1800;
const AGENT_TIMEOUT_ENV: &str = "CYANOS_AGENT_TIMEOUT_MS";
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Executes concrete agent CLI commands.
pub trait AgentCommandRunner {
    /// Runs a command in the provided working directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the command cannot be launched or exits
    /// unsuccessfully.
    fn run(&self, cwd: &Path, command: &CommandSpec) -> Result<String, AgentRuntimeError>;

    /// Runs a command and streams parsed events to the provided observer.
    ///
    /// # Errors
    ///
    /// Returns an error when the command cannot be launched, stdout cannot be
    /// read, or the command exits unsuccessfully.
    fn run_with_observer(
        &self,
        cwd: &Path,
        command: &CommandSpec,
        observer: &mut dyn AgentStreamObserver,
    ) -> Result<String, AgentRuntimeError> {
        let _ = observer;
        self.run(cwd, command)
    }
}

impl<T> AgentCommandRunner for &T
where
    T: AgentCommandRunner,
{
    fn run(&self, cwd: &Path, command: &CommandSpec) -> Result<String, AgentRuntimeError> {
        (*self).run(cwd, command)
    }

    fn run_with_observer(
        &self,
        cwd: &Path,
        command: &CommandSpec,
        observer: &mut dyn AgentStreamObserver,
    ) -> Result<String, AgentRuntimeError> {
        (*self).run_with_observer(cwd, command, observer)
    }
}

/// System process runner for agent CLIs.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SystemAgentCommandRunner;

impl AgentCommandRunner for SystemAgentCommandRunner {
    fn run(&self, cwd: &Path, command: &CommandSpec) -> Result<String, AgentRuntimeError> {
        let mut observer = NoopAgentStreamObserver;
        self.run_with_observer(cwd, command, &mut observer)
    }

    fn run_with_observer(
        &self,
        cwd: &Path,
        command: &CommandSpec,
        observer: &mut dyn AgentStreamObserver,
    ) -> Result<String, AgentRuntimeError> {
        run_command_with_timeout(cwd, command, observer, agent_timeout())
    }
}

fn run_command_with_timeout(
    cwd: &Path,
    command: &CommandSpec,
    observer: &mut dyn AgentStreamObserver,
    timeout: Duration,
) -> Result<String, AgentRuntimeError> {
    let mut process = Command::new(command.program());
    process
        .args(command.args())
        .current_dir(cwd)
        .stdin(if command.stdin().is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process.create_process_group();
    let mut child = process.spawn().map_err(|source| {
        AgentRuntimeError::new(format!(
            "failed to launch agent command `{}` in {}: {source}",
            command.display(),
            cwd.display()
        ))
    })?;
    let process_group = ChildProcessGroup::new(child.id());

    let mut stdin_handle = if let Some(input) = command.stdin() {
        let mut stdin = child.stdin.take().ok_or_else(|| {
            AgentRuntimeError::new(format!(
                "failed to capture stdin for agent command `{}` in {}",
                command.display(),
                cwd.display()
            ))
        })?;
        let input = input.to_owned();
        Some(thread::spawn(move || stdin.write_all(input.as_bytes())))
    } else {
        None
    };
    let stdout = child.stdout.take().ok_or_else(|| {
        AgentRuntimeError::new(format!(
            "failed to capture stdout for agent command `{}` in {}",
            command.display(),
            cwd.display()
        ))
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        AgentRuntimeError::new(format!(
            "failed to capture stderr for agent command `{}` in {}",
            command.display(),
            cwd.display()
        ))
    })?;
    let stderr_handle = thread::spawn(move || read_to_string(stderr));
    let (line_sender, line_receiver) = mpsc::channel();
    let stdout_handle = thread::spawn(move || read_lines(stdout, &line_sender));
    let mut output = String::new();
    let deadline = Instant::now() + timeout;

    loop {
        drain_lines(&line_receiver, &mut output, observer);

        if let Some(status) = child.try_wait().map_err(|source| {
            AgentRuntimeError::new(format!(
                "failed to poll agent command `{}` in {}: {source}",
                command.display(),
                cwd.display()
            ))
        })? {
            process_group.terminate();
            drain_lines(&line_receiver, &mut output, observer);
            let stdin_result = join_stdin(stdin_handle.take(), command, cwd);
            join_stdout(stdout_handle, command, cwd)?;
            drain_lines(&line_receiver, &mut output, observer);
            let stderr = join_stderr(stderr_handle, command, cwd)?;
            if status.success() {
                stdin_result?;
                return Ok(output);
            }

            return Err(AgentRuntimeError::new(format!(
                "agent command `{}` failed in {} with exit code {:?}: {}",
                command.display(),
                cwd.display(),
                status.code(),
                stderr.trim()
            )));
        }

        if Instant::now() >= deadline {
            process_group.terminate();
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_stdin(stdin_handle.take(), command, cwd);
            drain_lines(&line_receiver, &mut output, observer);
            drop(stdout_handle);
            drop(stderr_handle);
            return Err(AgentRuntimeError::new(format!(
                "agent command `{}` timed out in {}",
                command.display(),
                cwd.display()
            )));
        }

        thread::sleep(PROCESS_POLL_INTERVAL);
    }
}

fn join_stdin(
    handle: Option<thread::JoinHandle<std::io::Result<()>>>,
    command: &CommandSpec,
    cwd: &Path,
) -> Result<(), AgentRuntimeError> {
    let Some(handle) = handle else {
        return Ok(());
    };
    handle
        .join()
        .map_err(|_panic| {
            AgentRuntimeError::new(format!(
                "stdin writer panicked for agent command `{}` in {}",
                command.display(),
                cwd.display()
            ))
        })?
        .map_err(|source| {
            AgentRuntimeError::new(format!(
                "failed to write stdin for agent command `{}` in {}: {source}",
                command.display(),
                cwd.display()
            ))
        })
}

fn read_to_string(mut input: impl Read) -> std::io::Result<String> {
    let mut output = String::new();
    input.read_to_string(&mut output)?;
    Ok(output)
}

fn read_lines(input: impl Read, sender: &mpsc::Sender<String>) -> std::io::Result<()> {
    let mut reader = BufReader::new(input);
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Ok(());
        }
        if sender.send(line).is_err() {
            return Ok(());
        }
    }
}

fn drain_lines(
    receiver: &mpsc::Receiver<String>,
    output: &mut String,
    observer: &mut dyn AgentStreamObserver,
) {
    while let Ok(line) = receiver.try_recv() {
        output.push_str(&line);
        for event in AgentStreamParser::parse_line(line.trim_end_matches(['\r', '\n'])) {
            observer.observe(&event);
        }
    }
}

fn join_stdout(
    handle: thread::JoinHandle<std::io::Result<()>>,
    command: &CommandSpec,
    cwd: &Path,
) -> Result<(), AgentRuntimeError> {
    handle
        .join()
        .map_err(|_panic| {
            AgentRuntimeError::new(format!(
                "stdout reader panicked for agent command `{}` in {}",
                command.display(),
                cwd.display()
            ))
        })?
        .map_err(|source| {
            AgentRuntimeError::new(format!(
                "failed to read stdout for agent command `{}` in {}: {source}",
                command.display(),
                cwd.display()
            ))
        })
}

fn join_stderr(
    handle: thread::JoinHandle<std::io::Result<String>>,
    command: &CommandSpec,
    cwd: &Path,
) -> Result<String, AgentRuntimeError> {
    handle
        .join()
        .map_err(|_panic| {
            AgentRuntimeError::new(format!(
                "stderr reader panicked for agent command `{}` in {}",
                command.display(),
                cwd.display()
            ))
        })?
        .map_err(|source| {
            AgentRuntimeError::new(format!(
                "failed to read stderr for agent command `{}` in {}: {source}",
                command.display(),
                cwd.display()
            ))
        })
}

fn agent_timeout() -> Duration {
    std::env::var(AGENT_TIMEOUT_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map_or_else(
            || Duration::from_secs(DEFAULT_AGENT_TIMEOUT_SECONDS),
            Duration::from_millis,
        )
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, fs, io, path::Path, sync::mpsc, thread, time::Duration};

    use super::{
        AgentCommandRunner, SystemAgentCommandRunner, join_stderr, join_stdin, join_stdout,
        read_lines, run_command_with_timeout,
    };
    use crate::agent::{AgentRuntimeError, AgentStreamEvent, AgentStreamObserver, CommandSpec};

    #[derive(Debug, Default)]
    struct RecordingObserver {
        events: RefCell<Vec<String>>,
    }

    impl RecordingObserver {
        fn events(&self) -> Vec<String> {
            self.events.borrow().clone()
        }
    }

    impl AgentStreamObserver for RecordingObserver {
        fn observe(&mut self, event: &AgentStreamEvent) {
            self.events.borrow_mut().push(event.to_string());
        }
    }

    #[test]
    fn system_agent_command_runner_reports_process_results()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("cyanos-agent-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let runner = SystemAgentCommandRunner;
        let success = CommandSpec::new("printf", vec!["ok".to_owned()]);
        let failure = CommandSpec::new("false", Vec::new());

        assert_eq!(runner.run(Path::new(&root), &success)?, "ok");
        let result = runner.run(Path::new(&root), &failure);
        assert!(matches!(result, Err(AgentRuntimeError { .. })));

        let runner_ref = &runner;
        assert_eq!(runner_ref.run(Path::new(&root), &success)?, "ok");

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn system_agent_command_runner_streams_tool_events() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("cyanos-agent-stream-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let runner = SystemAgentCommandRunner;
        let runner_ref = &runner;
        let command = CommandSpec::new(
            "sh",
            vec![
                "-c".to_owned(),
                "printf '%s\\n' '{\"type\":\"tool_use\",\"name\":\"Bash\",\"input\":{\"command\":\"cargo check\"}}'".to_owned(),
            ],
        );
        let mut observer = RecordingObserver::default();

        let output = runner_ref.run_with_observer(Path::new(&root), &command, &mut observer)?;

        assert!(output.contains("tool_use"));
        assert_eq!(observer.events(), ["[Bash: cargo check]".to_owned()]);

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn system_agent_command_runner_writes_prompt_to_stdin() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = std::env::temp_dir().join(format!("cyanos-agent-stdin-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let runner = SystemAgentCommandRunner;
        let prompt = "long prompt line\n".repeat(20_000);
        let command = CommandSpec::with_stdin(
            "sh",
            vec![
                "-c".to_owned(),
                "cat > prompt.txt; printf 'done\\n'".to_owned(),
            ],
            prompt.clone(),
        );

        let output = runner.run(Path::new(&root), &command)?;

        assert_eq!(output, "done\n");
        assert_eq!(fs::read_to_string(root.join("prompt.txt"))?, prompt);
        assert!(!command.args().iter().any(|arg| arg.contains(&prompt)));

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn system_agent_command_runner_reports_spawn_errors() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = std::env::temp_dir().join(format!("cyanos-agent-spawn-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let runner = SystemAgentCommandRunner;
        let command = CommandSpec::new("definitely-missing-cyanos-agent", Vec::new());

        let result = runner.run(Path::new(&root), &command);
        let message = result.err().map(|error| error.to_string());

        assert!(
            message
                .as_deref()
                .is_some_and(|message| message.contains("failed to launch agent command"))
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn system_agent_command_runner_reports_stderr_on_failure()
    -> Result<(), Box<dyn std::error::Error>> {
        let root =
            std::env::temp_dir().join(format!("cyanos-agent-failure-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let runner = SystemAgentCommandRunner;
        let command = CommandSpec::new(
            "sh",
            vec![
                "-c".to_owned(),
                "printf 'agent denied\\n' >&2; exit 7".to_owned(),
            ],
        );

        let result = runner.run(Path::new(&root), &command);
        let message = result.err().map(|error| error.to_string());

        assert!(
            message
                .as_deref()
                .is_some_and(|message| message.contains("agent denied"))
        );

        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn system_agent_command_runner_times_out_and_cleans_up()
    -> Result<(), Box<dyn std::error::Error>> {
        let root =
            std::env::temp_dir().join(format!("cyanos-agent-timeout-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let command = CommandSpec::new(
            "sh",
            vec!["-c".to_owned(), "printf 'started\\n'; sleep 1".to_owned()],
        );
        let mut observer = RecordingObserver::default();

        let result =
            run_command_with_timeout(&root, &command, &mut observer, Duration::from_millis(1));

        assert!(
            result
                .err()
                .is_some_and(|error| error.to_string().contains("timed out"))
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn system_agent_command_runner_kills_descendants_on_timeout()
    -> Result<(), Box<dyn std::error::Error>> {
        let root =
            std::env::temp_dir().join(format!("cyanos-agent-process-group-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        let marker = root.join("descendant-survived");
        let command = CommandSpec::new(
            "sh",
            vec![
                "-c".to_owned(),
                format!(
                    "(sleep 1; touch '{}') & printf started; sleep 10",
                    marker.display()
                ),
            ],
        );
        let mut observer = RecordingObserver::default();

        let result =
            run_command_with_timeout(&root, &command, &mut observer, Duration::from_millis(1));

        assert!(
            result
                .err()
                .is_some_and(|error| error.to_string().contains("timed out"))
        );
        thread::sleep(Duration::from_millis(1200));
        assert!(!marker.exists());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    #[expect(clippy::panic, reason = "exercises join panic error mapping")]
    fn reader_join_helpers_report_errors_and_panics() {
        let command = CommandSpec::new("agent", vec!["exec".to_owned()]);
        let cwd = Path::new("/tmp/cyanos-agent");
        let stdout_error = thread::spawn(|| Err(io::Error::other("stdout broken")));
        let stdout_panic = thread::spawn(|| -> io::Result<()> {
            std::panic::panic_any("stdout panic");
        });
        let stderr_error = thread::spawn(|| Err(io::Error::other("stderr broken")));
        let stderr_panic = thread::spawn(|| -> io::Result<String> {
            std::panic::panic_any("stderr panic");
        });
        let stdin_error = Some(thread::spawn(|| Err(io::Error::other("stdin broken"))));
        let stdin_panic = Some(thread::spawn(|| -> io::Result<()> {
            std::panic::panic_any("stdin panic");
        }));

        assert!(
            join_stdin(stdin_error, &command, cwd)
                .err()
                .is_some_and(|error| error.to_string().contains("failed to write stdin"))
        );
        assert!(
            join_stdin(stdin_panic, &command, cwd)
                .err()
                .is_some_and(|error| error.to_string().contains("stdin writer panicked"))
        );
        assert!(
            join_stdout(stdout_error, &command, cwd)
                .err()
                .is_some_and(|error| error.to_string().contains("failed to read stdout"))
        );
        assert!(
            join_stdout(stdout_panic, &command, cwd)
                .err()
                .is_some_and(|error| error.to_string().contains("stdout reader panicked"))
        );
        assert!(
            join_stderr(stderr_error, &command, cwd)
                .err()
                .is_some_and(|error| error.to_string().contains("failed to read stderr"))
        );
        assert!(
            join_stderr(stderr_panic, &command, cwd)
                .err()
                .is_some_and(|error| error.to_string().contains("stderr reader panicked"))
        );
    }

    #[test]
    fn read_lines_stops_when_receiver_is_closed() -> io::Result<()> {
        let (sender, receiver) = mpsc::channel();
        drop(receiver);

        read_lines(&b"line\n"[..], &sender)
    }
}
