//! Authentication: who this server is, and who may use it.
//!
//! Phase 1 of the remote-access work (`PLAN.md` decisions 52 / 52a / 52c).
//! Five slices:
//!
//! 1. **Server identity** — the database and the keypair, plus the two
//!    unauthenticated endpoints a client needs to pin a server ([`identity`]).
//! 2. **Users** — accounts, roles and Argon2id passwords ([`users`],
//!    [`password`]), reachable only from the operator CLI.
//! 3. **Sessions** — `(user, device)`, opaque tokens, login, refresh and
//!    revocation ([`sessions`], [`token`], [`devices`]).
//! 4. **Three-tier middleware** — `api.rs` checks credentials on every
//!    handler; device tier for unauthenticated flows, session tier for
//!    authenticated ones.
//! 5. **Pairing** — QR-based device enrolment ([`pairing`]).
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
pub mod devices;
pub mod identity;
pub mod pairing;
pub mod password;
pub mod sessions;
pub mod token;
pub mod users;

pub use db::{AUTH_DB_FILE, AuthDb};
pub use identity::{IDENTITY_DIR, ServerIdentity};
pub use password::Hasher;
