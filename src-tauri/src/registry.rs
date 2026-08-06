//! `projects.json` + `settings.json` load/save — SPEC.md §4 (Storage) and §5 (data model).
//!
//! Rules this module enforces:
//! - every write is atomic (serialize -> temp file in the same directory -> rename over the target),
//! - a present-but-unparseable `projects.json` is NEVER overwritten: it is renamed to
//!   `projects.json.broken-<unix-timestamp>` and an empty registry is returned alongside a
//!   report the UI shows as a persistent banner,
//! - unknown JSON fields are ignored, never fatal.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PROJECTS_FILE: &str = "projects.json";
pub const SETTINGS_FILE: &str = "settings.json";

/// SPEC.md §5. `#[serde(rename_all = "kebab-case")]` produces exactly the eight wire values
/// (`stopped` … `stop-failed`) that `src/types.ts` mirrors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Stopped,
    Updating,
    Installing,
    Starting,
    Running,
    Stopping,
    Crashed,
    StopFailed,
}

/// The persisted project record (SPEC.md §5). Only these fields ever reach `projects.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    pub command: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default = "default_update_on_run")]
    pub update_on_run: bool,
    #[serde(default = "default_ready_timeout_sec")]
    pub ready_timeout_sec: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_lockfile_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
}

fn default_update_on_run() -> bool {
    true
}

fn default_ready_timeout_sec() -> u32 {
    60
}

/// What the frontend receives. Derived fields are computed here and never persisted (SPEC.md §5).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    #[serde(flatten)]
    pub project: Project,
    pub status: Status,
    pub path_exists: bool,
}

/// SPEC.md §4 — the one-key settings file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub editor_command: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            editor_command: "code".to_string(),
        }
    }
}

/// Surfaced to the UI as the persistent corrupt-registry banner (SPEC.md §4, §12).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryError {
    /// Absolute path of the `.broken-<timestamp>` backup, when one was made.
    pub backup_path: Option<String>,
    pub error: String,
}

/// Result of a startup load: the registry plus, if the file was unreadable, what happened to it.
#[derive(Debug, Clone)]
pub struct RegistryLoad {
    pub projects: Vec<Project>,
    pub error: Option<RegistryError>,
}

/// SPEC.md §5 dev fixture — written only when `HANGAR_DEV_SEED` is set AND `projects.json` is absent.
/// The path is the literal placeholder from the spec, so the card renders in the `pathExists: false`
/// warning state. That is expected and correct.
fn seed_projects() -> Vec<Project> {
    vec![Project {
        id: "ielts-coach".to_string(),
        name: "IELTS Coach".to_string(),
        path: "REPLACE_WITH_ABSOLUTE_PATH".to_string(),
        command: "npm run dev".to_string(),
        port: 3000,
        url: None,
        update_on_run: true,
        ready_timeout_sec: 60,
        last_lockfile_hash: None,
        last_run_at: None,
    }]
}

/// Serialize -> temp file in the SAME directory -> rename over the target.
/// Never truncates the target in place, so a crash mid-write cannot destroy the registry.
pub fn atomic_write(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;

    let dir = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "target path has no parent directory",
        )
    })?;
    std::fs::create_dir_all(dir)?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid file name"))?;
    let tmp = dir.join(format!("{file_name}.tmp"));

    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }

    // rename is atomic on both macOS/Linux and Windows for same-directory targets.
    std::fs::rename(&tmp, path)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn projects_path(dir: &Path) -> PathBuf {
    dir.join(PROJECTS_FILE)
}

pub fn settings_path(dir: &Path) -> PathBuf {
    dir.join(SETTINGS_FILE)
}

/// Startup load. Reads `HANGAR_DEV_SEED` for the first-run seed decision; the actual logic lives in
/// [`load_projects_with_seed`] so tests never have to mutate process environment.
pub fn load_projects(dir: &Path) -> RegistryLoad {
    load_projects_with_seed(dir, std::env::var_os("HANGAR_DEV_SEED").is_some())
}

pub fn load_projects_with_seed(dir: &Path, seed: bool) -> RegistryLoad {
    let path = projects_path(dir);

    if !path.exists() {
        // SPEC.md §5: a true first run writes an EMPTY array — the §11 empty state is the
        // first-run experience. The seed fixture is written only under HANGAR_DEV_SEED.
        let projects = if seed { seed_projects() } else { Vec::new() };
        let mut error = None;
        if let Err(e) = save_projects(dir, &projects) {
            error = Some(RegistryError {
                backup_path: None,
                error: format!("could not create {}: {e}", path.display()),
            });
        }
        return RegistryLoad { projects, error };
    }

    let raw = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            // Unreadable, not unparseable: leave the file completely alone.
            return RegistryLoad {
                projects: Vec::new(),
                error: Some(RegistryError {
                    backup_path: None,
                    error: format!("could not read {}: {e}", path.display()),
                }),
            };
        }
    };

    match serde_json::from_slice::<Vec<Project>>(&raw) {
        Ok(projects) => RegistryLoad {
            projects,
            error: None,
        },
        Err(parse_err) => {
            // NEVER overwrite. Move the original aside first; only then start empty.
            let backup = dir.join(format!("{PROJECTS_FILE}.broken-{}", unix_timestamp()));
            let (backup_path, detail) = match std::fs::rename(&path, &backup) {
                Ok(()) => (
                    Some(backup.to_string_lossy().into_owned()),
                    parse_err.to_string(),
                ),
                Err(rename_err) => (
                    None,
                    format!("{parse_err} (backup failed as well: {rename_err})"),
                ),
            };

            // Only write a fresh empty registry once the original bytes are safely elsewhere.
            if backup_path.is_some() {
                let _ = save_projects(dir, &[]);
            }

            RegistryLoad {
                projects: Vec::new(),
                error: Some(RegistryError {
                    backup_path,
                    error: detail,
                }),
            }
        }
    }
}

pub fn save_projects(dir: &Path, projects: &[Project]) -> Result<(), String> {
    let json = serde_json::to_string_pretty(projects)
        .map_err(|e| format!("could not serialize projects: {e}"))?;
    atomic_write(&projects_path(dir), &json)
        .map_err(|e| format!("could not write {PROJECTS_FILE}: {e}"))
}

/// Settings load. A missing file is created with the default. A present-but-unparseable file is
/// left untouched and the default is used — the next `set_settings` rewrites it atomically.
pub fn load_settings(dir: &Path) -> Settings {
    let path = settings_path(dir);
    if !path.exists() {
        let defaults = Settings::default();
        let _ = save_settings(dir, &defaults);
        return defaults;
    }
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Settings>(&bytes).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save_settings(dir: &Path, settings: &Settings) -> Result<(), String> {
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("could not serialize settings: {e}"))?;
    atomic_write(&settings_path(dir), &json)
        .map_err(|e| format!("could not write {SETTINGS_FILE}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique scratch directory under the OS temp dir — avoids adding a `tempfile` dependency.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hangar-registry-test-{tag}-{}-{:?}",
            unix_timestamp(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn sample() -> Project {
        Project {
            id: "abc123".into(),
            name: "IELTS Coach".into(),
            path: "/tmp/ielts".into(),
            command: "npm run dev".into(),
            port: 3000,
            url: Some("http://localhost:3000".into()),
            update_on_run: true,
            ready_timeout_sec: 60,
            last_lockfile_hash: Some("deadbeef".into()),
            last_run_at: Some("2026-08-05T10:00:00Z".into()),
        }
    }

    #[test]
    fn project_serde_round_trip_is_camel_case() {
        let p = sample();
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"updateOnRun\""), "got {json}");
        assert!(json.contains("\"readyTimeoutSec\""), "got {json}");
        assert!(json.contains("\"lastLockfileHash\""), "got {json}");
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn status_serializes_kebab_case() {
        assert_eq!(serde_json::to_string(&Status::StopFailed).unwrap(), "\"stop-failed\"");
        assert_eq!(serde_json::to_string(&Status::Stopped).unwrap(), "\"stopped\"");
    }

    #[test]
    fn unknown_fields_are_ignored_and_optionals_default() {
        let json = r#"[{
            "id": "x",
            "name": "X",
            "path": "/tmp/x",
            "command": "npm run dev",
            "port": 5173,
            "someFutureField": {"nested": true},
            "anotherOne": 42
        }]"#;
        let parsed: Vec<Project> = serde_json::from_str(json).expect("unknown fields must not be fatal");
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].update_on_run, "updateOnRun defaults to true");
        assert_eq!(parsed[0].ready_timeout_sec, 60);
        assert_eq!(parsed[0].url, None);
    }

    #[test]
    fn first_run_writes_an_empty_array() {
        let dir = scratch("firstrun");
        let load = load_projects_with_seed(&dir, false);
        assert!(load.projects.is_empty());
        assert!(load.error.is_none());
        let written = std::fs::read_to_string(projects_path(&dir)).unwrap();
        assert_eq!(written.trim(), "[]");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dev_seed_writes_the_spec_fixture() {
        let dir = scratch("seed");
        let load = load_projects_with_seed(&dir, true);
        assert_eq!(load.projects.len(), 1);
        assert_eq!(load.projects[0].name, "IELTS Coach");
        assert_eq!(load.projects[0].path, "REPLACE_WITH_ABSOLUTE_PATH");
        // and it round-trips from disk
        let reloaded = load_projects_with_seed(&dir, false);
        assert_eq!(reloaded.projects, load.projects);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_renamed_not_overwritten_and_bytes_survive() {
        let dir = scratch("corrupt");
        let original = "{ this is not valid json at all ";
        std::fs::write(projects_path(&dir), original).unwrap();

        let load = load_projects_with_seed(&dir, false);

        assert!(load.projects.is_empty(), "corrupt registry must start empty");
        let err = load.error.expect("a corrupt file must report an error");
        let backup = err.backup_path.expect("a backup path must be reported");
        assert!(backup.contains("projects.json.broken-"), "got {backup}");

        // The original bytes survive untouched in the backup.
        let saved = std::fs::read_to_string(&backup).unwrap();
        assert_eq!(saved, original);

        // And the live file is a fresh empty registry, not the corrupt content.
        let live = std::fs::read_to_string(projects_path(&dir)).unwrap();
        assert_eq!(live.trim(), "[]");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_leaves_no_temp_file_and_replaces_content() {
        let dir = scratch("atomic");
        let target = dir.join("projects.json");
        atomic_write(&target, "[]").unwrap();
        atomic_write(&target, "[1,2,3]").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "[1,2,3]");
        assert!(!dir.join("projects.json.tmp").exists(), "temp file must be renamed away");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn settings_default_and_round_trip() {
        let dir = scratch("settings");
        let s = load_settings(&dir);
        assert_eq!(s.editor_command, "code");
        let written = std::fs::read_to_string(settings_path(&dir)).unwrap();
        assert!(written.contains("\"editorCommand\""), "got {written}");

        save_settings(&dir, &Settings { editor_command: "subl".into() }).unwrap();
        assert_eq!(load_settings(&dir).editor_command, "subl");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_view_flattens_project_and_adds_derived_fields() {
        let view = ProjectView {
            project: sample(),
            status: Status::Running,
            path_exists: true,
        };
        let json = serde_json::to_string(&view).unwrap();
        assert_eq!(
            json,
            concat!(
                r#"{"id":"abc123","name":"IELTS Coach","path":"/tmp/ielts","command":"npm run dev","port":3000,"#,
                r#""url":"http://localhost:3000","updateOnRun":true,"readyTimeoutSec":60,"#,
                r#""lastLockfileHash":"deadbeef","lastRunAt":"2026-08-05T10:00:00Z","#,
                r#""status":"running","pathExists":true}"#
            )
        );
        assert!(
            !json.contains("\"project\""),
            "the flatten must stay flat — no nested project key: got {json}"
        );
    }

    #[test]
    fn registry_error_serializes_backup_path_as_nullable_camel_case() {
        let with_backup = RegistryError {
            backup_path: Some("/tmp/projects.json.broken-123".into()),
            error: "unexpected EOF".into(),
        };
        assert_eq!(
            serde_json::to_string(&with_backup).unwrap(),
            r#"{"backupPath":"/tmp/projects.json.broken-123","error":"unexpected EOF"}"#
        );

        // `types.ts` declares `backupPath: string | null` — not optional — so `None` must
        // serialize as an explicit `null`, never be omitted from the object.
        let without_backup = RegistryError {
            backup_path: None,
            error: "could not read projects.json".into(),
        };
        assert_eq!(
            serde_json::to_string(&without_backup).unwrap(),
            r#"{"backupPath":null,"error":"could not read projects.json"}"#
        );
    }

    /// The §7 contract is frozen and mirrored BY HAND in `src/types.ts`; this test is the only
    /// thing linking the two sides. It is deliberately key-presence only — exact-shape
    /// assertions live in the per-struct wire tests above (and in `process.rs`).
    ///
    /// Limitation, noted rather than fixed: this only catches Rust-side keys missing from TS. It
    /// cannot catch TS-side keys the backend never emits — that direction is already protected by
    /// `tsc --noEmit` failing when the frontend reads a field it never declared.
    #[test]
    fn every_wire_key_the_backend_emits_appears_in_types_ts() {
        let types_ts = include_str!("../../src/types.ts");

        // Every Option = Some, so `skip_serializing_if` fields are present in the sample.
        let project_view = ProjectView {
            project: sample(),
            status: Status::Running,
            path_exists: true,
        };
        let status_changed = crate::process::StatusChangedPayload {
            project_id: "abc".into(),
            status: Status::Running,
            message: Some("boom".into()),
        };
        let log_lines = crate::process::LogLinesPayload {
            project_id: "abc".into(),
            lines: vec![crate::process::LogLine {
                stream: crate::process::Stream::Stdout,
                line: "hi".into(),
            }],
        };
        let settings = Settings::default();
        let registry_error = RegistryError {
            backup_path: Some("/tmp/projects.json.broken-123".into()),
            error: "boom".into(),
        };

        let samples: Vec<serde_json::Value> = vec![
            serde_json::to_value(&project_view).unwrap(),
            serde_json::to_value(&status_changed).unwrap(),
            serde_json::to_value(&log_lines).unwrap(),
            serde_json::to_value(&settings).unwrap(),
            serde_json::to_value(&registry_error).unwrap(),
        ];
        for sample in &samples {
            assert_keys_in(sample, types_ts);
        }

        // Covers the kebab-case `Status` union separately: its values are strings, not object keys.
        for status in [
            Status::Stopped,
            Status::Updating,
            Status::Installing,
            Status::Starting,
            Status::Running,
            Status::Stopping,
            Status::Crashed,
            Status::StopFailed,
        ] {
            let wire = serde_json::to_value(status).unwrap();
            let wire_str = wire.as_str().expect("Status serializes to a string");
            assert!(
                types_ts.contains(wire_str),
                "Status::{status:?} serializes to {wire_str:?}, which does not appear anywhere \
                 in src/types.ts"
            );
        }
    }

    /// Recursively asserts every object key in `value` is declared as a TypeScript property
    /// somewhere in `types_ts`'s source text. Presence-only: it does not check nesting,
    /// optionality or type.
    ///
    /// Deliberately NOT a bare `types_ts.contains(key)`: a short key like `id` or `url` is a
    /// substring of plenty of unrelated identifiers (`projectId`, `backupPath`'s doc comment,
    /// …), so a bare substring search can never fail on a renamed short key — it just finds the
    /// rename hiding inside some other word. Interface properties in this file are always
    /// written `key: type;` or, for the optional ones, `key?: type;`, so requiring the trailing
    /// colon anchors the match to an actual property declaration instead.
    ///
    /// This does NOT apply to the `Status` union check in the test above: those eight strings
    /// are string-literal *members* of a type union (`| "stopped" | …`), not object keys, and
    /// have no `key:` form to anchor on — plain `contains` is correct for them.
    fn assert_keys_in(value: &serde_json::Value, types_ts: &str) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, v) in map {
                    let declared = types_ts.contains(&format!("{key}:"))
                        || types_ts.contains(&format!("{key}?:"));
                    assert!(
                        declared,
                        "wire key {key:?} is not declared as a property in src/types.ts — \
                         the frozen §7 contract and its hand-written mirror have drifted"
                    );
                    assert_keys_in(v, types_ts);
                }
            }
            serde_json::Value::Array(items) => {
                for v in items {
                    assert_keys_in(v, types_ts);
                }
            }
            _ => {}
        }
    }
}
