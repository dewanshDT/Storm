//! `storm-serverw`: `storm-server` built for the Windows GUI subsystem, as
//! `pythonw` is to `python` (PLAN 99, W-S1). A console program started at
//! logon by Task Scheduler opens a console window; this one opens nothing,
//! and logs to the file `serve --log-file` names. Elsewhere it is the same
//! program as `storm-server`.
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    storm_server::main()
}
