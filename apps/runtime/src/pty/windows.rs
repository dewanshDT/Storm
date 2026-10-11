//! ConPTY and a Job Object per session (D16, AM51, AM52).
//!
//! - **The terminal** is a pseudoconsole. Its output is an anonymous pipe the
//!   reader drains. Its input is a named pipe whose host end is opened for
//!   overlapped I/O, because Windows has no `select` on a pipe: a write is
//!   waited for with the deadline's remainder and cancelled past it, which is
//!   77c's bounded write exactly.
//! - **The process** is created suspended, put in its job, then resumed, so
//!   not one instruction of it runs outside the job. Nothing is inherited but
//!   the pseudoconsole, and the standard handles are explicitly invalid so a
//!   host whose own are redirected does not hand them on.
//! - **The job** kills every process in it when its last handle closes, and
//!   has no `BREAKAWAY_OK`, so nothing the agent starts can leave: the host
//!   dying, however it dies, ends every session it ran.

use std::ffi::{OsStr, c_void};
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, GENERIC_READ, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
    PIPE_ACCESS_OUTBOUND, WriteFile,
};
use windows_sys::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, CreatePipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateEventW, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::{Exit, Launch, Spawned};
use crate::platform::wincmd;
use crate::platform::{OwnerOnly, wide};
use crate::provider::TerminalSize;

/// How much one overlapped write carries.
const WRITE_CHUNK: usize = 4096;

/// The input pipe's buffer: bounded, so an input flood waits for conhost.
const INPUT_BUFFER: u32 = 64 * 1024;

fn last_error() -> io::Error {
    io::Error::last_os_error()
}

fn check(ok: i32) -> io::Result<()> {
    if ok == 0 { Err(last_error()) } else { Ok(()) }
}

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(last_error());
    }
    // SAFETY: a fresh handle this function's caller just received and owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle as RawHandle) })
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle() as HANDLE
}

/// The pseudoconsole, closed once: by the hang-up, or when the last of the
/// session's ends lets it go.
struct Console(Mutex<Option<usize>>);

impl Console {
    fn take(&self) -> Option<HPCON> {
        self.0.lock().unwrap().take().map(|h| h as HPCON)
    }

    fn with<T>(&self, f: impl FnOnce(HPCON) -> T) -> Option<T> {
        self.0.lock().unwrap().map(|h| f(h as HPCON))
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        if let Some(hpc) = self.take() {
            // SAFETY: a pseudoconsole this session created and nothing else
            // closes, since `take` hands it out once.
            unsafe { ClosePseudoConsole(hpc) };
        }
    }
}

/// Starts `launch` on a new pseudoconsole at `size`, inside a new job.
pub(crate) fn spawn(launch: &Launch, size: TerminalSize) -> io::Result<Spawned> {
    let (out_read, out_write) = anonymous_pipe()?;
    let (in_ours, in_theirs) = input_pipe()?;
    let mut hpc: HPCON = 0 as HPCON;
    // SAFETY: both pipe ends are open; `hpc` receives the console.
    let hr =
        unsafe { CreatePseudoConsole(coord(size), raw(&in_theirs), raw(&out_write), 0, &mut hpc) };
    if hr != 0 {
        return Err(io::Error::from_raw_os_error(hr));
    }
    let console = Arc::new(Console(Mutex::new(Some(hpc as usize))));
    // The pseudoconsole holds its own duplicates; ours would keep the output
    // open after it closes.
    drop(in_theirs);
    drop(out_write);

    let job = create_job()?;
    let process = create_process(launch, hpc, &job)?;
    // SAFETY: plain call; the event is closed by its OwnedHandle.
    let event = owned(unsafe { CreateEventW(null(), 1, 0, null()) })?;
    Ok(Spawned {
        child: Child { process },
        output: File::from(out_read),
        input: Input {
            pipe: in_ours,
            event,
            console: console.clone(),
        },
        group: Group { job, console },
    })
}

fn coord(size: TerminalSize) -> COORD {
    COORD {
        X: size.cols().min(i16::MAX as u16) as i16,
        Y: size.rows().min(i16::MAX as u16) as i16,
    }
}

fn anonymous_pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write): (HANDLE, HANDLE) = (null_mut(), null_mut());
    // SAFETY: both out-pointers are live locals.
    check(unsafe { CreatePipe(&mut read, &mut write, null(), 0) })?;
    Ok((owned(read)?, owned(write)?))
}

/// A one-instance named pipe: ours to write, overlapped; the pseudoconsole's
/// to read. Only this account and SYSTEM may open it, and only as its first
/// instance, so nothing can stand in for either end.
fn input_pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = wide(format!(
        r"\\.\pipe\storm-runtime-conin-{}-{}-{:016x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        rand::random::<u64>()
    ));
    let security = OwnerOnly::for_pipe()?;
    // SAFETY: `name` is NUL-terminated; the attributes outlive the call.
    let ours = owned(unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_OUTBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            INPUT_BUFFER,
            0,
            0,
            security.attributes().cast(),
        )
    })?;
    // SAFETY: as above; no sharing, the one instance.
    let theirs = owned(unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ,
            0,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    })?;
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    // SAFETY: the client end is already open, so this completes at once.
    if unsafe { ConnectNamedPipe(raw(&ours), &mut overlapped) } == 0 {
        let e = last_error();
        if e.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
            return Err(e);
        }
    }
    Ok((ours, theirs))
}

fn create_job() -> io::Result<OwnedHandle> {
    // SAFETY: an anonymous job.
    let job = owned(unsafe { CreateJobObjectW(null(), null()) })?;
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: `limits` is the struct this information class takes.
    check(unsafe {
        SetInformationJobObject(
            raw(&job),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    })?;
    Ok(job)
}

fn system_cmd() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    Path::new(&root).join("System32").join("cmd.exe")
}

/// The environment block: the host's own, then the launch's, as
/// `CreateProcessW` takes it (sorted, NUL-separated, NUL-terminated).
fn environment_block(launch: &Launch) -> Vec<u16> {
    let lossy = |(k, v): (&OsStr, &OsStr)| {
        (
            k.to_string_lossy().into_owned(),
            v.to_string_lossy().into_owned(),
        )
    };
    let host: Vec<(String, String)> = std::env::vars_os().map(|(k, v)| lossy((&k, &v))).collect();
    let env = wincmd::merge_environment(host, launch.env.iter().map(|(k, v)| lossy((k, v))));
    let mut block: Vec<u16> = Vec::new();
    for (k, v) in env {
        block.extend(OsStr::new(&format!("{k}={v}")).encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

fn create_process(launch: &Launch, hpc: HPCON, job: &OwnedHandle) -> io::Result<OwnedHandle> {
    let program = launch.program.to_string_lossy().into_owned();
    let args: Vec<String> = launch
        .args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let (application, line) = if wincmd::is_batch(&program) {
        let cmd = system_cmd();
        let line = wincmd::batch_command_line(&cmd.to_string_lossy(), &program, &args)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        (cmd, line)
    } else {
        (
            launch.program.clone(),
            wincmd::command_line(&program, &args),
        )
    };
    let application = wide(&application);
    let mut line = wide(&line);
    let cwd = wide(&launch.cwd);
    let mut env = environment_block(launch);

    let mut list_size = 0usize;
    // SAFETY: a size query.
    unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut list_size) };
    let mut list_buf = vec![0u8; list_size];
    let list = list_buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
    // SAFETY: `list_buf` is the size the query asked for.
    check(unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut list_size) })?;
    struct List(LPPROC_THREAD_ATTRIBUTE_LIST);
    impl Drop for List {
        fn drop(&mut self) {
            // SAFETY: initialized above, deleted once.
            unsafe { DeleteProcThreadAttributeList(self.0) };
        }
    }
    let _list = List(list);
    // SAFETY: the attribute's value is the HPCON itself, not a pointer to it.
    check(unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            hpc as *const c_void,
            std::mem::size_of::<HPCON>(),
            null_mut(),
            null(),
        )
    })?;

    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    startup.lpAttributeList = list;

    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every string is NUL-terminated and outlives the call; `line`
    // is writable, as CreateProcessW requires.
    check(unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            null(),
            null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED,
            env.as_mut_ptr().cast(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut info,
        )
    })?;
    let process = owned(info.hProcess)?;
    let thread = owned(info.hThread)?;
    // SAFETY: both handles are live.
    if unsafe { AssignProcessToJobObject(raw(job), raw(&process)) } == 0 {
        let e = last_error();
        // SAFETY: it never ran; end it rather than let it run unconfined.
        unsafe { TerminateProcess(raw(&process), 1) };
        return Err(e);
    }
    // SAFETY: the suspended main thread.
    if unsafe { ResumeThread(raw(&thread)) } == u32::MAX {
        let e = last_error();
        unsafe { TerminateJobObject(raw(job), 1) };
        return Err(e);
    }
    Ok(process)
}

/// The same lookup `cmd` makes (AM53): a path as given, or each directory of
/// `path` in order, trying every `PATHEXT` extension in each.
pub(crate) fn resolve(command: &OsStr, path: Option<&OsStr>) -> Option<PathBuf> {
    let pathext = std::env::var("PATHEXT").unwrap_or_default();
    let command = command.to_string_lossy();
    let candidates = wincmd::candidates(&command, &pathext);
    if command.contains(['\\', '/', ':']) {
        return candidates
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.is_file());
    }
    let search = path
        .map(OsStr::to_os_string)
        .or_else(|| std::env::var_os("PATH"))?;
    std::env::split_paths(&search)
        .find_map(|dir| candidates.iter().map(|c| dir.join(c)).find(|p| p.is_file()))
}

pub(crate) struct Child {
    process: OwnedHandle,
}

impl Child {
    /// Waits for the agent's own process. Windows has no signals (AM51), so
    /// its exit code is all there is; a crash's is an `NTSTATUS`.
    pub fn wait(self) -> io::Result<Exit> {
        // SAFETY: a live process handle.
        if unsafe { WaitForSingleObject(raw(&self.process), INFINITE) } != WAIT_OBJECT_0 {
            return Err(last_error());
        }
        let mut code = 0u32;
        // SAFETY: as above.
        check(unsafe { GetExitCodeProcess(raw(&self.process), &mut code) })?;
        Ok(Exit {
            code: Some(code as i32),
            signal: None,
        })
    }
}

pub(crate) struct Input {
    pipe: OwnedHandle,
    event: OwnedHandle,
    console: Arc<Console>,
}

impl Input {
    /// Writes chunk by chunk, each waited for with what is left of the
    /// deadline and cancelled past it.
    pub fn write(&mut self, bytes: &[u8], deadline: Instant) -> io::Result<()> {
        for chunk in bytes.chunks(WRITE_CHUNK) {
            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            overlapped.hEvent = raw(&self.event);
            // SAFETY: `chunk` and `overlapped` stay alive and unmoved until
            // the operation completes or is cancelled and reaped below.
            let started = unsafe {
                WriteFile(
                    raw(&self.pipe),
                    chunk.as_ptr(),
                    chunk.len() as u32,
                    null_mut(),
                    &mut overlapped,
                )
            };
            if started == 0 {
                let e = last_error();
                if e.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                    return Err(ended_or(e));
                }
                let left = deadline.saturating_duration_since(Instant::now());
                let ms = left.as_millis().min(u32::MAX as u128 - 1) as u32;
                // SAFETY: the event belongs to this write.
                if unsafe { WaitForSingleObject(raw(&self.event), ms) } == WAIT_TIMEOUT {
                    let mut n = 0u32;
                    // SAFETY: cancel this write, then reap it so `chunk` and
                    // `overlapped` are no longer in use.
                    unsafe {
                        CancelIoEx(raw(&self.pipe), &overlapped);
                        GetOverlappedResult(raw(&self.pipe), &overlapped, &mut n, 1);
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the agent is not reading its input",
                    ));
                }
            }
            let mut n = 0u32;
            // SAFETY: the write has completed; this reads its result.
            if unsafe { GetOverlappedResult(raw(&self.pipe), &overlapped, &mut n, 1) } == 0 {
                return Err(ended_or(last_error()));
            }
        }
        Ok(())
    }

    pub fn resize(&self, size: TerminalSize) -> io::Result<()> {
        // SAFETY: the console is open while `with` holds its lock.
        match self
            .console
            .with(|hpc| unsafe { ResizePseudoConsole(hpc, coord(size)) })
        {
            Some(0) => Ok(()),
            Some(hr) => Err(io::Error::from_raw_os_error(hr)),
            None => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the session has ended",
            )),
        }
    }
}

/// A write that failed because the console went away is the session's end.
fn ended_or(e: io::Error) -> io::Error {
    match e.raw_os_error() {
        Some(c) if c == ERROR_NO_DATA as i32 || c == ERROR_BROKEN_PIPE as i32 => {
            io::Error::new(io::ErrorKind::BrokenPipe, "the session has ended")
        }
        _ => e,
    }
}

/// The session's job and its pseudoconsole.
pub(crate) struct Group {
    job: OwnedHandle,
    console: Arc<Console>,
}

// SAFETY: a job handle may be used from any thread, and the console is
// behind its mutex.
unsafe impl Send for Group {}
unsafe impl Sync for Group {}
unsafe impl Send for Input {}

impl Group {
    /// Closes the pseudoconsole: `CTRL_CLOSE_EVENT` to every console process
    /// on it, Windows' SIGHUP. Off this thread, because closing waits for
    /// conhost, which may wait for the reader.
    pub fn hang_up(&self) {
        if let Some(hpc) = self.console.take() {
            let hpc = hpc as usize;
            let _ = std::thread::Builder::new()
                .name("conpty-close".into())
                // SAFETY: taken once, so closed once.
                .spawn(move || unsafe { ClosePseudoConsole(hpc as HPCON) });
        }
    }

    /// The same: there is nothing more polite for what is left.
    pub fn hang_up_rest(&self) {
        self.hang_up();
    }

    /// Ends every process in the job.
    pub fn kill(&self) {
        // SAFETY: a live job handle.
        unsafe { TerminateJobObject(raw(&self.job), 1) };
    }

    pub fn is_empty(&self) -> bool {
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is the struct this information class fills.
        let ok = unsafe {
            QueryInformationJobObject(
                raw(&self.job),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                null_mut(),
            )
        };
        ok == 0 || info.ActiveProcesses == 0
    }
}
