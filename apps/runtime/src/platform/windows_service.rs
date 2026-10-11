//! Executes [`super::winsvc`] on Windows (D16, AM48–AM50): the layout and
//! its DACLs, the `StormRuntime` service under its virtual account, its
//! environment and recovery, and the entry point the Service Control Manager
//! calls.
//!
//! Elevated only, idempotent, and every tool by absolute path, because it runs
//! elevated with the invoking user's `PATH`.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

use super::winsvc::{
    ACCOUNT, DISPLAY_NAME, Layout, REVOKED_EXIT, SERVICE_NAME, acl_plan, default_config,
    git_safe_directory, service_environment,
};
use super::{InstallOptions, UninstallOptions};

/// The exit code of a host started unenrolled (after a revocation, say).
const NOT_ENROLLED_EXIT: u32 = 2;

/// The layout under this machine's `%ProgramData%` and `%ProgramFiles%`.
fn layout() -> Layout {
    let var = |name: &str, default: &str| {
        std::env::var(name)
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| default.into())
    };
    Layout::under(
        &var("ProgramData", r"C:\ProgramData"),
        &var("ProgramFiles", r"C:\Program Files"),
    )
}

fn system32(tool: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    Path::new(&root).join("System32").join(tool)
}

fn require_elevation(what: &str) -> Result<()> {
    if !super::windows::is_elevated() {
        bail!("{what} must run in an elevated PowerShell (Run as administrator)");
    }
    Ok(())
}

fn manager(access: ServiceManagerAccess) -> Result<ServiceManager> {
    ServiceManager::local_computer(None::<&str>, access).context("opening the service manager")
}

fn service_info(layout: &Layout, start_type: ServiceStartType) -> ServiceInfo {
    ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type,
        error_control: ServiceErrorControl::Normal,
        executable_path: PathBuf::from(&layout.bin),
        launch_arguments: vec!["service".into()],
        dependencies: vec![],
        account_name: Some(ACCOUNT.into()),
        account_password: None,
    }
}

/// Stops the service if it runs, waiting until it has. Returns whether it ran.
fn stop_and_wait(service: &windows_service::service::Service) -> Result<bool> {
    let status = service.query_status()?;
    if status.current_state == ServiceState::Stopped {
        return Ok(false);
    }
    if status.current_state != ServiceState::StopPending {
        let _ = service.stop();
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if service.query_status()?.current_state == ServiceState::Stopped {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    bail!("the StormRuntime service did not stop within 30 s")
}

/// The service's registry `Environment` (AM50, AM53).
fn set_environment(layout: &Layout) -> Result<()> {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, REG_MULTI_SZ, RegSetKeyValueW};
    let mut data: Vec<u16> = Vec::new();
    for entry in service_environment(layout) {
        data.extend(entry.encode_utf16());
        data.push(0);
    }
    data.push(0);
    let key = super::wide(format!(r"SYSTEM\CurrentControlSet\Services\{SERVICE_NAME}"));
    let value = super::wide("Environment");
    // SAFETY: `key` and `value` are NUL-terminated; `data` holds the bytes
    // passed.
    let rc = unsafe {
        RegSetKeyValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            REG_MULTI_SZ,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::from_raw_os_error(rc as i32))
            .context("setting the service's environment");
    }
    Ok(())
}

fn icacls(path: &str, args: &[String]) -> Result<()> {
    let out = Command::new(system32("icacls.exe"))
        .arg(path)
        .args(args)
        .output()
        .context("running icacls")?;
    if !out.status.success() {
        bail!(
            "icacls {path} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout).trim()
        );
    }
    Ok(())
}

/// The person who opened the elevated shell, as `DOMAIN\name`.
fn invoking_user() -> Option<String> {
    let user = std::env::var("USERNAME").ok().filter(|u| !u.is_empty())?;
    match std::env::var("USERDOMAIN").ok().filter(|d| !d.is_empty()) {
        Some(domain) => Some(format!(r"{domain}\{user}")),
        None => Some(user),
    }
}

/// `storm-runtime install` (AM50).
pub fn install(options: &InstallOptions) -> Result<()> {
    require_elevation("storm-runtime install")?;
    let layout = layout();
    let operator = options
        .operator
        .clone()
        .or_else(invoking_user)
        .context("no --operator, and no USERNAME to share the workspaces with")?;

    let manager = manager(ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)?;
    let access = ServiceAccess::QUERY_STATUS
        | ServiceAccess::START
        | ServiceAccess::STOP
        | ServiceAccess::CHANGE_CONFIG
        | ServiceAccess::QUERY_CONFIG;
    let existing = manager.open_service(SERVICE_NAME, access).ok();
    let was_running = match &existing {
        Some(service) => stop_and_wait(service)?,
        None => false,
    };

    for dir in layout.directories() {
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {dir}"))?;
    }
    std::fs::create_dir_all(&layout.bin_dir)
        .with_context(|| format!("creating {}", layout.bin_dir))?;
    let me = std::env::current_exe().context("finding this binary")?;
    if !same_file(&me, Path::new(&layout.bin)) {
        std::fs::copy(&me, &layout.bin).with_context(|| format!("copying to {}", layout.bin))?;
    }
    if !Path::new(&layout.config).exists() {
        std::fs::write(&layout.config, default_config(&layout))
            .with_context(|| format!("writing {}", layout.config))?;
    }

    // An enrolled host keeps starting with Windows; a fresh one waits for
    // its enrollment (AM50).
    let enrolled = Path::new(&layout.state).join("host.json").exists();
    let start_type = if enrolled {
        ServiceStartType::AutoStart
    } else {
        ServiceStartType::OnDemand
    };
    let info = service_info(&layout, start_type);
    let service = match existing {
        Some(service) => {
            service
                .change_config(&info)
                .context("updating the service")?;
            service
        }
        None => manager
            .create_service(&info, access)
            .context("creating the service")?,
    };
    service.set_description(
        "Runs Storm agent sessions for one Storm Server. Started by enrolling: \
         storm-runtime enroll.",
    )?;
    service.update_failure_actions(ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 3600)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(10),
            };
            3
        ]),
    })?;
    // A clean stop with an exit code (revoked, unenrolled) is not a failure.
    service.set_failure_actions_on_non_crash_failures(false)?;
    service.set_preshutdown_timeout(Duration::from_secs(20))?;
    if enrolled {
        service.set_delayed_auto_start(true)?;
    }
    set_environment(&layout)?;

    // The account exists now that the service does, so its grants resolve.
    for (path, args) in acl_plan(&layout, &operator) {
        icacls(&path, &args)?;
    }

    let gitconfig = Path::new(&layout.home).join(".gitconfig");
    let current = std::fs::read_to_string(&gitconfig).unwrap_or_default();
    if let Some(updated) =
        super::launchd::with_safe_directory(&current, &git_safe_directory(&layout))
    {
        std::fs::write(&gitconfig, updated)
            .with_context(|| format!("writing {}", gitconfig.display()))?;
    }

    if was_running {
        service
            .start(&[] as &[&OsStr])
            .context("restarting the service")?;
    }

    println!("Installed the Storm Runtime Host.");
    println!("  service    : {SERVICE_NAME}, as {ACCOUNT}");
    println!("  binary     : {}", layout.bin);
    println!("  config     : {}", layout.config);
    println!("  state      : {}", layout.state);
    println!(
        "  workspaces : {} (shared with {operator})",
        layout.workspaces
    );
    println!("  log        : {}", layout.log_file);
    if was_running {
        println!("\nUpgraded and restarted.");
    } else if enrolled {
        println!("\nEnrolled already: Start-Service {SERVICE_NAME}");
    } else {
        println!("\nNext, in this elevated shell, paste the enrollment string from the app:");
        println!("  & \"{}\" enroll", layout.bin);
        println!("Enrolling starts the service, and it starts with Windows from then on.");
    }
    println!(
        "\nSo your own git trusts checkouts the host made, run once as yourself:\n  \
         git config --global --add safe.directory \"{}\"",
        git_safe_directory(&layout)
    );
    println!(
        "\nAgent CLIs must be on the machine PATH or installed by the host itself:\n  \
         Claude Code needs Git for Windows; log it in from a Storm shell session\n  \
         on this host (it runs as {ACCOUNT}), or with `claude setup-token` into\n  \
         an env_file in {}.",
        layout.root
    );
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// `storm-runtime uninstall [--purge]`. Workspaces are never deleted.
pub fn uninstall(options: &UninstallOptions) -> Result<()> {
    require_elevation("storm-runtime uninstall")?;
    let layout = layout();
    let manager = manager(ServiceManagerAccess::CONNECT)?;
    if let Ok(service) = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    ) {
        stop_and_wait(&service)?;
        service.delete().context("deleting the service")?;
        println!("Removed the {SERVICE_NAME} service.");
    }
    let me = std::env::current_exe().ok();
    if me
        .as_deref()
        .is_some_and(|me| same_file(me, Path::new(&layout.bin)))
    {
        // Windows cannot delete a running binary; it goes when this exits.
        println!("Remove {} once this command has finished.", layout.bin_dir);
    } else if Path::new(&layout.bin_dir).exists() {
        std::fs::remove_dir_all(&layout.bin_dir)
            .with_context(|| format!("removing {}", layout.bin_dir))?;
    }
    if options.purge {
        for path in [&layout.state, &layout.logs] {
            if Path::new(path).exists() {
                std::fs::remove_dir_all(path).with_context(|| format!("removing {path}"))?;
            }
        }
        let _ = std::fs::remove_file(&layout.config);
        println!("Purged the identity, state, logs and config.");
    }
    println!("Workspaces are kept: {}", layout.workspaces);
    Ok(())
}

/// After enrollment (AM50): the service starts with Windows, and now.
pub fn enable_and_start() -> Result<()> {
    let manager = manager(ServiceManagerAccess::CONNECT)?;
    let service = manager
        .open_service(
            SERVICE_NAME,
            ServiceAccess::CHANGE_CONFIG
                | ServiceAccess::QUERY_CONFIG
                | ServiceAccess::QUERY_STATUS
                | ServiceAccess::START
                | ServiceAccess::STOP,
        )
        .context("the StormRuntime service is not installed (storm-runtime install)")?;
    service.change_config(&service_info(&layout(), ServiceStartType::AutoStart))?;
    service.set_delayed_auto_start(true)?;
    // A host already running holds the old enrollment; restart it.
    stop_and_wait(&service)?;
    service
        .start(&[] as &[&OsStr])
        .context("starting the service")?;
    Ok(())
}

// ---- the service itself ----------------------------------------------------

define_windows_service!(ffi_service_main, service_entry);

/// `storm-runtime service`: hands this process to the Service Control
/// Manager, which calls back on its own thread.
pub fn service_main() -> Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .context("this command is run by the Service Control Manager, not by hand")
}

fn service_entry(_arguments: Vec<OsString>) {
    if let Err(e) = run_service() {
        tracing::error!(error = %format!("{e:#}"), "the service failed");
    }
}

/// Opens the log, keeping one previous file past 10 MiB.
fn open_log(layout: &Layout) -> Option<std::fs::File> {
    let path = Path::new(&layout.log_file);
    if std::fs::metadata(path).is_ok_and(|m| m.len() > 10 * 1024 * 1024) {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}

fn run_service() -> Result<()> {
    let layout = layout();
    if let Some(log) = open_log(&layout) {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "info".into()),
            )
            .with_ansi(false)
            .with_writer(Mutex::new(log))
            .try_init();
    }

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Preshutdown | ServiceControl::Shutdown => {
            let _ = stop_tx.send(true);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status = service_control_handler::register(SERVICE_NAME, handler)?;
    let report = |state, exit_code, wait_hint| {
        let _ = status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: if state == ServiceState::Running {
                ServiceControlAccept::STOP | ServiceControlAccept::PRESHUTDOWN
            } else {
                ServiceControlAccept::empty()
            },
            exit_code,
            checkpoint: 0,
            wait_hint,
            process_id: None,
        });
    };

    let state = PathBuf::from(&layout.state);
    if !crate::identity::HostConfig::path(&state).exists() {
        tracing::warn!(
            state = %state.display(),
            "not enrolled; stopping. Enroll with `storm-runtime enroll` in an elevated shell"
        );
        report(
            ServiceState::Stopped,
            ServiceExitCode::ServiceSpecific(NOT_ENROLLED_EXIT),
            Duration::ZERO,
        );
        return Ok(());
    }
    if let Err(e) = super::check_exclusive_account() {
        tracing::error!(error = %e, "refusing to run");
        report(
            ServiceState::Stopped,
            ServiceExitCode::ServiceSpecific(1),
            Duration::ZERO,
        );
        return Ok(());
    }

    report(
        ServiceState::Running,
        ServiceExitCode::Win32(0),
        Duration::ZERO,
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    let mut stop_rx = stop_rx;
    let stop = async {
        let _ = stop_rx.wait_for(|stop| *stop).await;
        // At once, with the shutdown's bound as the hint: a service still
        // RUNNING after a Stop is one `Stop-Service` reports as failed.
        report(
            ServiceState::StopPending,
            ServiceExitCode::Win32(0),
            Duration::from_secs(20),
        );
    };
    let config = PathBuf::from(&layout.config);
    let outcome = runtime.block_on(async {
        let outcome = crate::serve::run(&state, &config, true, stop);
        tokio::pin!(outcome);
        outcome.await
    });
    let exit = match outcome {
        Ok(crate::serve::Outcome::Stopped) => ServiceExitCode::Win32(0),
        // A clean stop with a service-specific code: recovery leaves it
        // stopped, unlike a crash (AM36).
        Ok(crate::serve::Outcome::Revoked) => ServiceExitCode::ServiceSpecific(REVOKED_EXIT),
        Ok(crate::serve::Outcome::Failed(e)) | Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "the host stopped");
            ServiceExitCode::ServiceSpecific(1)
        }
    };
    report(
        ServiceState::StopPending,
        ServiceExitCode::Win32(0),
        Duration::from_secs(5),
    );
    drop(runtime);
    report(ServiceState::Stopped, exit, Duration::ZERO);
    Ok(())
}
