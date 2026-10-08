//! `storm-runtime` — a Storm Runtime Host, the execution plane of the Agent
//! Runtime (`PLAN.md` decision 77; the vault's *Agent Runtime/V1
//! Specification Freeze* is the spec).
//!
//! The Storm Server is the control plane. It owns identity, ownership,
//! authorization and the session records. A Runtime Host only runs agents:
//! it never learns who owns a session and holds no user data. If a change would
//! have this crate know what a Storm *user* is, the change is wrong.
//!
//! ## What is here (slice 1)
//!
//! - [`provider`]: the provider contract. A provider has a *kind* (how a session
//!   starts) and offers *interactions* (how a human talks to it). The runtime,
//!   not the provider, owns the carrier of an interaction. For a terminal that
//!   carrier is a PTY, and **no PTY type appears in the contract** (freeze
//!   §9.1, AM20).
//! - [`status`]: the one session status vocabulary (freeze §7.2).
//! - [`scrollback`]: the offset-addressed terminal output ring (freeze §11.2,
//!   §11.5).
//! - [`fake`]: a provider with an in-memory terminal and no PTY. It proves the
//!   contract is not the PTY, and it lets every later layer be tested without
//!   an agent binary (AC-A1).
//!
//! ## Slice 2
//!
//! - [`cli`]: the `cli` providers (`claude-code`, `opencode`, `shell`) and
//!   how their sessions end: the whole process group, never just the agent.
//! - `pty`: the carrier the runtime builds around a `cli` launch description.
//!
//! ## Slice 3
//!
//! - [`identity`]: the host key, `host.json`, and the wire formats pinned by
//!   `docs/runtime-vectors.json`.
//! - [`client`]: enrollment and key authentication against the server, which
//!   is verified against its pinned key before anything is sent.
//!
//! ## Slice 4 (decision 77c)
//!
//! - [`config`]: `runtime.toml`, provider env files, and workspace resolution.
//! - [`host`]: `storm-runtime serve`. It runs the link, one uploader per
//!   session, and the `sessions.json` restart table.
//!
//! ## The MCP Gateway (decision 81f)
//!
//! - [`mcp`]: the per-provider config writers, session handles and the
//!   daemon socket's line format. The host forwards; it never authorizes.
//! - [`bridge`]: `storm-runtime mcp-bridge`, the agent's stdio MCP server.
//!
//! ## macOS (D14, decision 83)
//!
//! - [`platform`]: everything that differs between Linux and macOS — paths,
//!   the service account and service manager, the default `PATH`, and a PTY
//!   write's wait for room. Nothing else matches on `target_os`.

pub mod bridge;
pub mod cli;
pub mod client;
pub mod config;
pub mod fake;
pub mod host;
pub mod identity;
pub mod mcp;
pub mod platform;
pub mod provider;
mod pty;
pub mod scrollback;
pub mod status;
