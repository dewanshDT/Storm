//! The vault access boundary — **the seam, not yet the policy**.
//!
//! Every merged auth slice answers *who are you*. None answered *may you touch
//! this vault*, and the answer was a silent yes for everyone, everywhere. This
//! module is where that question now gets asked.
//!
//! # What this is not
//!
//! **It is not role-based access control.** Storm is
//! single-user (decision 82). Every caller is the one account, reached through
//! a session, a key or an agent session. [`AllowAuthenticated`] lets each reach
//! any vault. What the seam is *for* now is narrowing what a credential may do
//! on the account's behalf: an agent session writing only to the vault chosen
//! at its launch is the policy this boundary exists to carry.
//!
//! What is *not* deferred is the boundary. A handler can no longer reach a
//! vault without saying who is asking and what for, because
//! [`crate::api::vault_of`] will not hand one over without both. When the
//! policy grows up, it replaces [`VaultPolicy`] — and nothing else moves.
//!
//! *Auth Authorization Review (A9)* in the personal vault is where this shape
//! came from; its role questions (Q19–Q25) were retired by decision 82.

/// Who is asking.
///
/// Constructed once per request by the auth middleware, from the credential
/// that got through it. Every variant here is **already authenticated** —
/// there is no `Anonymous`, because a caller with no valid credential never
/// reaches a handler at all.
#[derive(Debug, Clone)]
pub enum Actor {
    /// The account, signed in on a paired device: the ordinary case.
    Session { user_id: String },
    /// An MCP key (A14): a machine acting **as the user who minted it**.
    ///
    /// Note what this is not. It is not a principal of its own — the identity
    /// is `user_id`, exactly as a session's is, which is why a key needs no
    /// special case in a policy and why the note below about `Actor::Mcp`
    /// still holds. The `key_id` rides along so an audit row can say *which*
    /// key acted and so revoking one key is not revoking the person.
    ///
    /// **A14 gives a key its owner's full authority and no way to narrow it.**
    /// Per-vault scoping is deferred to the authorization release, which has
    /// to answer the same question for users anyway.
    Key {
        /// Which key acted. **For the audit trail, never for a decision** —
        /// see [`Actor::key_id`].
        #[allow(dead_code)]
        key_id: String,
        user_id: String,
    },
    /// An agent in an Agent Runtime session, reaching Storm's vault through
    /// the MCP Gateway's built-in `storm` connection (AM30, AM31; decision
    /// 81e).
    ///
    /// **The principal is the account** (A14.3's rule, as for a key).
    /// `session_id` and `host_id` ride along **for audit and stamping,
    /// never for a decision** — whether the session may make the call at all
    /// was decided in `ops::integration_call` before this actor existed.
    Agent {
        #[allow(dead_code)] // Audit, like `Key::key_id`; read by tests.
        session_id: String,
        #[allow(dead_code)]
        host_id: String,
        user_id: String,
    },
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
            Actor::Key { .. } => "mcp-key",
            Actor::Agent { .. } => "agent",
        }
    }

    /// The account this caller acts as — always the one account.
    ///
    /// **This is the accessor a permission model must read, not the variant.**
    /// A session and a key are the same principal reached two ways, and a
    /// policy that matches on the variant instead has to be revisited every
    /// time a new way to hold a credential is added — which is exactly the
    /// bug slice 11 fixed for MCP and A14 is written to avoid repeating.
    ///
    /// **Always present since the cutover.** While the shared token existed
    /// there was one caller with no user behind it, so this returned an
    /// `Option` every call site had to unwrap. Removing that credential
    /// removed the only `None`, and the type says so now.
    pub fn user_id(&self) -> &str {
        match self {
            Actor::Session { user_id, .. }
            | Actor::Key { user_id, .. }
            | Actor::Agent { user_id, .. } => user_id,
        }
    }

    /// The key behind this caller, if it is one. **For audit, never for
    /// access decisions** — a policy that asks "is this a key?" is the branch
    /// this design exists to make unnecessary.
    ///
    /// Asserted by `an_mcp_key_request_carries_its_owners_identity`; no
    /// shipping caller yet, because the audit work that wants it belongs to a
    /// later release.
    #[allow(dead_code)]
    pub fn key_id(&self) -> Option<&str> {
        match self {
            Actor::Key { key_id, .. } => Some(key_id),
            _ => None,
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
        }
    }

    fn key() -> Actor {
        Actor::Key {
            key_id: "key_1".into(),
            user_id: "usr_1".into(),
        }
    }

    fn agent() -> Actor {
        Actor::Agent {
            session_id: "ags_1".into(),
            host_id: "hst_1".into(),
            user_id: "usr_1".into(),
        }
    }

    #[test]
    fn an_agent_acts_as_the_account() {
        // AM31: the account's identity, read through the accessor a policy
        // uses — never the variant.
        let a = agent();
        assert_eq!(a.user_id(), "usr_1");
        assert_eq!(a.describe(), "agent");
        assert_eq!(a.key_id(), None);
    }

    #[test]
    fn the_shipped_policy_allows_every_authenticated_actor() {
        // The current answer, stated so a change to it is a visible diff
        // rather than a behaviour someone notices in production.
        let policy = AllowAuthenticated;
        for actor in [session(), key(), agent()] {
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
    fn an_actor_never_describes_itself_with_a_secret() {
        // `describe()` goes into logs and `security_events`, so it has to stay
        // a fixed label rather than anything derived from a credential.
        for actor in [session(), key(), agent()] {
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
