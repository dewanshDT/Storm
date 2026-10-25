//! `storm-runtime-isolation` — the root side of per-session isolation on a
//! Linux Runtime Host (decision 136; the vault's *Session Isolation —
//! decision 136* is the spec).
//!
//! Two programs in one binary, both root:
//!
//! - `serve`: the helper service (136c). It listens only on the socket
//!   systemd hands it, answers only the host account (peer credentials), and
//!   accepts only its fixed set of operations, each validated against its own
//!   configuration: identities against the pool, paths against the
//!   configured roots, programs never from a request. Anything else is
//!   refused and audited.
//! - `on`, `off`, `status`: the operator's command (136a). Turning isolation
//!   on declares the identity pools, converts the workspaces to a group each
//!   (136h) and enables the helper; turning it off restores the layout it
//!   recorded.
//!
//! Nothing here runs on a host whose `isolation` is `"off"`, the default:
//! the units ship disabled and the pools are created only by `on`.

pub mod accounts;
pub mod audit;
pub mod config;
pub mod convert;
pub mod leases;
pub mod operator;
pub mod policy;
pub mod pool;
pub mod protocol;
pub mod serve;
pub mod toml_edit;

/// The Runtime Host's own account (decision 77e), the only peer the helper
/// answers.
pub const HOST_ACCOUNT: &str = "storm-runtime";

/// The operator's configuration, shared with the host.
pub const RUNTIME_CONFIG: &str = "/etc/storm-runtime/runtime.toml";

/// The helper's own state: the workspace manifests `off` restores from.
pub const STATE_DIR: &str = "/var/lib/storm-runtime-isolation";

/// The pool declaration the package installs and `on` applies.
pub const SYSUSERS_FILE: &str = "/usr/share/storm-runtime-isolation/sysusers.conf";

/// The units the package ships, disabled.
pub const SOCKET_UNIT: &str = "storm-runtime-isolation.socket";
pub const SERVICE_UNIT: &str = "storm-runtime-isolation.service";
pub const HOST_UNIT: &str = "storm-runtime.service";
