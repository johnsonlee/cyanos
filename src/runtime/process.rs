//! Timed process execution utilities.

use std::{
    ffi::{OsStr, OsString},
    io::{self, Read},
    path::Path,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use crate::runtime::process_group::{ChildProcessGroup, CommandProcessGroupExt};

const COMMAND_POLL_INTERVAL_MS: u64 = 50;
const COMMAND_TIMEOUT_ENV: &str = "CYANOS_COMMAND_TIMEOUT_MS";
const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 300_000;

type PipeReader = thread::JoinHandle<io::Result<Vec<u8>>>;

/// Runs a command and captures stdout/stderr with a bounded timeout.
///
/// # Errors
///
/// Returns an I/O error when the process cannot be spawned, cannot be polled,
/// cannot be read, or exceeds the configured timeout.
pub fn output(program: &str, cwd: &Path, args: &[&str]) -> io::Result<Output> {
    output_args(program, cwd, args.iter().copied())
}

/// Runs a command with owned string arguments and captures stdout/stderr.
///
/// # Errors
///
/// Returns an I/O error when the process cannot be spawned, cannot be polled,
/// cannot be read, or exceeds the configured timeout.
pub fn output_strings(program: &str, cwd: &Path, args: &[String]) -> io::Result<Output> {
    output_args(program, cwd, args.iter())
}

fn command_timeout() -> Duration {
    std::env::var(COMMAND_TIMEOUT_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map_or_else(
            || Duration::from_millis(DEFAULT_COMMAND_TIMEOUT_MS),
            Duration::from_millis,
        )
}

fn output_args<I, S>(program: &str, cwd: &Path, args: I) -> io::Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    output_args_with_timeout(program, cwd, args, command_timeout())
}

fn output_args_with_timeout<I, S>(
    program: &str,
    cwd: &Path,
    args: I,
    timeout: Duration,
) -> io::Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect::<Vec<OsString>>();
    let started = Instant::now();
    let mut command = Command::new(program);
    command
        .current_dir(cwd)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.create_process_group();
    let mut child = command.spawn()?;
    let process_group = ChildProcessGroup::new(child.id());
    let stdout_reader = child.stdout.take().map(spawn_pipe_reader);
    let stderr_reader = child.stderr.take().map(spawn_pipe_reader);

    loop {
        if let Some(status) = child.try_wait()? {
            process_group.terminate();
            let stdout = collect_pipe(stdout_reader, "stdout")?;
            let stderr = collect_pipe(stderr_reader, "stderr")?;
            return Ok(Output {
                status,
                stdout,
                stderr,
            });
        }

        if started.elapsed() >= timeout {
            process_group.terminate();
            let _ = child.kill();
            let _ = child.wait();
            let args_label = args
                .iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{} {} timed out after {}ms in {}",
                    program,
                    args_label,
                    timeout.as_millis(),
                    cwd.display()
                ),
            ));
        }

        thread::sleep(Duration::from_millis(COMMAND_POLL_INTERVAL_MS));
    }
}

fn spawn_pipe_reader<R>(mut pipe: R) -> PipeReader
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut output = Vec::new();
        pipe.read_to_end(&mut output)?;
        Ok(output)
    })
}

fn collect_pipe(handle: Option<PipeReader>, stream: &'static str) -> io::Result<Vec<u8>> {
    match handle {
        Some(handle) => handle
            .join()
            .map_err(|_panic| io::Error::other(format!("{stream} reader thread panicked")))?,
        None => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use std::{io, path::Path, thread, time::Duration};

    use super::{collect_pipe, output, output_args_with_timeout, output_strings};

    #[test]
    #[expect(clippy::expect_used, reason = "test fixture command must succeed")]
    fn captures_stdout_and_exit_status() {
        let output = output("sh", Path::new("."), &["-c", "printf ready"]).expect("capture stdout");

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), "ready");
        assert!(output.stderr.is_empty());
    }

    #[test]
    #[expect(clippy::expect_used, reason = "test fixture command must succeed")]
    fn captures_owned_string_arguments() {
        let output = output_strings(
            "sh",
            Path::new("."),
            &["-c".to_owned(), "printf owned".to_owned()],
        )
        .expect("capture owned stdout");

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), "owned");
    }

    #[test]
    #[expect(clippy::expect_used, reason = "test fixture command must succeed")]
    fn drains_large_stdout_before_child_exits() {
        let output = output_args_with_timeout(
            "sh",
            Path::new("."),
            ["-c", "yes x | head -c 200000"],
            std::time::Duration::from_secs(2),
        )
        .expect("capture large stdout");

        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 200_000);
    }

    #[test]
    #[expect(clippy::expect_used, reason = "test fixture command must succeed")]
    fn drains_large_stderr_before_child_exits() {
        let output = output_args_with_timeout(
            "sh",
            Path::new("."),
            ["-c", "yes x | head -c 200000 >&2"],
            std::time::Duration::from_secs(2),
        )
        .expect("capture large stderr");

        assert!(output.status.success());
        assert_eq!(output.stderr.len(), 200_000);
    }

    #[test]
    #[expect(clippy::expect_used, reason = "test fixture command must fail")]
    fn reports_spawn_errors() {
        let error = output("definitely-not-a-cyanos-command", Path::new("."), &[])
            .expect_err("missing command should fail");

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    #[expect(clippy::expect_used, reason = "test fixture command must fail")]
    fn times_out_and_kills_child_process() {
        let error = output_args_with_timeout(
            "sh",
            Path::new("."),
            ["-c", "sleep 1"],
            std::time::Duration::from_millis(1),
        )
        .expect_err("sleep should exceed the tiny timeout");

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("timed out"));
    }

    #[test]
    #[expect(clippy::expect_used, reason = "test fixture command must time out")]
    fn timeout_kills_descendant_holding_stdout() {
        let root =
            std::env::temp_dir().join(format!("cyanos-process-group-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create process group fixture");
        let marker = root.join("descendant-survived");
        let script = format!(
            "(sleep 1; touch '{}') & printf started; sleep 10",
            marker.display()
        );

        let error = output_args_with_timeout(
            "sh",
            &root,
            ["-c", script.as_str()],
            Duration::from_millis(1),
        )
        .expect_err("command should time out");

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        thread::sleep(Duration::from_millis(1200));
        assert!(!marker.exists());
        std::fs::remove_dir_all(root).expect("remove process group fixture");
    }

    #[test]
    #[expect(clippy::expect_used, reason = "test fixture command must succeed")]
    fn empty_pipe_collection_returns_empty_output() {
        assert!(
            collect_pipe(None, "stdout")
                .expect("collect empty pipe")
                .is_empty()
        );
    }
}
