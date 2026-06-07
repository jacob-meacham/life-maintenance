//! Data-access layer: load tasks, vendors, and the event log from the data
//! directory, validating vendor references.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

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

#[cfg(test)]
mod tests {
    use super::{
        load_all, load_completions, load_events, load_tasks, load_vendors, DataDir, EVENTS_FILE,
        TASKS_FILE, VENDORS_FILE,
    };
    use crate::error::Error;
    use crate::model::Event;
    use crate::schedule::Schedule;
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
        assert!(matches!(load_events(&data), Err(Error::DataFile(_))));
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
