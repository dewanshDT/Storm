//! Which session holds which pool user (136b). The helper never leases a
//! user that is already leased, and returns users least recently used
//! first. The lease and release operations themselves are 136-2; requests
//! are checked against this table already.

use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Default)]
pub struct Leases {
    /// user → session.
    held: BTreeMap<String, String>,
    /// Free users, least recently used first.
    free: VecDeque<String>,
}

impl Leases {
    pub fn new(users: impl IntoIterator<Item = String>) -> Self {
        Self {
            held: BTreeMap::new(),
            free: users.into_iter().collect(),
        }
    }

    pub fn holder(&self, user: &str) -> Option<&str> {
        self.held.get(user).map(String::as_str)
    }

    pub fn user_of(&self, session: &str) -> Option<&str> {
        self.held
            .iter()
            .find(|(_, s)| *s == session)
            .map(|(u, _)| u.as_str())
    }

    /// The least recently used free user, now held by `session`. A session
    /// holds at most one.
    pub fn lease(&mut self, session: &str) -> Option<String> {
        if self.user_of(session).is_some() {
            return None;
        }
        let user = self.free.pop_front()?;
        self.held.insert(user.clone(), session.to_string());
        Some(user)
    }

    /// Only once the user's processes are gone and its files handed back
    /// (136b); the caller guarantees that.
    pub fn release(&mut self, session: &str, user: &str) -> bool {
        if self.holder(user) != Some(session) {
            return false;
        }
        self.held.remove(user);
        self.free.push_back(user.to_string());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Leases {
        Leases::new(["storm-s001", "storm-s002", "storm-s003"].map(String::from))
    }

    #[test]
    fn a_leased_user_is_never_leased_again_and_returns_last() {
        let mut l = table();
        assert_eq!(l.lease("ags_A").as_deref(), Some("storm-s001"));
        assert_eq!(l.lease("ags_B").as_deref(), Some("storm-s002"));
        assert_eq!(l.lease("ags_A"), None, "one user per session");
        assert!(l.release("ags_A", "storm-s001"));
        assert_eq!(
            l.lease("ags_C").as_deref(),
            Some("storm-s003"),
            "least recently used first"
        );
        assert_eq!(l.lease("ags_D").as_deref(), Some("storm-s001"));
        assert_eq!(l.lease("ags_E"), None, "the pool is exhausted");
    }

    #[test]
    fn only_the_holder_releases() {
        let mut l = table();
        l.lease("ags_A");
        assert!(!l.release("ags_B", "storm-s001"));
        assert!(!l.release("ags_A", "storm-s002"));
        assert_eq!(l.holder("storm-s001"), Some("ags_A"));
    }
}
