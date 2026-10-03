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

pub mod fake;
pub mod provider;
pub mod scrollback;
pub mod status;
