//! Authentication: who this server is, and later who may use it.
//!
//! Phase 1 of the remote-access work (`PLAN.md` decisions 52 / 52a / 52c). Two
//! slices exist so far:
//!
//! 1. **Server identity** — the database and the keypair, plus the two
//!    unauthenticated endpoints a client needs to pin a server ([`identity`]).
//! 2. **Users** — accounts, roles and Argon2id passwords ([`users`],
//!    [`password`]), reachable only from the operator CLI. There is no HTTP
//!    surface on them yet: creating a user over the network needs device auth
//!    (A8), which arrives with pairing.
//!
//! Sessions, pairing and the three-tier middleware are later slices, and the
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
pub mod password;
pub mod users;

pub use db::{AUTH_DB_FILE, AuthDb};
pub use identity::{IDENTITY_DIR, ServerIdentity};
pub use password::Hasher;
