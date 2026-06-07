//! The application service (controller): orchestrates the store and the status
//! engine. It performs no I/O of its own beyond store calls and no presentation
//! concerns — read operations return owned domain data (or a stable JSON feed),
//! leaving formatting and stdout to the CLI layer.

use jiff::civil::Date;
use serde_json::{json, Value};

use std::collections::BTreeMap;

use crate::error::{Error, Result};
use crate::model::{Completion, Punt, Task, Vendor};
use crate::status::{compute_status, Bucket, TaskStatus};
use crate::store::{self, DataDir};

/// Which built-in summary [`Service::report`] produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    /// Total recorded spend per task id.
    SpendByTask,
    /// Total recorded spend per completion year.
    PerYear,
    /// Counts of tasks by urgency bucket.
    OverdueCount,
}

/// The application service (controller): orchestrates the store + engine.
pub struct Service {
    /// The data directory the service reads from.
    pub data_dir: DataDir,
}

impl Service {
    /// Create a service backed by `data_dir`.
    #[must_use]
    pub fn new(data_dir: DataDir) -> Self {
        Self { data_dir }
    }

    /// All task statuses for `today`, optionally filtered by `query` and by a
    /// set of buckets.
    ///
    /// `query` matches case-insensitively against a task's id, name, notes, and
    /// vendor id. `only`, when present, keeps only statuses whose bucket is in
    /// the set.
    ///
    /// # Errors
    ///
    /// Returns any error from the store loaders (including
    /// [`crate::error::Error::UnknownVendor`] from vendor-reference validation)
    /// or from the status engine.
    pub fn list_tasks(
        &self,
        today: Date,
        query: Option<&str>,
        only: Option<&[Bucket]>,
    ) -> Result<Vec<TaskStatus>> {
        // `load_all` runs vendor-reference validation as a side effect; the
        // status engine additionally needs the full event log (punts included),
        // which `load_all` does not return, so load the events separately.
        let (tasks, _vendors, _completions) = store::load_all(&self.data_dir)?;
        let events = store::load_events(&self.data_dir)?;
        let mut statuses = compute_status(&tasks, &events, today)?;
        if let Some(q) = query {
            let q = q.to_lowercase();
            statuses.retain(|s| matches(&s.task, &q));
        }
        if let Some(buckets) = only {
            statuses.retain(|s| buckets.contains(&s.bucket));
        }
        Ok(statuses)
    }

    /// Tasks that are overdue, due, or in their prep window.
    ///
    /// # Errors
    ///
    /// Returns any error from [`Service::list_tasks`].
    pub fn due(&self, today: Date) -> Result<Vec<TaskStatus>> {
        self.list_tasks(
            today,
            None,
            Some(&[Bucket::Overdue, Bucket::Due, Bucket::Prep]),
        )
    }

    /// Completion history, optionally filtered by task id and/or a since-date
    /// (inclusive).
    ///
    /// # Errors
    ///
    /// Returns any error from loading the completion log.
    pub fn history(&self, task_id: Option<&str>, since: Option<Date>) -> Result<Vec<Completion>> {
        let mut rows = store::load_completions(&self.data_dir)?;
        if let Some(id) = task_id {
            rows.retain(|c| c.id == id);
        }
        if let Some(d) = since {
            rows.retain(|c| c.done >= d);
        }
        Ok(rows)
    }

    /// The vendor directory.
    ///
    /// # Errors
    ///
    /// Returns any error from the store loaders, including vendor-reference
    /// validation.
    pub fn vendors(&self) -> Result<Vec<Vendor>> {
        let (_tasks, vendors, _completions) = store::load_all(&self.data_dir)?;
        Ok(vendors)
    }

    /// Full denormalized dataset as JSON (the stable export feed).
    ///
    /// Per-task `completions` come from the completion log only; punts are part
    /// of status computation but must not appear as completions.
    ///
    /// # Errors
    ///
    /// Returns any error from the store loaders or the status engine.
    pub fn export(&self, today: Date) -> Result<Value> {
        // `load_all` gives validated tasks/vendors/completions; the engine also
        // needs the punt events from the full log.
        let (tasks, vendors, completions) = store::load_all(&self.data_dir)?;
        let events = store::load_events(&self.data_dir)?;
        let statuses = compute_status(&tasks, &events, today)?;

        let rows: Vec<Value> = statuses
            .iter()
            .map(|s| task_row(s, &vendors, &completions))
            .collect();

        Ok(json!({
            "schema_version": 1,
            "generated_for": today.to_string(),
            "tasks": rows,
        }))
    }

    /// Record a task as completed; validates the task id exists before writing.
    ///
    /// When `do_commit` is set, the change is committed to the data directory's
    /// git repository (a no-op if it is not a repo).
    ///
    /// # Errors
    ///
    /// Returns [`Error::DataFile`] if the id is unknown (no write occurs), and
    /// propagates any store or commit error.
    #[allow(clippy::too_many_arguments)] // each argument is a distinct completion field
    pub fn complete(
        &self,
        task_id: &str,
        done: Date,
        via: &str,
        by: &str,
        cost_cents: Option<i64>,
        note: Option<&str>,
        do_commit: bool,
    ) -> Result<Completion> {
        let (tasks, _vendors, _completions) = store::load_all(&self.data_dir)?;
        if !tasks.iter().any(|t| t.id == task_id) {
            return Err(Error::DataFile(format!("unknown task id {task_id}")));
        }
        let completion = Completion {
            id: task_id.to_string(),
            done,
            via: via.to_string(),
            by: Some(by.to_string()),
            cost_cents,
            note: note.map(str::to_string),
        };
        store::append_completion(&self.data_dir, &completion)?;
        if do_commit {
            store::commit(&self.data_dir, &format!("complete {task_id} on {done}"))?;
        }
        Ok(completion)
    }

    /// Defer a task's next occurrence to `to`; validates the task id exists
    /// before writing.
    ///
    /// When `do_commit` is set, the change is committed to the data directory's
    /// git repository (a no-op if it is not a repo).
    ///
    /// # Errors
    ///
    /// Returns [`Error::DataFile`] if the id is unknown (no write occurs), and
    /// propagates any store or commit error.
    pub fn punt(&self, task_id: &str, to: Date, via: &str, do_commit: bool) -> Result<Punt> {
        let (tasks, _vendors, _completions) = store::load_all(&self.data_dir)?;
        if !tasks.iter().any(|t| t.id == task_id) {
            return Err(Error::DataFile(format!("unknown task id {task_id}")));
        }
        let punt = Punt {
            id: task_id.to_string(),
            punt_to: to,
            via: via.to_string(),
        };
        store::append_punt(&self.data_dir, &punt)?;
        if do_commit {
            store::commit(&self.data_dir, &format!("punt {task_id} to {to}"))?;
        }
        Ok(punt)
    }

    /// A built-in summary report as JSON.
    ///
    /// Spend reports draw on the completion log (the cost data); the overdue
    /// report draws on the full event log so punts shift the counts.
    ///
    /// # Errors
    ///
    /// Returns any error from the store loaders or the status engine.
    pub fn report(&self, kind: ReportKind, today: Date) -> Result<Value> {
        match kind {
            ReportKind::SpendByTask => {
                let completions = store::load_completions(&self.data_dir)?;
                let mut spend: BTreeMap<String, i64> = BTreeMap::new();
                for c in &completions {
                    if let Some(cents) = c.cost_cents {
                        *spend.entry(c.id.clone()).or_default() += cents;
                    }
                }
                Ok(json!({ "spend_by_task": spend }))
            }
            ReportKind::PerYear => {
                let completions = store::load_completions(&self.data_dir)?;
                let mut per_year: BTreeMap<String, i64> = BTreeMap::new();
                for c in &completions {
                    if let Some(cents) = c.cost_cents {
                        *per_year.entry(c.done.year().to_string()).or_default() += cents;
                    }
                }
                Ok(json!({ "per_year": per_year }))
            }
            ReportKind::OverdueCount => {
                let tasks = store::load_tasks(&self.data_dir)?;
                let events = store::load_events(&self.data_dir)?;
                let statuses = compute_status(&tasks, &events, today)?;
                let count = |bucket: Bucket| statuses.iter().filter(|s| s.bucket == bucket).count();
                Ok(json!({
                    "overdue": count(Bucket::Overdue),
                    "due": count(Bucket::Due),
                    "prep": count(Bucket::Prep),
                }))
            }
        }
    }
}

/// Whether `task` matches the lowercased query `q` in any searchable field.
fn matches(task: &Task, q: &str) -> bool {
    [
        task.id.as_str(),
        task.name.as_str(),
        task.notes.as_deref().unwrap_or(""),
        task.vendor.as_deref().unwrap_or(""),
    ]
    .iter()
    .any(|f| f.to_lowercase().contains(q))
}

/// Build the denormalized JSON row for a single task status.
fn task_row(status: &TaskStatus, vendors: &[Vendor], completions: &[Completion]) -> Value {
    let task = &status.task;

    let vendor = task
        .vendor
        .as_deref()
        .and_then(|vid| vendors.iter().find(|v| v.id == vid))
        .map_or(
            Value::Null,
            |v| json!({ "id": v.id, "name": v.name, "phone": v.phone }),
        );

    let task_completions: Vec<Value> = completions
        .iter()
        .filter(|c| c.id == task.id)
        .map(|c| {
            json!({
                "done": c.done.to_string(),
                "via": c.via,
                "by": c.by,
                "cost_cents": c.cost_cents,
                "note": c.note,
            })
        })
        .collect();

    json!({
        "id": task.id,
        "name": task.name,
        "bucket": status.bucket,
        "last_done": status.last_done.map(|d| d.to_string()),
        "next_due": status.next_due.to_string(),
        "prep_due": status.prep_due.map(|d| d.to_string()),
        "vendor": vendor,
        "completions": task_completions,
    })
}

#[cfg(test)]
mod tests {
    use super::{ReportKind, Service};
    use crate::error::Error;
    use crate::status::Bucket;
    use crate::store::DataDir;
    use jiff::civil::{date, Date};
    use std::fs;
    use std::path::Path;
    use std::process::Command;
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

    /// Build a service over a fresh temp dir holding the standard sample data.
    fn service() -> (TempDir, Service) {
        let dir = tempdir().unwrap();
        write(dir.path(), "tasks.yaml", SAMPLE_TASKS);
        write(dir.path(), "vendors.yaml", SAMPLE_VENDORS);
        write(dir.path(), "completions.jsonl", SAMPLE_EVENTS);
        let svc = Service::new(DataDir::new(dir.path().to_path_buf()));
        (dir, svc)
    }

    /// Build a service over a git-initialised temp dir holding the sample data,
    /// with an initial commit so later commits have a parent to compare against.
    fn git_service() -> (TempDir, Service) {
        let dir = tempdir().unwrap();
        let root = dir.path();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@e.com"],
            vec!["config", "user.name", "T"],
        ] {
            Command::new("git")
                .args(&args)
                .current_dir(root)
                .status()
                .unwrap();
        }
        write(root, "tasks.yaml", SAMPLE_TASKS);
        write(root, "vendors.yaml", SAMPLE_VENDORS);
        write(root, "completions.jsonl", SAMPLE_EVENTS);
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(root)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-q", "-m", "seed"])
            .current_dir(root)
            .status()
            .unwrap();
        let svc = Service::new(DataDir::new(root.to_path_buf()));
        (dir, svc)
    }

    /// Number of commits reachable from HEAD in `root`.
    fn commit_count(root: &Path) -> u32 {
        let out = Command::new("git")
            .args(["rev-list", "--count", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    const TODAY: Date = date(2026, 6, 6);

    fn ids(statuses: &[crate::status::TaskStatus]) -> Vec<&str> {
        statuses.iter().map(|s| s.task.id.as_str()).collect()
    }

    #[test]
    fn list_tasks_returns_all_three_ids() {
        let (_dir, svc) = service();
        let statuses = svc.list_tasks(TODAY, None, None).unwrap();
        assert_eq!(
            ids(&statuses),
            ["groceries", "clean-drains", "blow-out-sprinklers"]
        );
    }

    #[test]
    fn list_tasks_query_matches_name() {
        let (_dir, svc) = service();
        let statuses = svc.list_tasks(TODAY, Some("drain"), None).unwrap();
        assert_eq!(ids(&statuses), ["clean-drains"]);
    }

    #[test]
    fn list_tasks_query_matches_vendor_id() {
        let (_dir, svc) = service();
        let statuses = svc.list_tasks(TODAY, Some("roto"), None).unwrap();
        assert_eq!(ids(&statuses), ["clean-drains"]);
    }

    #[test]
    fn list_tasks_only_overdue_filters_to_clean_drains() {
        let (_dir, svc) = service();
        let statuses = svc
            .list_tasks(TODAY, None, Some(&[Bucket::Overdue]))
            .unwrap();
        assert_eq!(ids(&statuses), ["clean-drains"]);
    }

    #[test]
    fn due_includes_overdue_excludes_ok() {
        let (_dir, svc) = service();
        let statuses = svc.due(TODAY).unwrap();
        let got = ids(&statuses);
        assert!(got.contains(&"clean-drains"), "got: {got:?}");
        assert!(!got.contains(&"groceries"), "got: {got:?}");
    }

    #[test]
    fn history_all_returns_two() {
        let (_dir, svc) = service();
        let rows = svc.history(None, None).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn history_by_task_id_filters_and_carries_cost() {
        let (_dir, svc) = service();
        let rows = svc.history(Some("clean-drains"), None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "clean-drains");
        assert_eq!(rows[0].cost_cents, Some(28500));
    }

    #[test]
    fn history_since_keeps_only_recent() {
        let (_dir, svc) = service();
        let rows = svc.history(None, Some(date(2026, 1, 1))).unwrap();
        let got: Vec<&str> = rows.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(got, ["groceries"]);
    }

    #[test]
    fn vendors_returns_directory() {
        let (_dir, svc) = service();
        let vendors = svc.vendors().unwrap();
        let got: Vec<&str> = vendors.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(got, ["roto-rooter"]);
    }

    #[test]
    fn export_carries_schema_version() {
        let (_dir, svc) = service();
        let payload = svc.export(TODAY).unwrap();
        assert_eq!(payload["schema_version"], 1);
        assert_eq!(payload["generated_for"], "2026-06-06");
    }

    #[test]
    fn export_clean_drains_row_is_denormalized() {
        let (_dir, svc) = service();
        let payload = svc.export(TODAY).unwrap();
        let tasks = payload["tasks"].as_array().unwrap();
        let row = tasks
            .iter()
            .find(|t| t["id"] == "clean-drains")
            .expect("clean-drains row");
        assert_eq!(row["bucket"], "overdue");
        assert_eq!(row["vendor"]["name"], "Roto-Rooter");
        assert_eq!(row["next_due"], "2026-05-01");
        let completions = row["completions"].as_array().unwrap();
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0]["cost_cents"], 28500);
    }

    #[test]
    fn export_task_without_vendor_has_null_vendor() {
        let (_dir, svc) = service();
        let payload = svc.export(TODAY).unwrap();
        let tasks = payload["tasks"].as_array().unwrap();
        let row = tasks
            .iter()
            .find(|t| t["id"] == "groceries")
            .expect("groceries row");
        assert!(row["vendor"].is_null());
    }

    #[test]
    fn complete_appends_and_returns_completion_visible_in_history() {
        let (_dir, svc) = service();
        let completion = svc
            .complete(
                "groceries",
                date(2026, 6, 6),
                "cli",
                "Me",
                Some(4200),
                Some("weekly run"),
                false,
            )
            .unwrap();
        assert_eq!(completion.id, "groceries");
        assert_eq!(completion.cost_cents, Some(4200));
        let rows = svc.history(Some("groceries"), None).unwrap();
        assert_eq!(rows.len(), 2);
        let last = rows.last().unwrap();
        assert_eq!(last.done, date(2026, 6, 6));
        assert_eq!(last.cost_cents, Some(4200));
        assert_eq!(last.by, Some("Me".to_string()));
    }

    #[test]
    fn complete_unknown_id_is_data_file_error() {
        let (_dir, svc) = service();
        match svc.complete("nope", date(2026, 6, 6), "cli", "Me", None, None, false) {
            Err(Error::DataFile(msg)) => assert!(msg.contains("nope"), "msg: {msg}"),
            other => panic!("expected DataFile, got {other:?}"),
        }
    }

    #[test]
    fn complete_resets_next_due() {
        let (_dir, svc) = service();
        svc.complete(
            "clean-drains",
            date(2026, 6, 6),
            "cli",
            "Me",
            None,
            None,
            false,
        )
        .unwrap();
        let statuses = svc.list_tasks(TODAY, Some("clean-drains"), None).unwrap();
        let status = &statuses[0];
        assert_eq!(status.next_due, date(2027, 6, 6));
        assert_eq!(status.bucket, Bucket::Ok);
    }

    #[test]
    fn complete_without_commit_appends_but_creates_no_commit() {
        let (dir, svc) = git_service();
        let before = commit_count(dir.path());
        svc.complete(
            "groceries",
            date(2026, 6, 6),
            "cli",
            "Me",
            None,
            None,
            false,
        )
        .unwrap();
        assert_eq!(commit_count(dir.path()), before);
        let rows = svc.history(Some("groceries"), None).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn complete_with_commit_creates_a_commit() {
        let (dir, svc) = git_service();
        let before = commit_count(dir.path());
        svc.complete("groceries", date(2026, 6, 6), "cli", "Me", None, None, true)
            .unwrap();
        assert_eq!(commit_count(dir.path()), before + 1);
    }

    #[test]
    fn punt_unknown_id_is_data_file_error() {
        let (_dir, svc) = service();
        match svc.punt("nope", date(2026, 9, 1), "cli", false) {
            Err(Error::DataFile(msg)) => assert!(msg.contains("nope"), "msg: {msg}"),
            other => panic!("expected DataFile, got {other:?}"),
        }
    }

    #[test]
    fn punt_defers_next_due_at_service_level() {
        let (_dir, svc) = service();
        let punt = svc
            .punt("clean-drains", date(2026, 9, 1), "cli", false)
            .unwrap();
        assert_eq!(punt.punt_to, date(2026, 9, 1));
        let statuses = svc.list_tasks(TODAY, Some("clean-drains"), None).unwrap();
        let status = &statuses[0];
        assert_eq!(status.next_due, date(2026, 9, 1));
        assert_eq!(status.bucket, Bucket::Ok);
    }

    #[test]
    fn punt_does_not_appear_in_history() {
        let (_dir, svc) = service();
        svc.punt("groceries", date(2026, 9, 1), "cli", false)
            .unwrap();
        let rows = svc.history(Some("groceries"), None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].done, date(2026, 6, 1));
    }

    #[test]
    fn complete_after_punt_reverts_to_schedule() {
        let (_dir, svc) = service();
        svc.punt("clean-drains", date(2026, 9, 1), "cli", false)
            .unwrap();
        svc.complete(
            "clean-drains",
            date(2026, 6, 6),
            "cli",
            "Me",
            None,
            None,
            false,
        )
        .unwrap();
        let statuses = svc.list_tasks(TODAY, Some("clean-drains"), None).unwrap();
        assert_eq!(statuses[0].next_due, date(2027, 6, 6));
    }

    #[test]
    fn punt_without_commit_creates_no_commit() {
        let (dir, svc) = git_service();
        let before = commit_count(dir.path());
        svc.punt("groceries", date(2026, 9, 1), "cli", false)
            .unwrap();
        assert_eq!(commit_count(dir.path()), before);
    }

    #[test]
    fn report_spend_by_task_sums_costs() {
        let (_dir, svc) = service();
        let report = svc.report(ReportKind::SpendByTask, TODAY).unwrap();
        assert_eq!(report["spend_by_task"]["clean-drains"], 28500);
    }

    #[test]
    fn report_per_year_groups_by_year() {
        let (_dir, svc) = service();
        let report = svc.report(ReportKind::PerYear, TODAY).unwrap();
        assert_eq!(report["per_year"]["2025"], 28500);
    }

    #[test]
    fn report_overdue_count_today() {
        let (_dir, svc) = service();
        let report = svc.report(ReportKind::OverdueCount, TODAY).unwrap();
        assert!(report["overdue"].as_i64().unwrap() >= 1, "report: {report}");
        assert_eq!(report["due"], 0);
    }

    #[test]
    fn report_overdue_count_includes_prep_in_window() {
        // The shared fixture's sprinklers task has no lead time, so it never
        // enters the prep bucket; give it one here to exercise that path.
        let dir = tempdir().unwrap();
        let tasks = "\
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
  lead_time: 2 weeks
";
        write(dir.path(), "tasks.yaml", tasks);
        write(dir.path(), "vendors.yaml", SAMPLE_VENDORS);
        write(dir.path(), "completions.jsonl", SAMPLE_EVENTS);
        let svc = Service::new(DataDir::new(dir.path().to_path_buf()));
        let report = svc
            .report(ReportKind::OverdueCount, date(2026, 10, 5))
            .unwrap();
        // groceries (weekly, last done 2026-06-01) and clean-drains (yearly,
        // last done 2025-05-01) are both overdue by 2026-10-05; sprinklers is in
        // its 2-week prep window before 2026-10-15.
        assert_eq!(report["prep"], 1);
        assert_eq!(report["overdue"], 2);
    }

    #[test]
    fn report_spend_sums_multiple_same_task_completions() {
        let (dir, svc) = git_service();
        svc.complete(
            "groceries",
            date(2026, 6, 2),
            "cli",
            "Me",
            Some(1000),
            None,
            false,
        )
        .unwrap();
        svc.complete(
            "groceries",
            date(2026, 6, 3),
            "cli",
            "Me",
            Some(2000),
            None,
            false,
        )
        .unwrap();
        let report = svc.report(ReportKind::SpendByTask, TODAY).unwrap();
        assert_eq!(report["spend_by_task"]["groceries"], 3000);
        drop(dir);
    }
}
