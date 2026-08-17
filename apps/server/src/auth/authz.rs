//! The vault access boundary — **the seam, not yet the policy**.
//!
//! Every merged auth slice answers *who are you*. None answered *may you touch
//! this vault*, and the answer was a silent yes for everyone, everywhere. This
//! module is where that question now gets asked.
//!
//! # What this is not
//!
//! **It is not role-based access control.** [`AllowAuthenticated`] — the policy
//! Storm ships — lets any authenticated caller reach any vault, which is the
//! correct behaviour for a single-user self-hosted server and exactly what the
//! server did before this module existed. Roles, `vault_grants`, per-vault
//! access levels and the MCP capability rules are all deliberately deferred:
//! deciding a whole permission model before Storm has real multi-user
//! behaviour to shape it would mean redesigning it later, with 24 call sites
//! already depending on the first guess.
//!
//! What is *not* deferred is the boundary. A handler can no longer reach a
//! vault without saying who is asking and what for, because
//! [`crate::api::vault_of`] will not hand one over without both. When the
//! policy grows up, it replaces [`VaultPolicy`] — and nothing else moves.
//!
//! See *Auth Authorization Review (A9)* in the personal vault for the
//! measurements this shape came from, and Q19–Q25 for what the real policy
//! still has to settle.

use super::users::Role;

/// Who is asking.
///
/// Constructed once per request by the auth middleware, from the credential
/// that got through it. Every variant here is **already authenticated** —
/// there is no `Anonymous`, because a caller with no valid credential never
/// reaches a handler at all.
#[derive(Debug, Clone)]
pub enum Actor {
    /// A logged-in user on a paired device: the ordinary case.
    Session {
        #[allow(dead_code)] // The policy that reads these is the next slice.
        user_id: String,
        #[allow(dead_code)]
        role: Role,
    },
    /// The legacy shared token (A10), which is owner-equivalent and has **no
    /// user behind it** — nothing to look grants up against, and nothing
    /// honest to write into `security_events.user_id`. It disappears with the
    /// token.
    Legacy,
}

// There is deliberately **no `Mcp` variant**. There was, briefly, and it was a
// second identity concept: an MCP call resolved as "some MCP session" while the
// equivalent REST call resolved as a user. Under a real policy that would have
// been the one caller whose grants could not be checked. An MCP request now
// carries the same `Actor` its REST equivalent would — see `mcp::scope_actor`
// for how it crosses rmcp's spawn boundary.

// There is deliberately **no `System` variant**. The file watcher reaches a
// vault without one, and that is correct rather than a gap: it is the server
// reacting to its own filesystem, not a caller asking for something. Giving it
// an `Actor` would model the disk as a principal requesting permission, which
// is the wrong shape and would put a policy decision on a path that must never
// refuse. The single non-boundary lookup is commented where it lives, in
// `watcher::apply`.

impl Actor {
    /// For logs and audit rows. Never a secret.
    pub fn describe(&self) -> &str {
        match self {
            Actor::Session { .. } => "session",
            Actor::Legacy => "legacy-token",
        }
    }
}

/// What the caller intends to do with the vault.
///
/// Carried now even though [`AllowAuthenticated`] ignores it, because adding a
/// parameter later means revisiting every call site to decide what each one
/// meant — and doing that retrospectively, against handlers written without
/// the question in mind, is how a read path quietly gets labelled a write.
/// Deciding it once, where the operation is written, is cheap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// The answer, and why — the reason is for the audit row, never for the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Nothing in the shipping binary constructs this, and that is the honest
    /// state of the slice: [`AllowAuthenticated`] never refuses, so the only
    /// producer today is the `DenyAll` policy in `api.rs`'s tests.
    ///
    /// It is not dead weight. The refusal *path* — 403 rather than 404,
    /// consulted before the registry so it cannot double as an existence
    /// probe, collections filtering instead — is fully exercised through that
    /// policy, which is the point of making the policy swappable. Were this
    /// variant absent, the RBAC slice would be writing that path for the first
    /// time and running it for the first time in production.
    #[allow(dead_code)]
    Deny(&'static str),
}

impl Decision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Decision::Allow)
    }
}

/// Decides whether an [`Actor`] may reach a vault.
///
/// One trait so the policy is swappable without touching a handler. That is
/// the entire point of the seam: the RBAC slice replaces the implementation
/// below and changes nothing else.
pub trait VaultPolicy: Send + Sync + std::fmt::Debug {
    fn decide(&self, actor: &Actor, vault_id: &str, access: Access) -> Decision;
}

/// **The policy Storm ships today: every authenticated caller, every vault.**
///
/// Not a placeholder that forgot to be finished — it is the right answer for a
/// single-user self-hosted server, and it is what the server already did.
/// Making it explicit is the change: the permissiveness is now a policy object
/// with a name, tested and swappable, rather than the absence of a check.
#[derive(Debug, Clone, Copy)]
pub struct AllowAuthenticated;

impl VaultPolicy for AllowAuthenticated {
    fn decide(&self, _actor: &Actor, _vault_id: &str, _access: Access) -> Decision {
        // Every `Actor` variant is authenticated by construction — the
        // middleware refuses anything else long before here — so there is
        // nothing left to check.
        Decision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Actor {
        Actor::Session {
            user_id: "usr_1".into(),
            role: Role::Member,
        }
    }

    #[test]
    fn the_shipped_policy_allows_every_authenticated_actor() {
        // The current answer, stated so a change to it is a visible diff
        // rather than a behaviour someone notices in production.
        let policy = AllowAuthenticated;
        for actor in [session(), Actor::Legacy] {
            for access in [Access::Read, Access::Write] {
                assert_eq!(
                    policy.decide(&actor, "any-vault", access),
                    Decision::Allow,
                    "{} / {access:?}",
                    actor.describe()
                );
            }
        }
    }

    #[test]
    fn a_role_does_not_change_the_answer_yet() {
        // Guards the boundary between this slice and the next: if someone
        // starts consulting roles here without replacing the policy, the
        // permission model has been decided by accident.
        let policy = AllowAuthenticated;
        for role in [Role::Owner, Role::Admin, Role::Member] {
            let actor = Actor::Session {
                user_id: "usr_1".into(),
                role,
            };
            assert!(policy.decide(&actor, "vault", Access::Write).is_allowed());
        }
    }

    #[test]
    fn an_actor_never_describes_itself_with_a_secret() {
        // `describe()` goes into logs and `security_events`. The legacy actor
        // is the one that could plausibly carry a token if someone extended
        // this carelessly.
        for actor in [session(), Actor::Legacy] {
            let described = actor.describe();
            assert!(!described.contains("testtoken"));
            assert!(
                described
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '-')
            );
        }
    }
}
