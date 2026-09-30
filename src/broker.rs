use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const BROKER_ARG: &str = "--broker";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BrokerStatusSnapshot {
    pub broker_running: bool,
    pub mutex_owned: bool,
    pub hotkey_registered: bool,
    pub pid: Option<u32>,
    pub launch_target: Option<String>,
    pub message: String,
    pub updated_at_unix: u64,
    /// Version of the running broker. A GUI of a different version (or one
    /// launching a different executable) replaces the broker, so Ctrl+Alt+B
    /// never keeps opening an old or moved copy.
    #[serde(default)]
    pub version: Option<String>,
}

const BROKER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether a running broker must be replaced: it launches a different
/// executable than this app would, or it is a different build. A snapshot
/// without a pid gives nothing to replace safely, so it is left alone.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn broker_is_outdated(snapshot: &BrokerStatusSnapshot, target_path: &Path) -> bool {
    if snapshot.pid.is_none() {
        return false;
    }
    let same_target = snapshot
        .launch_target
        .as_deref()
        .is_some_and(|target| target.eq_ignore_ascii_case(&target_path.display().to_string()));
    !same_target || snapshot.version.as_deref() != Some(BROKER_VERSION)
}

impl BrokerStatusSnapshot {
    pub fn summary(&self) -> String {
        let broker_state = if self.broker_running { "running" } else { "stopped" };
        let mutex_state = if self.mutex_owned { "owned" } else { "missing" };
        let hotkey_state = if self.hotkey_registered {
            "registered"
        } else {
            "not registered"
        };

        let mut summary = format!(
            "Broker {broker_state} · mutex {mutex_state} · hotkey {hotkey_state}"
        );
        if !self.message.is_empty() {
            summary.push_str(" · ");
            summary.push_str(&self.message);
        }
        summary
    }
}

pub struct AppInstanceGuard {
    #[cfg(target_os = "windows")]
    handle: Handle,
}

pub fn maybe_run_from_args() -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        if std::env::args_os().skip(1).any(|arg| arg == BROKER_ARG) {
            run_hotkey_broker()?;
            return Ok(true);
        }
    }

    Ok(false)
}

/// Bring Ctrl+Alt+B in line with the user's setting: on starts (or refreshes)
/// the listener and its sign-in entry, off stops it and removes them.
pub fn startup_status(hotkey_enabled: bool) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        let result = if hotkey_enabled {
            ensure_hotkey_integration()
        } else {
            disable_hotkey_integration()
        };
        match result {
            Ok(message) => Some(message),
            Err(err) => Some(format!("Hotkey broker setup skipped: {err}")),
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = hotkey_enabled;
        None
    }
}

/// Stop the listener and take it out of sign-in. The status file is marked
/// off so the GUI reports it as switched off rather than broken.
#[cfg(target_os = "windows")]
fn disable_hotkey_integration() -> Result<String, String> {
    remove_legacy_startup_shortcut();
    set_autostart(None)?;
    if broker_mutex_exists() {
        stop_broker()?;
    }
    write_status_snapshot(&BrokerStatusSnapshot {
        message: HOTKEY_OFF_MESSAGE.to_string(),
        updated_at_unix: unix_now(),
        version: Some(BROKER_VERSION.to_string()),
        ..Default::default()
    });
    Ok("Ctrl+Alt+B is off".to_string())
}

/// Terminate the running broker (verified to be a VaultForge process) and
/// wait for its mutex — and with it the hotkey — to be released.
#[cfg(target_os = "windows")]
fn stop_broker() -> Result<(), String> {
    let Some(pid) = read_status_snapshot_from_disk().and_then(|snapshot| snapshot.pid) else {
        return Err("the running broker's process id is unknown".to_string());
    };
    let ours = std::env::current_exe().ok().and_then(|exe| exe.file_name().map(|name| name.to_ascii_lowercase()));
    let is_vaultforge = process_image_path(pid)
        .and_then(|path| path.file_name().map(|name| name.to_ascii_lowercase()))
        .is_some_and(|name| Some(name) == ours);
    if !is_vaultforge || pid == std::process::id() {
        return Err("the recorded broker process is not VaultForge".to_string());
    }
    terminate_process(pid)?;
    for _ in 0..40 {
        if !broker_mutex_exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err("the broker did not exit".to_string())
}

/// Sign-in autostart lives in the per-user Run key: written directly (no
/// PowerShell, no COM), standard for tray-style helpers, and trivially removed
/// by the uninstaller. `None` removes the entry.
#[cfg(target_os = "windows")]
fn set_autostart(target_path: Option<&Path>) -> Result<(), String> {
    let subkey = wide_null(RUN_KEY);
    let value_name = wide_null(RUN_VALUE);
    let status = match target_path {
        Some(path) => {
            let command = wide_null(&format!("\"{}\" {}", path.display(), BROKER_ARG));
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    subkey.as_ptr(),
                    value_name.as_ptr(),
                    REG_SZ,
                    command.as_ptr().cast(),
                    (command.len() * 2) as u32,
                )
            }
        }
        None => match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), value_name.as_ptr()) } {
            ERROR_FILE_NOT_FOUND => 0,
            status => status,
        },
    };
    if status == 0 {
        Ok(())
    } else {
        Err(format!("could not update the sign-in entry (registry error {status})"))
    }
}

/// Builds before 0.1.0 started the broker from a Startup-folder shortcut;
/// the Run key replaces it, and two entries would start two brokers.
#[cfg(target_os = "windows")]
fn remove_legacy_startup_shortcut() {
    if let Ok(programs) = start_menu_programs_dir() {
        let _ = std::fs::remove_file(programs.join("Startup").join("VaultForge Hotkey Broker.lnk"));
    }
}

pub fn current_status_snapshot() -> Option<BrokerStatusSnapshot> {
    #[cfg(target_os = "windows")]
    {
        let mut snapshot = read_status_snapshot_from_disk().unwrap_or_default();
        let broker_running = broker_mutex_exists();
        if !broker_running && snapshot.message.is_empty() && snapshot.updated_at_unix == 0 {
            return None;
        }

        let stale_snapshot = !broker_running
            && (snapshot.broker_running
                || snapshot.mutex_owned
                || snapshot.hotkey_registered
                || snapshot.pid.is_some()
                || !snapshot.message.starts_with("broker offline"));

        snapshot.broker_running = broker_running;
        snapshot.mutex_owned = broker_running;
        if !broker_running {
            snapshot.hotkey_registered = false;
            snapshot.pid = None;
            snapshot.message = offline_broker_message(&snapshot.message);
        }
        if stale_snapshot {
            write_status_snapshot(&snapshot);
        }
        Some(snapshot)
    }

    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Claim the single-GUI-instance mutex. If another instance holds it and has a
/// visible window, that window is focused and `None` is returned. If it holds
/// the mutex *without* a window, it is an instance that is still shutting down
/// (the hotkey was pressed right after closing); wait for it rather than
/// exiting silently, and as a last resort clear one that never finished.
pub fn acquire_app_instance_guard() -> Result<Option<AppInstanceGuard>, String> {
    #[cfg(target_os = "windows")]
    {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(CLOSING_INSTANCE_WAIT_MS);
        loop {
            if let Some(guard) = try_create_gui_mutex()? {
                return Ok(Some(guard));
            }
            if focus_running_window() {
                return Ok(None);
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        if let Some(pid) = stale_gui_pid() {
            let _ = terminate_process(pid);
            for _ in 0..20 {
                if let Some(guard) = try_create_gui_mutex()? {
                    return Ok(Some(guard));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        Ok(None)
    }

    #[cfg(not(target_os = "windows"))]
    {
        Ok(Some(AppInstanceGuard {}))
    }
}

#[cfg(target_os = "windows")]
fn try_create_gui_mutex() -> Result<Option<AppInstanceGuard>, String> {
    let name = wide_null("Local\\VaultForgeGuiInstance");
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(last_os_error("CreateMutexW failed for GUI instance"));
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(handle);
        }
        return Ok(None);
    }
    if let Some(path) = gui_pid_path() {
        let _ = std::fs::write(path, std::process::id().to_string());
    }
    Ok(Some(AppInstanceGuard { handle }))
}

/// The pid recorded by the instance that holds the GUI mutex — but only if it
/// is still a VaultForge process with no visible window, so a recycled pid or a
/// live, usable instance is never touched.
#[cfg(target_os = "windows")]
fn stale_gui_pid() -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(gui_pid_path()?).ok()?.trim().parse().ok()?;
    if pid == std::process::id() {
        return None;
    }
    let image = process_image_path(pid)?;
    let ours = std::env::current_exe().ok()?;
    let same_program = image.file_name().map(|name| name.to_ascii_lowercase())
        == ours.file_name().map(|name| name.to_ascii_lowercase());
    (same_program && !process_has_visible_window(pid)).then_some(pid)
}

#[cfg(target_os = "windows")]
fn gui_pid_path() -> Option<PathBuf> {
    status_file_path().map(|path| path.with_file_name("vaultforge-gui.pid"))
}

/// Stamp recording which executable the Start Menu shortcuts point at. Writing
/// a shortcut costs a PowerShell + COM start (~0.5s warm, seconds on a cold
/// boot), and it used to run twice on every launch before the window appeared.
#[cfg(target_os = "windows")]
fn shortcut_stamp_path() -> Option<PathBuf> {
    status_file_path().map(|path| path.with_file_name("vaultforge-shortcuts.stamp"))
}

/// Shortcuts need rewriting only when one is missing or they were written for a
/// different executable (the exe moved, or a release build superseded debug).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn shortcuts_need_refresh(stamp: Option<&str>, target_path: &Path, shortcuts_exist: bool) -> bool {
    !shortcuts_exist || stamp.map(str::trim) != Some(target_path.to_string_lossy().as_ref())
}

#[cfg(target_os = "windows")]
fn ensure_hotkey_integration() -> Result<String, String> {
    let target_path = preferred_launch_executable()?;

    // Broker first: it is what makes Ctrl+Alt+B work, and it is cheap. The
    // shortcut refresh below can take seconds, and this runs on a worker thread
    // that dies with the app if the window is closed that quickly.
    let broker_state = if !broker_mutex_exists() {
        spawn_background_broker(&target_path)?;
        "started"
    } else if read_status_snapshot_from_disk().is_some_and(|snapshot| broker_is_outdated(&snapshot, &target_path)) {
        replace_broker(&target_path)?;
        "restarted"
    } else {
        "active"
    };

    set_autostart(Some(&target_path))?;
    remove_legacy_startup_shortcut();

    // A portable copy gets a Start Menu entry of its own. An installed copy
    // already has the installer's "VaultForge" shortcut; don't add a second.
    let installed = start_menu_programs_dir()?.join("VaultForge.lnk").is_file();
    let stamp_path = shortcut_stamp_path();
    let stamp = stamp_path.as_ref().and_then(|path| std::fs::read_to_string(path).ok());
    let shortcut_exists = manual_launch_shortcut_path()?.is_file();
    if !installed && shortcuts_need_refresh(stamp.as_deref(), &target_path, shortcut_exists) {
        // Non-essential (needs PowerShell, which some machines lock down):
        // the hotkey and sign-in entry above already work without it.
        if ensure_manual_launch_shortcut(&target_path).is_ok() {
            if let Some(path) = stamp_path {
                let _ = std::fs::write(path, target_path.to_string_lossy().as_bytes());
            }
        }
    }

    Ok(format!(
        "Hotkey broker {broker_state} on Ctrl+Alt+B -> {}",
        target_path.display()
    ))
}

#[cfg(target_os = "windows")]
fn spawn_background_broker(target_path: &Path) -> Result<(), String> {
    let working_dir = target_path
        .parent()
        .ok_or_else(|| "broker target directory is unavailable".to_string())?;
    spawn_hidden_process(target_path, &[BROKER_ARG], working_dir).map(|_| ())
}

/// Stop the running broker (verified to be a VaultForge process) and start one
/// for `target_path`. Used when the broker belongs to a moved or older copy.
#[cfg(target_os = "windows")]
fn replace_broker(target_path: &Path) -> Result<(), String> {
    let Some(pid) = read_status_snapshot_from_disk().and_then(|snapshot| snapshot.pid) else {
        return Ok(());
    };
    let is_vaultforge = process_image_path(pid)
        .and_then(|path| path.file_name().map(|name| name.to_ascii_lowercase()))
        .zip(target_path.file_name().map(|name| name.to_ascii_lowercase()))
        .is_some_and(|(running, ours)| running == ours);
    if !is_vaultforge || pid == std::process::id() {
        return Ok(());
    }
    terminate_process(pid)?;
    // The hotkey and mutex are released as the process dies; wait for that
    // so the replacement does not find the old mutex and bow out.
    for _ in 0..40 {
        if !broker_mutex_exists() {
            return spawn_background_broker(target_path);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err("previous broker did not exit".to_string())
}

/// The background hotkey listener. Built to stay up: a failed launch is
/// recorded and the next press tries again, and if another program holds
/// Ctrl+Alt+B the broker keeps retrying on a timer instead of exiting — a
/// dead broker would leave the hotkey off until the next login.
#[cfg(target_os = "windows")]
fn run_hotkey_broker() -> Result<(), String> {
    let _mutex = match BrokerMutex::acquire()? {
        Some(mutex) => mutex,
        None => return Ok(()),
    };
    let launch_target = preferred_launch_executable()
        .ok()
        .map(|path| path.display().to_string());
    let report = |running: bool, hotkey_registered: bool, message: String| {
        write_status_snapshot(&BrokerStatusSnapshot {
            broker_running: running,
            mutex_owned: running,
            hotkey_registered,
            pid: Some(std::process::id()),
            launch_target: launch_target.clone(),
            message,
            updated_at_unix: unix_now(),
            version: Some(BROKER_VERSION.to_string()),
        });
    };
    report(true, false, "broker starting".to_string());

    // Held (not read) so the registration is released when the broker exits.
    let mut _hotkey = None;
    let mut retry_timer = 0_usize;
    match HotkeyRegistration::register() {
        Ok(hotkey) => {
            _hotkey = Some(hotkey);
            report(true, true, "ready".to_string());
        }
        Err(err) => {
            retry_timer = unsafe { SetTimer(std::ptr::null_mut(), 0, HOTKEY_RETRY_MS, None) };
            report(true, false, hotkey_busy_message(&err));
        }
    }

    let mut last_launch: Option<(std::time::Instant, u32)> = None;
    let mut launch_failed = false;
    let mut message = Msg::default();
    loop {
        match unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } {
            -1 => {
                let err = last_os_error("GetMessageW failed");
                report(false, false, err.clone());
                return Err(err);
            }
            0 => {
                report(false, false, "broker stopped".to_string());
                return Ok(());
            }
            _ if message.message == WM_TIMER && retry_timer != 0 && message.w_param == retry_timer => {
                if let Ok(hotkey) = HotkeyRegistration::register() {
                    _hotkey = Some(hotkey);
                    unsafe {
                        KillTimer(std::ptr::null_mut(), retry_timer);
                    }
                    retry_timer = 0;
                    report(true, true, "ready".to_string());
                }
            }
            _ if message.message == WM_HOTKEY && message.w_param == HOTKEY_ID as usize => {
                match handle_hotkey_press(&mut last_launch) {
                    Ok(()) if launch_failed => {
                        launch_failed = false;
                        report(true, true, "ready".to_string());
                    }
                    Ok(()) => {}
                    Err(err) => {
                        launch_failed = true;
                        report(true, true, format!("last launch failed: {err}"));
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(target_os = "windows")]
fn hotkey_busy_message(err: &str) -> String {
    format!(
        "Ctrl+Alt+B is taken by another program ({err}); retrying every {}s",
        HOTKEY_RETRY_MS / 1_000
    )
}

/// Ctrl+Alt+B: bring VaultForge up, or put it away if it is already in front.
#[cfg(target_os = "windows")]
fn handle_hotkey_press(last_launch: &mut Option<(std::time::Instant, u32)>) -> Result<(), String> {
    // The hotkey gives this process the right to change the foreground window.
    // Pass it on, or a freshly launched window opens behind the current app
    // (and a second instance could not focus the first).
    unsafe {
        AllowSetForegroundWindow(ASFW_ANY);
    }

    let hwnd = find_app_window();
    if !hwnd.is_null() {
        let in_front = unsafe { GetForegroundWindow() } == hwnd && unsafe { IsIconic(hwnd) } == 0;
        if in_front {
            unsafe {
                ShowWindow(hwnd, SW_MINIMIZE);
            }
        } else {
            focus_window(hwnd);
        }
        return Ok(());
    }

    // An instance launched a moment ago that is still alive is opening its
    // window; a second press must not start another. Only a live launch
    // counts — if that window was already closed, this press relaunches.
    if last_launch.is_some_and(|(at, pid)| {
        at.elapsed() < std::time::Duration::from_millis(LAUNCH_DEBOUNCE_MS) && process_image_path(pid).is_some()
    }) {
        return Ok(());
    }

    let target_path = preferred_launch_executable()?;
    let working_dir = target_path
        .parent()
        .ok_or_else(|| "launch target directory is unavailable".to_string())?;
    let pid = spawn_hidden_process(&target_path, &[], working_dir)?;
    *last_launch = Some((std::time::Instant::now(), pid));
    Ok(())
}

/// Focus the running instance's window if there is one. Returns whether a
/// window exists — not whether Windows allowed the focus change. Treating a
/// refused focus as "no window" launched a duplicate instance.
#[cfg(target_os = "windows")]
fn focus_running_window() -> bool {
    let hwnd = find_app_window();
    if hwnd.is_null() {
        return false;
    }
    focus_window(hwnd);
    true
}

#[cfg(target_os = "windows")]
fn focus_window(hwnd: Hwnd) {
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
}

#[cfg(target_os = "windows")]
fn find_app_window() -> Hwnd {
    let mut state = WindowSearchState {
        hwnd: std::ptr::null_mut(),
        program: std::env::current_exe()
            .ok()
            .and_then(|exe| exe.file_name().map(std::ffi::OsStr::to_os_string)),
    };

    unsafe {
        EnumWindows(
            Some(enum_windows_find_app_window),
            &mut state as *mut WindowSearchState as Lparam,
        );
    }

    state.hwnd
}

/// Matches only a *visible* top-level window titled VaultForge that belongs to a
/// VaultForge executable. EnumWindows also yields hidden windows: an instance
/// mid-shutdown still has one, and "focusing" it made the hotkey do nothing
/// instead of launching a fresh window. The owner check keeps any unrelated
/// window that happens to share the title out of it.
#[cfg(target_os = "windows")]
unsafe extern "system" fn enum_windows_find_app_window(hwnd: Hwnd, l_param: Lparam) -> Bool {
    if unsafe { IsWindowVisible(hwnd) } == 0 || window_title(hwnd) != APP_WINDOW_TITLE {
        return 1;
    }
    let state = unsafe { &mut *(l_param as *mut WindowSearchState) };
    if !window_owned_by_program(hwnd, state.program.as_deref()) {
        return 1;
    }
    state.hwnd = hwnd;
    0
}

#[cfg(target_os = "windows")]
fn window_owned_by_program(hwnd: Hwnd, program: Option<&std::ffi::OsStr>) -> bool {
    let Some(program) = program else {
        return true;
    };
    let mut pid = 0_u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    process_image_path(pid)
        .and_then(|path| path.file_name().map(|name| name.to_ascii_lowercase()))
        .is_some_and(|name| name == program.to_ascii_lowercase())
}

/// Whether a process owns any visible top-level window.
#[cfg(target_os = "windows")]
fn process_has_visible_window(pid: u32) -> bool {
    let mut state = VisibleWindowSearch { pid, found: false };
    unsafe {
        EnumWindows(
            Some(enum_windows_find_visible_for_pid),
            &mut state as *mut VisibleWindowSearch as Lparam,
        );
    }
    state.found
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn enum_windows_find_visible_for_pid(hwnd: Hwnd, l_param: Lparam) -> Bool {
    let state = unsafe { &mut *(l_param as *mut VisibleWindowSearch) };
    let mut owner = 0_u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut owner);
    }
    if owner == state.pid && unsafe { IsWindowVisible(hwnd) } != 0 {
        state.found = true;
        return 0;
    }
    1
}

#[cfg(target_os = "windows")]
fn window_title(hwnd: Hwnd) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    if length <= 0 {
        return String::new();
    }

    let mut buffer = vec![0_u16; length as usize + 1];
    let written = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    if written <= 0 {
        return String::new();
    }

    String::from_utf16_lossy(&buffer[..written as usize])
}

#[cfg(target_os = "windows")]
fn start_menu_programs_dir() -> Result<PathBuf, String> {
    let app_data = std::env::var_os("APPDATA")
        .ok_or_else(|| "APPDATA is unavailable".to_string())?;
    Ok(PathBuf::from(app_data)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs"))
}

#[cfg(target_os = "windows")]
fn manual_launch_shortcut_path() -> Result<PathBuf, String> {
    Ok(start_menu_programs_dir()?.join("VaultForge Portable.lnk"))
}

#[cfg(target_os = "windows")]
fn ensure_manual_launch_shortcut(target_path: &Path) -> Result<(), String> {
    let shortcut_path = manual_launch_shortcut_path()?;

    ensure_shortcut(
        &shortcut_path,
        target_path,
        None,
        "VaultForge portable launcher",
        1,
    )
}

#[cfg(target_os = "windows")]
fn ensure_shortcut(
    shortcut_path: &Path,
    target_path: &Path,
    arguments: Option<&str>,
    description: &str,
    window_style: i32,
) -> Result<(), String> {
    let working_dir = target_path
        .parent()
        .ok_or_else(|| "shortcut target directory is unavailable".to_string())?;
    if let Some(parent) = shortcut_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let arguments_assignment = arguments.map_or(String::new(), |arguments| {
        format!(
            "$shortcut.Arguments = '{}'; ",
            powershell_single_quoted(arguments)
        )
    });

    let script = format!(
        concat!(
            "$shell = New-Object -ComObject WScript.Shell; ",
            "$shortcut = $shell.CreateShortcut('{shortcut}'); ",
            "$shortcut.TargetPath = '{target}'; ",
            "$shortcut.WorkingDirectory = '{working_dir}'; ",
            "$shortcut.Description = '{description}'; ",
            "$shortcut.IconLocation = '{icon},0'; ",
            "$shortcut.Hotkey = ''; ",
            "$shortcut.WindowStyle = {window_style}; ",
            "{arguments}",
            "$shortcut.Save()"
        ),
        shortcut = powershell_single_quoted_path(shortcut_path),
        target = powershell_single_quoted_path(target_path),
        working_dir = powershell_single_quoted_path(working_dir),
        description = powershell_single_quoted(description),
        icon = powershell_single_quoted_path(target_path),
        window_style = window_style,
        arguments = arguments_assignment,
    );

    let status = hidden_powershell_status(&script)?;
    if !status.success() {
        return Err(format!("PowerShell exited with status {status}"));
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn hidden_powershell_status(script: &str) -> Result<std::process::ExitStatus, String> {
    use std::os::windows::process::CommandExt;

    std::process::Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .status()
        .map_err(|err| err.to_string())
}

#[cfg(target_os = "windows")]
fn spawn_hidden_process(target_path: &Path, args: &[&str], working_dir: &Path) -> Result<u32, String> {
    use std::os::windows::process::CommandExt;

    let spawn = |flags: u32| {
        std::process::Command::new(target_path)
            .args(args)
            .current_dir(working_dir)
            .creation_flags(flags)
            .spawn()
            .map(|child| child.id())
    };
    // Break away from the launcher's job object: an app started from a
    // terminal or IDE shares its kill-on-close job, and the broker (the whole
    // point of which is to outlive the window) would die with that terminal.
    // Jobs that forbid breakaway reject the flag, so fall back without it.
    spawn(CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB)
        .or_else(|_| spawn(CREATE_NO_WINDOW))
        .map_err(|err| err.to_string())
}

#[cfg(target_os = "windows")]
pub fn preferred_launch_executable() -> Result<PathBuf, String> {
    let current_exe = std::env::current_exe().map_err(|err| err.to_string())?;
    Ok(preferred_launch_executable_for(&current_exe).unwrap_or(current_exe))
}

pub fn preferred_launch_executable_for(current_exe: &Path) -> Option<PathBuf> {
    let exe_name = current_exe.file_name()?;
    let exe_dir = current_exe.parent()?;

    let profile_dir = match exe_dir.file_name().and_then(|name| name.to_str()) {
        Some("debug") | Some("release") => exe_dir,
        Some("deps") => exe_dir.parent()?,
        _ => return None,
    };

    let profile_name = profile_dir.file_name().and_then(|name| name.to_str())?;
    if profile_name == "release" {
        return Some(current_exe.to_path_buf());
    }
    if profile_name != "debug" {
        return None;
    }

    let target_dir = profile_dir.parent()?;
    if target_dir.file_name().and_then(|name| name.to_str()) != Some("target") {
        return None;
    }

    let release_exe = target_dir.join("release").join(exe_name);
    should_prefer_release_executable(current_exe, &release_exe).then_some(release_exe)
}

fn should_prefer_release_executable(current_exe: &Path, release_exe: &Path) -> bool {
    let Ok(release_modified) = release_exe.metadata().and_then(|metadata| metadata.modified()) else {
        return false;
    };
    let Ok(current_modified) = current_exe.metadata().and_then(|metadata| metadata.modified()) else {
        return false;
    };

    release_modified >= current_modified
}

#[cfg(target_os = "windows")]
fn offline_broker_message(previous_message: &str) -> String {
    if previous_message == HOTKEY_OFF_MESSAGE {
        return HOTKEY_OFF_MESSAGE.to_string();
    }
    if previous_message.is_empty() || previous_message.starts_with("broker offline") {
        return "broker offline".to_string();
    }

    format!("broker offline; last status was {previous_message}")
}

#[cfg(target_os = "windows")]
fn read_status_snapshot_from_disk() -> Option<BrokerStatusSnapshot> {
    let path = status_file_path()?;
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

#[cfg(target_os = "windows")]
fn write_status_snapshot(snapshot: &BrokerStatusSnapshot) {
    let Some(path) = status_file_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // Atomic: the broker and the GUI both write this file and the GUI polls
    // it, so a plain write could be read half-finished.
    if let Ok(json) = serde_json::to_string_pretty(snapshot) {
        let _ = crate::history::write_atomically(&path, json.as_bytes());
    }
}

#[cfg(target_os = "windows")]
fn status_file_path() -> Option<PathBuf> {
    let app_data = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(app_data)
            .join("VaultTools")
            .join("vaultforge-broker-status.json"),
    )
}

#[cfg(target_os = "windows")]
fn broker_mutex_exists() -> bool {
    let name = wide_null("Local\\VaultForgeHotkeyBroker");
    let handle = unsafe { OpenMutexW(SYNCHRONIZE, 0, name.as_ptr()) };
    if handle.is_null() {
        return false;
    }
    unsafe {
        CloseHandle(handle);
    }
    true
}

#[cfg(target_os = "windows")]
fn powershell_single_quoted_path(path: &Path) -> String {
    powershell_single_quoted(&path.to_string_lossy())
}

#[cfg(target_os = "windows")]
fn powershell_single_quoted(value: &str) -> String {
    value.replace("'", "''")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(target_os = "windows")]
const APP_WINDOW_TITLE: &str = "VaultForge";
/// How long a new launch waits for a closing instance to release the GUI mutex.
#[cfg(target_os = "windows")]
const CLOSING_INSTANCE_WAIT_MS: u64 = 3_000;
#[cfg(target_os = "windows")]
const HOTKEY_ID: i32 = 1;
#[cfg(target_os = "windows")]
const WM_HOTKEY: u32 = 0x0312;
#[cfg(target_os = "windows")]
const MOD_ALT: u32 = 0x0001;
#[cfg(target_os = "windows")]
const MOD_CONTROL: u32 = 0x0002;
#[cfg(target_os = "windows")]
const MOD_NOREPEAT: u32 = 0x4000;
#[cfg(target_os = "windows")]
const SW_RESTORE: i32 = 9;
#[cfg(target_os = "windows")]
const SW_MINIMIZE: i32 = 6;
/// Status message while the user has switched Ctrl+Alt+B off.
pub const HOTKEY_OFF_MESSAGE: &str = "hotkey turned off";
#[cfg(target_os = "windows")]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(target_os = "windows")]
const RUN_VALUE: &str = "VaultForge Hotkey Broker";
/// `HKEY_CURRENT_USER` is `(HKEY)(LONG)0x80000001`: sign-extended on 64-bit.
#[cfg(target_os = "windows")]
const HKEY_CURRENT_USER: Handle = (0x8000_0001_u32 as i32) as isize as Handle;
#[cfg(target_os = "windows")]
const REG_SZ: u32 = 1;
#[cfg(target_os = "windows")]
const ERROR_FILE_NOT_FOUND: i32 = 2;
#[cfg(target_os = "windows")]
const WM_TIMER: u32 = 0x0113;
#[cfg(target_os = "windows")]
const ASFW_ANY: u32 = u32::MAX;
#[cfg(target_os = "windows")]
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
/// How often a broker that could not get Ctrl+Alt+B tries again.
#[cfg(target_os = "windows")]
const HOTKEY_RETRY_MS: u32 = 30_000;
/// Presses this soon after a launch wait for that launch's window instead of
/// starting another instance.
#[cfg(target_os = "windows")]
const LAUNCH_DEBOUNCE_MS: u64 = 2_000;
#[cfg(target_os = "windows")]
const ERROR_ALREADY_EXISTS: u32 = 183;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(target_os = "windows")]
const SYNCHRONIZE: u32 = 0x0010_0000;

#[cfg(target_os = "windows")]
const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;

#[cfg(target_os = "windows")]
const PROCESS_TERMINATE: u32 = 0x0000_0001;

/// Enough to read another process's image path without demanding full access —
/// the broad `PROCESS_QUERY_INFORMATION` right is denied for elevated processes.
#[cfg(target_os = "windows")]
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x0000_1000;

/// `PROCESSENTRY32W`. Only the fields up to `sz_exe_file` are read, but the
/// layout must match exactly — `Process32FirstW` validates `dw_size`.
#[cfg(target_os = "windows")]
#[repr(C)]
struct ProcessEntry32W {
    dw_size: u32,
    cnt_usage: u32,
    th32_process_id: u32,
    th32_default_heap_id: usize,
    th32_module_id: u32,
    cnt_threads: u32,
    th32_parent_process_id: u32,
    pc_pri_class_base: i32,
    dw_flags: u32,
    sz_exe_file: [u16; 260],
}

#[cfg(target_os = "windows")]
type Bool = i32;
#[cfg(target_os = "windows")]
type Handle = *mut std::ffi::c_void;
#[cfg(target_os = "windows")]
type Hwnd = *mut std::ffi::c_void;
#[cfg(target_os = "windows")]
type Wparam = usize;
#[cfg(target_os = "windows")]
type Lparam = isize;

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: i32,
    y: i32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Msg {
    hwnd: Hwnd,
    message: u32,
    w_param: Wparam,
    l_param: Lparam,
    time: u32,
    pt: Point,
    l_private: u32,
}

#[cfg(target_os = "windows")]
struct WindowSearchState {
    hwnd: Hwnd,
    /// Executable file name the window's process must have.
    program: Option<std::ffi::OsString>,
}

#[cfg(target_os = "windows")]
struct VisibleWindowSearch {
    pid: u32,
    found: bool,
}

#[cfg(target_os = "windows")]
struct BrokerMutex(Handle);

#[cfg(target_os = "windows")]
impl BrokerMutex {
    fn acquire() -> Result<Option<Self>, String> {
        let name = wide_null("Local\\VaultForgeHotkeyBroker");
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(last_os_error("CreateMutexW failed"));
        }

        let last_error = unsafe { GetLastError() };
        if last_error == ERROR_ALREADY_EXISTS {
            unsafe {
                CloseHandle(handle);
            }
            return Ok(None);
        }

        Ok(Some(Self(handle)))
    }
}

#[cfg(target_os = "windows")]
impl Drop for BrokerMutex {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

impl Drop for AppInstanceGuard {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        {
            if let Some(path) = gui_pid_path() {
                let _ = std::fs::remove_file(path);
            }
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(target_os = "windows")]
struct HotkeyRegistration;

#[cfg(target_os = "windows")]
impl HotkeyRegistration {
    fn register() -> Result<Self, String> {
        let success = unsafe {
            RegisterHotKey(
                std::ptr::null_mut(),
                HOTKEY_ID,
                MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                u32::from(b'B'),
            )
        };
        if success == 0 {
            return Err(last_os_error("RegisterHotKey failed"));
        }
        Ok(Self)
    }
}

#[cfg(target_os = "windows")]
impl Drop for HotkeyRegistration {
    fn drop(&mut self) {
        unsafe {
            UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID);
        }
    }
}

// ---------------------------------------------------------------------------
// Process inspection
//
// Windows holds an exclusive lock on a running executable, so building a
// workspace whose binary is still running fails at the *link* step — after the
// entire compile has already been paid for (Arachnia: 386s to reach "Access is
// denied"). These primitives let the build side detect it up front. All Win32
// FFI in this crate lives in this module by convention.
// ---------------------------------------------------------------------------

/// A running process whose executable image lives inside a given directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningProcess {
    pub pid: u32,
    pub name: String,
    pub image_path: PathBuf,
}

/// Processes whose executable image sits anywhere under `dir`. Used to find what
/// is holding a workspace's `target/` artifacts open. The current process is
/// always excluded — VaultForge building itself is the caller's business.
#[cfg(target_os = "windows")]
pub fn processes_running_under(dir: &Path) -> Vec<RunningProcess> {
    let root = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let root_key = normalized_prefix(&root);
    let current = std::process::id();
    let mut found = Vec::new();

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot.is_null() || snapshot as isize == -1 {
            return found;
        }

        let mut entry: ProcessEntry32W = std::mem::zeroed();
        entry.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;

        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let pid = entry.th32_process_id;
                if pid != 0 && pid != current {
                    if let Some(path) = process_image_path(pid) {
                        if normalized_prefix(&path).starts_with(&root_key) {
                            let name = path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| wide_to_string(&entry.sz_exe_file));
                            found.push(RunningProcess { pid, name, image_path: path });
                        }
                    }
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
    }

    found
}

#[cfg(not(target_os = "windows"))]
pub fn processes_running_under(_dir: &Path) -> Vec<RunningProcess> {
    Vec::new()
}

/// Force-terminate a process by pid. Only ever called from an explicit user
/// click — never speculatively.
#[cfg(target_os = "windows")]
pub fn terminate_process(pid: u32) -> Result<(), String> {
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            return Err(last_os_error(&format!("could not open process {pid}")));
        }
        let terminated = TerminateProcess(handle, 1);
        let failure = (terminated == 0)
            .then(|| last_os_error(&format!("could not terminate process {pid}")));
        CloseHandle(handle);
        match failure {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn terminate_process(_pid: u32) -> Result<(), String> {
    Err("process termination is only implemented on Windows".to_string())
}

/// Full image path of a pid, or `None` if it exited or denies access (system
/// processes routinely do — they are never inside a user workspace anyway).
#[cfg(target_os = "windows")]
fn process_image_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buffer = vec![0u16; 4_096];
        let mut size = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size);
        CloseHandle(handle);
        if ok == 0 || size == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize])))
    }
}

/// Case-folded, forward-slashed, trailing-separator form used for prefix tests.
/// The trailing separator matters: without it `…/target` would also match a
/// sibling directory named `target-old`.
fn normalized_prefix(path: &Path) -> String {
    let lossy = path.to_string_lossy().replace('\\', "/").to_lowercase();
    // `canonicalize` yields the \\?\ extended form; strip it so a canonicalized
    // root and a Win32-API image path compare on equal terms.
    let mut key = lossy.strip_prefix("//?/").unwrap_or(&lossy).to_string();
    if !key.ends_with('/') {
        key.push('/');
    }
    key
}

#[cfg(target_os = "windows")]
fn wide_to_string(raw: &[u16]) -> String {
    let end = raw.iter().position(|&unit| unit == 0).unwrap_or(raw.len());
    String::from_utf16_lossy(&raw[..end])
}

#[cfg(target_os = "windows")]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn last_os_error(context: &str) -> String {
    let error = std::io::Error::last_os_error();
    format!("{context}: {error}")
}

#[cfg(target_os = "windows")]
unsafe extern "system" {
    fn RegisterHotKey(h_wnd: Hwnd, id: i32, fs_modifiers: u32, vk: u32) -> Bool;
    fn UnregisterHotKey(h_wnd: Hwnd, id: i32) -> Bool;
    fn GetMessageW(msg: *mut Msg, h_wnd: Hwnd, min: u32, max: u32) -> i32;
    fn EnumWindows(callback: Option<unsafe extern "system" fn(Hwnd, Lparam) -> Bool>, l_param: Lparam) -> Bool;
    fn GetWindowTextLengthW(h_wnd: Hwnd) -> i32;
    fn GetWindowTextW(h_wnd: Hwnd, text: *mut u16, max_count: i32) -> i32;
    fn ShowWindow(h_wnd: Hwnd, command: i32) -> Bool;
    fn SetForegroundWindow(h_wnd: Hwnd) -> Bool;
    fn IsIconic(h_wnd: Hwnd) -> Bool;
    fn IsWindowVisible(h_wnd: Hwnd) -> Bool;
    fn GetForegroundWindow() -> Hwnd;
    fn AllowSetForegroundWindow(process_id: u32) -> Bool;
    fn SetTimer(h_wnd: Hwnd, id: usize, elapse_ms: u32, callback: Option<unsafe extern "system" fn(Hwnd, u32, usize, u32)>) -> usize;
    fn KillTimer(h_wnd: Hwnd, id: usize) -> Bool;
    fn GetWindowThreadProcessId(h_wnd: Hwnd, process_id: *mut u32) -> u32;
    fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
    fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> Bool;
    fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> Bool;
    fn OpenProcess(desired_access: u32, inherit_handle: Bool, process_id: u32) -> Handle;
    fn QueryFullProcessImageNameW(process: Handle, flags: u32, buffer: *mut u16, size: *mut u32) -> Bool;
    fn TerminateProcess(process: Handle, exit_code: u32) -> Bool;
    fn CreateMutexW(attributes: *const std::ffi::c_void, initial_owner: Bool, name: *const u16) -> Handle;
    fn OpenMutexW(desired_access: u32, inherit_handle: Bool, name: *const u16) -> Handle;
    fn CloseHandle(handle: Handle) -> Bool;
    fn GetLastError() -> u32;
}

#[cfg(target_os = "windows")]
#[link(name = "advapi32")]
unsafe extern "system" {
    fn RegSetKeyValueW(key: Handle, sub_key: *const u16, value_name: *const u16, value_type: u32, data: *const std::ffi::c_void, data_len: u32) -> i32;
    fn RegDeleteKeyValueW(key: Handle, sub_key: *const u16, value_name: *const u16) -> i32;
}

#[cfg(test)]
mod tests {
    use super::preferred_launch_executable_for;
    #[cfg(target_os = "windows")]
    use super::offline_broker_message;
    use super::{normalized_prefix, processes_running_under};
    #[cfg(target_os = "windows")]
    use super::terminate_process;
    use std::path::{Path, PathBuf};

    /// End-to-end over the real Win32 snapshot: a process launched from a
    /// directory is found by path, and can then be cleared. This is the
    /// mechanism behind the pre-flight check that stops a build from spending
    /// six minutes compiling only to fail at the link step.
    #[cfg(target_os = "windows")]
    #[test]
    fn finds_then_terminates_a_process_running_from_a_directory() {
        // `ping` is the probe rather than `cmd /c ping`: it idles for a known
        // duration without a console or stdin, and — critically — spawns no
        // child of its own. A grandchild would survive TerminateProcess and keep
        // the inherited stdio pipe open, hanging the whole test run.
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
        let source = PathBuf::from(system_root).join("System32").join("PING.EXE");
        let dir = unique_temp_dir("proc-scan");
        std::fs::create_dir_all(&dir).expect("create probe dir");
        let probe = dir.join("vf-probe.exe");
        std::fs::copy(&source, &probe).expect("copy probe exe");

        let mut child = std::process::Command::new(&probe)
            .args(["-n", "30", "127.0.0.1"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn probe");

        let mut found = Vec::new();
        for _ in 0..50 {
            found = processes_running_under(&dir);
            if !found.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }

        assert_eq!(found.len(), 1, "probe should be the only process under the temp dir");
        assert_eq!(found[0].pid, child.id());
        assert_eq!(found[0].name, "vf-probe.exe");

        terminate_process(found[0].pid).expect("terminate probe");
        let _ = child.wait();
        assert!(processes_running_under(&dir).is_empty(), "probe should be gone");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The trailing separator is load-bearing: without it a workspace at
    /// `…/Arachnia` would also claim processes running from `…/Arachnia-old`.
    #[test]
    fn normalized_prefix_does_not_match_sibling_directories() {
        let root = normalized_prefix(Path::new(r"C:\repo\Arachnia\target"));
        let inside = normalized_prefix(Path::new(r"C:\repo\Arachnia\target\release\arachnia.exe"));
        let sibling = normalized_prefix(Path::new(r"C:\repo\Arachnia\target-old\release\x.exe"));

        assert_eq!(root, "c:/repo/arachnia/target/");
        assert!(inside.starts_with(&root));
        assert!(!sibling.starts_with(&root));
    }

    /// `canonicalize` returns the `\\?\` extended form; image paths from Win32 do
    /// not. They must still compare equal.
    #[test]
    fn normalized_prefix_strips_extended_length_prefix() {
        let extended = normalized_prefix(Path::new(r"\\?\C:\repo\target"));
        let plain = normalized_prefix(Path::new(r"C:\repo\target"));
        assert_eq!(extended, plain);
    }
    use std::time::{Duration, SystemTime};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("vaultforge-broker-{name}-{nanos}"))
    }

    fn write_file(path: &std::path::Path, bytes: &[u8]) -> SystemTime {
        std::fs::write(path, bytes).unwrap();
        path.metadata().unwrap().modified().unwrap()
    }

    fn rewrite_until_newer(path: &std::path::Path, bytes: &[u8], threshold: SystemTime) {
        for _ in 0..8 {
            let modified = write_file(path, bytes);
            if modified > threshold {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }

        panic!("failed to create a newer test artifact timestamp");
    }

    #[test]
    fn quick_launch_prefers_release_executable_when_present_and_current_is_older() {
        let root = unique_temp_dir("release-target");
        let project_dir = root.join("VaultForger");
        let debug_exe = project_dir.join("target").join("debug").join("VaultForge.exe");
        let release_exe = project_dir.join("target").join("release").join("VaultForge.exe");

        std::fs::create_dir_all(debug_exe.parent().unwrap()).unwrap();
        std::fs::create_dir_all(release_exe.parent().unwrap()).unwrap();
        let debug_modified = write_file(&debug_exe, b"debug");
        rewrite_until_newer(&release_exe, b"release", debug_modified);

        let preferred = preferred_launch_executable_for(&debug_exe).unwrap();
        assert_eq!(preferred, release_exe);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn quick_launch_keeps_current_executable_when_release_is_stale() {
        let root = unique_temp_dir("stale-release-target");
        let project_dir = root.join("VaultForger");
        let debug_exe = project_dir.join("target").join("debug").join("VaultForge.exe");
        let release_exe = project_dir.join("target").join("release").join("VaultForge.exe");

        std::fs::create_dir_all(debug_exe.parent().unwrap()).unwrap();
        std::fs::create_dir_all(release_exe.parent().unwrap()).unwrap();
        let release_modified = write_file(&release_exe, b"release");
        rewrite_until_newer(&debug_exe, b"debug", release_modified);

        let preferred = preferred_launch_executable_for(&debug_exe);
        assert!(preferred.is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn quick_launch_uses_current_executable_when_release_is_absent() {
        let root = unique_temp_dir("no-release-target");
        let project_dir = root.join("VaultForger");
        let debug_exe = project_dir.join("target").join("debug").join("VaultForge.exe");

        std::fs::create_dir_all(debug_exe.parent().unwrap()).unwrap();

        let preferred = preferred_launch_executable_for(&debug_exe);
        assert!(preferred.is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    /// A broker for a moved exe or an older build must be replaced, or
    /// Ctrl+Alt+B keeps opening the old copy; a matching one is left alone.
    #[test]
    fn broker_is_replaced_only_when_it_launches_something_else() {
        use super::{BROKER_VERSION, BrokerStatusSnapshot, broker_is_outdated};
        let exe = Path::new(r"C:\tools\VaultForge.exe");
        let current = BrokerStatusSnapshot {
            pid: Some(42),
            launch_target: Some(r"C:\tools\VaultForge.exe".to_string()),
            version: Some(BROKER_VERSION.to_string()),
            ..Default::default()
        };
        assert!(!broker_is_outdated(&current, exe));
        assert!(
            !broker_is_outdated(
                &BrokerStatusSnapshot { launch_target: Some(r"c:\TOOLS\vaultforge.exe".to_string()), ..current.clone() },
                exe
            ),
            "Windows paths compare case-insensitively"
        );
        assert!(broker_is_outdated(
            &BrokerStatusSnapshot { launch_target: Some(r"C:\old\VaultForge.exe".to_string()), ..current.clone() },
            exe
        ));
        assert!(broker_is_outdated(&BrokerStatusSnapshot { version: None, ..current.clone() }, exe), "pre-versioning broker");
        assert!(broker_is_outdated(&BrokerStatusSnapshot { version: Some("0.0.1".to_string()), ..current.clone() }, exe));
        assert!(!broker_is_outdated(&BrokerStatusSnapshot { pid: None, version: None, ..current }, exe), "nothing to stop safely");
    }

    /// Launch must not pay for a PowerShell shortcut rewrite unless something
    /// actually changed.
    #[test]
    fn shortcuts_refresh_only_when_missing_or_retargeted() {
        use super::shortcuts_need_refresh;
        let exe = Path::new(r"C:\tools\VaultForge.exe");
        assert!(!shortcuts_need_refresh(Some(r"C:\tools\VaultForge.exe"), exe, true));
        assert!(!shortcuts_need_refresh(Some("C:\\tools\\VaultForge.exe\n"), exe, true), "stamp whitespace is ignored");
        assert!(shortcuts_need_refresh(Some(r"C:\old\VaultForge.exe"), exe, true), "exe moved");
        assert!(shortcuts_need_refresh(None, exe, true), "first run writes the stamp");
        assert!(shortcuts_need_refresh(Some(r"C:\tools\VaultForge.exe"), exe, false), "a deleted shortcut is restored");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn offline_broker_message_does_not_preserve_ready_as_current_state() {
        assert_eq!(offline_broker_message("ready"), "broker offline; last status was ready");
        assert_eq!(offline_broker_message(""), "broker offline");
        assert_eq!(offline_broker_message("broker offline"), "broker offline");
    }
}
