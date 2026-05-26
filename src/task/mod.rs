//! Task intake and runtime bootstrap boundary.

mod run;

pub use run::{
    SystemTaskGitRunner, TaskGitRunner, TaskRunError, TaskRunReport, TaskRunRequest, TaskRunner,
};
