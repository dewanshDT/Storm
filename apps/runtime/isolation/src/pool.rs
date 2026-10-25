//! The identity pools (136b): a fixed set of system users that sessions
//! lease, and of groups that workspaces get, declared at install and never
//! created at run time.

use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, bail};

use crate::accounts::Accounts;

/// How many session users and workspace groups there are (136b's default).
pub const POOL_SIZE: u32 = 64;

/// The install identity (136g): it owns the agents Storm installs, so no
/// session and not the host account can write them.
pub const INSTALL_ACCOUNT: &str = "storm-install";

const USER_PREFIX: &str = "storm-s";
const GROUP_PREFIX: &str = "storm-w";
pub const NOLOGIN: &[&str] = &[
    "/usr/sbin/nologin",
    "/sbin/nologin",
    "/bin/false",
    "/usr/bin/false",
];

pub fn user_name(n: u32) -> String {
    format!("{USER_PREFIX}{n:03}")
}

pub fn group_name(n: u32) -> String {
    format!("{GROUP_PREFIX}{n:03}")
}

fn member(name: &str, prefix: &str) -> Option<u32> {
    let digits = name.strip_prefix(prefix)?;
    if digits.len() != 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = digits.parse().ok()?;
    (1..=POOL_SIZE).contains(&n).then_some(n)
}

/// The pool index of a session user's name, or `None` for anything else.
pub fn user_index(name: &str) -> Option<u32> {
    member(name, USER_PREFIX)
}

/// The pool index of a workspace group's name, or `None`.
pub fn group_index(name: &str) -> Option<u32> {
    member(name, GROUP_PREFIX)
}

/// The `sysusers.d` declaration the package ships. Each session user gets a
/// primary group of its own name; workspace groups have no members, since a
/// session gets its workspace's group from its unit, never from the
/// database.
pub fn sysusers_conf() -> String {
    let mut out = String::from(
        "# Storm Runtime Host isolation pools (decision 136b). Applied by\n\
         # `storm-runtime-isolation on`, never at boot: the package installs this\n\
         # file outside sysusers.d, so a host with isolation off has none of them.\n",
    );
    for n in 1..=POOL_SIZE {
        out.push_str(&format!(
            "u {} - \"Storm isolated session\" /nonexistent /usr/sbin/nologin\n",
            user_name(n)
        ));
    }
    for n in 1..=POOL_SIZE {
        out.push_str(&format!("g {} -\n", group_name(n)));
    }
    out.push_str(&format!(
        "u {INSTALL_ACCOUNT} - \"Storm agent installs\" /nonexistent /usr/sbin/nologin\n"
    ));
    out
}

/// The pools as the system has them, every member checked.
#[derive(Debug, Clone)]
pub struct Pool {
    pub host_uid: u32,
    pub host_gid: u32,
    /// Session user name → uid.
    pub users: BTreeMap<String, u32>,
    /// Workspace group name → gid.
    pub groups: BTreeMap<String, u32>,
    /// Every session user's primary gid.
    pub user_gids: HashSet<u32>,
    /// Session user name → its primary gid.
    pub primary: BTreeMap<String, u32>,
    /// The install identity's uid and gid (136g). Absent on a host turned
    /// on before 136-4 until `on` runs again; the helper refuses to serve
    /// without it, and `off` still restores.
    pub install: Option<(u32, u32)>,
}

impl Pool {
    /// Resolves every member and refuses a pool that is not exactly what
    /// 136b declares: a missing member, root, the host account, a login
    /// shell, a supplementary group, or two members sharing an id.
    pub fn resolve(accounts: &dyn Accounts, host_account: &str) -> Result<Self> {
        let Some(host) = accounts.user(host_account) else {
            bail!("the host account {host_account} does not exist");
        };
        if host.uid == 0 {
            bail!("the host account {host_account} is root");
        }
        let mut users = BTreeMap::new();
        let mut primary = BTreeMap::new();
        let mut user_gids = HashSet::new();
        let mut uids = HashSet::new();
        for n in 1..=POOL_SIZE {
            let name = user_name(n);
            let Some(u) = accounts.user(&name) else {
                bail!("pool user {name} does not exist");
            };
            if u.uid == 0 || u.gid == 0 {
                bail!("pool user {name} is root");
            }
            if u.uid == host.uid || u.gid == host.gid {
                bail!("pool user {name} shares an id with {host_account}");
            }
            if !NOLOGIN.contains(&u.shell.as_str()) {
                bail!("pool user {name} has a login shell ({})", u.shell);
            }
            let extra = accounts.supplementary_groups(&u);
            if !extra.is_empty() {
                bail!("pool user {name} is in other groups: {}", extra.join(", "));
            }
            if !uids.insert(u.uid) || !user_gids.insert(u.gid) {
                bail!("pool user {name} shares an id with another pool user");
            }
            primary.insert(name.clone(), u.gid);
            users.insert(name, u.uid);
        }
        let mut groups = BTreeMap::new();
        let mut gids = HashSet::new();
        for n in 1..=POOL_SIZE {
            let name = group_name(n);
            let Some(gid) = accounts.group(&name) else {
                bail!("pool group {name} does not exist");
            };
            if gid == 0 || gid == host.gid || user_gids.contains(&gid) || !gids.insert(gid) {
                bail!("pool group {name} shares an id with another account");
            }
            groups.insert(name, gid);
        }
        if !accounts.supplementary_groups(&host).is_empty() {
            bail!("the host account {host_account} must be in no other group (decision 77e)");
        }
        let install = match accounts.user(INSTALL_ACCOUNT) {
            None => None,
            Some(u) => {
                if u.uid == 0 || u.gid == 0 {
                    bail!("{INSTALL_ACCOUNT} is root");
                }
                if u.uid == host.uid
                    || u.gid == host.gid
                    || uids.contains(&u.uid)
                    || user_gids.contains(&u.gid)
                    || gids.contains(&u.gid)
                {
                    bail!("{INSTALL_ACCOUNT} shares an id with another account");
                }
                if !NOLOGIN.contains(&u.shell.as_str()) {
                    bail!("{INSTALL_ACCOUNT} has a login shell ({})", u.shell);
                }
                let extra = accounts.supplementary_groups(&u);
                if !extra.is_empty() {
                    bail!("{INSTALL_ACCOUNT} is in other groups: {}", extra.join(", "));
                }
                Some((u.uid, u.gid))
            }
        };
        Ok(Self {
            host_uid: host.uid,
            host_gid: host.gid,
            users,
            groups,
            user_gids,
            primary,
            install,
        })
    }

    pub fn is_pool_uid(&self, uid: u32) -> bool {
        self.users.values().any(|&u| u == uid)
    }

    pub fn is_pool_gid(&self, gid: u32) -> bool {
        self.user_gids.contains(&gid) || self.groups.values().any(|&g| g == gid)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::accounts::Fixed;

    /// A complete, valid pool: the host is 900, users 2001…, groups 3001….
    pub fn fixed() -> Fixed {
        let mut a = Fixed::default();
        a.add_user("storm-runtime", 900, 900, "/usr/sbin/nologin");
        for n in 1..=POOL_SIZE {
            a.add_user(&user_name(n), 2000 + n, 2000 + n, "/usr/sbin/nologin");
            a.groups.insert(group_name(n), 3000 + n);
        }
        a.add_user(INSTALL_ACCOUNT, 4000, 4000, "/usr/sbin/nologin");
        a
    }

    #[test]
    fn names_round_trip_and_nothing_else_is_a_member() {
        assert_eq!(user_name(1), "storm-s001");
        assert_eq!(group_name(64), "storm-w064");
        assert_eq!(user_index("storm-s064"), Some(64));
        for bad in [
            "storm-s000",
            "storm-s065",
            "storm-s1",
            "storm-s0001",
            "root",
            "storm-runtime",
            "storm-w001",
            "storm-s00a",
            "STORM-S001",
            " storm-s001",
            "storm-s+01",
        ] {
            assert_eq!(user_index(bad), None, "{bad}");
        }
        assert_eq!(group_index("storm-w001"), Some(1));
        assert_eq!(group_index("storm-s001"), None);
    }

    #[test]
    fn the_declaration_has_every_member_once() {
        let conf = sysusers_conf();
        let lines: Vec<_> = conf.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(lines.len(), 2 * POOL_SIZE as usize + 1);
        assert!(lines.iter().all(|l| l.starts_with("u storm-s")
            || l.starts_with("g storm-w")
            || l.starts_with("u storm-install ")));
        assert!(
            lines
                .iter()
                .filter(|l| l.starts_with('u'))
                .all(|l| l.ends_with("/usr/sbin/nologin"))
        );
    }

    #[test]
    fn the_package_ships_this_declaration() {
        let shipped = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../deploy/storm-runtime-isolation.sysusers");
        assert_eq!(std::fs::read_to_string(shipped).unwrap(), sysusers_conf());
    }

    #[test]
    fn a_complete_pool_resolves() {
        let pool = Pool::resolve(&fixed(), "storm-runtime").unwrap();
        assert_eq!(pool.users.len(), 64);
        assert_eq!(pool.groups["storm-w002"], 3002);
        assert!(pool.is_pool_uid(2010) && !pool.is_pool_uid(900));
        assert!(pool.is_pool_gid(3064) && pool.is_pool_gid(2001) && !pool.is_pool_gid(900));
        assert_eq!(pool.install, Some((4000, 4000)));
        assert!(
            !pool.is_pool_uid(4000),
            "the install identity is never leased"
        );
        let mut before = fixed();
        before.users.remove(INSTALL_ACCOUNT);
        assert_eq!(
            Pool::resolve(&before, "storm-runtime").unwrap().install,
            None
        );
    }

    #[test]
    fn a_pool_that_is_not_what_136b_declares_is_refused() {
        type Break = fn(&mut Fixed);
        let cases: &[(&str, Break)] = &[
            ("missing user", |a| {
                a.users.remove("storm-s007");
            }),
            ("root user", |a| {
                a.add_user("storm-s002", 0, 2002, "/usr/sbin/nologin")
            }),
            ("host uid", |a| {
                a.add_user("storm-s002", 900, 2002, "/usr/sbin/nologin")
            }),
            ("login shell", |a| {
                a.add_user("storm-s002", 2002, 2002, "/bin/bash")
            }),
            ("duplicate uid", |a| {
                a.add_user("storm-s003", 2002, 2003, "/usr/sbin/nologin")
            }),
            ("extra group", |a| {
                a.memberships
                    .insert("storm-s004".into(), vec!["docker".into()]);
            }),
            ("missing group", |a| {
                a.groups.remove("storm-w010");
            }),
            ("root group", |a| {
                a.groups.insert("storm-w010".into(), 0);
            }),
            ("group is a user's", |a| {
                a.groups.insert("storm-w010".into(), 2005);
            }),
            ("host in a group", |a| {
                a.memberships
                    .insert("storm-runtime".into(), vec!["storm-w001".into()]);
            }),
            ("host missing", |a| {
                a.users.remove("storm-runtime");
            }),
            ("install is root", |a| {
                a.add_user(INSTALL_ACCOUNT, 0, 4000, "/usr/sbin/nologin")
            }),
            ("install is the host", |a| {
                a.add_user(INSTALL_ACCOUNT, 900, 4000, "/usr/sbin/nologin")
            }),
            ("install is a session user", |a| {
                a.add_user(INSTALL_ACCOUNT, 2003, 4000, "/usr/sbin/nologin")
            }),
            ("install shares a workspace group", |a| {
                a.add_user(INSTALL_ACCOUNT, 4000, 3002, "/usr/sbin/nologin")
            }),
            ("install has a login shell", |a| {
                a.add_user(INSTALL_ACCOUNT, 4000, 4000, "/bin/sh")
            }),
            ("install in a group", |a| {
                a.memberships
                    .insert(INSTALL_ACCOUNT.into(), vec!["storm-runtime".into()]);
            }),
        ];
        for (what, break_it) in cases {
            let mut a = fixed();
            break_it(&mut a);
            assert!(Pool::resolve(&a, "storm-runtime").is_err(), "{what}");
        }
    }
}
