use crate::history::{HistoryEntry, SharedHistory};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::{DirEntry, WalkDir};

const EXCLUDED_DIRS: &[&str] = &[
    "venv",
    ".venv",
    "env",
    ".env",
    "node_modules",
    "__pycache__",
    ".git",
    ".hg",
    ".svn",
    ".idea",
    ".vscode",
    "site-packages",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".eggs",
    ".cargo",
    "target",
    "bin",
    "obj",
    ".gradle",
    ".next",
    ".nuxt",
    "vendor",
    "coverage",
    ".cache",
    "Lib",
    "lib",
    "Scripts",
    "Include",
    "build",
    "dist",
];

const PYTHON_MARKERS: &[&str] = &[
    "pyproject.toml",
    "requirements.txt",
    "setup.py",
    "setup.cfg",
    "Pipfile",
];

const RECENT_WINDOW_SECS: u64 = 14 * 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Toolchain {
    Rust,
    Python,
    Mixed,
}

impl Toolchain {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::Mixed => "Mixed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceTarget {
    pub id: String,
    pub root_path: PathBuf,
    pub project_name: String,
    pub toolchain: Toolchain,
    pub confidence: u8,
    pub last_modified_unix: u64,
    pub markers: Vec<String>,
    pub available_actions: Vec<String>,
    pub primary_entry_hint: Option<String>,
    pub last_built_at: Option<u64>,
    pub last_launched_at: Option<u64>,
    pub pinned: bool,
    pub ranking_score: i64,
}

impl WorkspaceTarget {
    pub fn age_label(&self) -> String {
        age_label(self.last_modified_unix)
    }

    pub fn short_root(&self) -> String {
        self.root_path.to_string_lossy().to_string()
    }

    pub fn history_label(&self) -> String {
        let mut labels = Vec::new();
        if let Some(ts) = self.last_built_at {
            labels.push(format!("built {}", age_label(ts)));
        }
        if let Some(ts) = self.last_launched_at {
            labels.push(format!("launched {}", age_label(ts)));
        }
        if self.pinned {
            labels.push("pinned".to_string());
        }
        if labels.is_empty() {
            "no shared history yet".to_string()
        } else {
            labels.join(" · ")
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SearchChip {
    Rust,
    Python,
    Pinned,
    Recent,
    HasRun,
    MarkerCargoToml,
}

impl SearchChip {
    pub const ALL: [Self; 6] = [
        Self::Rust,
        Self::Python,
        Self::Pinned,
        Self::Recent,
        Self::HasRun,
        Self::MarkerCargoToml,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rust => "RUST",
            Self::Python => "PYTHON",
            Self::Pinned => "PINNED",
            Self::Recent => "RECENT",
            Self::HasRun => "HAS:RUN",
            Self::MarkerCargoToml => "MARKER:CARGO",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SearchRequest {
    terms: Vec<String>,
    chips: Vec<SearchChip>,
}

impl SearchRequest {
    pub fn from_input(query: &str, active_chips: &[SearchChip]) -> Self {
        let mut chips = Vec::new();
        for chip in active_chips {
            push_unique_chip(&mut chips, *chip);
        }

        let mut terms = Vec::new();
        for token in query.split_whitespace() {
            let normalized = token.trim().to_lowercase();
            if normalized.is_empty() {
                continue;
            }

            match normalized.as_str() {
                "pinned" => push_unique_chip(&mut chips, SearchChip::Pinned),
                "recent" => push_unique_chip(&mut chips, SearchChip::Recent),
                "has:run" => push_unique_chip(&mut chips, SearchChip::HasRun),
                "marker:cargo.toml" | "marker:cargo" => {
                    push_unique_chip(&mut chips, SearchChip::MarkerCargoToml)
                }
                "toolchain:rust" => push_unique_chip(&mut chips, SearchChip::Rust),
                "toolchain:python" => push_unique_chip(&mut chips, SearchChip::Python),
                _ => terms.push(normalized),
            }
        }

        Self { terms, chips }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkspaceCatalog {
    scan_root: PathBuf,
    indexed_at_unix: u64,
    entries: Vec<IndexedWorkspace>,
}

impl WorkspaceCatalog {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn indexed_at_unix(&self) -> u64 {
        self.indexed_at_unix
    }

    #[cfg(test)]
    pub fn scan_root(&self) -> &Path {
        &self.scan_root
    }
}

/// Last completed scan, persisted so a launch can show the catalog at once and
/// refresh it in the background — a cold walk of a large root takes seconds.
fn catalog_cache_path() -> PathBuf {
    crate::history::shared_history_path().with_file_name("vaultforge-catalog.json")
}

/// The cached catalog, if one exists for exactly this scan root. A cache from an
/// older build that no longer deserializes is simply ignored.
pub fn load_cached_catalog(scan_root: &Path) -> Option<WorkspaceCatalog> {
    let contents = std::fs::read_to_string(catalog_cache_path()).ok()?;
    let catalog: WorkspaceCatalog = serde_json::from_str(&contents).ok()?;
    (catalog.scan_root == scan_root).then_some(catalog)
}

pub fn save_cached_catalog(catalog: &WorkspaceCatalog) {
    let path = catalog_cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec(catalog) {
        let _ = crate::history::write_atomically(&path, &json);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct IndexedWorkspace {
    id: String,
    root_path: PathBuf,
    project_name: String,
    project_search: String,
    path_search: String,
    path_segments: Vec<String>,
    toolchain: Toolchain,
    toolchain_search: String,
    contains_rust: bool,
    contains_python: bool,
    confidence: u8,
    last_modified_unix: u64,
    markers: Vec<String>,
    marker_search: Vec<String>,
    has_run_action: bool,
    primary_entry_hint: Option<String>,
}

#[derive(Default)]
struct CandidateRoot {
    markers: BTreeSet<String>,
    last_modified_unix: u64,
}

pub fn build_workspace_catalog(scan_root: &Path, previous: Option<&WorkspaceCatalog>) -> WorkspaceCatalog {
    let roots = collect_candidates_with_recency(scan_root);

    let previous_entries: HashMap<String, IndexedWorkspace> = previous
        .filter(|catalog| catalog.scan_root == scan_root)
        .map(|catalog| {
            catalog
                .entries
                .iter()
                .cloned()
                .map(|entry| (entry.id.clone(), entry))
                .collect()
        })
        .unwrap_or_default();

    let mut entries = Vec::with_capacity(roots.len());
    for (root_path, candidate) in roots {
        let id = crate::history::normalized_path_key(&root_path);
        let markers: Vec<String> = candidate.markers.iter().cloned().collect();
        let reused = previous_entries.get(&id).filter(|entry| {
            entry.last_modified_unix == candidate.last_modified_unix && entry.markers == markers
        });

        let indexed = if let Some(entry) = reused {
            entry.clone()
        } else {
            build_indexed_workspace(root_path, candidate, markers, id)
        };
        entries.push(indexed);
    }

    entries.sort_by(|left, right| {
        left.project_name
            .to_lowercase()
            .cmp(&right.project_name.to_lowercase())
            .then(left.root_path.cmp(&right.root_path))
    });

    WorkspaceCatalog {
        scan_root: scan_root.to_path_buf(),
        indexed_at_unix: unix_now(),
        entries,
    }
}

pub fn query_catalog(
    catalog: &WorkspaceCatalog,
    history: &SharedHistory,
    request: &SearchRequest,
) -> Vec<WorkspaceTarget> {
    let now = unix_now();
    let mut targets = Vec::new();

    for entry in &catalog.entries {
        // `entry.id` is already the normalized history key; `entry_for` would
        // canonicalize every workspace path again on each keystroke.
        let history_entry = history.entry_for_key(&entry.id).cloned().unwrap_or_default();
        if !matches_chip_filters(entry, &history_entry, &request.chips, now) {
            continue;
        }

        let Some(text_score) = query_text_score(entry, &request.terms) else {
            continue;
        };

        let history_recency = latest_activity_timestamp(entry.last_modified_unix, &history_entry);
        let ranking_score = if history_entry.pinned { 4_000_000_000 } else { 0 }
            + text_score * 1_000_000
            + recency_bonus(history_recency, now, 180)
            + recency_bonus(entry.last_modified_unix, now, 600)
            + i64::from(entry.confidence) * 1_000;
        let primary_entry_hint = resolved_primary_entry_hint(entry, &history_entry);
        let available_actions = available_actions(&entry.toolchain, &entry.markers, primary_entry_hint.is_some());

        targets.push(WorkspaceTarget {
            id: entry.id.clone(),
            root_path: entry.root_path.clone(),
            project_name: entry.project_name.clone(),
            toolchain: entry.toolchain.clone(),
            confidence: entry.confidence,
            last_modified_unix: entry.last_modified_unix,
            markers: entry.markers.clone(),
            available_actions,
            primary_entry_hint,
            last_built_at: history_entry.last_built_at,
            last_launched_at: history_entry.last_launched_at,
            pinned: history_entry.pinned,
            ranking_score,
        });
    }

    targets.sort_by(|left, right| {
        right
            .ranking_score
            .cmp(&left.ranking_score)
            .then(right.last_modified_unix.cmp(&left.last_modified_unix))
            .then(left.project_name.to_lowercase().cmp(&right.project_name.to_lowercase()))
    });

    targets
}

fn build_indexed_workspace(
    root_path: PathBuf,
    candidate: CandidateRoot,
    markers: Vec<String>,
    id: String,
) -> IndexedWorkspace {
    let toolchain = detect_toolchain(&candidate.markers);
    let project_name = root_path
        .file_name()
        .unwrap_or(root_path.as_os_str())
        .to_string_lossy()
        .to_string();
    let primary_entry_hint = detect_primary_entry(&root_path, &toolchain);
    let available_actions = available_actions(&toolchain, &markers, primary_entry_hint.is_some());
    let path_search = root_path.to_string_lossy().replace('\\', "/").to_lowercase();

    IndexedWorkspace {
        id,
        root_path,
        project_name: project_name.clone(),
        project_search: project_name.to_lowercase(),
        path_segments: path_search
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_string)
            .collect(),
        path_search,
        toolchain_search: toolchain.label().to_lowercase(),
        contains_rust: markers.iter().any(|marker| marker == "Cargo.toml"),
        contains_python: markers.iter().any(|marker| PYTHON_MARKERS.contains(&marker.as_str())),
        confidence: confidence_for_markers(&markers),
        last_modified_unix: candidate.last_modified_unix,
        markers: markers.clone(),
        marker_search: markers.iter().map(|marker| marker.to_lowercase()).collect(),
        has_run_action: available_actions.iter().any(|action| action == "Run"),
        primary_entry_hint,
        toolchain,
    }
}

fn collect_candidates_with_recency(scan_root: &Path) -> HashMap<PathBuf, CandidateRoot> {
    // Single WalkDir pass: collect workspace markers and per-directory max mtimes together,
    // then apply hierarchy pruning and propagate mtimes up to ancestor roots in memory.
    let mut raw_markers: HashMap<PathBuf, BTreeSet<String>> = HashMap::new();
    let mut dir_max_mtime: HashMap<PathBuf, u64> = HashMap::new();

    let walker = WalkDir::new(scan_root)
        .follow_links(false)
        .max_depth(14)
        .into_iter()
        .filter_entry(should_descend);

    for entry in walker.filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let parent = path.parent().unwrap_or(scan_root).to_path_buf();

        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            if is_workspace_marker(file_name) {
                raw_markers.entry(parent.clone()).or_default().insert(file_name.to_string());
            }
        }

        // `DirEntry::metadata` reuses the directory listing's find data on
        // Windows; `path.metadata()` opened every file (and woke the virus
        // scanner for each), which made a cold scan of ~200 workspaces take 37s.
        let mtime = entry
            .metadata()
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .map_or(0, system_time_unix);
        let slot = dir_max_mtime.entry(parent).or_insert(0);
        *slot = (*slot).max(mtime);
    }

    let all_root_paths: BTreeSet<PathBuf> = raw_markers.keys().cloned().collect();
    let strong_roots: BTreeSet<PathBuf> = raw_markers
        .iter()
        .filter(|(_, markers)| markers.iter().any(|m| is_strong_workspace_marker(m)))
        .map(|(path, _)| path.clone())
        .collect();

    let mut roots: HashMap<PathBuf, CandidateRoot> = HashMap::new();
    for (root_path, markers) in raw_markers {
        let has_strong = markers.iter().any(|m| is_strong_workspace_marker(m));
        if has_strong
            || (!has_ancestor_strong_root(&root_path, &strong_roots)
                && !has_descendant_root(&root_path, &all_root_paths))
        {
            roots.insert(root_path, CandidateRoot { markers, last_modified_unix: 0 });
        }
    }

    // Propagate per-directory max mtimes upward to ancestor workspace roots.
    // Each directory's mtime represents the max mtime of files directly inside it;
    // walking up covers nested workspace roots correctly (e.g. parent Cargo workspace
    // receives updates from files inside a child crate).
    for (dir, &mtime) in &dir_max_mtime {
        let mut current: Option<&Path> = Some(dir.as_path());
        while let Some(path) = current {
            if let Some(root) = roots.get_mut(path) {
                root.last_modified_unix = root.last_modified_unix.max(mtime);
            }
            current = path.parent();
        }
    }

    roots
}

fn is_strong_workspace_marker(file_name: &str) -> bool {
    matches!(file_name, "Cargo.toml" | "pyproject.toml" | "setup.py" | "Pipfile")
}

fn has_ancestor_strong_root(root_path: &Path, strong_roots: &BTreeSet<PathBuf>) -> bool {
    root_path
        .ancestors()
        .skip(1)
        .any(|ancestor| strong_roots.contains(ancestor))
}

fn has_descendant_root(root_path: &Path, all_roots: &BTreeSet<PathBuf>) -> bool {
    all_roots
        .iter()
        .any(|other| other != root_path && other.starts_with(root_path))
}

fn should_descend(entry: &DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return true;
    }

    let name = entry.file_name().to_string_lossy();
    !EXCLUDED_DIRS.contains(&name.as_ref()) && !name.ends_with(".egg-info")
}

fn is_workspace_marker(file_name: &str) -> bool {
    file_name == "Cargo.toml" || PYTHON_MARKERS.contains(&file_name)
}

fn detect_toolchain(markers: &BTreeSet<String>) -> Toolchain {
    let has_rust = markers.contains("Cargo.toml");
    let has_python = markers.iter().any(|marker| PYTHON_MARKERS.contains(&marker.as_str()));
    match (has_rust, has_python) {
        (true, true) => Toolchain::Mixed,
        (true, false) => Toolchain::Rust,
        (false, true) => Toolchain::Python,
        (false, false) => Toolchain::Mixed,
    }
}

fn confidence_for_markers(markers: &[String]) -> u8 {
    if markers.iter().any(|marker| marker == "Cargo.toml" || marker == "pyproject.toml") {
        100
    } else if markers.iter().any(|marker| marker == "Pipfile" || marker == "setup.py") {
        90
    } else {
        75
    }
}

fn available_actions(toolchain: &Toolchain, _markers: &[String], has_entry_hint: bool) -> Vec<String> {
    match toolchain {
        Toolchain::Rust => {
            let mut actions = vec!["Check", "Build"];
            if has_entry_hint {
                actions.push("Run");
            }
            actions.extend(["Test", "Lint", "Clean"]);
            actions.into_iter().map(String::from).collect()
        }
        Toolchain::Python => {
            let mut actions = vec!["Check".to_string()];
            if has_entry_hint {
                actions.push("Run".to_string());
            }
            actions
        }
        Toolchain::Mixed => Vec::new(),
    }
}

fn detect_primary_entry(root: &Path, toolchain: &Toolchain) -> Option<String> {
    match toolchain {
        Toolchain::Rust => detect_rust_entry(root),
        Toolchain::Python => detect_python_entry(root),
        Toolchain::Mixed => detect_rust_entry(root).or_else(|| detect_python_entry(root)),
    }
}

fn detect_rust_entry(root: &Path) -> Option<String> {
    let main_rs = root.join("src").join("main.rs");
    if main_rs.is_file() {
        return Some(main_rs.to_string_lossy().to_string());
    }

    let bin_dir = root.join("src").join("bin");
    let Ok(entries) = std::fs::read_dir(bin_dir) else {
        return None;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            return Some(path.to_string_lossy().to_string());
        }
    }
    None
}

fn detect_python_entry(root: &Path) -> Option<String> {
    let candidates = [
        root.join("main.py"),
        root.join("app.py"),
        root.join("run.py"),
        root.join("__main__.py"),
        root.join("src").join("main.py"),
    ];
    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

fn resolved_primary_entry_hint(entry: &IndexedWorkspace, history_entry: &HistoryEntry) -> Option<String> {
    validated_preferred_python_entry(&entry.root_path, history_entry.preferred_python_entry.as_deref())
        .or_else(|| entry.primary_entry_hint.clone())
}

pub(crate) fn validated_preferred_python_entry(root_path: &Path, preferred_entry: Option<&str>) -> Option<String> {
    let preferred_entry = preferred_entry?;
    let root = std::fs::canonicalize(root_path).unwrap_or_else(|_| root_path.to_path_buf());
    let candidate = PathBuf::from(preferred_entry);
    let candidate = if candidate.is_absolute() {
        candidate
    } else {
        root.join(candidate)
    };
    let candidate = std::fs::canonicalize(candidate).ok()?;

    if !candidate.starts_with(&root) {
        return None;
    }
    if candidate.extension().and_then(|ext| ext.to_str()) != Some("py") {
        return None;
    }

    candidate.is_file().then(|| candidate.to_string_lossy().to_string())
}

fn push_unique_chip(chips: &mut Vec<SearchChip>, chip: SearchChip) {
    if !chips.contains(&chip) {
        chips.push(chip);
    }
}

fn matches_chip_filters(
    entry: &IndexedWorkspace,
    history_entry: &HistoryEntry,
    chips: &[SearchChip],
    now: u64,
) -> bool {
    chips.iter().all(|chip| match chip {
        SearchChip::Rust => entry.contains_rust,
        SearchChip::Python => entry.contains_python,
        SearchChip::Pinned => history_entry.pinned,
        SearchChip::Recent => is_recent(latest_activity_timestamp(entry.last_modified_unix, history_entry), now),
        SearchChip::HasRun => entry.has_run_action,
        SearchChip::MarkerCargoToml => entry.markers.iter().any(|marker| marker == "Cargo.toml"),
    })
}

fn query_text_score(entry: &IndexedWorkspace, terms: &[String]) -> Option<i64> {
    if terms.is_empty() {
        return Some(1);
    }

    let mut total = 0;
    for term in terms {
        total += term_score(entry, term)?;
    }
    Some(total)
}

fn term_score(entry: &IndexedWorkspace, term: &str) -> Option<i64> {
    if entry.project_search == term {
        Some(240)
    } else if entry.project_search.starts_with(term) {
        Some(190)
    } else if entry.path_segments.iter().any(|segment| segment == term) {
        Some(170)
    } else if entry.project_search.contains(term) {
        Some(150)
    } else if entry.path_segments.iter().any(|segment| segment.starts_with(term)) {
        Some(125)
    } else if entry.marker_search.iter().any(|marker| marker == term) {
        Some(110)
    } else if entry.toolchain_search == term {
        Some(95)
    } else if entry.marker_search.iter().any(|marker| marker.contains(term)) {
        Some(80)
    } else if entry.path_segments.iter().any(|segment| segment.contains(term)) {
        Some(68)
    } else if entry.path_search.contains(term) {
        Some(50)
    } else {
        None
    }
}

fn latest_activity_timestamp(last_modified_unix: u64, history_entry: &HistoryEntry) -> u64 {
    [
        last_modified_unix,
        history_entry.last_built_at.unwrap_or(0),
        history_entry.last_launched_at.unwrap_or(0),
    ]
    .into_iter()
    .max()
    .unwrap_or(0)
}

fn is_recent(timestamp: u64, now: u64) -> bool {
    timestamp > 0 && now.saturating_sub(timestamp) <= RECENT_WINDOW_SECS
}

fn recency_bonus(timestamp: u64, now: u64, divisor: u64) -> i64 {
    if timestamp == 0 {
        return 0;
    }

    let age = now.saturating_sub(timestamp).min(RECENT_WINDOW_SECS);
    ((RECENT_WINDOW_SECS - age) / divisor) as i64
}

#[cfg(test)]
fn modified_unix(path: &Path) -> u64 {
    path.metadata()
        .and_then(|metadata| metadata.modified())
        .map_or(0, system_time_unix)
}

fn system_time_unix(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn age_label(unix_secs: u64) -> String {
    if unix_secs == 0 {
        return "unknown".to_string();
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
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

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{SearchRequest, Toolchain, available_actions, build_workspace_catalog, query_catalog, validated_preferred_python_entry};
    use crate::history::{HistoryEntry, SharedHistory, normalized_path_key};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::time::Duration;

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("vaultforge-discovery-{name}-{nanos}"))
    }

    fn rewrite_until_newer_unix(path: &std::path::Path, bytes: &[u8], threshold: u64) -> u64 {
        for _ in 0..4 {
            std::fs::write(path, bytes).unwrap();
            let modified = super::modified_unix(path);
            if modified > threshold {
                return modified;
            }
            std::thread::sleep(Duration::from_millis(1_100));
        }

        panic!("failed to create a newer unix timestamp for discovery test");
    }

    #[test]
    fn rust_actions_match_the_current_executor_surface() {
        let actions = available_actions(&Toolchain::Rust, &[], true);
        assert_eq!(actions, vec!["Check", "Build", "Run", "Test", "Lint", "Clean"]);
    }

    #[test]
    fn rust_run_requires_a_detected_entry() {
        let actions = available_actions(&Toolchain::Rust, &[], false);
        assert_eq!(actions, vec!["Check", "Build", "Test", "Lint", "Clean"]);
    }

    #[test]
    fn python_actions_only_include_runnable_operations() {
        let without_entry = available_actions(&Toolchain::Python, &[], false);
        assert_eq!(without_entry, vec!["Check"]);

        let with_entry = available_actions(&Toolchain::Python, &[], true);
        assert_eq!(with_entry, vec!["Check", "Run"]);
    }

    #[test]
    fn mixed_actions_stay_empty_until_adapter_execution_exists() {
        let actions = available_actions(&Toolchain::Mixed, &[], true);
        assert!(actions.is_empty());
    }

    #[test]
    fn preferred_python_entry_enables_run_when_detection_is_missing() {
        let root = unique_temp_dir("preferred-entry");
        let workspace = root.join("workspace");
        let entry = workspace.join("scripts").join("tool.py");

        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::fs::write(workspace.join("pyproject.toml"), "[project]\nname='vaultforge-python'\n").unwrap();
        std::fs::write(&entry, "print('hello')\n").unwrap();

        let catalog = build_workspace_catalog(&root, None);
        let mut history = SharedHistory {
            version: 1,
            entries: HashMap::new(),
            ..Default::default()
        };
        history.entries.insert(
            normalized_path_key(&workspace),
            HistoryEntry {
                preferred_python_entry: Some(entry.display().to_string()),
                ..Default::default()
            },
        );

        let targets = query_catalog(&catalog, &history, &SearchRequest::default());
        let target = targets.iter().find(|target| target.root_path == workspace).unwrap();
        let expected_entry = std::fs::canonicalize(&entry).unwrap().display().to_string();

        assert_eq!(target.primary_entry_hint.as_deref(), Some(expected_entry.as_str()));
        assert!(target.available_actions.iter().any(|action| action == "Run"));

        let _ = std::fs::remove_dir_all(root);
    }

    /// The on-disk cache must reproduce the catalog exactly — it is what the
    /// user sees at launch until the background rescan lands.
    #[test]
    fn catalog_survives_the_cache_round_trip() {
        let root = unique_temp_dir("catalog-cache");
        let workspace = root.join("crate-a");
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::write(workspace.join("Cargo.toml"), "[package]\nname='a'\n").unwrap();
        std::fs::write(workspace.join("src").join("main.rs"), "fn main() {}\n").unwrap();

        let catalog = build_workspace_catalog(&root, None);
        let json = serde_json::to_string(&catalog).unwrap();
        let restored: super::WorkspaceCatalog = serde_json::from_str(&json).unwrap();

        let history = SharedHistory::default();
        let before = query_catalog(&catalog, &history, &SearchRequest::default());
        let after = query_catalog(&restored, &history, &SearchRequest::default());
        assert_eq!(before.len(), 1);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].id, before[0].id);
        assert_eq!(after[0].available_actions, before[0].available_actions);
        assert_eq!(restored.scan_root(), root.as_path());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preferred_python_entry_overrides_detected_entry() {
        let root = unique_temp_dir("preferred-entry-override");
        let workspace = root.join("workspace");
        let detected = workspace.join("app.py");
        let preferred = workspace.join("scripts").join("tool.py");

        std::fs::create_dir_all(preferred.parent().unwrap()).unwrap();
        std::fs::write(workspace.join("pyproject.toml"), "[project]\nname='vaultforge-python'\n").unwrap();
        std::fs::write(&detected, "print('detected')\n").unwrap();
        std::fs::write(&preferred, "print('preferred')\n").unwrap();

        let catalog = build_workspace_catalog(&root, None);
        let mut history = SharedHistory {
            version: 1,
            entries: HashMap::new(),
            ..Default::default()
        };
        history.entries.insert(
            normalized_path_key(&workspace),
            HistoryEntry {
                preferred_python_entry: Some("scripts/tool.py".to_string()),
                ..Default::default()
            },
        );

        let targets = query_catalog(&catalog, &history, &SearchRequest::default());
        let target = targets.iter().find(|target| target.root_path == workspace).unwrap();
        let expected_entry = std::fs::canonicalize(&preferred).unwrap().display().to_string();

        assert_eq!(target.primary_entry_hint.as_deref(), Some(expected_entry.as_str()));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn preferred_python_entry_must_stay_inside_the_workspace() {
        let root = unique_temp_dir("preferred-entry-boundary");
        let workspace = root.join("workspace");
        let outside = root.join("outside.py");

        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(&outside, "print('outside')\n").unwrap();

        let resolved = validated_preferred_python_entry(&workspace, Some(outside.to_string_lossy().as_ref()));
        assert!(resolved.is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn weak_root_is_pruned_when_nested_strong_root_exists() {
        let root = unique_temp_dir("weak-shadowed-by-strong");
        let parent = root.join("parent");
        let child = parent.join("nested");

        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(parent.join("requirements.txt"), "pytest\n").unwrap();
        std::fs::write(child.join("pyproject.toml"), "[project]\nname='nested'\n").unwrap();

        let catalog = build_workspace_catalog(&root, None);
        let paths: Vec<_> = catalog.entries.iter().map(|entry| entry.root_path.clone()).collect();

        assert!(!paths.contains(&parent));
        assert!(paths.contains(&child));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn nested_strong_roots_are_kept() {
        let root = unique_temp_dir("nested-strong-roots");
        let parent = root.join("workspace");
        let child = parent.join("crate-a");

        std::fs::create_dir_all(parent.join("src")).unwrap();
        std::fs::create_dir_all(child.join("src")).unwrap();
        std::fs::write(parent.join("Cargo.toml"), "[package]\nname='workspace'\nversion='0.0.0'\n").unwrap();
        std::fs::write(child.join("Cargo.toml"), "[package]\nname='crate-a'\nversion='0.0.0'\n").unwrap();

        let catalog = build_workspace_catalog(&root, None);
        let paths: Vec<_> = catalog.entries.iter().map(|entry| entry.root_path.clone()).collect();

        assert!(paths.contains(&parent));
        assert!(paths.contains(&child));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_recency_propagates_to_all_ancestor_roots() {
        let root = unique_temp_dir("recency-propagation");
        let parent = root.join("workspace");
        let child = parent.join("crate-a");
        let child_file = child.join("src").join("main.rs");

        std::fs::create_dir_all(parent.join("src")).unwrap();
        std::fs::create_dir_all(child.join("src")).unwrap();
        std::fs::write(parent.join("Cargo.toml"), "[package]\nname='workspace'\nversion='0.0.0'\n").unwrap();
        std::fs::write(child.join("Cargo.toml"), "[package]\nname='crate-a'\nversion='0.0.0'\n").unwrap();
        std::fs::write(&child_file, "fn main() {}\n").unwrap();

        let baseline = super::modified_unix(&child_file);
        let updated = rewrite_until_newer_unix(&child_file, b"fn main() { println!(\"hi\"); }\n", baseline);

        let catalog = build_workspace_catalog(&root, None);
        let parent_entry = catalog.entries.iter().find(|entry| entry.root_path == parent).unwrap();
        let child_entry = catalog.entries.iter().find(|entry| entry.root_path == child).unwrap();

        assert!(parent_entry.last_modified_unix >= updated);
        assert_eq!(parent_entry.last_modified_unix, child_entry.last_modified_unix);

        let _ = std::fs::remove_dir_all(root);
    }
}
