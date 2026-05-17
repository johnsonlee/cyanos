//! Process-group cleanup for external commands.

use std::{
    process::{Command, Stdio},
    thread,
    time::Duration,
};

const PROCESS_GROUP_TERM_GRACE: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ChildProcessGroup {
    id: u32,
}

impl ChildProcessGroup {
    pub(crate) const fn new(id: u32) -> Self {
        Self { id }
    }

    pub(crate) fn terminate(self) {
        terminate_group(self.id);
    }
}

pub(crate) trait CommandProcessGroupExt {
    fn create_process_group(&mut self);
}

#[cfg(unix)]
impl CommandProcessGroupExt for Command {
    fn create_process_group(&mut self) {
        use std::os::unix::process::CommandExt;

        self.process_group(0);
    }
}

#[cfg(not(unix))]
impl CommandProcessGroupExt for Command {
    fn create_process_group(&mut self) {}
}

#[cfg(unix)]
fn terminate_group(id: u32) {
    let group = format!("-{id}");
    let _ = Command::new("kill")
        .args(["-TERM", "--", group.as_str()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    thread::sleep(PROCESS_GROUP_TERM_GRACE);
    let _ = Command::new("kill")
        .args(["-KILL", "--", group.as_str()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(unix))]
fn terminate_group(_id: u32) {}
