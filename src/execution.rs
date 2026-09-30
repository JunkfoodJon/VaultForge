use crate::discovery::{Toolchain, WorkspaceTarget};
use std::collections::{BTreeSet, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
};
use std::thread::{self, JoinHandle};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use walkdir::{DirEntry, WalkDir};
use serde::Deserialize;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Discover,
    Check,
    Build,
    Run,
    Test,
    Lint,
    Clean,
    Package,
    Reveal,
    OpenShellHere,
    Inspect,
}

impl ActionKind {
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "Discover" => Some(Self::Discover),
            "Check" => Some(Self::Check),
            "Build" => Some(Self::Build),
            "Run" => Some(Self::Run),
            "Test" => Some(Self::Test),
            "Lint" => Some(Self::Lint),
            "Clean" => Some(Self::Clean),
            "Package" => Some(Self::Package),
            "Reveal" => Some(Self::Reveal),
            "Open Shell Here" => Some(Self::OpenShellHere),
            "Inspect" => Some(Self::Inspect),
            _ => None,
        }
    }

    pub fn records_build_history(&self) -> bool {
        matches!(
            self,
            Self::Check | Self::Build | Self::Test | Self::Lint | Self::Package
        )
    }

    pub fn records_launch_history(&self) -> bool {
        matches!(self, Self::Run)
    }
}

#[derive(Clone, Debug)]
pub struct ExecutionPlan {
    pub toolchain: Toolchain,
    pub action: ActionKind,
    pub action_label: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env_sets: Vec<(String, String)>,
    pub env_removals: Vec<String>,
}

#[derive(Debug)]
pub struct ExecutionReport {
    pub plan: ExecutionPlan,
    pub success: bool,
    pub cancelled: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub finished_at_unix: u64,
    /// Wall-clock build time, used to learn a duration for the next build's bar.
    pub duration_ms: u64,
    /// Countable units (crates / modules) compiled, paired with `duration_ms`.
    pub unit_count: u32,
    /// How many of `unit_count` were cached (`fresh`) rather than compiled.
    pub fresh_units: u32,
    pub errors_total: u32,
    pub warnings_total: u32,
    /// Error diagnostics captured outside the rolling display window, so a
    /// failure is always explainable even when the log overflowed with warnings.
    pub error_digest: Vec<TerminalLine>,
    /// On-disk log holding this execution's complete, uncapped output.
    pub log_path: Option<PathBuf>,
}

/// Directory whose locked artifacts would break this plan, if any. Only actions
/// that link or delete binaries care: `cargo check` and `cargo clippy` never
/// touch the executable, so they are safe to run against a live process.
pub fn artifact_lock_dir(plan: &ExecutionPlan) -> Option<PathBuf> {
    if plan.toolchain != Toolchain::Rust {
        return None;
    }
    if !matches!(
        plan.action,
        ActionKind::Build | ActionKind::Run | ActionKind::Test | ActionKind::Clean
    ) {
        return None;
    }
    let dir = plan.cwd.join("target");
    dir.is_dir().then_some(dir)
}

/// Windows holds an exclusive lock on a running executable, so a Build issued
/// after a Run fails at the link step with a bare "Access is denied" — cargo
/// never names the process. VaultForger launches those binaries itself, so this
/// is a failure mode it causes and should therefore explain.
pub fn failure_hint(report: &ExecutionReport) -> Option<String> {
    if report.success || report.cancelled {
        return None;
    }
    // The toolchain itself could not be started: say what to install rather
    // than leaving the user with a bare "program not found".
    if report.exit_code.is_none()
        && (report.stderr.contains("program not found") || report.stderr.contains("os error 2"))
    {
        return Some(match report.plan.toolchain {
            Toolchain::Python => "Hint: VaultForge couldn't start Python. Install it from https://www.python.org/downloads/ (keep \"py launcher\" ticked), then start VaultForge again.".to_string(),
            _ => "Hint: VaultForge couldn't start cargo. Install Rust from https://rustup.rs, then start VaultForge again.".to_string(),
        });
    }
    let digest = report
        .error_digest
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let haystack = if digest.is_empty() { report.stderr.as_str() } else { digest.as_str() };

    let denied = haystack.contains("Access is denied") || haystack.contains("os error 5");
    let linking = haystack.contains("failed to remove file") || haystack.contains("failed to link");
    if !(denied && linking) {
        return None;
    }

    Some(match locked_file_name(haystack) {
        Some(name) => format!(
            "Hint: {} is still running \u{2014} Windows locks a running executable, so the linker cannot replace it. Close it (or Stop the Run) and build again.",
            name
        ),
        None => "Hint: the build output is locked by a running process. Close it and build again."
            .to_string(),
    })
}

/// Pull the executable name out of cargo's ``failed to remove file `…` `` line.
fn locked_file_name(haystack: &str) -> Option<String> {
    let line = haystack.lines().find(|line| line.contains("failed to remove file"))?;
    let path = line.split('`').nth(1)?;
    Path::new(path).file_name().map(|name| name.to_string_lossy().into_owned())
}

impl ExecutionReport {
    pub fn summary(&self) -> String {
        if self.cancelled {
            return format!("{} cancelled", self.plan.action_label);
        }
        if self.success {
            format!("{} finished successfully", self.plan.action_label)
        } else {
            format!(
                "{} failed{}",
                self.plan.action_label,
                self.exit_code
                    .map(|code| format!(" with exit code {}", code))
                    .unwrap_or_default()
            )
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExecutionProgressSnapshot {
    pub phase_label: String,
    pub detail_label: String,
    /// Parser-driven "floor" fraction from phase heuristics. The UI takes the
    /// max of this and the time-based estimate (see `build_progress_fraction`).
    pub fraction: f32,
    pub recent_output: String,
    /// Short name of the crate/module currently being worked on.
    pub current_crate: String,
    pub units_completed: u32,
    pub units_total: u32,
    /// How many of `units_completed` were cargo `fresh` (cached) artifacts —
    /// used to avoid learning durations from mostly-cached builds.
    pub fresh_units: u32,
    pub errors_total: u32,
    pub warnings_total: u32,
    /// Learned duration of the last identical build, if any — drives the smooth
    /// time-based bar. `None` on the first build of a configuration.
    pub est_duration_ms: Option<u64>,
    /// Runnable binary path reported by cargo (`executable` field), when present.
    pub binary_path: Option<String>,
    /// Header of the first error seen, captured live so the UI can flag a doomed
    /// build immediately instead of after the remaining minutes of compiling.
    pub first_error: Option<String>,
}

/// Count a rendered diagnostic header.
///
/// `--message-format=json-render-diagnostics` makes cargo render diagnostics as
/// text on stderr and emit **no** `compiler-message` JSON, so counting only the
/// JSON path leaves both totals at zero for every Rust build. Cargo's own
/// roll-up lines ("generated 992 warnings", "could not compile … due to 1
/// previous error") summarize diagnostics already counted, so they are skipped.
fn count_rendered_diagnostic(line: &str, progress: &mut ExecutionProgressSnapshot) {
    if is_diagnostic_rollup(line) {
        return;
    }
    if line.starts_with("error") {
        progress.errors_total += 1;
        if progress.first_error.is_none() {
            progress.first_error = Some(line.to_string());
        }
    } else if line.starts_with("warning:") {
        progress.warnings_total += 1;
    }
}

fn is_diagnostic_rollup(line: &str) -> bool {
    (line.contains("generated ") && line.contains(" warning"))
        || (line.contains("due to ") && line.contains(" previous error"))
        || line.starts_with("warning: build failed, waiting for other jobs")
}

/// Fraction of build work done, for the progress bar.
///
/// Ported from the Arachnia Forge model: crate count is a poor signal because on
/// an incremental build cargo reports every cached crate as a `fresh` artifact
/// in the first instant, so the count rockets to ~99% then freezes while the one
/// changed crate compiles. When a learned duration exists we drive the bar by
/// elapsed time on an asymptotic curve (~95% at the expected finish, always
/// creeping, never frozen). Before any duration is learned we fall back to the
/// crate ratio, and the caller maxes this with the parser's phase floor so the
/// bar still shows early life during dependency resolution.
pub fn build_progress_fraction(
    elapsed_ms: u64,
    est_duration_ms: Option<u64>,
    units_completed: u32,
    units_total: u32,
) -> f32 {
    match est_duration_ms {
        Some(est) if est > 0 => {
            let tau = est as f32 / 3.0;
            (1.0 - (-(elapsed_ms as f32) / tau).exp()).clamp(0.0, 0.99)
        }
        _ => {
            if units_total == 0 {
                0.0
            } else {
                (units_completed as f32 / units_total as f32).min(0.99)
            }
        }
    }
}

struct ExecutionRuntimeState {
    progress: ExecutionProgressSnapshot,
    parser: ProgressParser,
    stdout: String,
    stderr: String,
    /// Rolling window of display lines. Oldest lines are evicted once the window
    /// is full so the *tail* — where a failing build prints its errors — always
    /// survives. See `terminal_dropped` for how the UI cursor stays valid.
    terminal: VecDeque<TerminalLine>,
    /// How many lines have been evicted from the front of `terminal`. The
    /// absolute index of `terminal[i]` is `terminal_dropped + i`; the UI holds an
    /// absolute cursor, so eviction can never corrupt incremental draining.
    terminal_dropped: usize,
    /// Error diagnostics retained outside the rolling window. A build that emits
    /// tens of thousands of warning lines must still be able to show why it
    /// failed, so errors get their own reserved budget that warnings cannot
    /// crowd out.
    errors: Vec<TerminalLine>,
    /// Lines still being absorbed into the current retained error block.
    error_block_remaining: usize,
    /// Full, uncapped log of this execution on disk. Every bound above is a
    /// display concession; this is the complete record.
    log: Option<std::io::BufWriter<std::fs::File>>,
}

/// Open the on-disk log for an execution. Failure is deliberately non-fatal: an
/// unwritable log directory must not stop a build from running.
fn create_execution_log(plan: &ExecutionPlan) -> Option<(PathBuf, std::io::BufWriter<std::fs::File>)> {
    let dir = crate::history::log_dir();
    std::fs::create_dir_all(&dir).ok()?;

    let workspace = plan
        .cwd
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "workspace".to_string());
    // Milliseconds, not seconds: two runs of the same action in one second would
    // otherwise overwrite each other's log.
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!(
        "{}-{}-{}.log",
        sanitize_log_component(&workspace),
        sanitize_log_component(&plan.action_label),
        stamp
    ));

    let mut writer = std::io::BufWriter::new(std::fs::File::create(&path).ok()?);
    let _ = writeln!(writer, "# {} {}", plan.program, plan.args.join(" "));
    let _ = writeln!(writer, "# cwd: {}", plan.cwd.display());
    Some((path, writer))
}

fn sanitize_log_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '-' })
        .collect()
}

/// One display line of the in-app terminal, classified for coloring.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalLine {
    pub text: String,
    pub kind: TerminalLineKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalLineKind {
    /// The echoed `$ program args…` line when an action starts.
    Command,
    /// Plain program output.
    Output,
    /// Cargo/tool status chatter ("Compiling foo", "Finished release").
    Info,
    Warning,
    Error,
    /// Final "action finished" summary line.
    Success,
    /// Final "action failed / cancelled" summary line.
    Failure,
}

impl TerminalLine {
    pub fn new(text: impl Into<String>, kind: TerminalLineKind) -> Self {
        Self { text: text.into(), kind }
    }

    /// A line with its kind inferred from the text (error/warning/status prefix).
    pub fn classified(text: &str) -> Self {
        let text = text.trim_end();
        Self::new(text, classify_terminal_line(text))
    }
}

/// Size of the rolling display window per execution. Once full, the oldest line
/// is evicted for each new one, so the window always holds the most recent
/// output rather than the first `N` lines of it.
const MAX_TERMINAL_LINES: usize = 8_000;

/// Reserved budget for error diagnostics, kept independently of the rolling
/// window. Arachnia emits >11k lines of warnings before failing; without a
/// reserved budget the errors are just more lines competing with that flood.
const MAX_RETAINED_ERROR_LINES: usize = 600;

/// How many lines after an `error…` header belong to that diagnostic (the source
/// snippet, carets, `help:`/`note:` continuations). Rendered rustc diagnostics
/// are ~8-12 lines; 16 captures the block without swallowing the next one.
const ERROR_BLOCK_LINES: usize = 16;

/// Format one raw output line into displayable terminal lines. JSON messages
/// from `--message-format=json-render-diagnostics` are unreadable raw: keep only
/// rendered compiler diagnostics and drop artifact/build-script bookkeeping.
fn terminal_lines_for_output(line: &str) -> Vec<TerminalLine> {
    if let Some(message) = parse_cargo_json_message(line) {
        return match message.reason.as_str() {
            "compiler-message" => message
                .message
                .as_ref()
                .and_then(|diagnostic| diagnostic.rendered.as_deref())
                .map(terminal_lines_for_rendered_diagnostic)
                .unwrap_or_default(),
            _ => Vec::new(),
        };
    }

    let text = line.trim_end();
    vec![TerminalLine::new(text, classify_terminal_line(text))]
}

fn terminal_lines_for_rendered_diagnostic(rendered: &str) -> Vec<TerminalLine> {
    rendered
        .lines()
        .map(|line| {
            let text = line.trim_end();
            TerminalLine::new(text, classify_terminal_line(text))
        })
        .collect()
}

fn classify_terminal_line(text: &str) -> TerminalLineKind {
    let trimmed = text.trim_start();
    if trimmed.starts_with("error") {
        return TerminalLineKind::Error;
    }
    if trimmed.starts_with("warning") {
        return TerminalLineKind::Warning;
    }
    const STATUS_VERBS: [&str; 12] = [
        "Compiling ", "Checking ", "Building ", "Fresh ", "Finished ", "Downloading ",
        "Downloaded ", "Updating ", "Running ", "Doc-tests ", "Documenting ", "Cleaning ",
    ];
    if STATUS_VERBS.iter().any(|verb| trimmed.starts_with(verb)) {
        return TerminalLineKind::Info;
    }
    TerminalLineKind::Output
}

struct ProgressParser {
    toolchain: Toolchain,
    action: ActionKind,
    rust_units: BTreeSet<String>,
    rust_completed: BTreeSet<String>,
    rust_expected: usize,
    total_tests: Option<usize>,
    completed_tests: usize,
    python_units: BTreeSet<String>,
    python_expected: usize,
}

#[derive(Clone, Copy)]
enum OutputStream {
    Stdout,
    Stderr,
}

/// One incremental read of the live terminal window.
pub struct TerminalDrain {
    pub lines: Vec<TerminalLine>,
    /// Cursor to pass to the next `terminal_lines_since` call.
    pub next_cursor: usize,
    /// Lines evicted before the UI could read them. Non-zero only when the UI
    /// fell more than a full window behind a very loud build.
    pub dropped: usize,
}

pub struct RunningExecution {
    plan: ExecutionPlan,
    /// Log for this run, available while it is still in flight.
    log_path: Option<PathBuf>,
    started_at: Instant,
    runtime: Arc<Mutex<ExecutionRuntimeState>>,
    pid: Arc<Mutex<Option<u32>>>,
    cancel_requested: Arc<AtomicBool>,
    receiver: Receiver<ExecutionReport>,
}

impl RunningExecution {
    pub fn try_finish(&self) -> Option<ExecutionReport> {
        self.receiver.try_recv().ok()
    }

    pub fn plan(&self) -> &ExecutionPlan {
        &self.plan
    }

    pub fn log_path(&self) -> Option<&Path> {
        self.log_path.as_deref()
    }

    /// Millisecond-precision elapsed time — drives the smooth time-based bar and
    /// the shimmer sweep.
    pub fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    pub fn snapshot(&self) -> ExecutionProgressSnapshot {
        self.runtime
            .lock()
            .map(|state| state.progress.clone())
            .unwrap_or_else(|_| fallback_progress_snapshot(&self.plan))
    }

    /// Formatted terminal lines produced since absolute cursor `from` — the UI
    /// drains incrementally each frame so lines appear live as the build emits
    /// them. The cursor is absolute over the execution's whole output, so it
    /// stays valid even after the rolling window evicts older lines.
    pub fn terminal_lines_since(&self, from: usize) -> TerminalDrain {
        match self.runtime.lock() {
            Ok(state) => drain_terminal(&state, from),
            Err(_) => TerminalDrain { lines: Vec::new(), next_cursor: from, dropped: 0 },
        }
    }

    pub fn cancel(&self) -> Result<(), String> {
        self.cancel_requested.store(true, Ordering::SeqCst);

        if let Ok(mut state) = self.runtime.lock() {
            state.progress.phase_label = "Cancelling".to_string();
            state.progress.detail_label = "Stopping process tree".to_string();
            state.progress.fraction = state.progress.fraction.max(0.98);
        }

        let pid = self.pid.lock().map_err(|_| "Execution state is unavailable".to_string())?;
        if let Some(pid) = *pid {
            kill_process_tree(pid)
        } else {
            Ok(())
        }
    }
}

/// Per-toolchain build variants and advanced options selected in the UI.
///
/// Defaults reproduce the legacy behavior (Rust dev profile, no extra flags;
/// Python file-mode run), so `BuildOptions::default()` is always safe to pass.
#[derive(Clone, Debug, Default)]
pub struct BuildOptions {
    /// Rust: build with the `--release` profile instead of dev.
    pub rust_release: bool,
    /// Rust: cleaned, non-empty feature names to pass via `--features`.
    pub rust_features: Vec<String>,
    /// Rust: an explicit `--target` triple, when set.
    pub rust_target: Option<String>,
    /// Python: run a module via `-m <module>` instead of an entry file.
    pub python_run_as_module: bool,
    /// Python: the module name to run when `python_run_as_module` is set.
    pub python_module: Option<String>,
}

pub fn build_plan(
    target: &WorkspaceTarget,
    action_label: &str,
    options: &BuildOptions,
) -> Result<ExecutionPlan, String> {
    let Some(action) = ActionKind::from_label(action_label) else {
        return Err(format!("Unknown action: {}", action_label));
    };

    let cwd = std::fs::canonicalize(&target.root_path).unwrap_or_else(|_| target.root_path.clone());
    match target.toolchain {
        Toolchain::Rust => build_rust_plan(action, action_label, cwd, options),
        Toolchain::Python => build_python_plan(target, action, action_label, cwd, options),
        Toolchain::Mixed => Err("Mixed workspaces need adapter split before execution in this slice".to_string()),
    }
}

/// Stable identity for a build configuration — same workspace + same exact
/// command yields the same signature, so its learned duration is reused.
pub fn build_signature(plan: &ExecutionPlan) -> String {
    format!(
        "{}|{}|{}",
        crate::history::normalized_path_key(&plan.cwd),
        plan.program,
        plan.args.join(" ")
    )
}

pub fn spawn_execution(plan: ExecutionPlan, est_duration_ms: Option<u64>) -> RunningExecution {
    let (sender, receiver) = mpsc::channel();
    let thread_plan = plan.clone();
    let started_at = Instant::now();
    let (log_path, log) = match create_execution_log(&plan) {
        Some((path, writer)) => (Some(path), Some(writer)),
        None => (None, None),
    };
    let thread_log_path = log_path.clone();
    let runtime = Arc::new(Mutex::new(ExecutionRuntimeState {
        progress: ExecutionProgressSnapshot {
            est_duration_ms,
            ..initial_progress_snapshot(&plan)
        },
        parser: ProgressParser::new(&plan),
        stdout: String::new(),
        stderr: String::new(),
        terminal: VecDeque::new(),
        terminal_dropped: 0,
        errors: Vec::new(),
        error_block_remaining: 0,
        log,
    }));
    let thread_runtime = Arc::clone(&runtime);
    let pid = Arc::new(Mutex::new(None));
    let thread_pid = Arc::clone(&pid);
    let cancel_requested = Arc::new(AtomicBool::new(false));
    let thread_cancel_requested = Arc::clone(&cancel_requested);

    thread::spawn(move || {
        prime_progress_runtime(&thread_runtime, &thread_plan);

        let mut command = tool_command(&thread_plan.program);
        command
            .args(&thread_plan.args)
            .current_dir(&thread_plan.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        apply_hidden_spawn_flags(&mut command);

        for (key, value) in &thread_plan.env_sets {
            command.env(key, value);
        }
        for key in &thread_plan.env_removals {
            command.env_remove(key);
        }

        let report = match command.spawn() {
            Ok(mut child) => {
                if let Ok(mut pid_slot) = thread_pid.lock() {
                    *pid_slot = Some(child.id());
                }

                if thread_cancel_requested.load(Ordering::SeqCst) {
                    let _ = kill_process_tree(child.id());
                }

                let stdout_reader = child
                    .stdout
                    .take()
                    .map(|stdout| spawn_output_reader(OutputStream::Stdout, stdout, Arc::clone(&thread_runtime)));
                let stderr_reader = child
                    .stderr
                    .take()
                    .map(|stderr| spawn_output_reader(OutputStream::Stderr, stderr, Arc::clone(&thread_runtime)));

                let wait_result = child.wait();

                if let Ok(mut pid_slot) = thread_pid.lock() {
                    *pid_slot = None;
                }

                if let Some(reader) = stdout_reader {
                    let _ = reader.join();
                }
                if let Some(reader) = stderr_reader {
                    let _ = reader.join();
                }

                match wait_result {
                    Ok(status) => {
                        let cancelled = thread_cancel_requested.load(Ordering::SeqCst);
                        let final_output = finalize_progress(
                            &thread_runtime,
                            status.success(),
                            status.code(),
                            cancelled,
                        );
                        ExecutionReport {
                            plan: thread_plan,
                            success: status.success() && !cancelled,
                            cancelled,
                            exit_code: status.code(),
                            stdout: final_output.stdout,
                            stderr: final_output.stderr,
                            finished_at_unix: unix_now(),
                            duration_ms: started_at.elapsed().as_millis() as u64,
                            unit_count: final_output.snapshot.units_completed,
                            fresh_units: final_output.snapshot.fresh_units,
                            errors_total: final_output.snapshot.errors_total,
                            warnings_total: final_output.snapshot.warnings_total,
                            error_digest: final_output.errors,
                            log_path: thread_log_path,
                        }
                    }
                    Err(err) => {
                        let cancelled = thread_cancel_requested.load(Ordering::SeqCst);
                        let final_output = finalize_progress(&thread_runtime, false, None, cancelled);
                        let mut stderr = final_output.stderr;
                        if !stderr.is_empty() {
                            stderr.push('\n');
                        }
                        stderr.push_str(&err.to_string());
                        ExecutionReport {
                            plan: thread_plan,
                            success: false,
                            cancelled,
                            exit_code: None,
                            stdout: final_output.stdout,
                            stderr,
                            finished_at_unix: unix_now(),
                            duration_ms: started_at.elapsed().as_millis() as u64,
                            unit_count: final_output.snapshot.units_completed,
                            fresh_units: final_output.snapshot.fresh_units,
                            errors_total: final_output.snapshot.errors_total,
                            warnings_total: final_output.snapshot.warnings_total,
                            error_digest: final_output.errors,
                            log_path: thread_log_path,
                        }
                    }
                }
            }
            Err(err) => ExecutionReport {
                plan: thread_plan,
                success: false,
                cancelled: false,
                exit_code: None,
                stdout: String::new(),
                stderr: err.to_string(),
                finished_at_unix: unix_now(),
                duration_ms: started_at.elapsed().as_millis() as u64,
                unit_count: 0,
                fresh_units: 0,
                errors_total: 0,
                warnings_total: 0,
                error_digest: Vec::new(),
                log_path: thread_log_path,
            },
        };

        let _ = sender.send(report);
    });

    RunningExecution {
        plan,
        log_path,
        started_at,
        runtime,
        pid,
        cancel_requested,
        receiver,
    }
}

fn build_rust_plan(
    action: ActionKind,
    action_label: &str,
    cwd: PathBuf,
    options: &BuildOptions,
) -> Result<ExecutionPlan, String> {
    let mut args = match action {
        ActionKind::Check => vec!["check".to_string()],
        ActionKind::Build => vec!["build".to_string()],
        ActionKind::Test => vec!["test".to_string()],
        ActionKind::Lint => vec!["clippy".to_string()],
        ActionKind::Clean => vec!["clean".to_string()],
        ActionKind::Run => vec!["run".to_string()],
        _ => return Err(format!("{} is not executable for Rust yet", action_label)),
    };

    apply_rust_options(&mut args, &action, options);

    if rust_uses_json_messages(&action) {
        args.push("--message-format=json-render-diagnostics".to_string());
    }

    Ok(ExecutionPlan {
        toolchain: Toolchain::Rust,
        action,
        action_label: action_label.to_string(),
        program: "cargo".to_string(),
        args,
        cwd,
        env_sets: Vec::new(),
        env_removals: Vec::new(),
    })
}

fn apply_rust_options(args: &mut Vec<String>, action: &ActionKind, options: &BuildOptions) {
    if options.rust_release {
        args.push("--release".to_string());
    }
    if let Some(target) = options.rust_target.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        args.push("--target".to_string());
        args.push(target.to_string());
    }
    // `cargo clean` rejects `--features`; every other supported subcommand accepts it.
    if !options.rust_features.is_empty() && !matches!(action, ActionKind::Clean) {
        args.push("--features".to_string());
        args.push(options.rust_features.join(","));
    }
}

fn build_python_plan(
    target: &WorkspaceTarget,
    action: ActionKind,
    action_label: &str,
    cwd: PathBuf,
    options: &BuildOptions,
) -> Result<ExecutionPlan, String> {
    let interpreter = find_python_interpreter(&cwd);
    let (program, base_args, env_sets, env_removals) = python_invocation_parts(&cwd, interpreter.as_deref());

    let mut args = base_args;
    match action {
        ActionKind::Check => args.extend(["-m".to_string(), "compileall".to_string(), ".".to_string()]),
        ActionKind::Run => {
            if options.python_run_as_module {
                let Some(module) = options.python_module.as_deref().map(str::trim).filter(|m| !m.is_empty()) else {
                    return Err("Python module run requires a module name".to_string());
                };
                args.push("-m".to_string());
                args.push(module.to_string());
            } else {
                let Some(entry) = target.primary_entry_hint.as_ref() else {
                    return Err("Python Run requires a detected or pinned entry file".to_string());
                };
                args.push(entry.clone());
            }
        }
        _ => return Err(format!("{} is not executable for Python yet", action_label)),
    }

    Ok(ExecutionPlan {
        toolchain: Toolchain::Python,
        action,
        action_label: action_label.to_string(),
        program,
        args,
        cwd,
        env_sets,
        env_removals,
    })
}

fn python_invocation_parts(
    cwd: &Path,
    interpreter: Option<&Path>,
) -> (String, Vec<String>, Vec<(String, String)>, Vec<String>) {
    if let Some(interpreter) = interpreter {
        let mut env_sets = Vec::new();
        let mut env_removals = Vec::new();
        if let Some(venv_root) = interpreter.parent().and_then(Path::parent) {
            env_sets.push(("VIRTUAL_ENV".to_string(), venv_root.display().to_string()));
            let existing_path = std::env::var("PATH").unwrap_or_default();
            let interpreter_dir = interpreter.parent().unwrap_or(cwd);
            env_sets.push((
                "PATH".to_string(),
                prepend_path_entry(interpreter_dir, &existing_path),
            ));
            env_removals.push("PYTHONHOME".to_string());
        }
        return (
            interpreter.display().to_string(),
            Vec::new(),
            env_sets,
            env_removals,
        );
    }

    if cfg!(target_os = "windows") {
        ("py".to_string(), vec!["-3".to_string()], Vec::new(), Vec::new())
    } else {
        ("python3".to_string(), Vec::new(), Vec::new(), Vec::new())
    }
}

fn find_python_interpreter(root: &Path) -> Option<PathBuf> {
    let mut current = Some(root.to_path_buf());
    let mut depth = 0;
    while let Some(path) = current {
        for dir_name in ["venv", ".venv", "env"] {
            let venv_root = path.join(dir_name);
            for candidate in python_interpreter_candidates(&venv_root) {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        depth += 1;
        if depth > 4 {
            break;
        }
        current = path.parent().map(Path::to_path_buf);
    }
    None
}

fn prepend_path_entry(entry: &Path, existing_path: &str) -> String {
    if existing_path.is_empty() {
        entry.display().to_string()
    } else {
        format!("{}{}{}", entry.display(), python_path_separator(), existing_path)
    }
}

fn python_path_separator() -> char {
    if cfg!(target_os = "windows") {
        ';'
    } else {
        ':'
    }
}

fn python_interpreter_candidates(venv_root: &Path) -> Vec<PathBuf> {
    if cfg!(target_os = "windows") {
        vec![venv_root.join("Scripts").join("python.exe")]
    } else {
        vec![
            venv_root.join("bin").join("python3"),
            venv_root.join("bin").join("python"),
        ]
    }
}

fn spawn_output_reader<R>(
    stream: OutputStream,
    reader: R,
    runtime: Arc<Mutex<ExecutionRuntimeState>>,
) -> JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let reader = BufReader::new(reader);
        for line in reader.lines() {
            match line {
                Ok(line) => record_output_line(&runtime, stream, &line),
                Err(err) => record_output_line(&runtime, OutputStream::Stderr, &err.to_string()),
            }
        }
    })
}

fn record_output_line(runtime: &Arc<Mutex<ExecutionRuntimeState>>, stream: OutputStream, line: &str) {
    let Ok(mut state) = runtime.lock() else {
        return;
    };

    match stream {
        OutputStream::Stdout => push_output(&mut state.stdout, line),
        OutputStream::Stderr => push_output(&mut state.stderr, line),
    }

    if let Some(log) = state.log.as_mut() {
        let _ = writeln!(log, "{}", line);
    }

    for display in terminal_lines_for_output(line) {
        retain_error_line(&mut state, &display);
        state.terminal.push_back(display);
    }
    while state.terminal.len() > MAX_TERMINAL_LINES {
        state.terminal.pop_front();
        state.terminal_dropped += 1;
    }

    push_recent_output(&mut state.progress, stream, line);
    let ExecutionRuntimeState {
        parser,
        progress,
        ..
    } = &mut *state;
    parser.consume_line(stream, line, progress);
}

/// Final captured output plus the terminal progress snapshot (for unit/error
/// counts and the learned duration record).
struct FinalizedOutput {
    stdout: String,
    stderr: String,
    snapshot: ExecutionProgressSnapshot,
    /// Error diagnostics kept outside the rolling window.
    errors: Vec<TerminalLine>,
}

fn finalize_progress(
    runtime: &Arc<Mutex<ExecutionRuntimeState>>,
    success: bool,
    exit_code: Option<i32>,
    cancelled: bool,
) -> FinalizedOutput {
    let Ok(mut state) = runtime.lock() else {
        return FinalizedOutput {
            stdout: String::new(),
            stderr: String::new(),
            snapshot: ExecutionProgressSnapshot::default(),
            errors: Vec::new(),
        };
    };

    let ExecutionRuntimeState {
        parser,
        progress,
        stdout,
        stderr,
        errors,
        log,
        ..
    } = &mut *state;
    parser.finish(success, exit_code, cancelled, progress);
    // Flush before the report is handed to the UI, so "Open log" always shows a
    // complete file rather than whatever happened to be in the buffer.
    if let Some(log) = log.as_mut() {
        let _ = log.flush();
    }
    FinalizedOutput {
        stdout: stdout.clone(),
        stderr: stderr.clone(),
        snapshot: progress.clone(),
        errors: errors.clone(),
    }
}

/// Read the rolling window from absolute cursor `from`. Because the cursor is
/// absolute over the execution's entire output, eviction of already-consumed
/// lines cannot cause replays or gaps; `dropped` reports lines that were evicted
/// before the caller ever saw them.
fn drain_terminal(state: &ExecutionRuntimeState, from: usize) -> TerminalDrain {
    let first_available = state.terminal_dropped;
    let dropped = first_available.saturating_sub(from);
    let start = from.saturating_sub(first_available).min(state.terminal.len());
    let lines: Vec<TerminalLine> = state.terminal.iter().skip(start).cloned().collect();
    TerminalDrain {
        next_cursor: first_available + start + lines.len(),
        lines,
        dropped,
    }
}

/// Mirror error diagnostics into the reserved `errors` budget so the reason a
/// build failed survives however much warning output precedes it. An `error…`
/// header opens a block and the following snippet/`note:`/`help:` lines are
/// absorbed until the block budget runs out, a blank line closes it, or a new
/// warning/status header starts something else.
fn retain_error_line(state: &mut ExecutionRuntimeState, line: &TerminalLine) {
    if state.errors.len() >= MAX_RETAINED_ERROR_LINES {
        return;
    }
    match line.kind {
        TerminalLineKind::Error => state.error_block_remaining = ERROR_BLOCK_LINES,
        TerminalLineKind::Warning | TerminalLineKind::Info => {
            state.error_block_remaining = 0;
            return;
        }
        _ => {
            if state.error_block_remaining == 0 {
                return;
            }
            state.error_block_remaining -= 1;
            // A blank line separates an error from its `Caused by:` detail, so it
            // must not close the block — that detail ("Access is denied") is
            // usually the only actionable part. Blanks count against the budget
            // but are not themselves retained.
            if line.text.trim().is_empty() {
                return;
            }
        }
    }
    state.errors.push(line.clone());
}

fn push_output(output: &mut String, line: &str) {
    const MAX_OUTPUT_BYTES: usize = 64_000;

    output.push_str(line);
    output.push('\n');

    if output.len() > MAX_OUTPUT_BYTES {
        let keep_from = output.len() - MAX_OUTPUT_BYTES;
        let keep_from = (keep_from..output.len())
            .find(|&i| output.is_char_boundary(i))
            .unwrap_or(output.len());
        let tail = output[keep_from..].to_string();
        *output = format!("<truncated to last {} bytes>\n{}", MAX_OUTPUT_BYTES, tail);
    }
}

fn push_recent_output(progress: &mut ExecutionProgressSnapshot, stream: OutputStream, line: &str) {
    const MAX_RECENT_BYTES: usize = 1_200;
    let stream_label = match stream {
        OutputStream::Stdout => "stdout",
        OutputStream::Stderr => "stderr",
    };
    if !progress.recent_output.is_empty() {
        progress.recent_output.push('\n');
    }
    progress
        .recent_output
        .push_str(&format!("[{}] {}", stream_label, line.trim_end()));

    if progress.recent_output.len() > MAX_RECENT_BYTES {
        let keep_from = progress.recent_output.len() - MAX_RECENT_BYTES;
        let keep_from = (keep_from..progress.recent_output.len())
            .find(|&i| progress.recent_output.is_char_boundary(i))
            .unwrap_or(progress.recent_output.len());
        let tail = progress.recent_output[keep_from..].to_string();
        progress.recent_output = format!("...{}", tail);
    }
}

fn fallback_progress_snapshot(plan: &ExecutionPlan) -> ExecutionProgressSnapshot {
    initial_progress_snapshot(plan)
}

fn initial_progress_snapshot(plan: &ExecutionPlan) -> ExecutionProgressSnapshot {
    ExecutionProgressSnapshot {
        phase_label: format!("Starting {}", plan.action_label.to_lowercase()),
        detail_label: format!("{} {}", plan.program, plan.args.join(" ")).trim().to_string(),
        fraction: 0.04,
        ..Default::default()
    }
}

impl ProgressParser {
    fn new(plan: &ExecutionPlan) -> Self {
        let python_expected = if matches!(plan.toolchain, Toolchain::Python)
            && matches!(plan.action, ActionKind::Check)
        {
            count_python_files(&plan.cwd)
        } else {
            0
        };

        Self {
            toolchain: plan.toolchain.clone(),
            action: plan.action.clone(),
            rust_units: BTreeSet::new(),
            rust_completed: BTreeSet::new(),
            rust_expected: 0,
            total_tests: None,
            completed_tests: 0,
            python_units: BTreeSet::new(),
            python_expected,
        }
    }

    fn consume_line(
        &mut self,
        _stream: OutputStream,
        raw_line: &str,
        progress: &mut ExecutionProgressSnapshot,
    ) {
        let line = raw_line.trim();
        if line.is_empty() {
            return;
        }

        progress.detail_label = line.to_string();

        match self.toolchain {
            Toolchain::Rust => self.consume_rust_line(line, progress),
            Toolchain::Python => self.consume_python_line(line, progress),
            Toolchain::Mixed => {}
        }
    }

    fn finish(
        &mut self,
        success: bool,
        exit_code: Option<i32>,
        cancelled: bool,
        progress: &mut ExecutionProgressSnapshot,
    ) {
        if cancelled {
            progress.phase_label = "Cancelled".to_string();
            progress.detail_label = "Process tree stopped by user".to_string();
            progress.fraction = progress.fraction.max(0.98);
            return;
        }

        if success {
            progress.fraction = 1.0;
            progress.phase_label = match self.action {
                ActionKind::Run => "Process exited cleanly".to_string(),
                _ => "Completed".to_string(),
            };
            if progress.detail_label.is_empty() {
                progress.detail_label = "Execution complete".to_string();
            }
        } else {
            progress.phase_label = "Failed".to_string();
            progress.fraction = progress.fraction.max(0.96);
            if progress.detail_label.is_empty() {
                progress.detail_label = exit_code
                    .map(|code| format!("Process exited with code {}", code))
                    .unwrap_or_else(|| "Process failed".to_string());
            }
        }
    }

    fn consume_rust_line(&mut self, line: &str, progress: &mut ExecutionProgressSnapshot) {
        if let Some(message) = parse_cargo_json_message(line) {
            self.consume_rust_json_message(&message, progress);
            return;
        }

        count_rendered_diagnostic(line, progress);

        if let Some(total_tests) = parse_rust_test_total(line) {
            self.total_tests = Some(total_tests);
            set_progress(progress, 0.82, "Executing tests");
            return;
        }

        if parse_rust_test_result(line) {
            self.completed_tests += 1;
            let total = self.total_tests.unwrap_or(self.completed_tests.max(1));
            let fraction = 0.82 + 0.14 * (self.completed_tests as f32 / total as f32);
            set_progress(progress, fraction, "Executing tests");
            return;
        }

        if line.starts_with("Blocking waiting for file lock") {
            set_progress(progress, 0.10, "Waiting for cargo lock");
            return;
        }
        if line.starts_with("Updating ") {
            set_progress(progress, 0.12, "Resolving dependencies");
            return;
        }
        if line.starts_with("Downloading ") || line.starts_with("Downloaded ") {
            set_progress(progress, 0.18, "Fetching crates");
            return;
        }
        if line.starts_with("Checking ") {
            let count = register_rust_unit(&mut self.rust_units, line);
            let fraction = staged_fraction(0.32, 0.36, count);
            if let Some(name) = rust_status_crate_name(line) {
                progress.current_crate = name;
            }
            set_progress(progress, fraction, "Checking crates");
            return;
        }
        if line.starts_with("Compiling ") || line.starts_with("Building ") || line.starts_with("Fresh ") {
            let count = register_rust_unit(&mut self.rust_units, line);
            let fraction = staged_fraction(0.42, 0.36, count);
            if let Some(name) = rust_status_crate_name(line) {
                progress.current_crate = name;
            }
            set_progress(progress, fraction, "Compiling crates");
            return;
        }
        if line.starts_with("Running ") {
            let label = match self.action {
                ActionKind::Run => "Launching binary",
                ActionKind::Test => "Running test binaries",
                _ => "Running cargo task",
            };
            set_progress(progress, 0.90, label);
            return;
        }
        if line.starts_with("Finished ") {
            set_progress(progress, 0.97, "Finalizing artifacts");
            return;
        }
        if line.starts_with("error:") || line.contains("could not compile") {
            set_phase(progress, "Compile error");
        }
    }

    fn consume_rust_json_message(
        &mut self,
        message: &CargoJsonMessage,
        progress: &mut ExecutionProgressSnapshot,
    ) {
        match message.reason.as_str() {
            "compiler-artifact" => {
                let Some(package_id) = message.package_id.as_deref() else {
                    return;
                };
                let newly_completed = self.rust_completed.insert(package_id.to_string());
                let fresh = message.fresh.unwrap_or(false);
                if newly_completed && fresh {
                    progress.fresh_units += 1;
                }
                let completed = self.rust_completed.len();
                let label = if fresh {
                    "Reusing crates"
                } else {
                    "Compiling crates"
                };
                set_progress(progress, self.rust_compile_fraction(completed), label);
                let target_name = message
                    .target
                    .as_ref()
                    .map(|target| target.name.as_str())
                    .unwrap_or_else(|| short_package_id(package_id));
                let expected = self.rust_expected.max(completed).max(1);
                progress.current_crate = target_name.to_string();
                progress.units_completed = completed as u32;
                progress.units_total = expected as u32;
                progress.detail_label = format!("{} ({}/{})", target_name, completed, expected);
                if let Some(executable) = message.executable.as_deref() {
                    if Path::new(executable).is_file() {
                        progress.binary_path = Some(executable.to_string());
                    }
                }
            }
            "build-script-executed" => {
                set_progress(progress, 0.18, "Running build scripts");
                if let Some(package_id) = message.package_id.as_deref() {
                    progress.detail_label = format!("build script for {}", short_package_id(package_id));
                }
            }
            "compiler-message" => {
                if let Some(diagnostic) = message.message.as_ref() {
                    match diagnostic.level.as_deref() {
                        Some("error") => {
                            progress.errors_total += 1;
                            set_phase(progress, "Compile error");
                        }
                        Some("warning") => progress.warnings_total += 1,
                        _ => {}
                    }
                    if let Some(detail) = cargo_message_detail(diagnostic) {
                        progress.detail_label = detail;
                    }
                }
            }
            "build-finished" => {
                if message.success.unwrap_or(false) {
                    set_progress(progress, 0.97, "Finalizing artifacts");
                    let expected = self.rust_expected.max(self.rust_completed.len());
                    if expected > 0 {
                        progress.detail_label = format!(
                            "Compiled {} of {} crates",
                            self.rust_completed.len(),
                            expected
                        );
                    }
                } else {
                    set_progress(progress, 0.96, "Compile error");
                }
            }
            _ => {}
        }
    }

    fn rust_compile_fraction(&self, completed: usize) -> f32 {
        let ceiling = if matches!(self.action, ActionKind::Test) {
            0.80
        } else {
            0.94
        };

        if self.rust_expected > 0 {
            let expected = self.rust_expected.max(completed).max(1) as f32;
            0.22 + (ceiling - 0.22) * (completed as f32 / expected)
        } else {
            staged_fraction(0.30, ceiling - 0.30, completed)
        }
    }

    fn consume_python_line(&mut self, line: &str, progress: &mut ExecutionProgressSnapshot) {
        if matches!(self.action, ActionKind::Check) {
            if line.starts_with("Listing ") {
                set_progress(progress, 0.16, "Scanning packages");
                return;
            }
            if let Some(unit) = parse_python_compile_target(line) {
                self.python_units.insert(unit.to_string());
                let total = self.python_expected.max(self.python_units.len()).max(1);
                let fraction = 0.26 + 0.66 * (self.python_units.len() as f32 / total as f32);
                progress.current_crate = unit.rsplit(['/', '\\']).next().unwrap_or(unit).to_string();
                progress.units_completed = self.python_units.len() as u32;
                progress.units_total = total as u32;
                set_progress(progress, fraction.min(0.94), "Compiling modules");
                return;
            }
            if line.starts_with("***") || line.contains("Error") || line.contains("Can't list") {
                set_phase(progress, "Compile error");
            }
            return;
        }

        if matches!(self.action, ActionKind::Run) {
            set_progress(progress, 0.78, "Streaming script output");
        }
    }
}

fn set_progress(progress: &mut ExecutionProgressSnapshot, fraction: f32, phase_label: &str) {
    progress.fraction = progress.fraction.max(fraction.clamp(0.0, 1.0));
    progress.phase_label = phase_label.to_string();
}

/// Rename the phase without touching the fraction.
///
/// A diagnostic says *what* the build is doing, not *how far along* it is.
/// Cargo keeps compiling the rest of the graph after a crate or build script
/// fails, so ratcheting the bar on the first `error:` line pinned a build that
/// was seconds in and a third of the way through the crate graph at 96% for the
/// remaining minutes. Only `build-finished` (and `finish`) may move the bar to a
/// terminal position; until then the fraction stays driven by real progress.
fn set_phase(progress: &mut ExecutionProgressSnapshot, phase_label: &str) {
    progress.phase_label = phase_label.to_string();
}

fn staged_fraction(base: f32, span: f32, count: usize) -> f32 {
    if count == 0 {
        return base;
    }
    let count = count as f32;
    base + span * (count / (count + 6.0))
}

fn register_rust_unit(units: &mut BTreeSet<String>, line: &str) -> usize {
    units.insert(line.to_string());
    units.len()
}

fn parse_rust_test_total(line: &str) -> Option<usize> {
    let remainder = line.strip_prefix("running ")?;
    let total = remainder.split_whitespace().next()?;
    total.parse().ok()
}

fn parse_rust_test_result(line: &str) -> bool {
    line.starts_with("test ")
        && (line.ends_with(" ... ok")
            || line.ends_with(" ... FAILED")
            || line.ends_with(" ... ignored")
            || line.ends_with(" ... measured"))
}

fn parse_python_compile_target(line: &str) -> Option<&str> {
    let remainder = line.strip_prefix("Compiling '")?;
    remainder.split('\'').next()
}

fn rust_uses_json_messages(action: &ActionKind) -> bool {
    matches!(action, ActionKind::Check | ActionKind::Build | ActionKind::Test | ActionKind::Lint)
}

fn prime_progress_runtime(runtime: &Arc<Mutex<ExecutionRuntimeState>>, plan: &ExecutionPlan) {
    if !matches!(plan.toolchain, Toolchain::Rust) || !rust_uses_json_messages(&plan.action) {
        return;
    }

    if let Some(expected) = cargo_metadata_package_count(plan) {
        if let Ok(mut state) = runtime.lock() {
            state.parser.rust_expected = expected;
            state.progress.units_total = expected as u32;
            state.progress.phase_label = "Resolving crate graph".to_string();
            state.progress.detail_label = format!("Indexed {} crates from cargo metadata", expected);
            state.progress.fraction = state.progress.fraction.max(0.08);
        }
    }
}

/// Extract the crate name from a human-readable cargo status line such as
/// "Compiling serde v1.0.0 (…)" — used for the live current-crate label.
fn rust_status_crate_name(line: &str) -> Option<String> {
    for verb in ["Checking ", "Compiling ", "Building ", "Fresh "] {
        if let Some(rest) = line.strip_prefix(verb) {
            let name = rest.split_whitespace().next().unwrap_or("");
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// A `Command` for a toolchain program. A GUI app inherits the PATH Explorer
/// had at sign-in, so a toolchain installed since then (rustup, the Python
/// launcher) is invisible to it until the user signs out and back in. Fall back
/// to the tools' standard install folders, and put that folder on the child's
/// PATH as well so the tool finds its own siblings (rustc, rustup proxies).
fn tool_command(program: &str) -> Command {
    let Some(path) = locate_tool(program) else {
        return Command::new(program);
    };
    let mut command = Command::new(&path);
    if let Some(dir) = path.parent() {
        let existing = std::env::var("PATH").unwrap_or_default();
        command.env("PATH", prepend_path_entry(dir, &existing));
    }
    command
}

/// Where `program` lives when it is not on PATH; `None` when PATH already has
/// it (or it is an explicit path) and nothing needs overriding.
fn locate_tool(program: &str) -> Option<PathBuf> {
    if Path::new(program).components().count() > 1 || program_on_path(program) {
        return None;
    }
    fallback_tool_locations(program).into_iter().find(|path| path.is_file())
}

fn program_on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .any(|dir| dir.join(format!("{program}.exe")).is_file() || dir.join(program).is_file())
}

fn fallback_tool_locations(program: &str) -> Vec<PathBuf> {
    let env_dir = |name: &str| std::env::var_os(name).map(PathBuf::from);
    match program {
        "cargo" => [env_dir("CARGO_HOME"), env_dir("USERPROFILE").map(|home| home.join(".cargo"))]
            .into_iter()
            .flatten()
            .map(|cargo_home| cargo_home.join("bin").join("cargo.exe"))
            .collect(),
        "py" => [
            env_dir("LOCALAPPDATA").map(|dir| dir.join("Programs").join("Python").join("Launcher").join("py.exe")),
            env_dir("SystemRoot").map(|dir| dir.join("py.exe")),
        ]
        .into_iter()
        .flatten()
        .collect(),
        _ => Vec::new(),
    }
}

fn cargo_metadata_package_count(plan: &ExecutionPlan) -> Option<usize> {
    let mut command = tool_command("cargo");
    command
        .args(["metadata", "--format-version", "1", "--quiet"])
        .current_dir(&plan.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    #[cfg(windows)]
    apply_hidden_spawn_flags(&mut command);

    for (key, value) in &plan.env_sets {
        command.env(key, value);
    }
    for key in &plan.env_removals {
        command.env_remove(key);
    }

    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }

    let metadata: CargoMetadata = serde_json::from_slice(&output.stdout).ok()?;
    Some(metadata.packages.len().max(1))
}

fn parse_cargo_json_message(line: &str) -> Option<CargoJsonMessage> {
    if !line.starts_with('{') {
        return None;
    }

    serde_json::from_str(line).ok()
}

fn cargo_message_detail(diagnostic: &CargoJsonDiagnostic) -> Option<String> {
    diagnostic
        .rendered
        .as_deref()
        .and_then(first_non_empty_line)
        .map(str::to_string)
        .or_else(|| diagnostic.message.clone())
}

fn first_non_empty_line(rendered: &str) -> Option<&str> {
    // Skip rustc's decoration-only lines (caret underlines, gutter pipes,
    // arrows) so the detail label shows the message, not "^^^^^^^".
    rendered
        .lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line
                    .chars()
                    .all(|c| matches!(c, '^' | '~' | '|' | '-' | '>' | '=' | '_' | ' ' | '0'..='9'))
        })
}

fn short_package_id(package_id: &str) -> &str {
    package_id
        .rsplit('#')
        .next()
        .unwrap_or(package_id)
}

fn kill_process_tree(pid: u32) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut command = Command::new("taskkill");
        command
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW);
        let output = command.output().map_err(|err| err.to_string())?;
        if output.status.success() {
            return Ok(());
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("not found") || stderr.contains("There is no running instance") {
            return Ok(());
        }

        return Err(stderr.trim().to_string());
    }

    #[cfg(not(windows))]
    {
        let output = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|err| err.to_string())?;
        if output.status.success() {
            return Ok(());
        }

        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(windows)]
fn apply_hidden_spawn_flags(command: &mut Command) {
    // CREATE_NO_WINDOW is ignored when combined with DETACHED_PROCESS, which left
    // cargo console-less so its console children (rustc, rustup shims, build
    // scripts) each allocated a fresh *visible* console window. A hidden console
    // is inherited by the whole child tree instead.
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct CargoJsonMessage {
    reason: String,
    package_id: Option<String>,
    target: Option<CargoJsonTarget>,
    message: Option<CargoJsonDiagnostic>,
    success: Option<bool>,
    fresh: Option<bool>,
    executable: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CargoJsonTarget {
    name: String,
}

#[derive(Debug, Deserialize)]
struct CargoJsonDiagnostic {
    level: Option<String>,
    rendered: Option<String>,
    message: Option<String>,
}

fn count_python_files(root: &Path) -> usize {
    WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| !is_excluded_progress_dir(entry))
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("py"))
        .count()
}

fn is_excluded_progress_dir(entry: &DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }

    matches!(
        entry.file_name().to_str(),
        Some(
            "venv"
                | ".venv"
                | "env"
                | ".env"
                | "__pycache__"
                | ".git"
                | "site-packages"
                | "Scripts"
                | "Lib"
                | "lib"
                | "target"
                | "build"
                | "dist"
        )
    )
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("vaultforge-execution-{name}-{nanos}"))
    }

    fn rust_plan(action: ActionKind) -> ExecutionPlan {
        ExecutionPlan {
            toolchain: Toolchain::Rust,
            action,
            action_label: "Check".to_string(),
            program: "cargo".to_string(),
            args: vec!["check".to_string()],
            cwd: PathBuf::from("."),
            env_sets: Vec::new(),
            env_removals: Vec::new(),
        }
    }

    fn runtime_for(plan: &ExecutionPlan) -> Arc<Mutex<ExecutionRuntimeState>> {
        Arc::new(Mutex::new(ExecutionRuntimeState {
            progress: initial_progress_snapshot(plan),
            parser: ProgressParser::new(plan),
            stdout: String::new(),
            stderr: String::new(),
            terminal: VecDeque::new(),
            terminal_dropped: 0,
            errors: Vec::new(),
            error_block_remaining: 0,
            log: None,
        }))
    }

    #[test]
    fn terminal_window_keeps_the_tail_not_the_head() {
        let plan = rust_plan(ActionKind::Build);
        let runtime = runtime_for(&plan);
        let overflow = MAX_TERMINAL_LINES + 500;
        for i in 0..overflow {
            record_output_line(&runtime, OutputStream::Stderr, &format!("line {i}"));
        }

        let state = runtime.lock().unwrap();
        assert_eq!(state.terminal.len(), MAX_TERMINAL_LINES);
        assert_eq!(state.terminal_dropped, 500);
        // The regression: the old head-cap stopped recording at 8k lines, so a
        // build that emitted more than that lost its tail — which is exactly
        // where cargo prints the errors that explain the failure.
        assert_eq!(state.terminal.front().unwrap().text, "line 500");
        assert_eq!(
            state.terminal.back().unwrap().text,
            format!("line {}", overflow - 1)
        );
    }

    #[test]
    fn terminal_cursor_survives_window_eviction() {
        let plan = rust_plan(ActionKind::Build);
        let runtime = runtime_for(&plan);
        for i in 0..10 {
            record_output_line(&runtime, OutputStream::Stderr, &format!("line {i}"));
        }

        let first = drain_terminal(&runtime.lock().unwrap(), 0);
        assert_eq!(first.lines.len(), 10);
        assert_eq!(first.next_cursor, 10);
        assert_eq!(first.dropped, 0);

        // Overflow the window so lines the UI already consumed get evicted.
        for i in 10..(MAX_TERMINAL_LINES + 10) {
            record_output_line(&runtime, OutputStream::Stderr, &format!("line {i}"));
        }

        let second = drain_terminal(&runtime.lock().unwrap(), first.next_cursor);
        // Absolute cursor: resume is exact, nothing replayed and nothing lost.
        assert_eq!(second.lines.first().unwrap().text, "line 10");
        assert_eq!(second.next_cursor, MAX_TERMINAL_LINES + 10);
        assert_eq!(second.dropped, 0);
    }

    #[test]
    fn error_digest_survives_a_flood_of_warnings() {
        let plan = rust_plan(ActionKind::Build);
        let runtime = runtime_for(&plan);
        record_output_line(&runtime, OutputStream::Stderr, "error[E0308]: mismatched types");
        record_output_line(&runtime, OutputStream::Stderr, "   --> src/silk_cortex.rs:12:5");
        for i in 0..(MAX_TERMINAL_LINES + 1_000) {
            record_output_line(
                &runtime,
                OutputStream::Stderr,
                &format!("warning: method `passes_{i}` is never used"),
            );
        }

        let state = runtime.lock().unwrap();
        // The error has scrolled out of the rolling display window …
        assert!(!state.terminal.iter().any(|line| line.text.starts_with("error[E0308]")));
        // … but the reserved capture still explains why the build failed.
        assert!(state.errors.iter().any(|line| line.text.starts_with("error[E0308]")));
        assert!(state.errors.iter().any(|line| line.text.contains("silk_cortex.rs:12:5")));
        // Warnings must never crowd out that reserved budget.
        assert!(!state.errors.iter().any(|line| line.text.starts_with("warning:")));
    }

    /// The real Arachnia failure: cargo prints the error header, a blank line,
    /// then the `Caused by:` detail that is the only actionable part.
    #[test]
    fn error_digest_keeps_caused_by_detail_across_blank_line() {
        let plan = rust_plan(ActionKind::Build);
        let runtime = runtime_for(&plan);
        for line in [
            "error: failed to remove file `target\\release\\arachnia.exe`",
            "",
            "Caused by:",
            "  Access is denied. (os error 5)",
        ] {
            record_output_line(&runtime, OutputStream::Stderr, line);
        }

        let state = runtime.lock().unwrap();
        let digest: Vec<&str> = state.errors.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            digest,
            vec![
                "error: failed to remove file `target\\release\\arachnia.exe`",
                "Caused by:",
                "  Access is denied. (os error 5)",
            ]
        );
    }

    #[test]
    fn locked_output_file_produces_an_actionable_hint() {
        let plan = rust_plan(ActionKind::Build);
        let runtime = runtime_for(&plan);
        for line in [
            "error: failed to remove file `C:\\repo\\target\\release\\arachnia.exe`",
            "",
            "Caused by:",
            "  Access is denied. (os error 5)",
        ] {
            record_output_line(&runtime, OutputStream::Stderr, line);
        }
        let error_digest = runtime.lock().unwrap().errors.clone();

        let report = ExecutionReport {
            plan: rust_plan(ActionKind::Build),
            success: false,
            cancelled: false,
            exit_code: Some(101),
            stdout: String::new(),
            stderr: String::new(),
            finished_at_unix: 0,
            duration_ms: 386_400,
            unit_count: 0,
            fresh_units: 0,
            errors_total: 1,
            warnings_total: 992,
            error_digest,
            log_path: None,
        };

        let hint = failure_hint(&report).expect("locked-output failure should be explained");
        assert!(hint.contains("arachnia.exe"), "hint should name the process: {hint}");

        // An ordinary compile failure must not be dressed up with this hint.
        let plain = ExecutionReport {
            stderr: "error[E0308]: mismatched types".to_string(),
            error_digest: Vec::new(),
            ..report
        };
        assert!(failure_hint(&plain).is_none());
    }

    /// A machine without the toolchain gets told what to install, not a bare
    /// "program not found".
    #[test]
    fn missing_toolchain_names_what_to_install() {
        let report = ExecutionReport {
            plan: rust_plan(ActionKind::Build),
            success: false,
            cancelled: false,
            exit_code: None,
            stdout: String::new(),
            stderr: "program not found".to_string(),
            finished_at_unix: 0,
            duration_ms: 3,
            unit_count: 0,
            fresh_units: 0,
            errors_total: 0,
            warnings_total: 0,
            error_digest: Vec::new(),
            log_path: None,
        };
        assert!(failure_hint(&report).unwrap().contains("rustup.rs"));

        let mut python_plan = rust_plan(ActionKind::Run);
        python_plan.toolchain = Toolchain::Python;
        let python = ExecutionReport { plan: python_plan, ..report };
        assert!(failure_hint(&python).unwrap().contains("python.org"));
    }

    #[test]
    fn tools_already_on_path_are_not_overridden() {
        // `cmd` is always on PATH on Windows; an absolute path is never searched.
        #[cfg(windows)]
        assert!(locate_tool("cmd").is_none());
        assert!(locate_tool(r"C:\somewhere\cargo.exe").is_none());
        assert!(locate_tool("definitely-not-a-real-tool").is_none());
    }

    /// `json-render-diagnostics` emits no `compiler-message` JSON, so counts have
    /// to come from cargo's rendered text — but its roll-up lines summarize
    /// diagnostics already counted and must not inflate the totals.
    #[test]
    fn rendered_diagnostics_are_counted_without_double_counting_rollups() {
        let plan = rust_plan(ActionKind::Build);
        let mut parser = ProgressParser::new(&plan);
        let mut snapshot = initial_progress_snapshot(&plan);

        for line in [
            "warning: method `passes` is never used",
            "warning: function `discard_worktree` is never used",
            "error: failed to remove file `target\\release\\arachnia.exe`",
            "warning: `arachnia` (bin \"arachnia\") generated 992 warnings",
            "error: could not compile `arachnia` (bin \"arachnia\") due to 1 previous error",
        ] {
            parser.consume_line(OutputStream::Stderr, line, &mut snapshot);
        }

        assert_eq!(snapshot.warnings_total, 2);
        assert_eq!(snapshot.errors_total, 1);
        assert_eq!(
            snapshot.first_error.as_deref(),
            Some("error: failed to remove file `target\\release\\arachnia.exe`")
        );
    }

    /// Runs a real process end to end: the log must exist, name the command, and
    /// contain the captured output. The in-app terminal is a bounded window, so
    /// this file is the only complete record of a noisy build.
    #[test]
    fn execution_writes_a_complete_log_to_disk() {
        let mut plan = rust_plan(ActionKind::Check);
        plan.args = vec!["--version".to_string()];
        plan.cwd = std::env::current_dir().expect("cwd");

        let running = spawn_execution(plan, None);
        let log_path = running.log_path().map(Path::to_path_buf).expect("log path");

        // Generous budget: `cargo metadata` + `cargo --version` take ~1s alone
        // but share the machine with the rest of the parallel test run.
        let mut report = None;
        for _ in 0..1_500 {
            if let Some(finished) = running.try_finish() {
                report = Some(finished);
                break;
            }
            thread::sleep(std::time::Duration::from_millis(20));
        }
        let report = report.expect("cargo --version should finish promptly");

        assert!(report.success, "cargo --version should succeed");
        assert_eq!(report.log_path.as_deref(), Some(log_path.as_path()));

        let contents = std::fs::read_to_string(&log_path).expect("read log");
        assert!(
            contents.contains("# cargo --version"),
            "log should record the exact command: {contents}"
        );
        assert!(
            contents.contains("cargo 1."),
            "log should capture the process output: {contents}"
        );

        let _ = std::fs::remove_file(&log_path);
    }

    #[test]
    fn rust_progress_advances_from_checking_to_finish() {
        let plan = rust_plan(ActionKind::Check);
        let mut parser = ProgressParser::new(&plan);
        let mut snapshot = initial_progress_snapshot(&plan);

        parser.consume_line(
            OutputStream::Stderr,
            "Checking serde v1.0.0",
            &mut snapshot,
        );
        assert_eq!(snapshot.phase_label, "Checking crates");
        assert!(snapshot.fraction > 0.32);

        parser.consume_line(
            OutputStream::Stderr,
            "Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.21s",
            &mut snapshot,
        );
        assert_eq!(snapshot.phase_label, "Finalizing artifacts");
        assert!(snapshot.fraction >= 0.97);
    }

    #[test]
    fn rust_json_progress_counts_compiled_crates() {
        let plan = rust_plan(ActionKind::Build);
        let mut parser = ProgressParser::new(&plan);
        parser.rust_expected = 4;
        let mut snapshot = initial_progress_snapshot(&plan);

        parser.consume_line(
            OutputStream::Stdout,
            r#"{"reason":"compiler-artifact","package_id":"registry+https://github.com/rust-lang/crates.io-index#serde@1.0.219","target":{"name":"serde"},"fresh":false}"#,
            &mut snapshot,
        );

        assert_eq!(snapshot.phase_label, "Compiling crates");
        assert_eq!(snapshot.detail_label, "serde (1/4)");
        assert!(snapshot.fraction > 0.22);

        parser.consume_line(
            OutputStream::Stdout,
            r#"{"reason":"build-finished","success":true}"#,
            &mut snapshot,
        );

        assert_eq!(snapshot.phase_label, "Finalizing artifacts");
        assert!(snapshot.fraction >= 0.97);
    }

    #[test]
    fn python_compile_progress_uses_compileall_lines() {
        let plan = ExecutionPlan {
            toolchain: Toolchain::Python,
            action: ActionKind::Check,
            action_label: "Check".to_string(),
            program: "py".to_string(),
            args: vec!["-3".to_string(), "-m".to_string(), "compileall".to_string(), ".".to_string()],
            cwd: PathBuf::from("."),
            env_sets: Vec::new(),
            env_removals: Vec::new(),
        };
        let mut parser = ProgressParser {
            toolchain: Toolchain::Python,
            action: ActionKind::Check,
            rust_units: BTreeSet::new(),
            rust_completed: BTreeSet::new(),
            rust_expected: 0,
            total_tests: None,
            completed_tests: 0,
            python_units: BTreeSet::new(),
            python_expected: 4,
        };
        let mut snapshot = initial_progress_snapshot(&plan);

        parser.consume_line(OutputStream::Stdout, "Listing '.'...", &mut snapshot);
        assert_eq!(snapshot.phase_label, "Scanning packages");

        parser.consume_line(OutputStream::Stdout, "Compiling 'app.py'...", &mut snapshot);
        assert_eq!(snapshot.phase_label, "Compiling modules");
        assert!(snapshot.fraction > 0.26);

        let first_fraction = snapshot.fraction;
        parser.consume_line(OutputStream::Stdout, "Compiling 'pkg/mod.py'...", &mut snapshot);
        assert!(snapshot.fraction > first_fraction);
        assert_eq!(snapshot.detail_label, "Compiling 'pkg/mod.py'...");
    }

    #[test]
    fn python_invocation_prefixes_virtualenv_bin_dir() {
        let interpreter = if cfg!(target_os = "windows") {
            PathBuf::from(r"C:\vaultforge\.venv\Scripts\python.exe")
        } else {
            PathBuf::from("/tmp/vaultforge/.venv/bin/python3")
        };

        let (_, _, env_sets, env_removals) = python_invocation_parts(Path::new("."), Some(&interpreter));
        let path_value = env_sets
            .iter()
            .find_map(|(key, value)| (key == "PATH").then_some(value))
            .unwrap();

        assert!(path_value.starts_with(&interpreter.parent().unwrap().display().to_string()));
        assert!(env_removals.iter().any(|key| key == "PYTHONHOME"));
    }

    #[test]
    fn find_python_interpreter_walks_up_to_virtualenv_root() {
        let root = unique_temp_dir("python-venv");
        let workspace = root.join("workspace");
        let nested = workspace.join("src").join("pkg");
        let interpreter = if cfg!(target_os = "windows") {
            workspace.join(".venv").join("Scripts").join("python.exe")
        } else {
            workspace.join(".venv").join("bin").join("python3")
        };

        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(interpreter.parent().unwrap()).unwrap();
        std::fs::write(&interpreter, b"python").unwrap();

        let detected = find_python_interpreter(&nested).unwrap();
        assert_eq!(detected, interpreter);

        let _ = std::fs::remove_dir_all(root);
    }

    fn target_with(toolchain: Toolchain, actions: &[&str], entry: Option<&str>) -> WorkspaceTarget {
        WorkspaceTarget {
            id: "build-options-target".to_string(),
            root_path: std::env::temp_dir(),
            project_name: "demo".to_string(),
            toolchain,
            confidence: 100,
            last_modified_unix: 0,
            markers: vec!["Cargo.toml".to_string()],
            available_actions: actions.iter().map(|action| action.to_string()).collect(),
            primary_entry_hint: entry.map(str::to_string),
            last_built_at: None,
            last_launched_at: None,
            pinned: false,
            ranking_score: 0,
        }
    }

    fn args_contain_pair(args: &[String], first: &str, second: &str) -> bool {
        args.windows(2).any(|pair| pair[0] == first && pair[1] == second)
    }

    #[test]
    fn rust_release_profile_appends_release_flag() {
        let target = target_with(Toolchain::Rust, &["Check", "Build"], None);
        let options = BuildOptions { rust_release: true, ..Default::default() };

        let plan = build_plan(&target, "Build", &options).unwrap();
        assert_eq!(plan.args.first().map(String::as_str), Some("build"));
        assert!(plan.args.iter().any(|arg| arg == "--release"));
    }

    #[test]
    fn rust_dev_profile_has_no_release_flag() {
        let target = target_with(Toolchain::Rust, &["Check", "Build"], None);
        let plan = build_plan(&target, "Build", &BuildOptions::default()).unwrap();
        assert!(!plan.args.iter().any(|arg| arg == "--release"));
    }

    #[test]
    fn rust_features_and_target_triple_become_flags() {
        let target = target_with(Toolchain::Rust, &["Check", "Build"], None);
        let options = BuildOptions {
            rust_features: vec!["serde".to_string(), "tokio".to_string()],
            rust_target: Some("x86_64-pc-windows-msvc".to_string()),
            ..Default::default()
        };

        let plan = build_plan(&target, "Build", &options).unwrap();
        assert!(args_contain_pair(&plan.args, "--target", "x86_64-pc-windows-msvc"));
        assert!(args_contain_pair(&plan.args, "--features", "serde,tokio"));
    }

    #[test]
    fn rust_clean_omits_features_but_keeps_release() {
        let target = target_with(Toolchain::Rust, &["Clean"], None);
        let options = BuildOptions {
            rust_release: true,
            rust_features: vec!["serde".to_string()],
            ..Default::default()
        };

        let plan = build_plan(&target, "Clean", &options).unwrap();
        assert!(plan.args.iter().any(|arg| arg == "--release"));
        assert!(!plan.args.iter().any(|arg| arg == "--features"));
    }

    #[test]
    fn python_module_run_uses_dash_m() {
        let target = target_with(Toolchain::Python, &["Check"], None);
        let options = BuildOptions {
            python_run_as_module: true,
            python_module: Some("package.main".to_string()),
            ..Default::default()
        };

        let plan = build_plan(&target, "Run", &options).unwrap();
        assert!(args_contain_pair(&plan.args, "-m", "package.main"));
    }

    #[test]
    fn python_module_run_requires_a_module_name() {
        let target = target_with(Toolchain::Python, &["Check"], None);
        let options = BuildOptions { python_run_as_module: true, ..Default::default() };

        assert!(build_plan(&target, "Run", &options).is_err());
    }

    #[test]
    fn python_file_run_still_uses_detected_entry() {
        let target = target_with(Toolchain::Python, &["Check", "Run"], Some("app.py"));
        let plan = build_plan(&target, "Run", &BuildOptions::default()).unwrap();
        assert!(plan.args.iter().any(|arg| arg == "app.py"));
    }

    #[test]
    fn time_based_fraction_reaches_most_of_the_way_at_the_estimate() {
        let est = 10_000;
        // Near the start the bar is low, at the estimate it is ~95%, and it never
        // reaches 1.0 while running (caps at 0.99).
        assert!(build_progress_fraction(0, Some(est), 0, 0) < 0.05);
        let at_estimate = build_progress_fraction(est, Some(est), 0, 0);
        assert!(at_estimate > 0.9 && at_estimate < 0.96, "got {at_estimate}");
        assert!(build_progress_fraction(est * 10, Some(est), 0, 0) <= 0.99);
    }

    #[test]
    fn crate_ratio_is_the_fallback_without_a_learned_duration() {
        assert_eq!(build_progress_fraction(0, None, 0, 0), 0.0);
        let half = build_progress_fraction(0, None, 5, 10);
        assert!((half - 0.5).abs() < 1e-6, "got {half}");
        // Never pins to a full bar before the build actually finishes.
        assert!(build_progress_fraction(0, None, 10, 10) <= 0.99);
    }

    #[test]
    fn a_mid_flight_error_renames_the_phase_without_jumping_the_bar() {
        let plan = rust_plan(ActionKind::Build);
        let mut parser = ProgressParser::new(&plan);
        let mut snapshot = initial_progress_snapshot(&plan);

        parser.consume_line(OutputStream::Stderr, "Compiling serde v1.0.219", &mut snapshot);
        let before = snapshot.fraction;

        // A failed build script does not stop cargo — it keeps compiling the rest
        // of the graph, so the bar must keep reflecting that work.
        parser.consume_line(
            OutputStream::Stderr,
            "error: failed to run custom build command for `audiopus_sys v0.2.2`",
            &mut snapshot,
        );
        assert_eq!(snapshot.phase_label, "Compile error");
        assert_eq!(snapshot.errors_total, 1);
        assert_eq!(snapshot.fraction, before, "an error must not advance the bar");

        parser.consume_line(OutputStream::Stderr, "Compiling winit v0.29.15", &mut snapshot);
        assert_eq!(snapshot.phase_label, "Compiling crates");
        assert!(snapshot.fraction > before);
        assert!(snapshot.fraction < 0.9, "got {}", snapshot.fraction);

        // Only the terminal message parks the bar near the end.
        parser.consume_line(
            OutputStream::Stdout,
            r#"{"reason":"build-finished","success":false}"#,
            &mut snapshot,
        );
        assert_eq!(snapshot.phase_label, "Compile error");
        assert!(snapshot.fraction >= 0.96);
    }
}
