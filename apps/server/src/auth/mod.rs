//! Authentication: who this server is, and later who may use it.
//!
//! Phase 1 of the remote-access work (`PLAN.md` decisions 52 / 52a). This slice
//! is **server identity only** — the database and the keypair, plus the two
//! unauthenticated endpoints a client needs to pin a server. Users, passwords,
//! sessions, pairing and the three-tier middleware are later slices, and the
//! shared bearer token in `api.rs` is untouched by all of it: nothing here
//! changes what an existing client has to send.
//!
//! Two rules from the design that the code has to keep saying out loud:
//!
//! - **Passwords and tokens will hash differently on purpose.** Argon2id for
//!   low-entropy guessable secrets, blake3 for 256-bit random ones. Running a
//!   memory-hard KDF on every API request would cost ~200 ms to defend against
//!   an attack that does not exist. Neither is a "fix" for the other.
//! - **`auth.db` is not derived.** Everything else in `state/` can be rebuilt
//!   from the markdown; this cannot. See [`db`].

pub mod db;
pub mod identity;

pub use db::{AUTH_DB_FILE, AuthDb};
pub use identity::{IDENTITY_DIR, ServerIdentity};
