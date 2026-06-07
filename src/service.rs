//! The application service (controller): orchestrates the store and the status
//! engine. It performs no I/O of its own beyond store calls and no presentation
//! concerns — read operations return owned domain data (or a stable JSON feed),
//! leaving formatting and stdout to the CLI layer.

use jiff::civil::Date;
use serde_json::{json, Value};

use crate::error::Result;
use crate::model::{Completion, Task, Vendor};
use crate::status::{compute_status, Bucket, TaskStatus};
use crate::store::{self, DataDir};

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
    use super::Service;
    use crate::status::Bucket;
    use crate::store::DataDir;
    use jiff::civil::{date, Date};
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

    /// Build a service over a fresh temp dir holding the standard sample data.
    fn service() -> (TempDir, Service) {
        let dir = tempdir().unwrap();
        write(dir.path(), "tasks.yaml", SAMPLE_TASKS);
        write(dir.path(), "vendors.yaml", SAMPLE_VENDORS);
        write(dir.path(), "completions.jsonl", SAMPLE_EVENTS);
        let svc = Service::new(DataDir::new(dir.path().to_path_buf()));
        (dir, svc)
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
}
