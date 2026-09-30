use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SharedHistory {
    pub version: u32,
    pub entries: HashMap<String, HistoryEntry>,
    /// Learned build durations keyed by a per-workspace command signature. Drives
    /// the smooth time-based progress bar (see execution::build_progress_fraction).
    #[serde(default)]
    pub builds: HashMap<String, LearnedBuild>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct LearnedBuild {
    pub unit_count: u32,
    pub duration_ms: u64,
    /// Diagnostic counts from the most recent completed run of this signature.
    #[serde(default)]
    pub warnings: u32,
    #[serde(default)]
    pub errors: u32,
    /// Warning count from the run before that, so growth in warning debt is
    /// visible. `None` until a second run exists to compare against.
    #[serde(default)]
    pub prev_warnings: Option<u32>,
    /// Whether any outcome has been recorded — distinguishes "first run" from
    /// "previous run had zero warnings".
    #[serde(default)]
    pub recorded_outcome: bool,
}

impl LearnedBuild {
    /// Change in warning count since the previous run, or `None` on a first run.
    pub fn warning_delta(&self) -> Option<i64> {
        self.prev_warnings
            .map(|prev| i64::from(self.warnings) - i64::from(prev))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub last_built_at: Option<u64>,
    pub last_launched_at: Option<u64>,
    pub last_successful_artifact: Option<String>,
    pub pinned: bool,
    pub preferred_python_entry: Option<String>,
}

impl SharedHistory {
    pub fn load() -> Self {
        let path = shared_history_path();
        let Ok(contents) = std::fs::read_to_string(&path) else {
            return Self {
                version: 1,
                ..Default::default()
            };
        };

        serde_json::from_str(&contents).unwrap_or_else(|_| {
            // This file is shared with VaultLauncher; defaulting here means the
            // next save overwrites it. Keep the unreadable copy so pins and
            // learned build times can be recovered by hand.
            let _ = std::fs::write(path.with_extension("json.corrupt"), &contents);
            Self {
                version: 1,
                ..Default::default()
            }
        })
    }

    /// Look up the learned duration/unit-count for a build signature.
    pub fn learned_build(&self, signature: &str) -> Option<LearnedBuild> {
        self.builds.get(signature).copied()
    }

    /// Record a completed build's duration and unit count for future estimates.
    /// Updates in place so it never clobbers the diagnostic history alongside it.
    pub fn record_build(&mut self, signature: &str, unit_count: u32, duration_ms: u64) {
        let entry = self.builds.entry(signature.to_string()).or_default();
        entry.unit_count = unit_count;
        entry.duration_ms = duration_ms;
    }

    /// Record the diagnostic outcome of a completed run, rolling the previous
    /// warning count forward so the UI can show whether warning debt is growing.
    /// Recorded for failed runs too — a failure's error count is the point.
    pub fn record_outcome(&mut self, signature: &str, errors: u32, warnings: u32) {
        let entry = self.builds.entry(signature.to_string()).or_default();
        entry.prev_warnings = entry.recorded_outcome.then_some(entry.warnings);
        entry.warnings = warnings;
        entry.errors = errors;
        entry.recorded_outcome = true;
    }

    pub fn save(&self) -> Result<(), String> {
        let path = shared_history_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|err| err.to_string())?;
        write_atomically(&path, json.as_bytes())
    }

    pub fn entry_for(&self, path: &Path) -> Option<&HistoryEntry> {
        self.entries.get(&normalized_path_key(path))
    }

    /// Lookup by an already-normalized key. `normalized_path_key` canonicalizes,
    /// which is a filesystem round trip — callers holding the key (catalog ids)
    /// must use this instead of paying it once per workspace per keystroke.
    pub fn entry_for_key(&self, key: &str) -> Option<&HistoryEntry> {
        self.entries.get(key)
    }

    pub fn set_pinned(&mut self, path: &Path, pinned: bool) {
        let entry = self
            .entries
            .entry(normalized_path_key(path))
            .or_default();
        entry.pinned = pinned;
    }

    pub fn set_preferred_python_entry(&mut self, path: &Path, preferred_python_entry: Option<String>) {
        let entry = self
            .entries
            .entry(normalized_path_key(path))
            .or_default();
        entry.preferred_python_entry = preferred_python_entry;
    }
}

/// VaultForge's own preferences (the shared history belongs to both Vault
/// tools; these don't). Missing or unreadable → defaults.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    /// The folder last scanned, so the next launch opens on it.
    #[serde(default)]
    pub scan_root: Option<String>,
    /// Whether Ctrl+Alt+B is on: a small background listener that also starts
    /// at sign-in. Off stops the listener and removes it from sign-in.
    #[serde(default = "default_true")]
    pub hotkey_enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            scan_root: None,
            hotkey_enabled: true,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        shared_history_path().with_file_name("vaultforge-settings.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|contents| serde_json::from_str(&contents).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|err| err.to_string())?;
        write_atomically(&path, json.as_bytes())
    }
}

/// Write via a sibling temp file and rename, so a reader (VaultLauncher shares
/// this file) never observes a half-written document. A torn read used to parse
/// as invalid JSON, load as empty, and then be saved back over the real history.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|err| err.to_string())?;
    if std::fs::rename(&tmp, path).is_ok() {
        return Ok(());
    }
    // A reader that opened the file without delete sharing blocks the rename;
    // a plain overwrite still beats losing the save.
    let _ = std::fs::remove_file(&tmp);
    std::fs::write(path, bytes).map_err(|err| err.to_string())
}

pub fn shared_history_path() -> PathBuf {
    if let Some(app_data) = std::env::var_os("APPDATA") {
        return PathBuf::from(app_data)
            .join("VaultTools")
            .join("shared-history.json");
    }

    PathBuf::from("shared-history.json")
}

/// Directory holding full execution logs. The in-app terminal is a bounded
/// window by design; the complete stream always lands here so a noisy build's
/// output is never actually lost.
pub fn log_dir() -> PathBuf {
    if let Some(app_data) = std::env::var_os("APPDATA") {
        return PathBuf::from(app_data).join("VaultTools").join("logs");
    }

    PathBuf::from("logs")
}

pub fn normalized_path_key(path: &Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical.to_string_lossy().replace('\\', "/").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learned_build_round_trips_and_survives_serialization() {
        let mut history = SharedHistory::default();
        assert!(history.learned_build("sig").is_none());

        history.record_build("sig", 42, 12_345);
        let learned = history.learned_build("sig").unwrap();
        assert_eq!(learned.unit_count, 42);
        assert_eq!(learned.duration_ms, 12_345);

        // A later build with the same signature overwrites the estimate.
        history.record_build("sig", 44, 9_000);
        assert_eq!(history.learned_build("sig").unwrap().duration_ms, 9_000);

        // The `builds` map serializes and reloads with the rest of the history.
        let json = serde_json::to_string(&history).unwrap();
        let restored: SharedHistory = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.learned_build("sig").unwrap().unit_count, 44);
    }

    #[test]
    fn outcome_tracks_warning_delta_across_runs() {
        let mut history = SharedHistory::default();

        history.record_outcome("sig", 0, 955);
        // A first run has nothing to compare against.
        assert!(history.learned_build("sig").unwrap().warning_delta().is_none());

        history.record_outcome("sig", 1, 992);
        let learned = history.learned_build("sig").unwrap();
        assert_eq!(learned.warning_delta(), Some(37));
        assert_eq!(learned.errors, 1);

        // Duration learning shares the entry, so it must update in place rather
        // than replacing it and wiping the diagnostic history.
        history.record_build("sig", 285, 386_400);
        let learned = history.learned_build("sig").unwrap();
        assert_eq!(learned.duration_ms, 386_400);
        assert_eq!(learned.unit_count, 285);
        assert_eq!(learned.warnings, 992);
        assert_eq!(learned.warning_delta(), Some(37));
    }

    #[test]
    fn history_without_builds_field_still_deserializes() {
        // Older shared-history.json files predate the `builds` map.
        let legacy = r#"{"version":1,"entries":{}}"#;
        let history: SharedHistory = serde_json::from_str(legacy).unwrap();
        assert!(history.builds.is_empty());
    }

    #[test]
    fn atomic_write_replaces_existing_file_and_leaves_no_temp() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("vaultforge-atomic-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shared-history.json");

        std::fs::write(&path, "old").unwrap();
        write_atomically(&path, b"new").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        let leftovers = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(leftovers, 1, "the temp file must be renamed away, not left behind");

        let _ = std::fs::remove_dir_all(dir);
    }
}
