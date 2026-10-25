//! What the helper checks before it does anything (136c): the peer is the
//! host account, every identity is a pool member leased to the session
//! that names it, every path is a workspace under a configured root, and a
//! provider is one the operator configured.

use std::path::{Component, Path, PathBuf};

use crate::leases::Leases;
use crate::pool::{self, Pool};
use crate::protocol::{Op, Refusal, Request};

/// Only the host account may talk to the helper. Root is refused too: no
/// part of the host runs as root, so a root peer is not the host.
pub fn authorize_peer(peer_uid: Option<u32>, host_uid: u32) -> Result<(), Refusal> {
    match peer_uid {
        Some(uid) if uid == host_uid && uid != 0 => Ok(()),
        _ => Err(Refusal::PeerNotHost),
    }
}

pub struct Context<'a> {
    pub pool: &'a Pool,
    /// Canonical workspace roots.
    pub roots: &'a [PathBuf],
    pub providers: &'a [String],
    pub leases: &'a Leases,
    /// Where sessions' homes are: a self-test probe's workspace is its own.
    pub homes: &'a Path,
}

pub const SIGNALS: &[&str] = &["hup", "int", "term", "kill"];

/// A session id as the server mints it: `ags_` and up to 64 alphanumerics.
pub fn session_ok(id: &str) -> bool {
    id.strip_prefix("ags_").is_some_and(|rest| {
        (1..=64).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

pub fn validate(req: &Request, cx: &Context) -> Result<(), Refusal> {
    for field in ["session", "parent", "child"] {
        if let Some(id) = req.fields.get(field)
            && !session_ok(id)
        {
            return Err(Refusal::BadSession);
        }
    }
    if let Some(user) = req.fields.get("user") {
        check_user(user, req.get("session"), cx)?;
    }
    if let Some(provider) = req.fields.get("provider")
        && !cx.providers.iter().any(|p| p == provider)
    {
        return Err(Refusal::BadProvider);
    }
    if let Some(path) = req.fields.get("workspace") {
        if req.fields.get("provider").map(String::as_str) == Some(crate::session::PROBE) {
            // A probe (136j) works in its own home, never in a workspace.
            if Path::new(path) != cx.homes.join(req.get("session")) {
                return Err(Refusal::PathOutsideRoots);
            }
        } else {
            check_workspace(path, cx.roots)?;
        }
    }
    if req.op == Op::Signal && !SIGNALS.contains(&req.get("signal")) {
        return Err(Refusal::BadSignal);
    }
    if req.op == Op::AgentJob
        && (!crate::agents::MANAGEABLE.contains(&req.get("agent"))
            || !crate::agents::ACTIONS.contains(&req.get("action")))
    {
        return Err(Refusal::BadAgent);
    }
    Ok(())
}

fn check_user(name: &str, session: &str, cx: &Context) -> Result<(), Refusal> {
    if name == "root" {
        return Err(Refusal::Root);
    }
    if name == crate::HOST_ACCOUNT {
        return Err(Refusal::HostAccount);
    }
    if pool::user_index(name).is_none() {
        return Err(Refusal::NotPoolUser);
    }
    match cx.pool.users.get(name) {
        None => return Err(Refusal::NotPoolUser),
        Some(0) => return Err(Refusal::Root),
        Some(&uid) if uid == cx.pool.host_uid => return Err(Refusal::HostAccount),
        Some(_) => {}
    }
    match cx.leases.holder(name) {
        Some(holder) if holder == session => Ok(()),
        Some(_) => Err(Refusal::LeasedElsewhere),
        None => Err(Refusal::NotLeased),
    }
}

/// A workspace path is absolute, written in its one normal form, has no
/// symlink anywhere in it, and is a directory directly under a configured
/// root whose name does not start with a dot.
pub fn check_workspace(path: &str, roots: &[PathBuf]) -> Result<PathBuf, Refusal> {
    let p = Path::new(path);
    let normal: PathBuf = p.components().collect();
    if !p.is_absolute()
        || normal.as_os_str() != p.as_os_str()
        || p.components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(Refusal::BadValue("workspace"));
    }
    let (Some(parent), Some(name)) = (p.parent(), p.file_name()) else {
        return Err(Refusal::PathOutsideRoots);
    };
    if name.as_encoded_bytes().first() == Some(&b'.') || !roots.iter().any(|r| r == parent) {
        return Err(Refusal::PathOutsideRoots);
    }
    match p.canonicalize() {
        Ok(real) if real == p && real.is_dir() => Ok(real),
        _ => Err(Refusal::PathOutsideRoots),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::parse;

    fn pool() -> Pool {
        Pool::resolve(&crate::pool::tests::fixed(), "storm-runtime").unwrap()
    }

    #[test]
    fn only_the_host_account_is_answered() {
        assert_eq!(authorize_peer(Some(900), 900), Ok(()));
        for peer in [None, Some(0), Some(901), Some(2001)] {
            assert_eq!(
                authorize_peer(peer, 900),
                Err(Refusal::PeerNotHost),
                "{peer:?}"
            );
        }
        assert_eq!(
            authorize_peer(Some(0), 0),
            Err(Refusal::PeerNotHost),
            "never root"
        );
    }

    #[test]
    fn session_ids_are_the_servers_shape() {
        assert!(session_ok("ags_01HXYZ"));
        for bad in [
            "",
            "ags_",
            "ags_a-b",
            "ags_../x",
            "AGS_a",
            "ags_a b",
            &format!("ags_{}", "a".repeat(65)),
        ] {
            assert!(!session_ok(bad), "{bad}");
        }
    }

    #[test]
    fn identities_are_pool_members_leased_to_the_session() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("w")).unwrap();
        let pool = pool();
        let mut leases = Leases::new(pool.users.keys().cloned());
        assert_eq!(leases.lease("ags_A").as_deref(), Some("storm-s001"));
        let providers = vec!["shell".to_string()];
        let roots = [root.clone()];
        let cx = Context {
            pool: &pool,
            roots: &roots,
            providers: &providers,
            leases: &leases,
            homes: Path::new("/var/lib/storm-runtime-isolation/homes"),
        };
        let req = |session: &str, user: &str| {
            parse(format!(r#"{{"op":"prepare","session":"{session}","user":"{user}"}}"#).as_bytes())
                .unwrap()
        };
        assert_eq!(validate(&req("ags_A", "storm-s001"), &cx), Ok(()));
        assert_eq!(
            validate(&req("ags_B", "storm-s001"), &cx),
            Err(Refusal::LeasedElsewhere)
        );
        assert_eq!(
            validate(&req("ags_A", "storm-s002"), &cx),
            Err(Refusal::NotLeased)
        );
        assert_eq!(validate(&req("ags_A", "root"), &cx), Err(Refusal::Root));
        assert_eq!(
            validate(&req("ags_A", "storm-runtime"), &cx),
            Err(Refusal::HostAccount)
        );
        for unpooled in [
            "storm-s065",
            "storm-s000",
            "daemon",
            "nobody",
            "storm-w001",
            "0",
        ] {
            assert_eq!(
                validate(&req("ags_A", unpooled), &cx),
                Err(Refusal::NotPoolUser),
                "{unpooled}"
            );
        }
        assert_eq!(
            validate(&req("ags_../A", "storm-s001"), &cx),
            Err(Refusal::BadSession)
        );
    }

    #[test]
    fn a_pool_name_resolving_to_root_or_the_host_is_refused() {
        let mut a = crate::pool::tests::fixed();
        a.add_user("storm-s005", 2005, 2005, "/usr/sbin/nologin");
        let mut pool = Pool::resolve(&a, "storm-runtime").unwrap();
        pool.users.insert("storm-s005".into(), 0);
        pool.users.insert("storm-s006".into(), 900);
        let leases = Leases::default();
        let cx = Context {
            pool: &pool,
            roots: &[],
            providers: &[],
            leases: &leases,
            homes: Path::new("/var/lib/storm-runtime-isolation/homes"),
        };
        let req = |user: &str| {
            parse(format!(r#"{{"op":"release","session":"ags_A","user":"{user}"}}"#).as_bytes())
                .unwrap()
        };
        assert_eq!(validate(&req("storm-s005"), &cx), Err(Refusal::Root));
        assert_eq!(validate(&req("storm-s006"), &cx), Err(Refusal::HostAccount));
    }

    #[test]
    fn workspaces_are_directly_under_a_root_with_no_symlink() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().canonicalize().unwrap();
        let root = base.join("ws");
        std::fs::create_dir_all(root.join("app/sub")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        std::fs::write(root.join("file"), b"").unwrap();
        std::os::unix::fs::symlink(base.join("outside"), root.join("link")).unwrap();
        std::os::unix::fs::symlink(&root, base.join("ws-link")).unwrap();
        let roots = [root.clone()];
        let s = |p: &Path| p.to_str().unwrap().to_string();
        assert_eq!(
            check_workspace(&s(&root.join("app")), &roots),
            Ok(root.join("app"))
        );
        for (path, want) in [
            (s(&root), Refusal::PathOutsideRoots),
            (s(&root.join("app/sub")), Refusal::PathOutsideRoots),
            (s(&root.join(".hidden")), Refusal::PathOutsideRoots),
            (s(&root.join("file")), Refusal::PathOutsideRoots),
            (s(&root.join("link")), Refusal::PathOutsideRoots),
            (s(&root.join("missing")), Refusal::PathOutsideRoots),
            (s(&base.join("ws-link/app")), Refusal::PathOutsideRoots),
            (s(&base.join("outside")), Refusal::PathOutsideRoots),
            ("/".into(), Refusal::PathOutsideRoots),
            (
                format!("{}/../ws/app", s(&root)),
                Refusal::BadValue("workspace"),
            ),
            (
                format!("{}/./app", s(&root)),
                Refusal::BadValue("workspace"),
            ),
            (format!("{}//app", s(&root)), Refusal::BadValue("workspace")),
            (format!("{}/app/", s(&root)), Refusal::BadValue("workspace")),
            ("ws/app".into(), Refusal::BadValue("workspace")),
        ] {
            assert_eq!(check_workspace(&path, &roots), Err(want), "{path}");
        }
    }

    #[test]
    fn providers_and_signals_are_from_fixed_sets() {
        let pool = pool();
        let mut leases = Leases::new(pool.users.keys().cloned());
        leases.lease("ags_A");
        let providers = vec!["shell".to_string()];
        let cx = Context {
            pool: &pool,
            roots: &[],
            providers: &providers,
            leases: &leases,
            homes: Path::new("/var/lib/storm-runtime-isolation/homes"),
        };
        let start = parse(br#"{"op":"start","session":"ags_A","user":"storm-s001","provider":"/bin/sh","workspace":"/x"}"#).unwrap();
        assert_eq!(validate(&start, &cx), Err(Refusal::BadProvider));
        let sig = |s: &str| {
            parse(format!(r#"{{"op":"signal","session":"ags_A","signal":"{s}"}}"#).as_bytes())
                .unwrap()
        };
        assert_eq!(validate(&sig("term"), &cx), Ok(()));
        for bad in ["stop", "9", "SIGKILL", "usr1"] {
            assert_eq!(validate(&sig(bad), &cx), Err(Refusal::BadSignal), "{bad}");
        }
    }
}
