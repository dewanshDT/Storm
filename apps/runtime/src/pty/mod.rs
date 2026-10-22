//! The runtime's carrier for a terminal interaction (decision 77a; AM51 on
//! Windows): a PTY on Linux and macOS, ConPTY on Windows.
//!
//! `cli.rs` decides how a session ends; this module only provides the means,
//! the same on every platform:
//! - [`spawn`] starts the agent on a new terminal, as the leader of
//!   everything it will start: a process group and session on Unix, a Job
//!   Object on Windows (AM52);
//! - [`Input`] writes within a deadline and resizes;
//! - [`Group`] hangs up (SIGHUP; closing the pseudoconsole), kills (SIGKILL
//!   to the session; terminating the job) and says whether anything is left.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as imp;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

pub(crate) use imp::{Child, Group, Input, Output, spawn};

/// What a session's process is.
pub(crate) struct Launch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    /// Set on top of the host's own environment, in order: a later entry
    /// wins.
    pub env: Vec<(OsString, OsString)>,
    /// Taken out after `env`, from the host's environment too.
    pub env_remove: Vec<OsString>,
    /// Set after everything else.
    pub secret_env: Vec<(OsString, crate::credential::SecretOsString)>,
    /// The session id, which marks everything the session starts. Unix
    /// finds what left the session by it; Windows' job holds everything
    /// already (AM52).
    #[cfg_attr(windows, allow(dead_code))]
    pub tag: String,
}

/// A started session's ends.
pub(crate) struct Spawned {
    pub child: Child,
    /// The terminal's output, read until it closes.
    pub output: Output,
    pub input: Input,
    pub group: Group,
}

/// How the agent's own process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Exit {
    pub code: Option<i32>,
    /// The signal that ended it. Never on Windows (AM51).
    pub signal: Option<i32>,
}

/// Resolves a command the way a shell would (AM38, AM53): as given when it
/// is a path, otherwise the first executable of that name on `path`, or on
/// the host's own `PATH` when there is none. Windows also tries each of
/// `PATHEXT`'s extensions.
pub(crate) fn resolve(command: &OsStr, path: Option<&OsStr>) -> Option<PathBuf> {
    imp::resolve(command, path)
}
