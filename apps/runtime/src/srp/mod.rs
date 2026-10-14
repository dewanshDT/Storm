//! An SRP v1 client (`docs/srp-v1.md` §5): reaches a Storm Server through a
//! relay by moving the same HTTP requests a LAN client would send (R13).
//!
//! The relay is untrusted and sees no credential of its own (R12). This
//! module moves frames only: whether whatever answers is the right server is
//! the caller's identity challenge, over the trunk like any other request.

pub mod codec;
mod error;
mod trunk;

pub use error::SrpError;
pub use trunk::{SrpBody, SrpResponse, SrpTrunk, TrunkConfig, plaintext_allowed};
