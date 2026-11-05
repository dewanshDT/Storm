//! The password and group databases, behind a trait so the pool's checks run
//! on injected values in tests.

use std::collections::HashMap;
use std::ffi::{CStr, CString};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub shell: String,
}

pub trait Accounts {
    fn user(&self, name: &str) -> Option<User>;
    fn group(&self, name: &str) -> Option<u32>;
    /// Every group `user` is in beyond its primary one, by name.
    fn supplementary_groups(&self, user: &User) -> Vec<String>;
}

/// The system's databases. The release build is static musl, which reads
/// `/etc/passwd` and `/etc/group` directly; the pools are local system
/// accounts, so that is the database that matters.
pub struct System;

impl Accounts for System {
    fn user(&self, name: &str) -> Option<User> {
        let cname = CString::new(name).ok()?;
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut buf = vec![0 as libc::c_char; 16 * 1024];
        let mut out: *mut libc::passwd = std::ptr::null_mut();
        let rc = unsafe {
            libc::getpwnam_r(
                cname.as_ptr(),
                &mut pwd,
                buf.as_mut_ptr(),
                buf.len(),
                &mut out,
            )
        };
        if rc != 0 || out.is_null() {
            return None;
        }
        let shell = if pwd.pw_shell.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(pwd.pw_shell) }
                .to_string_lossy()
                .into_owned()
        };
        Some(User {
            name: name.to_string(),
            uid: pwd.pw_uid,
            gid: pwd.pw_gid,
            shell,
        })
    }

    fn group(&self, name: &str) -> Option<u32> {
        let cname = CString::new(name).ok()?;
        let mut grp: libc::group = unsafe { std::mem::zeroed() };
        let mut buf = vec![0 as libc::c_char; 64 * 1024];
        let mut out: *mut libc::group = std::ptr::null_mut();
        let rc = unsafe {
            libc::getgrnam_r(
                cname.as_ptr(),
                &mut grp,
                buf.as_mut_ptr(),
                buf.len(),
                &mut out,
            )
        };
        if rc != 0 || out.is_null() {
            return None;
        }
        Some(grp.gr_gid)
    }

    fn supplementary_groups(&self, user: &User) -> Vec<String> {
        // /etc/group's member lists are where a supplementary membership is
        // declared; reading them avoids getgrouplist's NSS differences.
        let Ok(text) = std::fs::read_to_string("/etc/group") else {
            return vec!["<unreadable /etc/group>".into()];
        };
        group_memberships(&text, &user.name)
    }
}

/// The groups whose member list in an `/etc/group` text names `user`.
pub fn group_memberships(text: &str, user: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let members = fields.nth(2)?;
            members
                .split(',')
                .any(|m| m.trim() == user)
                .then(|| name.to_string())
        })
        .collect()
}

/// Accounts from tables, for tests.
#[derive(Default)]
pub struct Fixed {
    pub users: HashMap<String, User>,
    pub groups: HashMap<String, u32>,
    pub memberships: HashMap<String, Vec<String>>,
}

impl Fixed {
    pub fn add_user(&mut self, name: &str, uid: u32, gid: u32, shell: &str) {
        self.users.insert(
            name.into(),
            User {
                name: name.into(),
                uid,
                gid,
                shell: shell.into(),
            },
        );
    }
}

impl Accounts for Fixed {
    fn user(&self, name: &str) -> Option<User> {
        self.users.get(name).cloned()
    }
    fn group(&self, name: &str) -> Option<u32> {
        self.groups.get(name).copied()
    }
    fn supplementary_groups(&self, user: &User) -> Vec<String> {
        self.memberships
            .get(&user.name)
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memberships_are_read_from_the_member_list_only() {
        let text = "root:x:0:\nstorm-w001:x:990:storm-s003\nadm:x:4:syslog,storm-s003 \nstorm-s003:x:991:\n";
        assert_eq!(group_memberships(text, "storm-s003"), ["storm-w001", "adm"]);
        assert!(group_memberships(text, "storm-s00").is_empty());
    }

    #[test]
    fn the_system_database_finds_root() {
        let root = System.user("root").expect("root");
        assert_eq!(root.uid, 0);
        assert_eq!(System.group("root"), Some(0));
        assert!(System.user("storm-no-such-user").is_none());
    }
}
