use crate::discovery::{
    SearchChip, SearchRequest, Toolchain, WorkspaceCatalog, WorkspaceTarget,
    build_workspace_catalog, load_cached_catalog, query_catalog, save_cached_catalog,
};
use crate::execution::{
    BuildOptions, ExecutionPlan, ExecutionReport, RunningExecution, TerminalLine, TerminalLineKind,
    build_plan, spawn_execution,
};
use crate::history::SharedHistory;
use crate::theme;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Exact height of one painted catalog row. `show_rows` virtualizes on this, so
/// rows must never be taller than it (the old framed rows were, which made the
/// scrollbar drift and the last entries hard to reach).
const CATALOG_ROW_HEIGHT: f32 = 62.0;
const TERMINAL_FONT_SIZE: f32 = 12.0;

/// Non-ASCII symbols the UI draws; checked against the bundled fonts by test.
#[cfg(test)]
const UI_SYMBOLS: &str = "\u{00B7}\u{2014}\u{2026}\u{2500}\u{2716}\u{2714}\u{25A0}\u{26A0}\u{2192}\u{2605}\u{25C2}\u{25B8}\u{25B6}";

/// This app's semantic colours, mapped onto the shared ThoughtSlayer palette in
/// `theme` — the one source of truth for VaultForge and VaultLauncher.
mod palette {
    use crate::theme;
    use eframe::egui::Color32;

    pub const BG: Color32 = theme::COLOR_BG;
    pub const SHELL: Color32 = theme::COLOR_BG_ALT;
    pub const SHELL_BORDER: Color32 = Color32::from_rgb(40, 36, 22);
    pub const BORDER_SOFT: Color32 = theme::COLOR_RULE;
    pub const TEXT: Color32 = theme::COLOR_TEXT;
    pub const DIM: Color32 = theme::COLOR_TEXT_WARM;
    pub const MUTED: Color32 = theme::COLOR_TEXT_DIM;
    pub const ACCENT: Color32 = theme::COLOR_GOLD;
    pub const ACCENT_SOFT: Color32 = theme::COLOR_GROUP_BG;
    pub const ACCENT_STRONG: Color32 = theme::COLOR_GOLD_BRIGHT;
    pub const SUCCESS: Color32 = theme::EVIDENCE_POS;
    pub const ERROR: Color32 = theme::SEV_CRITICAL;
    pub const ERROR_TEXT: Color32 = theme::EVIDENCE_NEG;
    pub const ERROR_DIM: Color32 = theme::COLOR_FOE_BG;
    pub const WARN: Color32 = theme::SEV_MEDIUM;
    pub const WARN_DIM: Color32 = theme::SEV_MEDIUM_BG;
    pub const RUST: Color32 = theme::SEV_HIGH;
    pub const PYTHON: Color32 = theme::COLOR_LISTEN;
    pub const MIXED: Color32 = theme::COLOR_ROUTINE;
}

pub struct VaultForgeApp {
    scan_root: String,
    search_query: String,
    active_chips: Vec<SearchChip>,
    catalog: WorkspaceCatalog,
    targets: Vec<WorkspaceTarget>,
    selected_index: Option<usize>,
    pending_action: Option<String>,
    running_execution: Option<RunningExecution>,
    history: SharedHistory,
    status_line: String,
    broker_status: Option<crate::broker::BrokerStatusSnapshot>,
    last_broker_poll: Instant,
    broker_poll_deadline: Instant,
    pending_window_center: bool,
    request_search_focus: bool,
    scroll_catalog_to_selection: bool,
    /// Accumulated in-app terminal lines across executions (a rolling log).
    terminal_lines: Vec<TerminalLine>,
    /// How many of the current execution's runtime terminal lines are drained.
    terminal_seen: usize,
    preferred_python_entry_input: String,
    preferred_python_entry_target_id: Option<String>,
    pending_catalog: Option<Receiver<WorkspaceCatalog>>,
    rust_release: bool,
    build_features_input: String,
    build_target_input: String,
    python_run_as_module: bool,
    python_module_input: String,
    build_options_target_id: Option<String>,
    /// Pre-flight result awaiting a decision: processes holding this workspace's
    /// build output open, which would fail the link step.
    lock_conflict: Option<LockConflict>,
    /// Terminal view mode. Problems-only makes an 11k-line log navigable.
    terminal_problems_only: bool,
    /// Index into `terminal_lines` to scroll to on the next frame.
    terminal_jump_to: Option<usize>,
    /// Position within the problem list that the jump buttons last landed on.
    terminal_problem_cursor: usize,
    /// Log file of the most recently finished execution.
    last_log_path: Option<PathBuf>,
    /// Cached diagnostic trend for the selected workspace + action.
    build_health: Option<BuildHealth>,
    /// Key the cached `build_health` was computed for, so the lookup runs on
    /// change rather than every frame.
    build_health_key: Option<String>,
    title_bar: theme::TitleBar,
    /// Hotkey/shortcut integration result, computed off the UI thread.
    startup_status: Option<Receiver<Option<String>>>,
    /// Why hotkey integration failed, if it did — shown on the broker indicator.
    broker_setup_error: Option<String>,
    scan_started_at: Instant,
    /// Catalog scroll offset to apply next frame, for a selection that is
    /// outside the rows `show_rows` rendered.
    pending_catalog_offset: Option<f32>,
    settings: crate::history::Settings,
}

/// Processes that must be cleared before an action can succeed.
struct LockConflict {
    action_label: String,
    processes: Vec<crate::broker::RunningProcess>,
}

/// Diagnostic counts from the last run of the selected action.
struct BuildHealth {
    warnings: u32,
    errors: u32,
    /// Change in warnings since the run before, when there is one.
    delta: Option<i64>,
}

impl VaultForgeApp {
    pub fn new(startup_status: Receiver<Option<String>>, center_window: bool) -> Self {
        let history = SharedHistory::load();
        let settings = crate::history::Settings::load();
        let now = Instant::now();
        let mut app = Self {
            scan_root: default_scan_root(settings.scan_root.as_deref()).display().to_string(),
            search_query: String::new(),
            active_chips: Vec::new(),
            catalog: WorkspaceCatalog::default(),
            targets: Vec::new(),
            selected_index: None,
            pending_action: None,
            running_execution: None,
            history,
            status_line: "Ready".to_string(),
            broker_status: crate::broker::current_status_snapshot(),
            last_broker_poll: now,
            broker_poll_deadline: now + Duration::from_secs(3),
            pending_window_center: center_window,
            request_search_focus: false,
            scroll_catalog_to_selection: false,
            terminal_lines: Vec::new(),
            terminal_seen: 0,
            preferred_python_entry_input: String::new(),
            preferred_python_entry_target_id: None,
            pending_catalog: None,
            rust_release: false,
            build_features_input: String::new(),
            build_target_input: String::new(),
            python_run_as_module: false,
            python_module_input: String::new(),
            build_options_target_id: None,
            lock_conflict: None,
            terminal_problems_only: false,
            terminal_jump_to: None,
            terminal_problem_cursor: 0,
            last_log_path: None,
            build_health: None,
            build_health_key: None,
            title_bar: theme::TitleBar::new(),
            startup_status: Some(startup_status),
            broker_setup_error: None,
            scan_started_at: now,
            pending_catalog_offset: None,
            settings,
        };
        // Show the last scan immediately; the fresh one replaces it when done.
        if let Some(cached) = load_cached_catalog(Path::new(&app.scan_root)) {
            app.catalog = cached;
            app.refresh_targets_for_selection(None);
        }
        app.rebuild_catalog();
        app
    }

    fn poll_broker_status(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if let Some(receiver) = self.startup_status.as_ref() {
            match receiver.try_recv() {
                Ok(message) => {
                    self.startup_status = None;
                    // The broker may have only just been spawned; watch its
                    // status file settle.
                    self.broker_poll_deadline = now + Duration::from_secs(3);
                    self.broker_setup_error =
                        message.filter(|message| message.starts_with("Hotkey broker setup skipped"));
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(250)),
                Err(TryRecvError::Disconnected) => self.startup_status = None,
            }
        }
        if now >= self.last_broker_poll + Duration::from_millis(250) {
            self.broker_status = crate::broker::current_status_snapshot();
            self.last_broker_poll = now;
        }

        if now < self.broker_poll_deadline {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn center_window_once(&mut self, ctx: &egui::Context) {
        if !self.pending_window_center {
            return;
        }

        if let Some(command) = egui::ViewportCommand::center_on_screen(ctx) {
            ctx.send_viewport_cmd(command);
            self.pending_window_center = false;
        } else {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    fn rebuild_catalog(&mut self) {
        let trimmed = self.scan_root.trim().trim_matches('"').to_string();
        if trimmed != self.scan_root {
            self.scan_root = trimmed;
        }
        if !Path::new(&self.scan_root).is_dir() {
            self.status_line = format!("Folder not found: {} \u{2014} type another path or press BROWSE", self.scan_root);
            return;
        }
        let scan_root = self.scan_root.clone();
        let previous = self.catalog.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending_catalog = Some(rx);
        self.scan_started_at = Instant::now();
        self.status_line = if self.catalog.len() == 0 {
            "Scanning workspaces\u{2026}".to_string()
        } else {
            format!("Showing last scan of {} workspaces \u{00B7} refreshing\u{2026}", self.catalog.len())
        };

        std::thread::spawn(move || {
            let catalog = build_workspace_catalog(Path::new(&scan_root), Some(&previous));
            save_cached_catalog(&catalog);
            let _ = tx.send(catalog);
        });
    }

    fn poll_catalog(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.pending_catalog.as_ref() else {
            return;
        };

        match rx.try_recv() {
            Ok(catalog) => {
                let selected_id = self.selected_target().map(|t| t.id.clone());
                self.catalog = catalog;
                self.pending_catalog = None;
                self.refresh_targets_for_selection(selected_id);
                self.status_line = format!(
                    "Indexed {} workspaces in {:.1}s \u{00B7} showing {}",
                    self.catalog.len(),
                    self.scan_started_at.elapsed().as_secs_f32(),
                    self.targets.len()
                );
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending_catalog = None;
                self.status_line = "Scan thread terminated unexpectedly".to_string();
            }
        }
    }

    fn refresh_targets(&mut self) {
        let selected_id = self.selected_target().map(|target| target.id.clone());
        self.refresh_targets_for_selection(selected_id);
        self.status_line = format!(
            "Showing {} of {} indexed workspaces",
            self.targets.len(),
            self.catalog.len()
        );
    }

    fn refresh_targets_for_selection(&mut self, selected_id: Option<String>) {
        let request = SearchRequest::from_input(&self.search_query, &self.active_chips);
        self.targets = query_catalog(&self.catalog, &self.history, &request);
        self.selected_index = selected_id
            .and_then(|id| self.targets.iter().position(|target| target.id == id))
            .or_else(|| (!self.targets.is_empty()).then_some(0));
        if let Some(selected_action) = self.pending_action.as_ref() {
            let still_available = self
                .selected_target()
                .map(|target| target.available_actions.iter().any(|action| action == selected_action))
                .unwrap_or(false);
            if !still_available {
                self.pending_action = None;
            }
        }
    }

    fn selected_target(&self) -> Option<&WorkspaceTarget> {
        self.selected_index.and_then(|index| self.targets.get(index))
    }

    fn toggle_pin_selected(&mut self) {
        let Some(target) = self.selected_target().cloned() else {
            return;
        };
        let new_state = !target.pinned;
        self.history.set_pinned(&target.root_path, new_state);
        match self.history.save() {
            Ok(()) => {
                let status = if new_state {
                    format!("Pinned {}", target.project_name)
                } else {
                    format!("Unpinned {}", target.project_name)
                };
                self.refresh_targets();
                self.scroll_catalog_to_selection = true;
                self.status_line = status;
            }
            Err(err) => {
                self.status_line = format!("Failed to save shared history: {err}");
            }
        }
    }

    fn saved_preferred_python_entry(&self, target: &WorkspaceTarget) -> Option<String> {
        self.history
            .entry_for(&target.root_path)
            .and_then(|entry| entry.preferred_python_entry.clone())
    }

    fn sync_preferred_python_entry_input(&mut self, target: &WorkspaceTarget) {
        if target.toolchain != Toolchain::Python {
            return;
        }
        if self.preferred_python_entry_target_id.as_deref() == Some(target.id.as_str()) {
            return;
        }

        self.preferred_python_entry_target_id = Some(target.id.clone());
        self.preferred_python_entry_input = self.saved_preferred_python_entry(target).unwrap_or_default();
    }

    fn save_preferred_python_entry(&mut self, target: &WorkspaceTarget) {
        let raw_input = self.preferred_python_entry_input.trim();
        let Some(storage_value) = preferred_python_entry_storage_value(&target.root_path, raw_input) else {
            self.status_line = "Preferred Python entry must be a .py file inside the selected workspace".to_string();
            return;
        };

        self.history
            .set_preferred_python_entry(&target.root_path, Some(storage_value.clone()));
        match self.history.save() {
            Ok(()) => {
                self.preferred_python_entry_input = storage_value;
                self.refresh_targets();
                self.status_line = format!("Saved Python entry for {}", target.project_name);
            }
            Err(err) => {
                self.status_line = format!("Failed to save preferred Python entry: {err}");
            }
        }
    }

    fn clear_preferred_python_entry(&mut self, target: &WorkspaceTarget) {
        self.history.set_preferred_python_entry(&target.root_path, None);
        match self.history.save() {
            Ok(()) => {
                self.preferred_python_entry_input.clear();
                self.refresh_targets();
                self.status_line = format!("Cleared Python entry for {}", target.project_name);
            }
            Err(err) => {
                self.status_line = format!("Failed to clear preferred Python entry: {err}");
            }
        }
    }

    /// Snapshot the UI's build-option inputs into the typed struct the executor consumes.
    fn current_build_options(&self) -> BuildOptions {
        let rust_features = self
            .build_features_input
            .split([',', ' ', '\t', '\n'])
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(str::to_string)
            .collect();
        let rust_target = {
            let trimmed = self.build_target_input.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        };
        let python_module = {
            let trimmed = self.python_module_input.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        };

        BuildOptions {
            rust_release: self.rust_release,
            rust_features,
            rust_target,
            python_run_as_module: self.python_run_as_module,
            python_module,
        }
    }

    /// Actions to show in the stack for a target. Mirrors `target.available_actions`, but
    /// surfaces Python "Run" when module-mode is armed even without a detected entry file.
    fn display_actions(&self, target: &WorkspaceTarget) -> Vec<String> {
        let mut actions = target.available_actions.clone();
        let module_run_armed = target.toolchain == Toolchain::Python
            && self.python_run_as_module
            && !self.python_module_input.trim().is_empty();
        if module_run_armed && !actions.iter().any(|action| action == "Run") {
            let insert_at = actions
                .iter()
                .position(|action| action == "Check")
                .map_or(actions.len(), |index| index + 1);
            actions.insert(insert_at, "Run".to_string());
        }
        actions
    }

    /// Reset the per-target option inputs (features, target triple, Python module) when the
    /// selection changes. The Rust profile is intentionally global and is not reset here.
    fn sync_build_options_for_target(&mut self, target: &WorkspaceTarget) {
        if self.build_options_target_id.as_deref() == Some(target.id.as_str()) {
            return;
        }
        self.build_options_target_id = Some(target.id.clone());
        self.lock_conflict = None;
        self.build_features_input.clear();
        self.build_target_input.clear();
        self.python_run_as_module = false;
        self.python_module_input.clear();
    }

    fn reveal_selected(&mut self) {
        let Some(target) = self.selected_target() else {
            return;
        };
        if reveal_in_explorer(&target.root_path).is_err() {
            self.status_line = "Failed to open Explorer for selected workspace".to_string();
        }
    }

    fn poll_running_execution(&mut self) {
        let (drain, report) = {
            let Some(running) = self.running_execution.as_ref() else {
                return;
            };
            (running.terminal_lines_since(self.terminal_seen), running.try_finish())
        };
        self.terminal_seen = drain.next_cursor;
        if drain.dropped > 0 {
            self.extend_terminal(vec![TerminalLine::new(
                format!("<{} earlier lines dropped>", drain.dropped),
                TerminalLineKind::Info,
            )]);
        }
        self.extend_terminal(drain.lines);

        let Some(report) = report else {
            return;
        };

        let summary = report.summary();
        self.last_log_path = report.log_path.clone();
        self.push_terminal_summary(&report);
        let history_error = self.record_history_from_report(&report).err();
        self.running_execution = None;
        self.refresh_targets();
        self.status_line = if let Some(err) = history_error {
            format!("{}; {}", summary, err)
        } else {
            summary
        };
    }

    fn extend_terminal(&mut self, lines: Vec<TerminalLine>) {
        // Rolling log across executions; cap keeps per-frame rendering bounded.
        const MAX_APP_TERMINAL_LINES: usize = 4_000;
        self.terminal_lines.extend(lines);
        if self.terminal_lines.len() > MAX_APP_TERMINAL_LINES {
            let excess = self.terminal_lines.len() - MAX_APP_TERMINAL_LINES;
            self.terminal_lines.drain(..excess);
        }
    }

    fn push_terminal_summary(&mut self, report: &ExecutionReport) {
        // A spawn failure (bad program, missing cwd) produces no live output —
        // the error only lands in the report's stderr, so echo it here.
        if !report.success && self.terminal_seen == 0 && !report.stderr.trim().is_empty() {
            let lines = report.stderr.lines().map(TerminalLine::classified).collect();
            self.extend_terminal(lines);
        }

        // Long builds emit far more output than any window can hold, and cargo
        // keeps compiling (and warning) after the first error. Re-print the
        // reserved error capture directly above the summary so the reason a
        // build failed is always on screen, whatever the log volume was.
        if !report.success && !report.cancelled && !report.error_digest.is_empty() {
            let header = format!(
                "\u{2500}\u{2500} {} \u{2500}\u{2500}",
                match report.errors_total {
                    0 => "failure detail".to_string(),
                    1 => "1 error".to_string(),
                    n => format!("{} errors", n),
                }
            );
            self.extend_terminal(vec![
                TerminalLine::new("", TerminalLineKind::Output),
                TerminalLine::new(header, TerminalLineKind::Failure),
            ]);
            self.extend_terminal(report.error_digest.clone());
        }

        let elapsed = format!("{:.1}s", report.duration_ms as f64 / 1000.0);
        let (text, kind) = if report.cancelled {
            (format!("\u{2716} {} after {}", report.summary(), elapsed), TerminalLineKind::Failure)
        } else if report.success {
            let mut text = format!("\u{2714} {} in {}", report.summary(), elapsed);
            if report.warnings_total > 0 {
                text.push_str(&format!(" \u{00B7} {}", count(report.warnings_total, "warning")));
            }
            (text, TerminalLineKind::Success)
        } else {
            let mut text = format!("\u{2716} {} after {}", report.summary(), elapsed);
            if report.errors_total > 0 {
                text.push_str(&format!(" \u{00B7} {}", count(report.errors_total, "error")));
            }
            (text, TerminalLineKind::Failure)
        };
        let mut tail = vec![TerminalLine::new(text, kind)];
        if let Some(hint) = crate::execution::failure_hint(report) {
            tail.push(TerminalLine::new(hint, TerminalLineKind::Warning));
        }
        tail.push(TerminalLine::new("", TerminalLineKind::Output));
        self.extend_terminal(tail);
    }

    fn execute_pending_action(&mut self) {
        self.start_action(false);
    }

    /// Start the pending action. Unless `force` is set, a Rust action that links
    /// or deletes binaries is first checked against processes running out of the
    /// workspace's `target/` — Windows locks a running executable, and finding
    /// out at the link step costs the entire compile first.
    fn start_action(&mut self, force: bool) {
        if self.running_execution.is_some() {
            self.status_line = "An action is already running".to_string();
            return;
        }

        let Some(target) = self.selected_target().cloned() else {
            self.status_line = "No workspace selected".to_string();
            return;
        };
        let Some(action_label) = self.pending_action.clone() else {
            self.status_line = "No action selected".to_string();
            return;
        };

        match build_plan(&target, &action_label, &self.current_build_options()) {
            Ok(plan) => {
                if !force {
                    if let Some(blockers) = self.artifact_blockers(&plan) {
                        self.status_line = format!(
                            "{} would fail: {} still running from this workspace",
                            action_label,
                            describe_processes(&blockers)
                        );
                        self.lock_conflict = Some(LockConflict { action_label, processes: blockers });
                        return;
                    }
                }
                self.lock_conflict = None;
                let signature = crate::execution::build_signature(&plan);
                let est_duration_ms = self
                    .history
                    .learned_build(&signature)
                    .map(|learned| learned.duration_ms)
                    .filter(|&duration| duration > 0);
                self.status_line = format!("Running {} for {}", plan.action_label, target.project_name);
                self.terminal_seen = 0;
                self.extend_terminal(vec![TerminalLine::new(
                    format!("$ {}  \u{2014}  {}", command_line(&plan), target.project_name),
                    TerminalLineKind::Command,
                )]);
                self.running_execution = Some(spawn_execution(plan, est_duration_ms));
            }
            Err(err) => {
                self.status_line = err;
            }
        }
    }

    /// Processes that would block this plan's link/clean step, if any.
    fn artifact_blockers(&self, plan: &ExecutionPlan) -> Option<Vec<crate::broker::RunningProcess>> {
        let dir = crate::execution::artifact_lock_dir(plan)?;
        let blockers = crate::broker::processes_running_under(&dir);
        (!blockers.is_empty()).then_some(blockers)
    }

    /// Terminate the blocking processes, then start the action. Only reachable
    /// from an explicit click on "STOP & RUN".
    fn clear_lock_conflict_and_run(&mut self) {
        let Some(conflict) = self.lock_conflict.take() else {
            return;
        };

        let mut failures = Vec::new();
        for process in &conflict.processes {
            if let Err(err) = crate::broker::terminate_process(process.pid) {
                failures.push(format!("{} (pid {}): {}", process.name, process.pid, err));
            }
        }
        if !failures.is_empty() {
            self.status_line = format!("Could not stop {}", failures.join("; "));
            return;
        }

        // Termination is signalled asynchronously — the file handle may outlive
        // the call by a few milliseconds. Wait briefly for the lock to actually
        // clear so the build does not race it.
        let stopped = self.wait_for_processes_to_exit(&conflict.processes);
        self.status_line = if stopped {
            format!(
                "Stopped {} \u{00B7} starting {}",
                describe_processes(&conflict.processes),
                conflict.action_label
            )
        } else {
            format!("Stopped {}; lock may still be releasing", describe_processes(&conflict.processes))
        };
        self.start_action(true);
    }

    /// Poll until the given processes are gone, up to a short bound. Called only
    /// from a direct user action, where a sub-second pause reads as responsive.
    fn wait_for_processes_to_exit(&self, processes: &[crate::broker::RunningProcess]) -> bool {
        let Some(dir) = processes
            .first()
            .and_then(|process| process.image_path.parent())
            .map(Path::to_path_buf)
        else {
            return true;
        };

        let pids: Vec<u32> = processes.iter().map(|process| process.pid).collect();
        for _ in 0..40 {
            let still_running = crate::broker::processes_running_under(&dir)
                .into_iter()
                .any(|process| pids.contains(&process.pid));
            if !still_running {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    /// Step the problem cursor and queue a scroll to that line. Wraps at both
    /// ends so repeated clicks cycle the log rather than dead-ending.
    fn jump_to_problem(&mut self, problems: &[usize], delta: isize) {
        if problems.is_empty() {
            return;
        }
        let next = next_problem_index(problems.len(), self.terminal_problem_cursor, delta);
        self.terminal_problem_cursor = next;
        self.terminal_jump_to = Some(problems[next]);
    }

    /// Refresh the cached diagnostic trend when the selected workspace or action
    /// changes. Keyed rather than recomputed per frame: `build_plan` touches the
    /// filesystem and the render loop runs constantly.
    fn sync_build_health(&mut self, target: &WorkspaceTarget) {
        let action = self
            .pending_action
            .clone()
            .or_else(|| default_action_for(target));
        let key = format!(
            "{}|{}|{}",
            target.id,
            action.as_deref().unwrap_or(""),
            self.rust_release
        );
        if self.build_health_key.as_deref() == Some(key.as_str()) {
            return;
        }
        self.build_health_key = Some(key);
        self.build_health = action
            .and_then(|label| build_plan(target, &label, &self.current_build_options()).ok())
            .map(|plan| crate::execution::build_signature(&plan))
            .and_then(|signature| self.history.learned_build(&signature))
            .filter(|learned| learned.recorded_outcome)
            .map(|learned| BuildHealth {
                warnings: learned.warnings,
                errors: learned.errors,
                delta: learned.warning_delta(),
            });
    }

    /// Pre-flight prompt shown when something is holding the workspace's build
    /// output open. Offers to clear it, to proceed anyway, or to back out.
    fn render_lock_conflict_banner(&mut self, ui: &mut egui::Ui) {
        let Some(conflict) = self.lock_conflict.as_ref() else {
            return;
        };
        let action_label = conflict.action_label.clone();
        let summary = describe_processes(&conflict.processes);
        let detail = conflict
            .processes
            .iter()
            .map(|process| format!("{} (pid {})", process.name, process.pid))
            .collect::<Vec<_>>()
            .join(", ");

        let mut stop_and_run = false;
        let mut run_anyway = false;
        let mut dismiss = false;

        let banner = egui::Frame::default()
            .fill(palette::WARN_DIM)
            .stroke(egui::Stroke::new(1.0_f32, theme::HAZARD_YELLOW))
            .corner_radius(3.0)
            .inner_margin(egui::Margin { left: 20, right: 10, top: 8, bottom: 8 })
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("\u{26A0} {} is still running", summary))
                        .monospace()
                        .size(theme::FS_LG)
                        .strong()
                        .color(palette::WARN),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "Windows locks a running executable, so {} would compile in full and then fail at the link step. Holding: {}",
                        action_label.to_lowercase(),
                        detail
                    ))
                    .monospace()
                    .size(theme::FS_MD)
                    .color(palette::DIM),
                );
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    stop_and_run = ui
                        .add(primary_button(&format!("\u{25A0} STOP & {}", action_label.to_uppercase()), false))
                        .clicked();
                    run_anyway = flat_button(ui, "RUN ANYWAY", false).clicked();
                    dismiss = flat_button(ui, "CANCEL", false).clicked();
                });
            });
        hazard_edge(ui, banner.response.rect);

        if stop_and_run {
            self.clear_lock_conflict_and_run();
        } else if run_anyway {
            self.lock_conflict = None;
            self.start_action(true);
        } else if dismiss {
            self.lock_conflict = None;
            self.status_line = format!("{} cancelled", action_label);
        }
    }

    fn cancel_running_execution(&mut self) {
        let Some(running) = self.running_execution.as_ref() else {
            self.status_line = "No action is currently running".to_string();
            return;
        };

        match running.cancel() {
            Ok(()) => {
                self.status_line = format!("Cancellation requested for {}", running.plan().action_label);
            }
            Err(err) => {
                self.status_line = format!("Failed to cancel action: {}", err);
            }
        }
    }

    fn record_history_from_report(&mut self, report: &ExecutionReport) -> Result<(), String> {
        let key = crate::history::normalized_path_key(&report.plan.cwd);
        let entry = self.history.entries.entry(key).or_default();
        update_history_entry_from_report(entry, report);

        // Learn this configuration's build time so the next run gets a smooth,
        // time-based progress bar. Only successful, non-trivial builds are useful:
        // a mostly-cached build finishes in seconds and would poison the estimate
        // for the next real compile.
        let mostly_cached = report.unit_count > 0 && report.fresh_units * 2 >= report.unit_count;
        if report.success
            && !report.cancelled
            && report.plan.action.records_build_history()
            && report.duration_ms > 0
            && !mostly_cached
        {
            let signature = crate::execution::build_signature(&report.plan);
            self.history.record_build(&signature, report.unit_count, report.duration_ms);
        }

        // Diagnostic counts are recorded for failed runs too — a failure's error
        // count is precisely the interesting part — but not for cancelled ones,
        // whose totals are an artefact of where the user interrupted.
        if !report.cancelled && report.plan.action.records_build_history() {
            let signature = crate::execution::build_signature(&report.plan);
            self.history
                .record_outcome(&signature, report.errors_total, report.warnings_total);
            self.build_health_key = None;
        }

        self.history
            .save()
            .map_err(|err| format!("failed to save shared history: {}", err))
    }

    fn toggle_chip(&mut self, chip: SearchChip) {
        if let Some(index) = self.active_chips.iter().position(|active| *active == chip) {
            self.active_chips.remove(index);
        } else {
            self.active_chips.push(chip);
        }
        self.refresh_targets();
    }

    fn move_selection(&mut self, delta: isize) {
        if self.targets.is_empty() {
            self.selected_index = None;
            return;
        }

        let current = self.selected_index.unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, self.targets.len().saturating_sub(1) as isize) as usize;
        if self.selected_index != Some(next) {
            self.selected_index = Some(next);
            self.scroll_catalog_to_selection = true;
        }
    }

    fn execute_primary_action(&mut self) {
        if self.pending_action.is_none() {
            let Some(target) = self.selected_target().cloned() else {
                return;
            };
            self.pending_action = default_action_for(&target);
        }

        if self.pending_action.is_some() {
            self.execute_pending_action();
        }
    }

    fn handle_global_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::R)) {
            self.rebuild_catalog();
            return;
        }

        if ctx.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::P)) {
            self.toggle_pin_selected();
        }

        if ctx.wants_keyboard_input() {
            return;
        }

        let wants_search = ctx.input(|input| {
            input.events.iter().any(|event| match event {
                egui::Event::Text(text) => text == "/",
                _ => false,
            })
        });
        if wants_search {
            self.request_search_focus = true;
        }

        if ctx.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
            self.move_selection(1);
        }
        if ctx.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
            self.move_selection(-1);
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
            self.execute_primary_action();
        }
    }
}

impl eframe::App for VaultForgeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_running_execution();
        self.poll_catalog(ctx);
        self.poll_broker_status(ctx);
        self.center_window_once(ctx);
        self.handle_global_shortcuts(ctx);
        if self.running_execution.is_some() {
            // Keeps the progress bar's sweep and the live stats moving.
            ctx.request_repaint_after(Duration::from_millis(33));
        }

        egui::TopBottomPanel::top("title_bar")
            .exact_height(theme::TITLE_BAR_HEIGHT)
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| self.render_title_bar(ui));

        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(26.0)
            .frame(egui::Frame::default().fill(theme::COLOR_HEADER).inner_margin(egui::Margin::symmetric(14, 0)))
            .show(ctx, |ui| self.render_status_bar(ui));

        egui::TopBottomPanel::top("command_bar")
            .frame(
                egui::Frame::default()
                    .fill(palette::BG)
                    .inner_margin(egui::Margin { left: 14, right: 14, top: 10, bottom: 8 }),
            )
            .show(ctx, |ui| self.render_command_bar(ui));

        egui::SidePanel::left("workspace_list")
            .min_width(320.0)
            .default_width(360.0)
            .frame(egui::Frame::default().fill(palette::BG).inner_margin(egui::Margin { left: 12, right: 6, top: 4, bottom: 10 }))
            .show(ctx, |ui| self.render_catalog(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(palette::BG).inner_margin(egui::Margin { left: 8, right: 14, top: 4, bottom: 10 }))
            .show(ctx, |ui| self.render_workspace(ui));

        let screen = ctx.screen_rect();
        let overlay = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("crt_overlay")));
        theme::paint_scanlines(&overlay, screen);
        theme::paint_hazard_border(&overlay, screen);
        theme::handle_window_resize(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // A build left running keeps compiling and holding target/ locks after
        // the window is gone, and a Run child would die on its first write to
        // the closed output pipe anyway — stop the tree deterministically.
        if let Some(running) = self.running_execution.take() {
            let _ = running.cancel();
        }
        // What follows is eframe's own teardown (GL context, window). If any of
        // it stalls, the process sits on the single-instance mutex with no
        // window and the Ctrl+Alt+B broker cannot open a new one. Guarantee
        // the process is gone shortly after the window is.
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(2));
            std::process::exit(0);
        });
    }
}

impl VaultForgeApp {
    fn render_title_bar(&mut self, ui: &mut egui::Ui) {
        let workspaces = format!("{}/{}", self.targets.len(), self.catalog.len());
        let action = self
            .pending_action
            .as_deref()
            .map_or_else(|| "IDLE".to_string(), str::to_uppercase);
        let (forge_state, forge_color) = if self.running_execution.is_some() {
            ("RUNNING", theme::SEV_MEDIUM)
        } else if self.pending_catalog.is_some() {
            ("SCANNING", theme::COLOR_LISTEN)
        } else {
            ("READY", theme::EVIDENCE_POS)
        };
        let readouts = [
            theme::Readout { label: "WORKSPACES", value: &workspaces, color: theme::COLOR_GOLD_BRIGHT },
            theme::Readout { label: "ACTION", value: &action, color: theme::COLOR_GOLD },
            theme::Readout { label: "FORGE", value: forge_state, color: forge_color },
        ];
        self.title_bar.show(
            ui,
            "VAULTFORGE",
            concat!("BUILD FORGE v", env!("CARGO_PKG_VERSION"), " \u{00B7} CTRL+ALT+B"),
            title_icon,
            &readouts,
        );
    }

    /// Bottom strip: the live status line and the hotkey broker's health.
    fn render_status_bar(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        ui.painter().with_clip_rect(ui.ctx().screen_rect()).hline(
            rect.x_range(),
            rect.top(),
            egui::Stroke::new(1.0_f32, theme::COLOR_BORDER_WARM),
        );

        let (led, state) = if !self.settings.hotkey_enabled {
            (theme::COLOR_TEXT_DIM, "OFF")
        } else if self.broker_setup_error.is_some() {
            (theme::SEV_CRITICAL, "SETUP FAILED")
        } else {
            match self.broker_status.as_ref() {
                Some(snapshot) if snapshot.hotkey_registered => (theme::EVIDENCE_POS, "ARMED"),
                Some(snapshot) if snapshot.broker_running && snapshot.message.contains("is taken") => {
                    (theme::SEV_HIGH, "KEY BUSY")
                }
                Some(snapshot) if snapshot.broker_running => (theme::SEV_MEDIUM, "STARTING"),
                _ if self.startup_status.is_some() => (theme::SEV_MEDIUM, "STARTING"),
                _ => (theme::COLOR_TEXT_DIM, "OFFLINE"),
            }
        };

        let indicator_w = 230.0_f32.min(rect.width() * 0.45);
        let status_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right() - indicator_w, rect.bottom()));
        let indicator_rect = egui::Rect::from_min_max(egui::pos2(rect.right() - indicator_w, rect.top()), rect.max);

        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(status_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&self.status_line)
                            .monospace()
                            .size(theme::FS_SM + 0.5)
                            .color(palette::DIM),
                    )
                    .truncate(),
                );
            },
        );

        let indicator = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(indicator_rect)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(egui::RichText::new(state).monospace().size(theme::FS_SM).strong().color(led));
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(dot.center(), 5.0, theme::tint_bg(led));
                    ui.painter().circle_filled(dot.center(), 3.0, led);
                    ui.label(
                        egui::RichText::new("HOTKEY CTRL+ALT+B")
                            .monospace()
                            .size(theme::FS_SM)
                            .color(palette::MUTED),
                    );
                },
            )
            .response;

        let mut details = Vec::new();
        if let Some(error) = self.broker_setup_error.as_deref() {
            details.push(error.to_string());
        }
        if let Some(snapshot) = self.broker_status.as_ref() {
            details.push(snapshot.summary());
            if let Some(target) = snapshot.launch_target.as_deref() {
                details.push(format!("Launches: {target}"));
            }
        }
        details.push(format!("Shared history: {}", crate::history::shared_history_path().display()));
        details.push("Click to turn Ctrl+Alt+B on or off.".to_string());
        let indicator = ui
            .interact(indicator.rect, ui.id().with("hotkey_indicator"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(details.join("\n"));

        let popup_id = ui.id().with("hotkey_popup");
        if indicator.clicked() {
            ui.memory_mut(|memory| memory.toggle_popup(popup_id));
        }
        let mut enabled = self.settings.hotkey_enabled;
        egui::popup::popup_above_or_below_widget(
            ui,
            popup_id,
            &indicator,
            egui::AboveOrBelow::Above,
            egui::PopupCloseBehavior::CloseOnClickOutside,
            |ui| {
                ui.set_min_width(300.0);
                ui.checkbox(&mut enabled, "Ctrl+Alt+B opens VaultForge");
                ui.label(
                    egui::RichText::new(
                        "Runs a tiny background listener (about 2 MB, no CPU while idle) that also starts when you sign in. Off stops it and removes it from sign-in.",
                    )
                    .monospace()
                    .size(theme::FS_XS)
                    .color(palette::MUTED),
                );
            },
        );
        if enabled != self.settings.hotkey_enabled {
            self.set_hotkey_enabled(enabled);
        }
    }

    /// Apply the Ctrl+Alt+B switch: persist it, then start or stop the
    /// listener off the UI thread (either can take a moment).
    fn set_hotkey_enabled(&mut self, enabled: bool) {
        self.settings.hotkey_enabled = enabled;
        if let Err(err) = self.settings.save() {
            self.status_line = format!("Failed to save settings: {err}");
        }
        self.broker_setup_error = None;
        let (tx, rx) = std::sync::mpsc::channel();
        self.startup_status = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(crate::broker::startup_status(enabled));
        });
        self.status_line = if enabled {
            "Turning Ctrl+Alt+B on\u{2026}".to_string()
        } else {
            "Ctrl+Alt+B is off. Turn it back on from the HOTKEY switch in the status bar.".to_string()
        };
    }

    /// Keep the folder the user chose for the next launch.
    fn remember_scan_root(&mut self) {
        if !Path::new(&self.scan_root).is_dir() || self.settings.scan_root.as_deref() == Some(self.scan_root.as_str()) {
            return;
        }
        self.settings.scan_root = Some(self.scan_root.clone());
        if let Err(err) = self.settings.save() {
            self.status_line = format!("Failed to save settings: {err}");
        }
    }

    /// Scan root + search, filter tabs, and the live progress/pre-flight banners.
    fn render_command_bar(&mut self, ui: &mut egui::Ui) {
        let mut rescan = false;
        let mut root_changed = false;
        let mut search_changed = false;

        ui.horizontal(|ui| {
            ui.label(theme::section_label("ROOT"));
            let root_width = ui.available_width() * 0.40;
            let root_response = ui.add_sized(
                [root_width, 26.0],
                egui::TextEdit::singleline(&mut self.scan_root)
                    .font(egui::FontId::monospace(theme::FS_MD))
                    .hint_text(theme::hint("folder to scan")),
            );
            ui.add_space(6.0);
            ui.label(theme::section_label("FIND"));
            let search_response = ui.add_sized(
                [(ui.available_width() - 180.0).max(120.0), 26.0],
                egui::TextEdit::singleline(&mut self.search_query)
                    .id_salt("catalog_search_input")
                    .font(egui::FontId::monospace(theme::FS_MD))
                    .hint_text(theme::hint("workspace, path, marker or toolchain  ( / )")),
            );
            if self.request_search_focus {
                search_response.request_focus();
                self.request_search_focus = false;
            }
            if flat_button(ui, "BROWSE", false)
                .on_hover_text("Pick the folder that holds your projects")
                .clicked()
            {
                let mut dialog = rfd::FileDialog::new().set_title("Folder that holds your projects");
                if Path::new(&self.scan_root).is_dir() {
                    dialog = dialog.set_directory(&self.scan_root);
                }
                if let Some(folder) = dialog.pick_folder() {
                    self.scan_root = folder.display().to_string();
                    root_changed = true;
                }
            }
            rescan = flat_button(ui, "RESCAN", false)
                .on_hover_text("Rescan the root folder (Ctrl+R)")
                .clicked();
            root_changed |= root_response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            search_changed = search_response.changed();
        });

        if rescan || root_changed {
            self.rebuild_catalog();
            self.remember_scan_root();
        } else if search_changed {
            self.refresh_targets();
        }

        ui.add_space(6.0);
        let tabs = ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.label(theme::section_label("FILTER"));
            ui.add_space(8.0);
            for chip in SearchChip::ALL {
                if theme::tab_button(ui, self.active_chips.contains(&chip), chip.label(), None).clicked() {
                    self.toggle_chip(chip);
                }
            }
        });
        ui.painter().hline(
            ui.max_rect().x_range(),
            tabs.response.rect.bottom(),
            egui::Stroke::new(1.0_f32, theme::COLOR_RULE),
        );

        // Snapshot first, then render: the banners need `&mut self` for their
        // buttons, which a live borrow of the execution would block.
        let running_view = self
            .running_execution
            .as_ref()
            .map(|running| (running.elapsed_ms(), running.snapshot()));
        if let Some((elapsed_ms, snapshot)) = running_view {
            ui.add_space(8.0);
            let fraction = running_build_fraction(elapsed_ms, &snapshot);
            draw_forge_progress_bar(ui, fraction, elapsed_ms, &snapshot);
            if snapshot.errors_total > 0 && draw_doomed_build_banner(ui, &snapshot) {
                self.cancel_running_execution();
            }
        }
        if self.lock_conflict.is_some() {
            ui.add_space(8.0);
            self.render_lock_conflict_banner(ui);
        }
    }

    fn render_catalog(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(theme::section_label("CATALOG"));
            ui.label(
                egui::RichText::new(format!(
                    "{} of {} \u{00B7} indexed {}",
                    self.targets.len(),
                    self.catalog.len(),
                    age_label(self.catalog.indexed_at_unix())
                ))
                .monospace()
                .size(theme::FS_SM)
                .color(palette::MUTED),
            );
        });
        ui.add_space(6.0);

        if self.targets.is_empty() {
            let message = if self.pending_catalog.is_some() {
                "Scanning workspaces\u{2026}"
            } else if self.catalog.len() == 0 {
                "No Rust or Python projects under this folder. Press BROWSE to pick the folder that holds your projects."
            } else {
                "No workspaces match the search and filters."
            };
            ui.add(egui::Label::new(egui::RichText::new(message).monospace().color(palette::DIM)).wrap());
            return;
        }

        let selected = self.selected_index;
        let scroll_to_selection = std::mem::take(&mut self.scroll_catalog_to_selection);
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("workspace_catalog_scroll")
            .auto_shrink([false, false]);
        if let Some(offset) = self.pending_catalog_offset.take() {
            scroll = scroll.vertical_scroll_offset(offset.max(0.0));
        }

        ui.spacing_mut().item_spacing.y = 0.0;
        let mut clicked = None;
        let targets = &self.targets;
        let output = scroll.show_rows(ui, CATALOG_ROW_HEIGHT, targets.len(), |ui, row_range| {
            let mut selection_rendered = false;
            for index in row_range {
                let is_selected = selected == Some(index);
                let response = workspace_row(ui, &targets[index], is_selected, index % 2 == 1);
                if response.clicked() {
                    clicked = Some(index);
                }
                if scroll_to_selection && is_selected {
                    response.scroll_to_me(None);
                    selection_rendered = true;
                }
            }
            selection_rendered
        });
        if scroll_to_selection && !output.inner {
            // The selection is outside the rendered rows (a pin re-ranked it, or
            // a long keyboard jump); centre it on the next frame instead.
            if let Some(index) = selected {
                self.pending_catalog_offset =
                    Some(index as f32 * CATALOG_ROW_HEIGHT - (output.inner_rect.height() - CATALOG_ROW_HEIGHT) / 2.0);
                ui.ctx().request_repaint();
            }
        }
        if let Some(index) = clicked {
            self.selected_index = Some(index);
        }
    }

    fn render_workspace(&mut self, ui: &mut egui::Ui) {
        let Some(target) = self.selected_target().cloned() else {
            shell_block(ui, |ui| {
                section_header(ui, "WORKSPACE", Some("nothing selected"));
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new("Pick a workspace from the catalog, or press / to search.")
                        .monospace()
                        .color(palette::DIM),
                );
            });
            return;
        };

        // Header tile: identity + one-touch actions on the left, build options
        // on the right for easy reach.
        shell_block(ui, |ui| {
            if ui.available_width() > 760.0 {
                ui.columns(2, |columns| {
                    columns[0].vertical(|ui| {
                        self.render_workspace_summary(ui, &target);
                    });
                    columns[1].vertical(|ui| {
                        self.render_build_options_inline(ui, &target);
                    });
                });
            } else {
                self.render_workspace_summary(ui, &target);
                ui.add_space(8.0);
                self.render_build_options_inline(ui, &target);
            }
        });

        ui.add_space(8.0);
        self.render_terminal(ui, &target);
    }

    /// Header tile, left side: identity, meta row, one-touch action row.
    fn render_workspace_summary(&mut self, ui: &mut egui::Ui, target: &WorkspaceTarget) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(&target.project_name)
                    .monospace()
                    .size(17.0)
                    .strong()
                    .color(palette::ACCENT_STRONG),
            )
            .on_hover_text(format!("Ranking score {}", target.ranking_score));
            tag(ui, &target.toolchain.label().to_uppercase(), toolchain_color(&target.toolchain), false);
            if target.pinned {
                tag(ui, "PINNED", palette::ACCENT, false);
            }
            if self.running_execution.is_some() {
                tag(ui, "RUNNING", palette::WARN, true);
            }
        });
        ui.add(
            egui::Label::new(
                egui::RichText::new(target.root_path.display().to_string())
                    .monospace()
                    .size(theme::FS_SM + 0.5)
                    .color(palette::MUTED),
            )
            .wrap(),
        );

        ui.add_space(6.0);
        self.sync_build_health(target);
        let health = self
            .build_health
            .as_ref()
            .map(|health| (format_build_health(health), build_health_color(health)));
        ui.horizontal_wrapped(|ui| {
            if flat_button(ui, if target.pinned { "UNPIN" } else { "PIN" }, false)
                .on_hover_text("Pin to the top of the catalog (Ctrl+P)")
                .clicked()
            {
                self.toggle_pin_selected();
            }
            if flat_button(ui, "REVEAL", false).on_hover_text("Open in Explorer").clicked() {
                self.reveal_selected();
            }
            if let Some((text, color)) = health.as_ref() {
                tag(ui, text, *color, false);
            }
            for marker in &target.markers {
                tag(ui, marker, palette::DIM, false);
            }
            ui.label(
                egui::RichText::new(format!("{} \u{00B7} modified {}", target.history_label(), target.age_label()))
                    .monospace()
                    .size(theme::FS_SM + 0.5)
                    .color(palette::DIM),
            );
        });

        ui.add_space(8.0);
        let is_running = self.running_execution.is_some();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.label(theme::section_label("ACTION"));
            ui.add_space(8.0);
            for action in self.display_actions(target) {
                let selected = self.pending_action.as_deref() == Some(action.as_str());
                let clicked = ui
                    .add_enabled_ui(!is_running, |ui| theme::tab_button(ui, selected, &action.to_uppercase(), None))
                    .inner
                    .clicked();
                if clicked {
                    self.pending_action = Some(action);
                    // The pre-flight verdict belongs to the action it was run
                    // for; a different action may not touch the locked binary.
                    self.lock_conflict = None;
                }
            }
            ui.add_space(10.0);
            let execute_label = if is_running { "\u{25A0} CANCEL" } else { "\u{25B6} EXECUTE" };
            let response = ui
                .add_enabled(is_running || self.pending_action.is_some(), primary_button(execute_label, is_running))
                .on_hover_text(if is_running { "Stop the running action" } else { "Run the selected action (Enter)" });
            if response.clicked() {
                if is_running {
                    self.cancel_running_execution();
                } else {
                    self.execute_pending_action();
                }
            }
        });
    }

    /// Header tile, right side: compact per-toolchain build options.
    fn render_build_options_inline(&mut self, ui: &mut egui::Ui, target: &WorkspaceTarget) {
        self.sync_build_options_for_target(target);
        ui.label(theme::section_label("BUILD OPTIONS"));
        ui.add_space(4.0);

        match target.toolchain {
            Toolchain::Rust => {
                ui.horizontal(|ui| {
                    option_label(ui, "Profile");
                    let mut release = self.rust_release;
                    egui::ComboBox::from_id_salt("rust_profile_combo")
                        .selected_text(if release { "Release" } else { "Dev" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut release, false, "Dev (debug)");
                            ui.selectable_value(&mut release, true, "Release (optimized)");
                        });
                    self.rust_release = release;
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    option_label(ui, "Features");
                    ui.add_sized(
                        [ui.available_width(), 24.0],
                        egui::TextEdit::singleline(&mut self.build_features_input)
                            .hint_text(theme::hint("e.g. serde, tokio"))
                            .font(egui::FontId::monospace(theme::FS_MD)),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    option_label(ui, "Target");
                    ui.add_sized(
                        [ui.available_width(), 24.0],
                        egui::TextEdit::singleline(&mut self.build_target_input)
                            .hint_text(theme::hint("host default"))
                            .font(egui::FontId::monospace(theme::FS_MD)),
                    );
                });
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new("Applies to Build, Run, Test, Check, Lint. Features skip Clean.")
                        .monospace()
                        .size(theme::FS_XS)
                        .color(palette::MUTED),
                );
            }
            Toolchain::Python => {
                self.sync_preferred_python_entry_input(target);
                let detected_entry = target
                    .primary_entry_hint
                    .clone()
                    .unwrap_or_else(|| "none yet".to_string());
                let mut save_requested = false;
                let mut clear_requested = false;

                ui.horizontal(|ui| {
                    option_label(ui, "Entry");
                    let response = ui.add_sized(
                        [ui.available_width(), 24.0],
                        egui::TextEdit::singleline(&mut self.preferred_python_entry_input)
                            .hint_text(theme::hint("scripts/tool.py"))
                            .font(egui::FontId::monospace(theme::FS_MD)),
                    );
                    if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                        save_requested = true;
                    }
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if flat_button(ui, "SAVE ENTRY", false).clicked() {
                        save_requested = true;
                    }
                    if flat_button(ui, "CLEAR ENTRY", false).clicked() {
                        clear_requested = true;
                    }
                    ui.label(
                        egui::RichText::new(format!("Detected: {}", detected_entry))
                            .monospace()
                            .size(theme::FS_XS)
                            .color(palette::MUTED),
                    );
                });
                ui.add_space(4.0);
                ui.checkbox(&mut self.python_run_as_module, "Run as module (python -m)");
                if self.python_run_as_module {
                    ui.horizontal(|ui| {
                        option_label(ui, "Module");
                        ui.add_sized(
                            [ui.available_width(), 24.0],
                            egui::TextEdit::singleline(&mut self.python_module_input)
                                .hint_text(theme::hint("package.module"))
                                .font(egui::FontId::monospace(theme::FS_MD)),
                        );
                    });
                }

                if save_requested {
                    self.save_preferred_python_entry(target);
                }
                if clear_requested {
                    self.clear_preferred_python_entry(target);
                }
            }
            Toolchain::Mixed => {
                ui.label(
                    egui::RichText::new("Mixed workspace \u{2014} no toolchain options.")
                        .monospace()
                        .size(theme::FS_SM + 0.5)
                        .color(palette::MUTED),
                );
            }
        }
    }

    /// The terminal tile: a rolling, colour-classified log of every executed
    /// action's output, filling the space below the header. Rows are
    /// virtualized — only the visible lines are laid out each frame, which
    /// matters at 4,000 lines repainting 30 times a second during a build.
    fn render_terminal(&mut self, ui: &mut egui::Ui, target: &WorkspaceTarget) {
        shell_block(ui, |ui| {
            // Indices of warning/error lines: drives both the problems-only view
            // and the jump buttons, so an 11k-line log stays navigable.
            let problems: Vec<usize> = self
                .terminal_lines
                .iter()
                .enumerate()
                .filter(|(_, line)| is_problem_line(line))
                .map(|(index, _)| index)
                .collect();

            let log_path = self
                .running_execution
                .as_ref()
                .and_then(|running| running.log_path().map(Path::to_path_buf))
                .or_else(|| self.last_log_path.clone());

            ui.horizontal(|ui| {
                section_header(ui, "TERMINAL", Some("execution output"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if flat_button(ui, "CLEAR", false).clicked() {
                        self.terminal_lines.clear();
                        self.terminal_problems_only = false;
                        self.terminal_problem_cursor = 0;
                    }
                    if let Some(path) = log_path.as_deref() {
                        if flat_button(ui, "OPEN LOG", false)
                            .on_hover_text(path.display().to_string())
                            .clicked()
                        {
                            if let Err(err) = open_path(path) {
                                self.status_line = format!("Failed to open log: {}", err);
                            }
                        }
                    }
                    if !problems.is_empty() {
                        if flat_button(ui, "\u{25B8}", false).on_hover_text("Next problem").clicked() {
                            self.jump_to_problem(&problems, 1);
                        }
                        if flat_button(ui, "\u{25C2}", false).on_hover_text("Previous problem").clicked() {
                            self.jump_to_problem(&problems, -1);
                        }
                        let label = format!("PROBLEMS {}", problems.len());
                        if flat_button(ui, &label, self.terminal_problems_only)
                            .on_hover_text("Show only warnings and errors")
                            .clicked()
                        {
                            self.terminal_problems_only = !self.terminal_problems_only;
                        }
                    }
                    if let Some(running) = self.running_execution.as_ref() {
                        let snapshot = running.snapshot();
                        draw_stats_row(ui, running.elapsed_ms(), &snapshot);
                    }
                });
            });

            ui.add_space(6.0);
            let inner_height = (ui.available_height() - 22.0).max(160.0);
            egui::Frame::default()
                .fill(palette::BG)
                .stroke(egui::Stroke::new(1.0_f32, palette::BORDER_SOFT))
                .corner_radius(3.0)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_min_height(inner_height);
                    ui.set_min_width(ui.available_width());
                    let filtered = self.terminal_problems_only && !problems.is_empty();
                    if self.terminal_lines.is_empty() {
                        ui.label(
                            egui::RichText::new(format!(
                                "Ready \u{2014} pick an action above and hit EXECUTE. Output for {} streams here.",
                                target.project_name
                            ))
                            .monospace()
                            .size(TERMINAL_FONT_SIZE)
                            .color(palette::MUTED),
                        );
                        return;
                    }
                    if filtered {
                        ui.label(
                            egui::RichText::new(format!(
                                "Showing {} problem lines \u{2014} press PROBLEMS again for the full log.",
                                problems.len()
                            ))
                            .monospace()
                            .size(theme::FS_SM + 0.5)
                            .color(palette::MUTED),
                        );
                    }

                    ui.spacing_mut().item_spacing.y = 1.0;
                    let row_height = ui.fonts(|fonts| fonts.row_height(&egui::FontId::monospace(TERMINAL_FONT_SIZE)));
                    let row_count = if filtered { problems.len() } else { self.terminal_lines.len() };
                    let line_at = |row: usize| if filtered { problems[row] } else { row };

                    let jump_row = self.terminal_jump_to.take().and_then(|line| {
                        if filtered { problems.iter().position(|&index| index == line) } else { Some(line) }
                    });
                    let mut scroll = egui::ScrollArea::both()
                        .id_salt("terminal_scroll")
                        .auto_shrink([false, false])
                        .max_height(ui.available_height())
                        // A pending jump must win over follow-the-tail, or the
                        // view snaps straight back down.
                        .stick_to_bottom(jump_row.is_none());
                    if let Some(row) = jump_row {
                        let row_span = row_height + ui.spacing().item_spacing.y;
                        scroll = scroll.vertical_scroll_offset((row as f32 * row_span - inner_height / 2.0).max(0.0));
                    }
                    let lines = &self.terminal_lines;
                    scroll.show_rows(ui, row_height, row_count, |ui, row_range| {
                        for row in row_range {
                            terminal_line_label(ui, &lines[line_at(row)]);
                        }
                    });
                });
        });
    }
}

/// 1 warning, 2 warnings.
fn count(n: impl Into<u64>, noun: &str) -> String {
    let n = n.into();
    if n == 1 { format!("1 {noun}") } else { format!("{n} {noun}s") }
}

fn option_label(ui: &mut egui::Ui, text: &str) {
    ui.add_sized(
        [58.0, 18.0],
        egui::Label::new(egui::RichText::new(text).size(11.5).color(palette::DIM)),
    );
}

/// Banner for a build that has already produced an error. The progress bar's
/// smooth curve otherwise makes a doomed build look identical to a healthy one
/// for however many minutes of compiling remain. Returns true if STOP was hit.
fn draw_doomed_build_banner(
    ui: &mut egui::Ui,
    snapshot: &crate::execution::ExecutionProgressSnapshot,
) -> bool {
    let mut stop = false;
    ui.add_space(6.0);
    let frame = egui::Frame::default()
        .fill(palette::ERROR_DIM)
        .stroke(egui::Stroke::new(1.0_f32, palette::ERROR))
        .corner_radius(3.0)
        .inner_margin(egui::Margin { left: 20, right: 10, top: 8, bottom: 8 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let headline = match snapshot.errors_total {
                    1 => "1 ERROR \u{2014} THIS BUILD WILL FAIL".to_string(),
                    n => format!("{} ERRORS \u{2014} THIS BUILD WILL FAIL", n),
                };
                ui.label(
                    egui::RichText::new(headline)
                        .monospace()
                        .size(theme::FS_LG)
                        .strong()
                        .color(theme::COLOR_FOE),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    stop = ui.add(primary_button("\u{25A0} STOP", true)).clicked();
                });
            });
            if let Some(first) = snapshot.first_error.as_deref() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(first)
                            .monospace()
                            .size(theme::FS_MD)
                            .color(palette::TEXT),
                    )
                    .wrap(),
                );
            }
        });
    hazard_edge(ui, frame.response.rect);
    stop
}

/// Hazard tape down the left edge of a banner.
fn hazard_edge(ui: &egui::Ui, banner: egui::Rect) {
    let strip = egui::Rect::from_min_size(banner.min, egui::vec2(10.0, banner.height()));
    theme::paint_hazard_tape(ui.painter(), strip.shrink(1.0));
}

/// Step a cursor over `count` problems, wrapping at both ends so repeated
/// clicks cycle instead of dead-ending. `current` is clamped first: the problem
/// list shrinks whenever the terminal is cleared or its window rolls over.
fn next_problem_index(count: usize, current: usize, delta: isize) -> usize {
    if count == 0 {
        return 0;
    }
    let clamped = current.min(count - 1) as isize;
    (clamped + delta).rem_euclid(count as isize) as usize
}

/// "992 warnings (+37)" — the trend matters more than the absolute count, since
/// warning debt is what buries a real error in an 11k-line log.
fn format_build_health(health: &BuildHealth) -> String {
    let mut text = match health.warnings {
        0 => "no warnings".to_string(),
        1 => "1 warning".to_string(),
        count => format!("{} warnings", count),
    };
    if let Some(delta) = health.delta.filter(|delta| *delta != 0) {
        text.push_str(&format!(" ({:+})", delta));
    }
    if health.errors > 0 {
        text.push_str(&format!(" \u{00B7} {}", count(health.errors, "error")));
    }
    text
}

fn build_health_color(health: &BuildHealth) -> egui::Color32 {
    if health.errors > 0 {
        palette::ERROR
    } else if health.warnings > 0 {
        palette::WARN
    } else {
        palette::ACCENT
    }
}

/// Lines the PROBLEMS filter and the jump buttons navigate between.
fn is_problem_line(line: &TerminalLine) -> bool {
    matches!(
        line.kind,
        TerminalLineKind::Error | TerminalLineKind::Warning | TerminalLineKind::Failure
    )
}

fn open_path(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // `explorer <file>` hands the file to its registered handler.
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string())
    }
}

/// "arachnia.exe (pid 3852)", or "3 processes" once naming them all stops
/// being useful.
fn describe_processes(processes: &[crate::broker::RunningProcess]) -> String {
    match processes {
        [] => "nothing".to_string(),
        [one] => format!("{} (pid {})", one.name, one.pid),
        [first, second] => format!(
            "{} (pid {}) and {} (pid {})",
            first.name, first.pid, second.name, second.pid
        ),
        many => format!("{} processes", many.len()),
    }
}

fn terminal_line_label(ui: &mut egui::Ui, line: &TerminalLine) -> egui::Response {
    let (color, strong) = match line.kind {
        TerminalLineKind::Command => (palette::ACCENT_STRONG, true),
        TerminalLineKind::Output => (palette::TEXT, false),
        TerminalLineKind::Info => (palette::DIM, false),
        TerminalLineKind::Warning => (palette::WARN, false),
        TerminalLineKind::Error => (palette::ERROR_TEXT, false),
        TerminalLineKind::Success => (palette::SUCCESS, true),
        TerminalLineKind::Failure => (palette::ERROR, true),
    };
    let text = if line.text.is_empty() { " " } else { line.text.as_str() };
    let mut rich = egui::RichText::new(text).monospace().size(TERMINAL_FONT_SIZE).color(color);
    if strong {
        rich = rich.strong();
    }
    // Rows are fixed-height for virtualization: long lines scroll sideways
    // instead of wrapping.
    ui.add(egui::Label::new(rich).extend())
}

/// Forge model: max the time-based estimate with the parser's phase floor so
/// the bar shows early life on first builds and creeps smoothly thereafter.
fn running_build_fraction(elapsed_ms: u64, snapshot: &crate::execution::ExecutionProgressSnapshot) -> f32 {
    let time_fraction = crate::execution::build_progress_fraction(
        elapsed_ms,
        snapshot.est_duration_ms,
        snapshot.units_completed,
        snapshot.units_total,
    );
    time_fraction.max(snapshot.fraction).clamp(0.0, 0.99)
}

/// Phosphor progress bar with the current crate as its caption.
fn draw_forge_progress_bar(ui: &mut egui::Ui, fraction: f32, elapsed_ms: u64, snapshot: &crate::execution::ExecutionProgressSnapshot) {
    let phase = if snapshot.current_crate.is_empty() {
        "FORGING\u{2026}".to_string()
    } else {
        format!("FORGING {}", snapshot.current_crate)
    };
    let text = format!("{}%  \u{2014}  {}", (fraction * 100.0).round() as u32, phase);
    theme::phosphor_bar(ui, fraction, elapsed_ms as f32 / 1000.0, &text);
}

/// Row of pills summarizing the running build — elapsed, crates, errors,
/// warnings, and the runnable binary once cargo reports it.
fn draw_stats_row(ui: &mut egui::Ui, elapsed_ms: u64, snapshot: &crate::execution::ExecutionProgressSnapshot) {
    ui.horizontal_wrapped(|ui| {
        pill(ui, &format!("{:.1}s", elapsed_ms as f64 / 1000.0), palette::DIM);
        if snapshot.units_total > 0 {
            pill(ui, &format!("{} / {} crates", snapshot.units_completed, snapshot.units_total), palette::DIM);
        }
        if snapshot.errors_total > 0 {
            pill(ui, &format!("\u{2716} {}", count(snapshot.errors_total, "error")), palette::ERROR_TEXT);
        }
        if snapshot.warnings_total > 0 {
            pill(ui, &format!("\u{26A0} {}", count(snapshot.warnings_total, "warning")), palette::WARN);
        }
        if let Some(binary) = snapshot.binary_path.as_deref() {
            let short = Path::new(binary)
                .file_name()
                .map(|name| format!("\u{2192} {}", name.to_string_lossy()))
                .unwrap_or_else(|| format!("\u{2192} {}", binary));
            pill(ui, &short, palette::SUCCESS);
        }
    });
}

/// Tinted pill: text in `color` on a dark tint of the same hue.
fn pill(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    egui::Frame::default()
        .fill(theme::tint_bg(color))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).monospace().size(theme::FS_SM).color(color));
        });
}

fn shell_block(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::default()
        .fill(palette::SHELL)
        .stroke(egui::Stroke::new(1.0_f32, palette::SHELL_BORDER))
        .corner_radius(3.0)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| body(ui));
}

fn section_header(ui: &mut egui::Ui, title: &str, subtitle: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(theme::section_label(title));
        if let Some(subtitle) = subtitle {
            ui.label(egui::RichText::new(subtitle).monospace().size(theme::FS_SM).color(palette::MUTED));
        }
    });
}

/// Secondary button. Unselected buttons take their fill from the theme's
/// widget visuals, so they keep the amber hover state.
fn flat_button(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let text = egui::RichText::new(label)
        .monospace()
        .size(theme::FS_SM + 0.5)
        .strong()
        .color(if selected { palette::ACCENT_STRONG } else { palette::TEXT });
    let mut button = egui::Button::new(text).min_size(egui::vec2(0.0, 24.0));
    if selected {
        button = button
            .fill(palette::ACCENT_SOFT)
            .stroke(egui::Stroke::new(1.0_f32, palette::ACCENT_STRONG));
    }
    ui.add(button).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The one loud button: hazard-gold EXECUTE, or red when it stops something.
fn primary_button(label: &str, danger: bool) -> egui::Button<'static> {
    let (fill, text, stroke) = if danger {
        (
            egui::Color32::from_rgb(140, 35, 35),
            egui::Color32::WHITE,
            egui::Color32::from_rgb(180, 60, 60),
        )
    } else {
        (theme::HAZARD_YELLOW, egui::Color32::BLACK, theme::COLOR_GOLD_BRIGHT)
    };
    egui::Button::new(
        egui::RichText::new(label.to_string())
            .monospace()
            .size(theme::FS_SM + 0.5)
            .strong()
            .color(text),
    )
    .fill(fill)
    .stroke(egui::Stroke::new(1.0_f32, stroke))
    .min_size(egui::vec2(0.0, 24.0))
}

fn tag(ui: &mut egui::Ui, label: &str, color: egui::Color32, strong_fill: bool) {
    let (fill, text) = if strong_fill {
        (color, palette::BG)
    } else {
        (theme::tint_bg(color), color)
    };
    ui.label(
        egui::RichText::new(format!(" {} ", label))
            .monospace()
            .size(theme::FS_SM)
            .background_color(fill)
            .color(text),
    );
}

fn title_icon() -> egui::ColorImage {
    let raster = crate::icon::icon_raster();
    egui::ColorImage::from_rgba_unmultiplied([raster.width as usize, raster.height as usize], &raster.rgba)
}

/// One painted catalog row, exactly `CATALOG_ROW_HEIGHT` tall.
fn workspace_row(ui: &mut egui::Ui, target: &WorkspaceTarget, selected: bool, alternate: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), CATALOG_ROW_HEIGHT),
        egui::Sense::click(),
    );
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let painter = ui.painter_at(rect);
    let background = if selected {
        theme::COLOR_SELECTION
    } else if response.hovered() {
        theme::COLOR_HOVER_BG
    } else if alternate {
        theme::COLOR_BG_ALT
    } else {
        theme::COLOR_BG
    };
    painter.rect_filled(rect, 0.0, background);
    if selected {
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
            0.0,
            theme::COLOR_GOLD_BRIGHT,
        );
    }
    painter.hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0_f32, theme::COLOR_RULE));

    let left = rect.left() + 12.0;
    let right = rect.right() - 10.0;

    let tool_color = toolchain_color(&target.toolchain);
    let tool = painter.layout_no_wrap(
        target.toolchain.label().to_uppercase(),
        egui::FontId::monospace(theme::FS_XS),
        tool_color,
    );
    let tool_rect = egui::Rect::from_min_size(
        egui::pos2(right - tool.size().x - 8.0, rect.top() + 9.0),
        tool.size() + egui::vec2(8.0, 2.0),
    );
    painter.rect_filled(tool_rect, 3.0, theme::tint_bg(tool_color));
    painter.galley(tool_rect.min + egui::vec2(4.0, 1.0), tool, tool_color);
    let age = painter.text(
        egui::pos2(tool_rect.left() - 8.0, tool_rect.center().y),
        egui::Align2::RIGHT_CENTER,
        target.age_label(),
        egui::FontId::monospace(theme::FS_XS),
        theme::COLOR_TEXT_DIM,
    );

    let name = format!("{} {}", if target.pinned { "\u{2605}" } else { "\u{00B7}" }, target.project_name);
    let name_color = if selected { theme::COLOR_GOLD_BRIGHT } else { theme::COLOR_TEXT };
    elided_text(&painter, egui::pos2(left, rect.top() + 7.0), name, theme::FS_LG, name_color, age.left() - left - 8.0);
    elided_text(
        &painter,
        egui::pos2(left, rect.top() + 26.0),
        target.short_root(),
        theme::FS_SM,
        theme::COLOR_TEXT_DIM,
        right - left,
    );
    elided_text(
        &painter,
        egui::pos2(left, rect.top() + 42.0),
        format!("{} \u{00B7} {}% confidence", target.history_label(), target.confidence),
        theme::FS_XS,
        theme::COLOR_TEXT_WARM,
        right - left,
    );

    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Single-line text cut to `max_width` with an ellipsis.
fn elided_text(painter: &egui::Painter, pos: egui::Pos2, text: String, size: f32, color: egui::Color32, max_width: f32) {
    if max_width <= 8.0 {
        return;
    }
    let mut job = egui::text::LayoutJob::single_section(
        text,
        egui::TextFormat::simple(egui::FontId::monospace(size), color),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width);
    painter.galley(pos, painter.layout_job(job), color);
}

fn command_line(plan: &ExecutionPlan) -> String {
    if plan.args.is_empty() {
        return plan.program.clone();
    }

    format!("{} {}", plan.program, plan.args.join(" "))
}


fn age_label(unix_secs: u64) -> String {
    if unix_secs == 0 {
        return "unknown".to_string();
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(unix_secs);
    let delta = now.saturating_sub(unix_secs);

    if delta < 60 {
        "just now".to_string()
    } else if delta < 3_600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86_400 {
        format!("{}h ago", delta / 3_600)
    } else if delta < 2_592_000 {
        format!("{}d ago", delta / 86_400)
    } else if delta < 31_536_000 {
        format!("{}mo ago", delta / 2_592_000)
    } else {
        format!("{}y ago", delta / 31_536_000)
    }
}

fn toolchain_color(toolchain: &Toolchain) -> egui::Color32 {
    match toolchain {
        Toolchain::Rust => palette::RUST,
        Toolchain::Python => palette::PYTHON,
        Toolchain::Mixed => palette::MIXED,
    }
}

/// Where to scan when the user hasn't picked a folder (or theirs is gone):
/// 1. the folder they last scanned;
/// 2. for a development build, the folder holding this repository;
/// 3. the exe's own folder, when VaultForge was dropped into a projects folder;
/// 4. the first common projects folder that exists (source\repos, GitHub, ...);
/// 5. Documents.
fn default_scan_root(saved: Option<&str>) -> PathBuf {
    if let Some(saved) = saved.map(PathBuf::from).filter(|path| path.is_dir()) {
        return saved;
    }
    if let Some(root) = std::env::current_exe().ok().and_then(|exe| scan_root_for_executable_path(&exe)) {
        return root;
    }
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    common_project_folders(&home)
        .into_iter()
        .find(|path| path.is_dir())
        .unwrap_or_else(|| {
            let documents = home.join("Documents");
            if documents.is_dir() { documents } else { home }
        })
}

fn common_project_folders(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join("source").join("repos"),
        home.join("Documents").join("GitHub"),
        home.join("GitHub"),
        home.join("Projects"),
        home.join("Documents").join("Projects"),
        home.join("dev"),
        home.join("code"),
        home.join("src"),
        home.join("repos"),
    ]
}

fn scan_root_for_executable_path(exe_path: &Path) -> Option<PathBuf> {
    let exe_dir = exe_path.parent()?.to_path_buf();
    if let Some(dev_root) = dev_scan_root_from_executable_dir(&exe_dir) {
        return Some(dev_root);
    }
    looks_like_projects_folder(&exe_dir).then_some(exe_dir)
}

/// Whether a folder, or one of its immediate subfolders, is a Rust or Python
/// project, i.e. VaultForge was placed among the projects it should build
/// rather than in Downloads or Program Files.
fn looks_like_projects_folder(dir: &Path) -> bool {
    const MARKERS: [&str; 5] = ["Cargo.toml", "pyproject.toml", "setup.py", "requirements.txt", "Pipfile"];
    let has_marker = |folder: &Path| MARKERS.iter().any(|marker| folder.join(marker).is_file());
    if has_marker(dir) {
        return true;
    }
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .take(500)
                .any(|entry| has_marker(&entry.path()))
        })
        .unwrap_or(false)
}

fn dev_scan_root_from_executable_dir(exe_dir: &Path) -> Option<PathBuf> {
    let leaf = exe_dir.file_name()?.to_str()?;
    let target_dir = match leaf {
        "debug" | "release" => exe_dir.parent()?,
        "deps" => exe_dir.parent()?.parent()?,
        _ => return None,
    };

    if target_dir.file_name().and_then(|name| name.to_str()) != Some("target") {
        return None;
    }

    let project_dir = target_dir.parent()?;
    if !project_dir.join("Cargo.toml").is_file() || !project_dir.join("src").is_dir() {
        return None;
    }

    Some(project_dir.parent().map_or_else(|| project_dir.to_path_buf(), Path::to_path_buf))
}

fn preferred_python_entry_storage_value(root_path: &Path, preferred_entry: &str) -> Option<String> {
    let resolved = crate::discovery::validated_preferred_python_entry(root_path, Some(preferred_entry.trim()))?;
    let root = std::fs::canonicalize(root_path).unwrap_or_else(|_| root_path.to_path_buf());
    let candidate = PathBuf::from(resolved);

    candidate
        .strip_prefix(&root)
        .ok()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .filter(|relative| !relative.is_empty())
        .or_else(|| Some(candidate.to_string_lossy().replace('\\', "/")))
}

fn update_history_entry_from_report(entry: &mut crate::history::HistoryEntry, report: &ExecutionReport) {
    if !report.success {
        return;
    }

    if report.plan.action.records_build_history() {
        entry.last_built_at = Some(report.finished_at_unix);
    }
    if report.plan.action.records_launch_history() {
        entry.last_launched_at = Some(report.finished_at_unix);
    }
    if let Some(artifact) = last_successful_artifact_from_report(report) {
        entry.last_successful_artifact = Some(artifact);
    }
    if let Some(preferred_entry) = preferred_python_entry_from_report(report) {
        entry.preferred_python_entry = Some(preferred_entry);
    }
}

fn last_successful_artifact_from_report(report: &ExecutionReport) -> Option<String> {
    if report.cancelled || report.plan.toolchain != Toolchain::Rust {
        return None;
    }

    report
        .stdout
        .lines()
        .filter_map(parse_cargo_artifact_executable)
        .last()
}

fn parse_cargo_artifact_executable(line: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct CargoArtifactLine {
        reason: String,
        executable: Option<String>,
    }

    if !line.starts_with('{') {
        return None;
    }
    let message: CargoArtifactLine = serde_json::from_str(line).ok()?;
    if message.reason != "compiler-artifact" {
        return None;
    }

    let executable = message.executable?;
    Path::new(&executable)
        .is_file()
        .then_some(executable)
}

fn preferred_python_entry_from_report(report: &ExecutionReport) -> Option<String> {
    if report.cancelled
        || report.plan.toolchain != Toolchain::Python
        || report.plan.action != crate::execution::ActionKind::Run
    {
        return None;
    }

    preferred_python_entry_storage_value(&report.plan.cwd, report.plan.args.last()?)
}

#[cfg(test)]
mod tests {
    use super::{last_successful_artifact_from_report, preferred_python_entry_from_report, preferred_python_entry_storage_value, scan_root_for_executable_path, update_history_entry_from_report};
    use super::{BuildHealth, describe_processes, format_build_health, is_problem_line, next_problem_index};
    use crate::broker::RunningProcess;

    fn process(name: &str, pid: u32) -> RunningProcess {
        RunningProcess {
            pid,
            name: name.to_string(),
            image_path: std::path::PathBuf::from(format!("C:/repo/target/release/{name}")),
        }
    }

    /// Every symbol the UI draws must exist in egui's bundled fonts; a missing
    /// one renders as a box (U+2713 did, on every finished run).
    #[test]
    fn ui_symbols_exist_in_the_bundled_fonts() {
        let ctx = eframe::egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let _ = ctx.run(Default::default(), |_| {});
        let font = eframe::egui::FontId::monospace(12.0);
        let missing: Vec<String> = super::UI_SYMBOLS
            .chars()
            .filter(|symbol| !ctx.fonts(|fonts| fonts.has_glyph(&font, *symbol)))
            .map(|symbol| format!("U+{:04X} {symbol}", symbol as u32))
            .collect();
        assert!(missing.is_empty(), "no glyph for: {missing:?}");
    }

    #[test]
    fn process_descriptions_name_the_culprit_until_it_stops_helping() {
        assert_eq!(describe_processes(&[]), "nothing");
        assert_eq!(
            describe_processes(&[process("arachnia.exe", 3852)]),
            "arachnia.exe (pid 3852)"
        );
        assert_eq!(
            describe_processes(&[process("a.exe", 1), process("b.exe", 2)]),
            "a.exe (pid 1) and b.exe (pid 2)"
        );
        assert_eq!(
            describe_processes(&[process("a.exe", 1), process("b.exe", 2), process("c.exe", 3)]),
            "3 processes"
        );
    }

    #[test]
    fn build_health_shows_the_trend_not_just_the_count() {
        let first_run = BuildHealth { warnings: 992, errors: 0, delta: None };
        assert_eq!(format_build_health(&first_run), "992 warnings");

        let grown = BuildHealth { warnings: 992, errors: 0, delta: Some(37) };
        assert_eq!(format_build_health(&grown), "992 warnings (+37)");

        let improved = BuildHealth { warnings: 955, errors: 0, delta: Some(-37) };
        assert_eq!(format_build_health(&improved), "955 warnings (-37)");

        // An unchanged count is noise; only a real delta is worth the space.
        let flat = BuildHealth { warnings: 992, errors: 0, delta: Some(0) };
        assert_eq!(format_build_health(&flat), "992 warnings");

        let failed = BuildHealth { warnings: 992, errors: 1, delta: None };
        assert_eq!(format_build_health(&failed), "992 warnings \u{00B7} 1 error");

        let clean = BuildHealth { warnings: 0, errors: 0, delta: Some(-12) };
        assert_eq!(format_build_health(&clean), "no warnings (-12)");
    }

    #[test]
    fn problem_cursor_wraps_at_both_ends() {
        assert_eq!(next_problem_index(3, 0, 1), 1);
        assert_eq!(next_problem_index(3, 2, 1), 0, "forward past the end wraps");
        assert_eq!(next_problem_index(3, 0, -1), 2, "backward past the start wraps");
        assert_eq!(next_problem_index(0, 0, 1), 0, "empty list is inert");
        // The problem list shrinks when the terminal is cleared mid-session.
        assert_eq!(next_problem_index(2, 99, 1), 0);
    }

    #[test]
    fn problem_filter_selects_warnings_and_failures_only() {
        use crate::execution::{TerminalLine, TerminalLineKind};
        assert!(is_problem_line(&TerminalLine::new("e", TerminalLineKind::Error)));
        assert!(is_problem_line(&TerminalLine::new("w", TerminalLineKind::Warning)));
        assert!(is_problem_line(&TerminalLine::new("f", TerminalLineKind::Failure)));
        assert!(!is_problem_line(&TerminalLine::new("o", TerminalLineKind::Output)));
        assert!(!is_problem_line(&TerminalLine::new("i", TerminalLineKind::Info)));
        assert!(!is_problem_line(&TerminalLine::new("s", TerminalLineKind::Success)));
    }
    use crate::discovery::Toolchain;
    use crate::execution::{ActionKind, ExecutionPlan, ExecutionReport};
    use crate::history::HistoryEntry;
    use std::path::PathBuf;

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("vaultforge-{name}-{nanos}"))
    }

    #[test]
    fn portable_executable_uses_its_own_folder_when_it_holds_projects() {
        let root = unique_temp_dir("portable-root");
        let exe_path = root.join("VaultForge.exe");
        std::fs::create_dir_all(root.join("some-crate")).unwrap();
        std::fs::write(root.join("some-crate").join("Cargo.toml"), "[package]\nname='x'\n").unwrap();

        let detected = scan_root_for_executable_path(&exe_path).unwrap();
        assert_eq!(detected, root);

        let _ = std::fs::remove_dir_all(detected);
    }

    /// Unzipped into Downloads or installed under Programs, the exe's own
    /// folder holds no projects; the default must look elsewhere.
    #[test]
    fn executable_folder_without_projects_is_not_a_scan_root() {
        let root = unique_temp_dir("downloads-root");
        std::fs::create_dir_all(root.join("readme-folder")).unwrap();
        assert!(scan_root_for_executable_path(&root.join("VaultForge.exe")).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cargo_target_executable_uses_project_parent() {
        let root = unique_temp_dir("dev-root");
        let project_dir = root.join("VaultForger");
        let target_dir = project_dir.join("target").join("debug");
        let exe_path = target_dir.join("VaultForge.exe");

        std::fs::create_dir_all(project_dir.join("src")).unwrap();
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::write(project_dir.join("Cargo.toml"), "[package]\nname='vaultforge'\nversion='0.0.0'\n").unwrap();

        let detected = scan_root_for_executable_path(&exe_path).unwrap();
        assert_eq!(detected, root);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn successful_python_run_updates_preferred_entry_history() {
        let root = unique_temp_dir("python-history-entry");
        let workspace = root.join("workspace");
        let entry_path = workspace.join("scripts").join("tool.py");
        std::fs::create_dir_all(entry_path.parent().unwrap()).unwrap();
        std::fs::write(&entry_path, "print('hello')\n").unwrap();

        let report = ExecutionReport {
            plan: ExecutionPlan {
                toolchain: Toolchain::Python,
                action: ActionKind::Run,
                action_label: "Run".to_string(),
                program: "py".to_string(),
                args: vec!["-3".to_string(), entry_path.display().to_string()],
                cwd: workspace,
                env_sets: Vec::new(),
                env_removals: Vec::new(),
            },
            success: true,
            cancelled: false,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            finished_at_unix: 20,
            duration_ms: 10_000,
            unit_count: 0,
            fresh_units: 0,
            errors_total: 0,
            warnings_total: 0,
            error_digest: Vec::new(),
            log_path: None,
        };

        let mut history_entry = HistoryEntry::default();
        update_history_entry_from_report(&mut history_entry, &report);

        assert_eq!(history_entry.last_launched_at, Some(20));
        assert_eq!(
            history_entry.preferred_python_entry.as_deref(),
            Some("scripts/tool.py")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preferred_python_entry_storage_value_uses_workspace_relative_paths() {
        let root = unique_temp_dir("python-history-relative");
        let workspace = root.join("workspace");
        let entry_path = workspace.join("scripts").join("tool.py");
        std::fs::create_dir_all(entry_path.parent().unwrap()).unwrap();
        std::fs::write(&entry_path, "print('hello')\n").unwrap();

        let stored = preferred_python_entry_storage_value(&workspace, &entry_path.display().to_string()).unwrap();
        assert_eq!(stored, "scripts/tool.py");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn python_preferred_entry_from_report_rejects_outside_paths() {
        let root = unique_temp_dir("python-history-boundary");
        let workspace = root.join("workspace");
        let outside = root.join("outside.py");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(&outside, "print('nope')\n").unwrap();

        let report = ExecutionReport {
            plan: ExecutionPlan {
                toolchain: Toolchain::Python,
                action: ActionKind::Run,
                action_label: "Run".to_string(),
                program: "py".to_string(),
                args: vec!["-3".to_string(), outside.display().to_string()],
                cwd: workspace,
                env_sets: Vec::new(),
                env_removals: Vec::new(),
            },
            success: true,
            cancelled: false,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            finished_at_unix: 20,
            duration_ms: 10_000,
            unit_count: 0,
            fresh_units: 0,
            errors_total: 0,
            warnings_total: 0,
            error_digest: Vec::new(),
            log_path: None,
        };

        assert!(preferred_python_entry_from_report(&report).is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn successful_rust_build_records_last_artifact_from_cargo_json() {
        let root = unique_temp_dir("rust-history-artifact");
        let artifact = root.join("target").join("debug").join("tool.exe");
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, b"artifact").unwrap();

        let report = ExecutionReport {
            plan: ExecutionPlan {
                toolchain: Toolchain::Rust,
                action: ActionKind::Build,
                action_label: "Build".to_string(),
                program: "cargo".to_string(),
                args: vec!["build".to_string(), "--message-format=json-render-diagnostics".to_string()],
                cwd: root.clone(),
                env_sets: Vec::new(),
                env_removals: Vec::new(),
            },
            success: true,
            cancelled: false,
            exit_code: Some(0),
            stdout: format!(
                "{{\"reason\":\"compiler-artifact\",\"executable\":{}}}\n",
                serde_json::to_string(&artifact.display().to_string()).unwrap()
            ),
            stderr: String::new(),
            finished_at_unix: 20,
            duration_ms: 10_000,
            unit_count: 0,
            fresh_units: 0,
            errors_total: 0,
            warnings_total: 0,
            error_digest: Vec::new(),
            log_path: None,
        };

        let mut history_entry = HistoryEntry::default();
        update_history_entry_from_report(&mut history_entry, &report);

        let artifact_display = artifact.display().to_string();

        assert_eq!(history_entry.last_built_at, Some(20));
        assert_eq!(history_entry.last_successful_artifact.as_deref(), Some(artifact_display.as_str()));
        assert_eq!(last_successful_artifact_from_report(&report).as_deref(), Some(artifact_display.as_str()));

        let _ = std::fs::remove_dir_all(root);
    }
}

fn reveal_in_explorer(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|err| err.to_string())
    }
}

fn default_action_for(target: &WorkspaceTarget) -> Option<String> {
    // Default-action selection is profile-independent: availability doesn't change with
    // build options, so probe with defaults.
    let options = BuildOptions::default();
    ["Run", "Build", "Check", "Test", "Package"]
        .into_iter()
        .find(|action| build_plan(target, action, &options).is_ok())
        .map(str::to_string)
}

