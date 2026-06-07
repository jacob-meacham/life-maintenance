//! Data-access layer: load tasks, vendors, and the event log from the data
//! directory, validating vendor references.

use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::model::{Completion, Event, Punt, Task, Vendor};
use crate::schema::{RawCompletion, RawPunt, RawTask, RawVendor};

/// Filename of the task definitions (YAML).
const TASKS_FILE: &str = "tasks.yaml";
/// Filename of the vendor definitions (YAML).
const VENDORS_FILE: &str = "vendors.yaml";
/// Filename of the append-only event log (JSON Lines).
const EVENTS_FILE: &str = "completions.jsonl";

/// JSON key that distinguishes a punt event.
const PUNT_KEY: &str = "punt_to";
/// JSON key that distinguishes a completion event.
const DONE_KEY: &str = "done";

/// The directory holding the data files (the git repo).
#[derive(Debug, Clone)]
pub struct DataDir {
    /// Filesystem root of the data directory.
    pub root: PathBuf,
}

impl DataDir {
    /// Create a `DataDir` rooted at `root`.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Path to the tasks file.
    fn tasks_path(&self) -> PathBuf {
        self.root.join(TASKS_FILE)
    }

    /// Path to the vendors file.
    fn vendors_path(&self) -> PathBuf {
        self.root.join(VENDORS_FILE)
    }

    /// Path to the event-log file.
    fn events_path(&self) -> PathBuf {
        self.root.join(EVENTS_FILE)
    }
}

/// Read a file to a string, treating a missing file as `Ok(None)`.
///
/// `name` is the short filename used in any [`Error::Io`] so callers report a
/// stable, repo-relative path rather than a leaked absolute path.
///
/// # Errors
///
/// Returns [`Error::Io`] if the file exists but cannot be read.
fn read_opt(path: &Path, name: &str) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Io {
            path: name.to_string(),
            source,
        }),
    }
}

/// Load all task definitions from `tasks.yaml`.
///
/// A missing file yields an empty list.
///
/// # Errors
///
/// Returns [`Error::Io`] if the file cannot be read, or [`Error::DataFile`] if
/// it cannot be parsed or a task's schedule/lead-time is invalid.
pub fn load_tasks(d: &DataDir) -> Result<Vec<Task>> {
    let Some(text) = read_opt(&d.tasks_path(), TASKS_FILE)? else {
        return Ok(vec![]);
    };
    let raws: Vec<RawTask> =
        serde_yaml::from_str(&text).map_err(|e| Error::DataFile(format!("{TASKS_FILE}: {e}")))?;
    raws.into_iter()
        .map(|raw| Task::from_raw(raw).map_err(|e| Error::DataFile(format!("{TASKS_FILE}: {e}"))))
        .collect()
}

/// Load all vendor definitions from `vendors.yaml`.
///
/// A missing file yields an empty list.
///
/// # Errors
///
/// Returns [`Error::Io`] if the file cannot be read, or [`Error::DataFile`] if
/// it cannot be parsed.
pub fn load_vendors(d: &DataDir) -> Result<Vec<Vendor>> {
    let Some(text) = read_opt(&d.vendors_path(), VENDORS_FILE)? else {
        return Ok(vec![]);
    };
    let raws: Vec<RawVendor> =
        serde_yaml::from_str(&text).map_err(|e| Error::DataFile(format!("{VENDORS_FILE}: {e}")))?;
    Ok(raws.into_iter().map(Vendor::from_raw).collect())
}

/// Load every event from the append-only log `completions.jsonl`, in file
/// order.
///
/// Blank and whitespace-only lines are skipped. Each remaining line must be a
/// JSON object discriminated by the presence of a `done` (completion) or
/// `punt_to` (punt) key.
///
/// A missing file yields an empty list.
///
/// # Errors
///
/// Returns [`Error::Io`] if the file cannot be read, or [`Error::DataFile`]
/// (carrying the 1-indexed line number) if a line is not valid JSON, is not an
/// object, or does not match a known event shape.
pub fn load_events(d: &DataDir) -> Result<Vec<Event>> {
    let Some(text) = read_opt(&d.events_path(), EVENTS_FILE)? else {
        return Ok(vec![]);
    };
    let mut events = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let n = idx + 1;
        if line.trim().is_empty() {
            continue;
        }
        events.push(parse_event_line(line, n)?);
    }
    Ok(events)
}

/// Parse a single non-blank event-log line (`n` is its 1-indexed line number).
///
/// # Errors
///
/// Returns [`Error::DataFile`] tagged with `n` if the line is not a JSON
/// object or does not match a known event shape.
fn parse_event_line(line: &str, n: usize) -> Result<Event> {
    let value: serde_json::Value = serde_json::from_str(line)
        .map_err(|e| Error::DataFile(format!("{EVENTS_FILE}: line {n}: {e}")))?;
    let Some(map) = value.as_object() else {
        return Err(Error::DataFile(format!(
            "{EVENTS_FILE}: line {n}: expected a JSON object"
        )));
    };
    if map.contains_key(PUNT_KEY) {
        let raw: RawPunt = serde_json::from_value(value)
            .map_err(|e| Error::DataFile(format!("{EVENTS_FILE}: line {n}: {e}")))?;
        Ok(Event::Punt(Punt::from_raw(raw)))
    } else if map.contains_key(DONE_KEY) {
        let raw: RawCompletion = serde_json::from_value(value)
            .map_err(|e| Error::DataFile(format!("{EVENTS_FILE}: line {n}: {e}")))?;
        Ok(Event::Completion(Completion::from_raw(raw)))
    } else {
        Err(Error::DataFile(format!(
            "{EVENTS_FILE}: line {n}: unrecognized event (needs '{DONE_KEY}' or '{PUNT_KEY}')"
        )))
    }
}

/// Load only the completion events from the log, dropping punts.
///
/// # Errors
///
/// Returns the same errors as [`load_events`].
pub fn load_completions(d: &DataDir) -> Result<Vec<Completion>> {
    let mut completions = Vec::new();
    for ev in load_events(d)? {
        if let Event::Completion(c) = ev {
            completions.push(c);
        }
    }
    Ok(completions)
}

/// Verify that every task's `vendor` reference resolves to a defined vendor.
///
/// # Errors
///
/// Returns [`Error::UnknownVendor`] for the first task whose vendor id is not
/// among `vendors`.
fn check_vendor_refs(tasks: &[Task], vendors: &[Vendor]) -> Result<()> {
    let ids: HashSet<&str> = vendors.iter().map(|v| v.id.as_str()).collect();
    for task in tasks {
        if let Some(vendor_id) = &task.vendor {
            if !ids.contains(vendor_id.as_str()) {
                return Err(Error::UnknownVendor {
                    task_id: task.id.clone(),
                    vendor_id: vendor_id.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Load tasks, vendors, and completions together, validating vendor references.
///
/// # Errors
///
/// Returns any error from the individual loaders, or [`Error::UnknownVendor`]
/// if a task references a vendor that is not defined.
pub fn load_all(d: &DataDir) -> Result<(Vec<Task>, Vec<Vendor>, Vec<Completion>)> {
    let tasks = load_tasks(d)?;
    let vendors = load_vendors(d)?;
    let completions = load_completions(d)?;
    check_vendor_refs(&tasks, &vendors)?;
    Ok((tasks, vendors, completions))
}

/// Serialization form of a completion event, omitting absent optional fields.
#[derive(Serialize)]
struct OutCompletion<'a> {
    id: &'a str,
    done: String,
    via: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    by: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_cents: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<&'a str>,
}

/// Serialization form of a punt event.
#[derive(Serialize)]
struct OutPunt<'a> {
    id: &'a str,
    punt_to: String,
    via: &'a str,
}

/// Append a single JSON line to the event log, creating the file (and the data
/// directory) if needed.
///
/// # Errors
///
/// Returns [`Error::Io`] if the data directory or log file cannot be created or
/// written.
fn append_line(d: &DataDir, line: &str) -> Result<()> {
    std::fs::create_dir_all(&d.root).map_err(|source| Error::Io {
        path: d.root.display().to_string(),
        source,
    })?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(d.events_path())
        .map_err(|source| Error::Io {
            path: EVENTS_FILE.to_string(),
            source,
        })?;
    writeln!(file, "{line}").map_err(|source| Error::Io {
        path: EVENTS_FILE.to_string(),
        source,
    })
}

/// Append a completion event to `completions.jsonl`.
///
/// Absent optional fields (`by`, `cost_cents`, `note`) are omitted from the
/// emitted JSON. The file and data directory are created if missing.
///
/// # Errors
///
/// Returns [`Error::DataFile`] if the event cannot be serialized, or
/// [`Error::Io`] if the log cannot be created or written.
pub fn append_completion(d: &DataDir, c: &Completion) -> Result<()> {
    let out = OutCompletion {
        id: &c.id,
        done: c.done.to_string(),
        via: &c.via,
        by: c.by.as_deref(),
        cost_cents: c.cost_cents,
        note: c.note.as_deref(),
    };
    let line = serde_json::to_string(&out)
        .map_err(|e| Error::DataFile(format!("serialize completion: {e}")))?;
    append_line(d, &line)
}

/// Append a punt event to `completions.jsonl`.
///
/// The file and data directory are created if missing.
///
/// # Errors
///
/// Returns [`Error::DataFile`] if the event cannot be serialized, or
/// [`Error::Io`] if the log cannot be created or written.
pub fn append_punt(d: &DataDir, p: &Punt) -> Result<()> {
    let out = OutPunt {
        id: &p.id,
        punt_to: p.punt_to.to_string(),
        via: &p.via,
    };
    let line =
        serde_json::to_string(&out).map_err(|e| Error::DataFile(format!("serialize punt: {e}")))?;
    append_line(d, &line)
}

/// Stage and commit only the data files in the data directory, with `message`.
///
/// The data directory may be its own git repository *or* a subdirectory inside a
/// larger, active repository. To stay safe in the latter case this function:
///
/// - Detects the enclosing repository with `git rev-parse --show-toplevel`
///   (resolved from the data dir, not the current process directory). If the
///   directory is not inside any git repository, this is a non-fatal no-op and
///   returns `Ok(false)`.
/// - Stages and commits **only** the data files (`tasks.yaml`, `vendors.yaml`,
///   `completions.jsonl`) by pathspec — never `git add -A`. Committing by
///   pathspec means any *other* staged or working-tree changes the user has in
///   the enclosing repository are left untouched and uncommitted; only the data
///   files are swept into the commit.
/// - Considers only the data files that currently exist, so a missing optional
///   file (e.g. no `vendors.yaml`) does not cause `git add` to fail on a
///   non-existent pathspec. If none of the data files exist, returns `Ok(false)`.
///
/// Returns `Ok(false)` (a non-fatal no-op) when the directory is not in a git
/// repository, when no data files exist, or when there is nothing to commit.
/// Returns `Ok(true)` when a commit was created.
///
/// # Errors
///
/// Returns [`Error::Io`] if `git` cannot be spawned (e.g. it is not installed),
/// or [`Error::DataFile`] if `git add` fails.
pub fn commit(d: &DataDir, message: &str) -> Result<bool> {
    let root = d.root.as_os_str();
    // Detect the enclosing repository; resolve the repo from the data dir.
    let toplevel = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|source| Error::Io {
            path: "git".to_string(),
            source,
        })?;
    if !toplevel.status.success() {
        return Ok(false);
    }

    // Stage and commit only the data files that actually exist, by pathspec.
    let pathspecs: Vec<&str> = [TASKS_FILE, VENDORS_FILE, EVENTS_FILE]
        .into_iter()
        .filter(|name| d.root.join(name).exists())
        .collect();
    if pathspecs.is_empty() {
        return Ok(false);
    }

    let add = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["add", "--"])
        .args(&pathspecs)
        .status()
        .map_err(|source| Error::Io {
            path: "git".to_string(),
            source,
        })?;
    if !add.success() {
        return Err(Error::DataFile("git add failed".into()));
    }

    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["commit", "-m", message, "--"])
        .args(&pathspecs)
        .output()
        .map_err(|source| Error::Io {
            path: "git".to_string(),
            source,
        })?;
    Ok(output.status.success())
}

#[cfg(test)]
mod tests {
    use super::{
        append_completion, append_punt, commit, load_all, load_completions, load_events,
        load_tasks, load_vendors, DataDir, EVENTS_FILE, TASKS_FILE, VENDORS_FILE,
    };
    use crate::error::Error;
    use crate::model::{Completion, Event, Punt};
    use crate::schedule::Schedule;
    use jiff::civil::date;
    use std::fs;
    use std::path::Path;
    use tempfile::{tempdir, TempDir};

    const SAMPLE_TASKS: &str = "\
- id: groceries
  name: Groceries
  every: weekly
- id: clean-drains
  name: Clean drains
  every: yearly
  vendor: roto-rooter
- id: blow-out-sprinklers
  name: Blow out sprinklers
  every: yearly
  on: \"10-15\"
";

    const SAMPLE_VENDORS: &str = "\
- id: roto-rooter
  name: Roto-Rooter
";

    const SAMPLE_EVENTS: &str = "\
{\"id\":\"groceries\",\"done\":\"2026-06-01\"}
{\"id\":\"clean-drains\",\"done\":\"2025-05-01\",\"cost_cents\":28500}
";

    fn write(dir: &Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).unwrap();
    }

    /// Make a temp dir initialised as a git repo with a committer identity.
    fn git_dir() -> TempDir {
        let dir = tempdir().unwrap();
        let root = dir.path();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@e.com"],
            vec!["config", "user.name", "T"],
        ] {
            std::process::Command::new("git")
                .args(&args)
                .current_dir(root)
                .status()
                .unwrap();
        }
        dir
    }

    /// Write the standard sample data set into a fresh temp dir.
    fn sample_dir() -> (TempDir, DataDir) {
        let dir = tempdir().unwrap();
        write(dir.path(), TASKS_FILE, SAMPLE_TASKS);
        write(dir.path(), VENDORS_FILE, SAMPLE_VENDORS);
        write(dir.path(), EVENTS_FILE, SAMPLE_EVENTS);
        let data = DataDir::new(dir.path().to_path_buf());
        (dir, data)
    }

    #[test]
    fn load_tasks_returns_ids_in_file_order() {
        let (_dir, data) = sample_dir();
        let tasks = load_tasks(&data).unwrap();
        let ids: Vec<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["groceries", "clean-drains", "blow-out-sprinklers"]);
    }

    #[test]
    fn load_vendors_returns_ids_in_file_order() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            VENDORS_FILE,
            "- id: roto-rooter\n  name: Roto-Rooter\n- id: phone\n  name: Phone Co\n",
        );
        let data = DataDir::new(dir.path().to_path_buf());
        let vendors = load_vendors(&data).unwrap();
        let ids: Vec<&str> = vendors.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["roto-rooter", "phone"]);
    }

    #[test]
    fn load_events_returns_two_completions_for_sample() {
        let (_dir, data) = sample_dir();
        let events = load_events(&data).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], Event::Completion(_)));
        assert!(matches!(events[1], Event::Completion(_)));
    }

    #[test]
    fn load_events_preserves_completion_then_punt_order() {
        let dir = tempdir().unwrap();
        let mixed = "\
{\"id\":\"groceries\",\"done\":\"2026-06-01\"}
{\"id\":\"clean-drains\",\"punt_to\":\"2026-07-01\"}
";
        write(dir.path(), EVENTS_FILE, mixed);
        let data = DataDir::new(dir.path().to_path_buf());
        let events = load_events(&data).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], Event::Completion(_)));
        assert!(matches!(events[1], Event::Punt(_)));
    }

    #[test]
    fn load_completions_excludes_punts() {
        let dir = tempdir().unwrap();
        let mixed = "\
{\"id\":\"groceries\",\"done\":\"2026-06-01\"}
{\"id\":\"clean-drains\",\"punt_to\":\"2026-07-01\"}
";
        write(dir.path(), EVENTS_FILE, mixed);
        let data = DataDir::new(dir.path().to_path_buf());
        assert_eq!(load_completions(&data).unwrap().len(), 1);
        assert_eq!(load_events(&data).unwrap().len(), 2);
    }

    #[test]
    fn load_all_returns_expected_lengths() {
        let (_dir, data) = sample_dir();
        let (tasks, vendors, completions) = load_all(&data).unwrap();
        assert_eq!(tasks.len(), 3);
        assert_eq!(vendors.len(), 1);
        assert_eq!(completions.len(), 2);
    }

    #[test]
    fn load_all_rejects_unknown_vendor_reference() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            TASKS_FILE,
            "- id: clean-drains\n  name: Clean drains\n  every: yearly\n  vendor: ghost\n",
        );
        write(dir.path(), VENDORS_FILE, "[]\n");
        let data = DataDir::new(dir.path().to_path_buf());
        match load_all(&data) {
            Err(Error::UnknownVendor { task_id, vendor_id }) => {
                assert_eq!(task_id, "clean-drains");
                assert_eq!(vendor_id, "ghost");
            }
            other => panic!("expected UnknownVendor, got {other:?}"),
        }
    }

    #[test]
    fn load_tasks_malformed_schedule_reports_filename() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            TASKS_FILE,
            "- id: x\n  name: X\n  every: fortnightly\n",
        );
        let data = DataDir::new(dir.path().to_path_buf());
        match load_tasks(&data) {
            Err(Error::DataFile(msg)) => assert!(msg.contains("tasks.yaml"), "msg: {msg}"),
            other => panic!("expected DataFile, got {other:?}"),
        }
    }

    #[test]
    fn missing_files_yield_empty_vecs() {
        let dir = tempdir().unwrap();
        let data = DataDir::new(dir.path().to_path_buf());
        assert!(load_tasks(&data).unwrap().is_empty());
        assert!(load_vendors(&data).unwrap().is_empty());
        assert!(load_events(&data).unwrap().is_empty());
        assert!(load_completions(&data).unwrap().is_empty());
        let (tasks, vendors, completions) = load_all(&data).unwrap();
        assert!(tasks.is_empty());
        assert!(vendors.is_empty());
        assert!(completions.is_empty());
    }

    #[test]
    fn malformed_jsonl_reports_line_number() {
        let dir = tempdir().unwrap();
        let contents = "\
{\"id\":\"groceries\",\"done\":\"2026-06-01\"}
NOT JSON
";
        write(dir.path(), EVENTS_FILE, contents);
        let data = DataDir::new(dir.path().to_path_buf());
        match load_events(&data) {
            Err(Error::DataFile(msg)) => assert!(msg.contains("line 2"), "msg: {msg}"),
            other => panic!("expected DataFile, got {other:?}"),
        }
    }

    #[test]
    fn non_object_json_line_is_rejected() {
        let dir = tempdir().unwrap();
        write(dir.path(), EVENTS_FILE, "[]\n");
        let data = DataDir::new(dir.path().to_path_buf());
        match load_events(&data) {
            Err(Error::DataFile(msg)) => assert!(msg.contains("object"), "msg: {msg}"),
            other => panic!("expected DataFile, got {other:?}"),
        }
    }

    #[test]
    fn both_keys_line_is_rejected() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            EVENTS_FILE,
            "{\"id\":\"x\",\"done\":\"2026-01-01\",\"punt_to\":\"2026-02-01\"}\n",
        );
        let data = DataDir::new(dir.path().to_path_buf());
        match load_events(&data) {
            Err(Error::DataFile(msg)) => assert!(msg.contains("line 1"), "msg: {msg}"),
            other => panic!("expected DataFile, got {other:?}"),
        }
    }

    #[test]
    fn completion_with_bad_date_is_rejected_with_line_number() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            EVENTS_FILE,
            "{\"id\": \"x\", \"done\": \"not-a-date\"}\n",
        );
        let data = DataDir::new(dir.path().to_path_buf());
        let err = load_events(&data).unwrap_err();
        assert!(matches!(err, Error::DataFile(_)));
        assert!(err.to_string().contains("line 1"), "err: {err}");
    }

    #[test]
    fn unrecognized_event_line_is_rejected() {
        let dir = tempdir().unwrap();
        write(dir.path(), EVENTS_FILE, "{\"id\":\"x\"}\n");
        let data = DataDir::new(dir.path().to_path_buf());
        match load_events(&data) {
            Err(Error::DataFile(msg)) => assert!(msg.contains("unrecognized"), "msg: {msg}"),
            other => panic!("expected DataFile, got {other:?}"),
        }
    }

    #[test]
    fn append_completion_adds_one_line_and_round_trips() {
        let dir = tempdir().unwrap();
        write(dir.path(), EVENTS_FILE, SAMPLE_EVENTS);
        let data = DataDir::new(dir.path().to_path_buf());
        let before = load_completions(&data).unwrap().len();
        let c = Completion {
            id: "groceries".to_string(),
            done: date(2026, 6, 6),
            via: "cli".to_string(),
            by: Some("Acme".to_string()),
            cost_cents: Some(1234),
            note: Some("did it".to_string()),
        };
        append_completion(&data, &c).unwrap();
        let after = load_completions(&data).unwrap();
        assert_eq!(after.len(), before + 1);
        let last = after.last().unwrap();
        assert_eq!(last.id, "groceries");
        assert_eq!(last.done, date(2026, 6, 6));
        assert_eq!(last.via, "cli");
        assert_eq!(last.by, Some("Acme".to_string()));
        assert_eq!(last.cost_cents, Some(1234));
        assert_eq!(last.note, Some("did it".to_string()));
    }

    #[test]
    fn append_completion_omits_none_fields() {
        let dir = tempdir().unwrap();
        let data = DataDir::new(dir.path().to_path_buf());
        let c = Completion {
            id: "groceries".to_string(),
            done: date(2026, 6, 6),
            via: "cli".to_string(),
            by: None,
            cost_cents: None,
            note: None,
        };
        append_completion(&data, &c).unwrap();
        let raw = fs::read_to_string(dir.path().join(EVENTS_FILE)).unwrap();
        let line = raw.lines().last().unwrap();
        assert!(!line.contains("cost_cents"), "line: {line}");
        assert!(!line.contains("note"), "line: {line}");
        assert!(!line.contains("\"by\""), "line: {line}");
        assert!(line.contains("groceries") && line.contains("done") && line.contains("via"));
    }

    #[test]
    fn append_completion_creates_file_when_missing() {
        let dir = tempdir().unwrap();
        let data = DataDir::new(dir.path().to_path_buf());
        assert!(!dir.path().join(EVENTS_FILE).exists());
        let c = Completion {
            id: "groceries".to_string(),
            done: date(2026, 6, 6),
            via: "cli".to_string(),
            by: None,
            cost_cents: None,
            note: None,
        };
        append_completion(&data, &c).unwrap();
        assert!(dir.path().join(EVENTS_FILE).exists());
        assert_eq!(load_completions(&data).unwrap().len(), 1);
    }

    #[test]
    fn append_punt_round_trips_via_load_events() {
        let dir = tempdir().unwrap();
        write(dir.path(), EVENTS_FILE, SAMPLE_EVENTS);
        let data = DataDir::new(dir.path().to_path_buf());
        let p = Punt {
            id: "clean-drains".to_string(),
            punt_to: date(2027, 4, 1),
            via: "cli".to_string(),
        };
        append_punt(&data, &p).unwrap();
        let events = load_events(&data).unwrap();
        match events.last().unwrap() {
            Event::Punt(punt) => {
                assert_eq!(punt.id, "clean-drains");
                assert_eq!(punt.punt_to, date(2027, 4, 1));
            }
            other @ Event::Completion(_) => panic!("expected Punt, got {other:?}"),
        }
    }

    #[test]
    fn commit_persists_a_commit_with_message() {
        let dir = git_dir();
        let data = DataDir::new(dir.path().to_path_buf());
        let c = Completion {
            id: "groceries".to_string(),
            done: date(2026, 6, 6),
            via: "cli".to_string(),
            by: None,
            cost_cents: None,
            note: None,
        };
        append_completion(&data, &c).unwrap();
        assert!(commit(&data, "add groceries").unwrap());
        let log = std::process::Command::new("git")
            .args(["log", "--oneline"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        let text = String::from_utf8(log.stdout).unwrap();
        assert!(text.contains("add groceries"), "log: {text}");
    }

    #[test]
    fn commit_is_noop_when_nothing_changed() {
        let dir = git_dir();
        let data = DataDir::new(dir.path().to_path_buf());
        let c = Completion {
            id: "groceries".to_string(),
            done: date(2026, 6, 6),
            via: "cli".to_string(),
            by: None,
            cost_cents: None,
            note: None,
        };
        append_completion(&data, &c).unwrap();
        assert!(commit(&data, "first").unwrap());
        assert!(!commit(&data, "again").unwrap());
    }

    #[test]
    fn commit_returns_false_when_not_a_repo() {
        let dir = tempdir().unwrap();
        let data = DataDir::new(dir.path().to_path_buf());
        assert!(!commit(&data, "msg").unwrap());
    }

    /// Run a git command in `root` and return its captured stdout as a string.
    fn git_stdout(root: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed");
        String::from_utf8(out.stdout).unwrap()
    }

    #[test]
    fn commit_works_when_data_dir_is_a_subdir_of_an_enclosing_repo() {
        let dir = git_dir();
        let root = dir.path();
        let sub = root.join("maintenance");
        fs::create_dir_all(&sub).unwrap();
        let data = DataDir::new(sub.clone());
        let c = Completion {
            id: "groceries".to_string(),
            done: date(2026, 6, 6),
            via: "cli".to_string(),
            by: None,
            cost_cents: None,
            note: None,
        };
        append_completion(&data, &c).unwrap();
        assert!(commit(&data, "subdir commit").unwrap());

        let log = git_stdout(root, &["log", "--oneline"]);
        assert!(log.contains("subdir commit"), "log: {log}");
        let files = git_stdout(root, &["show", "--name-only", "--format=", "HEAD"]);
        assert!(
            files.contains("maintenance/completions.jsonl"),
            "files: {files}"
        );
    }

    #[test]
    fn commit_uses_pathspec_and_does_not_sweep_unrelated_staged_changes() {
        let dir = git_dir();
        let root = dir.path();
        let sub = root.join("maintenance");
        fs::create_dir_all(&sub).unwrap();

        // An unrelated file staged in the enclosing repo.
        fs::write(root.join("other.txt"), "unrelated\n").unwrap();
        assert!(std::process::Command::new("git")
            .args(["add", "other.txt"])
            .current_dir(root)
            .status()
            .unwrap()
            .success());

        let data = DataDir::new(sub);
        let c = Completion {
            id: "groceries".to_string(),
            done: date(2026, 6, 6),
            via: "cli".to_string(),
            by: None,
            cost_cents: None,
            note: None,
        };
        append_completion(&data, &c).unwrap();
        assert!(commit(&data, "scoped commit").unwrap());

        let files = git_stdout(root, &["show", "--name-only", "--format=", "HEAD"]);
        assert!(
            files.contains("maintenance/completions.jsonl"),
            "files: {files}"
        );
        assert!(
            !files.contains("other.txt"),
            "other.txt must not be committed; files: {files}"
        );

        // other.txt remains staged (still present in the index, not in HEAD).
        let staged = git_stdout(root, &["diff", "--cached", "--name-only"]);
        assert!(
            staged.contains("other.txt"),
            "other.txt should still be staged; staged: {staged}"
        );
    }

    #[test]
    fn on_field_task_loads_as_fixed_schedule() {
        let dir = tempdir().unwrap();
        write(
            dir.path(),
            TASKS_FILE,
            "- id: blow-out-sprinklers\n  name: Blow out sprinklers\n  every: yearly\n  on: \"10-15\"\n",
        );
        let data = DataDir::new(dir.path().to_path_buf());
        let tasks = load_tasks(&data).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(matches!(tasks[0].schedule, Schedule::Fixed(_)));
    }
}
