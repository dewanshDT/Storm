//! Windows: the Service Control Manager, `NT SERVICE\StormRuntime` and
//! `C:\ProgramData\StormRuntime` (D16, AM48–AM55).
//!
//! Private files are protected DACLs here, as they are modes on Unix: SYSTEM,
//! Administrators and this host's account, nothing inherited (AM49). The
//! account check and the sweep compare SIDs, not names, wherever they can.

use std::ffi::{OsStr, OsString, c_void};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    ConvertStringSidToSidW, GetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
    SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    CheckTokenMembership, DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation,
    GetSecurityDescriptorDacl, GetTokenInformation, LookupAccountNameW, LookupAccountSidW,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
    SID_NAME_USE, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{CreateDirectoryW, GetDriveTypeW};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows_sys::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, TerminateProcess,
};

use super::winsvc::{self, DriveKind, Layout};

pub const NAME: &str = "windows";
pub const DEFAULT_STATE: &str = winsvc::STATE;
pub const DEFAULT_CONFIG: &str = winsvc::CONFIG;
pub const SERVICE_ACCOUNT: &str = winsvc::ACCOUNT;
/// The machine decides: [`refused_root`].
pub const PROTECTED_ROOTS: &[&str] = &[];

pub use super::windows_service::{install, uninstall};

/// `s` as a NUL-terminated wide string.
pub(crate) fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> OsString {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    OsString::from_wide(&buf[..end])
}

/// The account's own bins, then the machine `PATH` as the registry holds it
/// now (AM53), whoever runs the host.
pub fn default_path() -> Option<OsString> {
    let home = std::env::var("USERPROFILE")
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| Layout::standard().home);
    let machine = machine_path()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    Some(winsvc::path_for_home(&home, &machine).into())
}

/// `HKLM\…\Session Manager\Environment\Path`, expanded.
fn machine_path() -> Option<String> {
    let key = wide(r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment");
    let name = wide("Path");
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    let mut size: u32 = 0;
    // SAFETY: a size query; every pointer is a live local or null.
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            null_mut(),
            &mut size,
        )
    };
    if rc != 0 || size == 0 {
        return None;
    }
    let mut raw = vec![0u16; size as usize / 2 + 1];
    // SAFETY: `raw` holds `size` bytes.
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            name.as_ptr(),
            flags,
            null_mut(),
            raw.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if rc != 0 {
        return None;
    }
    let mut out = vec![0u16; 32 * 1024];
    // SAFETY: `raw` is NUL-terminated; `out` holds its stated length.
    let n = unsafe { ExpandEnvironmentStringsW(raw.as_ptr(), out.as_mut_ptr(), out.len() as u32) };
    if n == 0 || n as usize > out.len() {
        return None;
    }
    Some(from_wide(&out).to_string_lossy().into_owned())
}

/// `COMPUTERNAME`, lower-cased (AM56).
pub fn host_name() -> String {
    let name: String = std::env::var("COMPUTERNAME")
        .unwrap_or_default()
        .trim()
        .to_lowercase()
        .chars()
        .take(64)
        .collect();
    if name.is_empty() {
        "runtime-host".into()
    } else {
        name
    }
}

/// AM54: profiles, network shares, and drives that come and go.
pub fn refused_root(root: &Path) -> Option<String> {
    let root = winsvc::normalize(&root.to_string_lossy());
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let users = format!(r"{drive}\Users");
    let kind = match root.as_bytes() {
        [letter, b':', ..] if letter.is_ascii_alphabetic() => {
            let r = wide(format!("{}:\\", *letter as char));
            // SAFETY: `r` is NUL-terminated.
            match unsafe { GetDriveTypeW(r.as_ptr()) } {
                2 => DriveKind::Removable,
                3 => DriveKind::Fixed,
                4 => DriveKind::Remote,
                5 => DriveKind::Optical,
                _ => DriveKind::Other,
            }
        }
        _ => DriveKind::Other,
    };
    winsvc::root_refusal(&root, &users, kind)
}

/// PowerShell 7 when `path` has it, else Windows PowerShell, which every
/// Windows has (AM53).
pub fn session_shell(path: Option<&OsStr>) -> (OsString, Vec<OsString>) {
    let shell = if crate::pty::resolve(OsStr::new("pwsh"), path).is_some() {
        "pwsh"
    } else {
        "powershell"
    };
    (shell.into(), vec!["-NoLogo".into()])
}

// ---- accounts and SIDs ----------------------------------------------------

/// A buffer `LocalAlloc`'d by Windows, freed on drop.
struct Local(*mut c_void);

impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: Windows allocated it with LocalAlloc and nothing else
            // frees it.
            unsafe { LocalFree(self.0) };
        }
    }
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: an open handle this module owns.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// A process token's user SID, as raw bytes (a SID copied out of
/// `TOKEN_USER`).
fn token_user(process: HANDLE) -> io::Result<Vec<u8>> {
    let mut token: HANDLE = null_mut();
    // SAFETY: `process` is a live process handle; `token` receives one.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Handle(token);
    let mut size = 0u32;
    // SAFETY: a size query.
    unsafe { GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut size) };
    let mut buf = vec![0u8; size as usize];
    // SAFETY: `buf` holds `size` bytes.
    if unsafe { GetTokenInformation(token.0, TokenUser, buf.as_mut_ptr().cast(), size, &mut size) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(buf)
}

/// The SID inside a `TOKEN_USER` buffer from [`token_user`].
fn user_sid(token_user: &[u8]) -> PSID {
    // SAFETY: the buffer begins with a TOKEN_USER, written by Windows.
    unsafe { (*(token_user.as_ptr() as *const TOKEN_USER)).User.Sid }
}

fn sid_string(sid: PSID) -> io::Result<String> {
    let mut out: *mut u16 = null_mut();
    // SAFETY: `sid` is valid; Windows allocates `out`, freed below.
    if unsafe { ConvertSidToStringSidW(sid, &mut out) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _free = Local(out.cast());
    // SAFETY: `out` is a NUL-terminated wide string.
    let len = (0..).take_while(|&i| unsafe { *out.add(i) } != 0).count();
    // SAFETY: `len` characters precede the NUL.
    Ok(String::from_utf16_lossy(unsafe {
        std::slice::from_raw_parts(out, len)
    }))
}

/// This process's user, as a SID string.
fn my_sid() -> io::Result<String> {
    // SAFETY: the pseudo-handle of this process.
    let user = token_user(unsafe { GetCurrentProcess() })?;
    sid_string(user_sid(&user))
}

/// `DOMAIN\name` for a SID, or `None`.
fn account_of(sid: PSID) -> Option<String> {
    let (mut name, mut domain) = (vec![0u16; 256], vec![0u16; 256]);
    let (mut n, mut d) = (name.len() as u32, domain.len() as u32);
    let mut kind: SID_NAME_USE = 0;
    // SAFETY: both buffers hold the lengths passed.
    let ok = unsafe {
        LookupAccountSidW(
            null(),
            sid,
            name.as_mut_ptr(),
            &mut n,
            domain.as_mut_ptr(),
            &mut d,
            &mut kind,
        )
    };
    (ok != 0).then(|| {
        format!(
            r"{}\{}",
            from_wide(&domain).to_string_lossy(),
            from_wide(&name).to_string_lossy()
        )
    })
}

/// The running process's account, `DOMAIN\name`.
pub fn current_account() -> Option<String> {
    // SAFETY: the pseudo-handle of this process.
    let user = token_user(unsafe { GetCurrentProcess() }).ok()?;
    account_of(user_sid(&user))
}

/// An account's SID string, if the account exists.
fn sid_of(account: &str) -> Option<String> {
    let name = wide(account);
    let (mut sid_len, mut dom_len) = (0u32, 0u32);
    let mut kind: SID_NAME_USE = 0;
    // SAFETY: a size query.
    unsafe {
        LookupAccountNameW(
            null(),
            name.as_ptr(),
            null_mut(),
            &mut sid_len,
            null_mut(),
            &mut dom_len,
            &mut kind,
        )
    };
    if io::Error::last_os_error().raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
        return None;
    }
    let mut sid = vec![0u8; sid_len as usize];
    let mut domain = vec![0u16; dom_len as usize];
    // SAFETY: both buffers hold the lengths the query asked for.
    let ok = unsafe {
        LookupAccountNameW(
            null(),
            name.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sid_len,
            domain.as_mut_ptr(),
            &mut dom_len,
            &mut kind,
        )
    };
    (ok != 0)
        .then(|| sid_string(sid.as_mut_ptr().cast()).ok())
        .flatten()
}

/// Whether this process runs elevated, or as SYSTEM.
pub(crate) fn is_elevated() -> bool {
    let check = |sid: &str| {
        let s = wide(sid);
        let mut psid: PSID = null_mut();
        // SAFETY: `s` is NUL-terminated; Windows allocates `psid`.
        if unsafe { ConvertStringSidToSidW(s.as_ptr(), &mut psid) } == 0 {
            return false;
        }
        let _free = Local(psid);
        let mut member = 0;
        // SAFETY: a null token means this thread's effective token.
        unsafe { CheckTokenMembership(null_mut(), psid, &mut member) != 0 && member != 0 }
    };
    check("S-1-5-32-544") || check("S-1-5-18")
}

// ---- the exclusive account (AM37, AM52) ----------------------------------

/// Whether `serve --exclusive-account` may run as `account`. Elevated, the
/// sweep would reach every account's processes; as a person, their session.
pub fn exclusive_account_allowed(account: Option<&str>, elevated: bool) -> Result<(), String> {
    if elevated {
        return Err("--exclusive-account refuses to run elevated: it would end \
                    other accounts' processes"
            .into());
    }
    match account {
        Some(a) if a.eq_ignore_ascii_case(SERVICE_ACCOUNT) => Ok(()),
        Some(a) => Err(format!(
            "--exclusive-account runs only as the service account {SERVICE_ACCOUNT}, \
             not as {a}: it ends every other process of its account"
        )),
        None => Err(format!(
            "--exclusive-account: this process's account has no name; it runs only as \
             {SERVICE_ACCOUNT}"
        )),
    }
}

pub fn check_exclusive_account() -> Result<(), String> {
    exclusive_account_allowed(current_account().as_deref(), is_elevated())
}

/// Every other live process whose token user is this process's.
pub fn sweep_targets() -> io::Result<Vec<u32>> {
    // SAFETY: the pseudo-handle of this process.
    let mine = token_user(unsafe { GetCurrentProcess() })?;
    // SAFETY: plain call.
    let me = unsafe { GetCurrentProcessId() };
    // SAFETY: plain call; the handle is closed by `Handle`.
    let snap = Handle(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) });
    if snap.0 == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut out = Vec::new();
    // SAFETY: `entry` is sized as the API requires.
    let mut more = unsafe { Process32FirstW(snap.0, &mut entry) } != 0;
    while more {
        let pid = entry.th32ProcessID;
        if pid != me && pid != 0 {
            // SAFETY: plain call; a refused open is skipped.
            let process = Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) });
            if !process.0.is_null()
                && let Ok(theirs) = token_user(process.0)
                // SAFETY: both are SIDs inside live TOKEN_USER buffers.
                && unsafe { EqualSid(user_sid(&mine), user_sid(&theirs)) } != 0
            {
                out.push(pid);
            }
        }
        // SAFETY: as above.
        more = unsafe { Process32NextW(snap.0, &mut entry) } != 0;
    }
    Ok(out)
}

/// Terminates every other process of this account (AM52's backstop). Windows
/// has no polite signal for a process outside a console, so there is no
/// hang-up and the grace is not used: what the host's own sessions ran
/// already died with their jobs. Call only after
/// [`check_exclusive_account`] passed.
pub fn sweep_account(_grace: Duration) -> (usize, usize) {
    let targets = sweep_targets().unwrap_or_default();
    let mut killed = 0;
    for pid in &targets {
        // SAFETY: plain calls; the handle is closed by `Handle`.
        let process = Handle(unsafe { OpenProcess(PROCESS_TERMINATE, 0, *pid) });
        if !process.0.is_null() && unsafe { TerminateProcess(process.0, 1) } != 0 {
            killed += 1;
        }
    }
    (targets.len(), killed)
}

// ---- private files and the pipe's DACL (AM49, AM55) ----------------------

/// A security descriptor built from SDDL, and the attributes that carry it.
pub(crate) struct OwnerOnly {
    descriptor: Local,
    attributes: SECURITY_ATTRIBUTES,
}

// SAFETY: the descriptor is immutable after construction and owned here.
unsafe impl Send for OwnerOnly {}

impl OwnerOnly {
    fn from_sddl(sddl: &str) -> io::Result<Self> {
        let text = wide(sddl);
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `text` is NUL-terminated; Windows allocates `sd`.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            attributes: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd,
                bInheritHandle: 0,
            },
            descriptor: Local(sd),
        })
    }

    /// The host's own account and SYSTEM, nothing else (AM55).
    pub fn for_pipe() -> io::Result<Self> {
        Self::from_sddl(&format!("D:P(A;;GA;;;SY)(A;;GA;;;{})", my_sid()?))
    }

    /// AM49: SYSTEM, Administrators, this process's account and the service
    /// account (which may differ: `enroll` runs elevated), protected.
    fn for_files(directory: bool) -> io::Result<Self> {
        let inherit = if directory { "OICI" } else { "" };
        let mut sids = vec!["SY".to_string(), "BA".to_string(), my_sid()?];
        if let Some(service) = sid_of(SERVICE_ACCOUNT)
            && !sids.contains(&service)
        {
            sids.push(service);
        }
        let aces: String = sids
            .iter()
            .map(|sid| format!("(A;{inherit};FA;;;{sid})"))
            .collect();
        Self::from_sddl(&format!("D:P{aces}"))
    }

    pub fn attributes(&self) -> *mut c_void {
        &self.attributes as *const SECURITY_ATTRIBUTES as *mut c_void
    }

    /// Replaces `path`'s DACL with this one, protected from inheritance.
    fn apply(&self, path: &Path) -> io::Result<()> {
        let (mut present, mut defaulted) = (0, 0);
        let mut dacl: *mut ACL = null_mut();
        // SAFETY: the descriptor is valid for the life of `self`.
        if unsafe {
            GetSecurityDescriptorDacl(self.descriptor.0, &mut present, &mut dacl, &mut defaulted)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let name = wide(path);
        // SAFETY: `name` is NUL-terminated; `dacl` points into the descriptor.
        let rc = unsafe {
            SetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                dacl,
                null(),
            )
        };
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc as i32));
        }
        Ok(())
    }
}

pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    if dir.exists() {
        return Ok(());
    }
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let security = OwnerOnly::for_files(true)?;
    let name = wide(dir);
    // SAFETY: `name` is NUL-terminated; the attributes live until return.
    if unsafe { CreateDirectoryW(name.as_ptr(), security.attributes().cast()) } == 0 {
        let e = io::Error::last_os_error();
        if !dir.is_dir() {
            return Err(e);
        }
    }
    Ok(())
}

pub fn tighten_private_dir(dir: &Path) -> io::Result<()> {
    create_private_dir(dir)?;
    OwnerOnly::for_files(true)?.apply(dir)
}

pub fn open_private(path: &Path, options: &mut std::fs::OpenOptions) -> io::Result<std::fs::File> {
    let file = options.open(path)?;
    OwnerOnly::for_files(false)?.apply(path)?;
    Ok(file)
}

/// `Everyone`, `Authenticated Users`, `Users`, `Guests`, `INTERACTIVE`.
const BROAD: &[&str] = &[
    "S-1-1-0",
    "S-1-5-11",
    "S-1-5-32-545",
    "S-1-5-32-546",
    "S-1-5-4",
];

/// Anything that reads the file's data.
const READS: u32 = 0x0001 /* FILE_READ_DATA */ | 0x8000_0000 /* GENERIC_READ */ | 0x1000_0000 /* GENERIC_ALL */;

pub fn is_private(path: &Path) -> io::Result<bool> {
    let name = wide(path);
    let mut dacl: *mut ACL = null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: `name` is NUL-terminated; Windows allocates `sd`, which `dacl`
    // points into, freed by `_free`.
    let rc = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc as i32));
    }
    let _free = Local(sd);
    // A null DACL grants everyone everything.
    if dacl.is_null() {
        return Ok(false);
    }
    let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: `dacl` is valid while `sd` lives.
    if unsafe {
        GetAclInformation(
            dacl,
            (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    for i in 0..info.AceCount {
        let mut ace: *mut c_void = null_mut();
        // SAFETY: `i` is below the ACE count.
        if unsafe { GetAce(dacl, i, &mut ace) } == 0 {
            continue;
        }
        // SAFETY: every ACE starts with an ACE_HEADER.
        let header = unsafe { &*(ace as *const ACE_HEADER) };
        if header.AceType != 0 {
            // Only ACCESS_ALLOWED_ACE_TYPE grants.
            continue;
        }
        // SAFETY: an ACCESS_ALLOWED_ACE, whose SID begins at SidStart.
        let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
        let sid = &allowed.SidStart as *const u32 as PSID;
        if allowed.Mask & READS != 0 && sid_string(sid).is_ok_and(|s| BROAD.contains(&s.as_str())) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sweep_runs_only_as_the_service_account_and_never_elevated() {
        assert!(exclusive_account_allowed(Some(SERVICE_ACCOUNT), true).is_err());
        assert_eq!(
            exclusive_account_allowed(Some(SERVICE_ACCOUNT), false),
            Ok(())
        );
        assert_eq!(
            exclusive_account_allowed(Some(r"nt service\stormruntime"), false),
            Ok(())
        );
        let person = exclusive_account_allowed(Some(r"PC\alice"), false).unwrap_err();
        assert!(person.contains(SERVICE_ACCOUNT), "{person}");
        assert!(exclusive_account_allowed(None, false).is_err());
    }

    #[test]
    fn the_sweep_targets_the_accounts_other_processes_never_itself() {
        // Lists without terminating, so it is safe whoever runs the suite.
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "ping -n 30 127.0.0.1 >nul"])
            .spawn()
            .unwrap();
        let targets = sweep_targets().unwrap();
        let _ = child.kill();
        let _ = child.wait();
        // SAFETY: plain call.
        let me = unsafe { GetCurrentProcessId() };
        assert!(
            !targets.contains(&me),
            "the sweep would kill the host itself"
        );
        assert!(
            targets.contains(&child.id()),
            "a process of this account is missing"
        );
    }

    #[test]
    fn the_running_account_has_a_name() {
        let me = current_account().expect("the test runner's account");
        assert!(me.contains('\\'), "{me}");
    }

    #[test]
    fn private_files_and_directories_are_unreadable_by_users() {
        let dir = tempfile::tempdir().unwrap();
        let open = dir.path().join("open.txt");
        std::fs::write(&open, "x").unwrap();
        let private = dir.path().join("private");
        create_private_dir(&private).unwrap();
        let key = private.join("key");
        open_private(
            &key,
            std::fs::OpenOptions::new().write(true).create_new(true),
        )
        .unwrap();
        assert!(is_private(&key).unwrap());
        assert!(is_private(&private).unwrap());
        // A file that grants Users read (as %TEMP% may not, so grant it).
        let status = std::process::Command::new("icacls")
            .arg(&open)
            .args(["/grant", "*S-1-5-32-545:(R)"])
            .status()
            .unwrap();
        assert!(status.success());
        assert!(!is_private(&open).unwrap());
    }

    #[test]
    fn profiles_are_refused_and_the_default_root_is_not() {
        let drive = std::env::var("SystemDrive").unwrap();
        assert!(refused_root(Path::new(&format!(r"{drive}\Users\x"))).is_some());
        assert!(refused_root(Path::new(&format!(r"\\?\{drive}\USERS"))).is_some());
        assert!(refused_root(Path::new(r"\\nas\share")).is_some());
        assert_eq!(
            refused_root(Path::new(&Layout::standard().workspaces)),
            None
        );
    }

    #[test]
    fn the_default_path_has_the_accounts_bins_first_and_the_machines_after() {
        let path = default_path().unwrap().to_string_lossy().to_lowercase();
        let windows = std::env::var("SystemRoot").unwrap().to_lowercase();
        assert!(path.contains(r"\.local\bin;"), "{path}");
        assert!(path.contains(&format!(r"{windows}\system32")), "{path}");
        assert!(path.find(r"\.local\bin") < path.find(&windows));
    }
}
