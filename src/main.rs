#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod broker;
mod discovery;
mod execution;
mod history;
mod icon;
mod theme;

const DEFAULT_INNER_SIZE: [f32; 2] = [1180.0, 760.0];
const DEFAULT_MIN_SIZE: [f32; 2] = [760.0, 520.0];
const DEFAULT_VIEWPORT_MARGIN: f32 = 24.0;
const BASE_DPI: f32 = 96.0;

fn main() {
    install_crash_reporter();

    let is_broker = std::env::args_os().skip(1).any(|arg| arg == "--broker");
    match broker::maybe_run_from_args() {
        Ok(true) => return,
        Ok(false) => {}
        // The broker is headless and restarts with the next launch or sign-in;
        // a dialog from a background process would only confuse.
        Err(_) if is_broker => return,
        Err(err) => return show_fatal_error(&format!("VaultForge could not start its hotkey listener:\n\n{err}")),
    }

    let app_instance_guard = match broker::acquire_app_instance_guard() {
        Ok(Some(guard)) => guard,
        Ok(None) => return,
        Err(err) => return show_fatal_error(&format!("VaultForge could not start:\n\n{err}")),
    };

    // Hotkey integration can shell out to PowerShell; it must never sit
    // between the user and the window. The app picks the result up when ready.
    let (startup_tx, startup_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let enabled = history::Settings::load().hotkey_enabled;
        let _ = startup_tx.send(broker::startup_status(enabled));
    });

    // OpenGL first (fast to start, the usual case); Direct3D/Vulkan through
    // wgpu when the machine has no usable OpenGL driver — virtual machines,
    // some remote-desktop sessions, very old or generic display drivers.
    let startup_rx = std::rc::Rc::new(std::cell::RefCell::new(Some(startup_rx)));
    // VAULTFORGE_RENDERER=wgpu tries Direct3D first: a documented workaround
    // for a driver whose OpenGL starts but draws incorrectly.
    let renderers = if std::env::var("VAULTFORGE_RENDERER").is_ok_and(|value| value.eq_ignore_ascii_case("wgpu")) {
        [eframe::Renderer::Wgpu, eframe::Renderer::Glow]
    } else {
        [eframe::Renderer::Glow, eframe::Renderer::Wgpu]
    };
    let mut failures = Vec::new();
    for renderer in renderers {
        let (viewport, positioned) = build_viewport(icon::icon_raster());
        let options = eframe::NativeOptions {
            viewport,
            renderer,
            ..Default::default()
        };
        let startup_rx = std::rc::Rc::clone(&startup_rx);
        let result = eframe::run_native(
            "VaultForge",
            options,
            Box::new(move |cc| {
                theme::apply_theme(&cc.egui_ctx);
                let receiver = startup_rx
                    .borrow_mut()
                    .take()
                    .unwrap_or_else(|| std::sync::mpsc::channel().1);
                Ok(Box::new(app::VaultForgeApp::new(receiver, !positioned)))
            }),
        );
        match result {
            Ok(()) => {
                failures.clear();
                break;
            }
            Err(err) => failures.push(format!("{renderer}: {err}")),
        }
    }

    // Release the single-instance mutex the moment the window is gone, so the
    // Ctrl+Alt+B broker can open a fresh window straight away.
    drop(app_instance_guard);
    if !failures.is_empty() {
        show_fatal_error(&format!(
            "VaultForge could not open its window. Your graphics driver may be missing or out of date.\n\n{}",
            failures.join("\n")
        ));
    }
}

/// A release build has no console, so a panic would vanish without a trace.
/// Write it to the log folder and tell the user where it is.
fn install_crash_reporter() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        let location = info
            .location()
            .map(|location| format!(" at {}:{}", location.file(), location.line()))
            .unwrap_or_default();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| text.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown error".to_string());
        let report = format!(
            "VaultForge {} crashed{location}\n{message}\n",
            env!("CARGO_PKG_VERSION")
        );
        let log = history::log_dir().join("vaultforge-crash.log");
        let saved = std::fs::create_dir_all(history::log_dir()).is_ok() && std::fs::write(&log, &report).is_ok();
        let is_broker = std::env::args_os().skip(1).any(|arg| arg == "--broker");
        if !is_broker {
            let where_saved = if saved { format!("\n\nDetails were saved to:\n{}", log.display()) } else { String::new() };
            show_fatal_error(&format!("VaultForge ran into a problem and has to close.\n\n{message}{where_saved}"));
        }
    }));
}

fn show_fatal_error(message: &str) {
    #[cfg(target_os = "windows")]
    {
        let wide = |text: &str| text.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let text = wide(message);
        let caption = wide("VaultForge");
        unsafe {
            MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONERROR);
        }
    }
    #[cfg(not(target_os = "windows"))]
    eprintln!("{message}");
}

/// The viewport, and whether it was already placed on the active monitor (in
/// which case the app must not re-center it on the first frame — that showed
/// as a visible jump).
fn build_viewport(icon: crate::icon::IconRaster) -> (eframe::egui::ViewportBuilder, bool) {
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("VaultForge")
        .with_decorations(false)
        .with_icon(std::sync::Arc::new(eframe::egui::IconData {
            rgba: icon.rgba,
            width: icon.width,
            height: icon.height,
        }))
        .with_inner_size(DEFAULT_INNER_SIZE)
        .with_min_inner_size(DEFAULT_MIN_SIZE);

    let metrics = active_monitor_viewport_metrics(DEFAULT_INNER_SIZE, DEFAULT_MIN_SIZE);
    let positioned = metrics.is_some();
    if let Some(metrics) = metrics {
        viewport = viewport
            .with_inner_size(metrics.inner_size)
            .with_min_inner_size(metrics.min_inner_size)
            .with_position(metrics.position);
    }

    (viewport, positioned)
}

struct ViewportMetrics {
    inner_size: [f32; 2],
    min_inner_size: [f32; 2],
    position: [f32; 2],
}

#[cfg(target_os = "windows")]
fn active_monitor_viewport_metrics(
    default_inner_size: [f32; 2],
    default_min_inner_size: [f32; 2],
) -> Option<ViewportMetrics> {
    let monitor = active_monitor_handle()?;
    if monitor.is_null() {
        return None;
    }

    let mut monitor_info = MonitorInfo::default();
    monitor_info.cb_size = std::mem::size_of::<MonitorInfo>() as u32;
    let got_monitor = unsafe { GetMonitorInfoW(monitor, &mut monitor_info) };
    if got_monitor == 0 {
        return None;
    }

    let (dpi_x, dpi_y) = monitor_dpi(monitor);
    let scale_x = BASE_DPI / dpi_x.max(1) as f32;
    let scale_y = BASE_DPI / dpi_y.max(1) as f32;

    let work_left = monitor_info.work.left as f32 * scale_x;
    let work_top = monitor_info.work.top as f32 * scale_y;
    let work_width = ((monitor_info.work.right - monitor_info.work.left).max(1) as f32) * scale_x;
    let work_height = ((monitor_info.work.bottom - monitor_info.work.top).max(1) as f32) * scale_y;

    let max_width = (work_width - DEFAULT_VIEWPORT_MARGIN * 2.0).max(420.0).min(work_width);
    let max_height = (work_height - DEFAULT_VIEWPORT_MARGIN * 2.0).max(360.0).min(work_height);

    let inner_width = default_inner_size[0].min(max_width).max(320.0_f32.min(work_width));
    let inner_height = default_inner_size[1].min(max_height).max(240.0_f32.min(work_height));
    let min_width = default_min_inner_size[0].min(inner_width);
    let min_height = default_min_inner_size[1].min(inner_height);

    let x = work_left + ((work_width - inner_width) / 2.0).max(0.0);
    let y = work_top + ((work_height - inner_height) / 2.0).max(0.0);

    Some(ViewportMetrics {
        inner_size: [inner_width, inner_height],
        min_inner_size: [min_width, min_height],
        position: [x, y],
    })
}

#[cfg(target_os = "windows")]
fn active_monitor_handle() -> Option<Hmonitor> {
    let foreground_window = unsafe { GetForegroundWindow() };
    if !foreground_window.is_null() {
        let monitor = unsafe { MonitorFromWindow(foreground_window, MONITOR_DEFAULTTONEAREST) };
        if !monitor.is_null() {
            return Some(monitor);
        }
    }

    let mut cursor = Point::default();
    let got_cursor = unsafe { GetCursorPos(&mut cursor) };
    if got_cursor == 0 {
        return None;
    }

    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    (!monitor.is_null()).then_some(monitor)
}

#[cfg(target_os = "windows")]
fn monitor_dpi(monitor: Hmonitor) -> (u32, u32) {
    let mut dpi_x = BASE_DPI as u32;
    let mut dpi_y = BASE_DPI as u32;
    let result = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    if result != 0 {
        return (BASE_DPI as u32, BASE_DPI as u32);
    }
    (dpi_x, dpi_y)
}

#[cfg(not(target_os = "windows"))]
fn active_monitor_viewport_metrics(
    _default_inner_size: [f32; 2],
    _default_min_inner_size: [f32; 2],
) -> Option<ViewportMetrics> {
    None
}

#[cfg(target_os = "windows")]
type Bool = i32;
#[cfg(target_os = "windows")]
type Hmonitor = *mut std::ffi::c_void;
#[cfg(target_os = "windows")]
type Hwnd = *mut std::ffi::c_void;

#[cfg(target_os = "windows")]
const MONITOR_DEFAULTTONEAREST: u32 = 0x0000_0002;
#[cfg(target_os = "windows")]
const MDT_EFFECTIVE_DPI: u32 = 0;

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
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct MonitorInfo {
    cb_size: u32,
    monitor: Rect,
    work: Rect,
    flags: u32,
}

#[cfg(target_os = "windows")]
impl Default for MonitorInfo {
    fn default() -> Self {
        Self {
            cb_size: 0,
            monitor: Rect::default(),
            work: Rect::default(),
            flags: 0,
        }
    }
}

#[cfg(target_os = "windows")]
unsafe extern "system" {
    fn GetCursorPos(point: *mut Point) -> Bool;
    fn GetForegroundWindow() -> Hwnd;
    fn MonitorFromPoint(point: Point, flags: u32) -> Hmonitor;
    fn MonitorFromWindow(window: Hwnd, flags: u32) -> Hmonitor;
    fn GetMonitorInfoW(monitor: Hmonitor, info: *mut MonitorInfo) -> Bool;
    fn MessageBoxW(owner: Hwnd, text: *const u16, caption: *const u16, kind: u32) -> i32;
}

#[cfg(target_os = "windows")]
const MB_OK: u32 = 0x0000_0000;
#[cfg(target_os = "windows")]
const MB_ICONERROR: u32 = 0x0000_0010;

#[cfg(target_os = "windows")]
#[link(name = "Shcore")]
unsafe extern "system" {
    fn GetDpiForMonitor(monitor: Hmonitor, dpi_type: u32, dpi_x: *mut u32, dpi_y: *mut u32) -> i32;
}
