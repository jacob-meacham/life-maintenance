# `lm show <id>` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an `lm show <id>` command that prints a single task's prep checklist, notes, vendor contact, and status/schedule in both text and `--json` form.

**Architecture:** A new `Service::show(today, id)` returns an owned `TaskDetail { status, vendor }`; `cli.rs` renders the text and JSON layouts. The `Task` model gains raw `every`/`on` strings so the recurrence rule prints faithfully (instead of the parsed `Span`'s ISO form).

**Tech Stack:** Rust, clap (derive), jiff, serde_json, assert_cmd + predicates for integration tests.

Spec: `docs/specs/2026-06-08-lm-show-task-detail-design.md`.

---

### Task 1: Carry raw recurrence (`every`/`on`) on the `Task` model

The parsed `Task` discards the human-authored cadence. Add the raw strings so `show` can print `yearly (on 10-15)` rather than ISO `P6M`.

**Files:**
- Modify: `src/model.rs` (the `Task` struct around lines 34-45 and `Task::from_raw` around lines 53-69)
- Test: `src/model.rs` (the `#[cfg(test)] mod tests` block)

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src/model.rs`:

```rust
#[test]
fn task_from_raw_carries_raw_recurrence_relative() {
    let raw: RawTask = serde_yaml::from_str("id: x\nname: X\nevery: 6 months\n").unwrap();
    let task = Task::from_raw(raw).unwrap();
    assert_eq!(task.every, "6 months");
    assert_eq!(task.on, None);
}

#[test]
fn task_from_raw_carries_raw_recurrence_fixed_monthday() {
    let raw: RawTask =
        serde_yaml::from_str("id: s\nname: S\nevery: yearly\non: \"10-15\"\n").unwrap();
    let task = Task::from_raw(raw).unwrap();
    assert_eq!(task.every, "yearly");
    assert_eq!(task.on, Some("10-15".to_string()));
}

#[test]
fn task_from_raw_carries_raw_recurrence_fixed_day() {
    let raw: RawTask = serde_yaml::from_str("id: m\nname: M\nevery: monthly\non: 15\n").unwrap();
    let task = Task::from_raw(raw).unwrap();
    assert_eq!(task.on, Some("15".to_string()));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib model::tests::task_from_raw_carries_raw_recurrence`
Expected: FAIL to compile — `Task` has no field `every` / `on`.

- [ ] **Step 3: Add the fields and populate them**

In `src/model.rs`, add the import for `OnSpec` at the top alongside the existing schema import:

```rust
use crate::schema::{OnSpec, RawCompletion, RawPunt, RawTask, RawVendor};
```

Add two fields to the `Task` struct (after `name`, before `schedule`):

```rust
pub struct Task {
    pub id: String,
    pub name: String,
    /// The raw `every:` cadence string, verbatim, for display.
    pub every: String,
    /// The raw `on:` anchor stringified (`"15"` or `"10-15"`), for display.
    pub on: Option<String>,
    pub schedule: Schedule,
    pub lead_time: Option<Span>,
    pub prep: Vec<String>,
    pub vendor: Option<String>,
    pub notes: Option<String>,
    pub start: Option<Date>,
}
```

Add a free function in `src/model.rs` (above `impl Task`) that stringifies the anchor:

```rust
/// Render a raw `on:` spec to its canonical display string.
fn on_to_string(on: &OnSpec) -> String {
    match on {
        OnSpec::Day(d) => d.to_string(),
        OnSpec::MonthDay(s) => s.clone(),
    }
}
```

In `Task::from_raw`, capture the raw values *before* `raw` is moved into the
struct, and set the new fields. The full method becomes:

```rust
pub fn from_raw(raw: RawTask) -> Result<Self> {
    let lead_time = match raw.lead_time.as_deref() {
        Some(s) => Some(parse_interval(s)?),
        None => None,
    };
    let schedule = parse_schedule(&raw.every, raw.on.as_ref())?;
    let on = raw.on.as_ref().map(on_to_string);
    Ok(Self {
        id: raw.id,
        name: raw.name,
        every: raw.every,
        on,
        schedule,
        lead_time,
        prep: raw.prep,
        vendor: raw.vendor,
        notes: raw.notes,
        start: raw.start,
    })
}
```

Confirm `OnSpec` is `pub` in `src/schema.rs` (it is, line 9). No schema change.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib model::tests::task_from_raw`
Expected: PASS (all `task_from_raw_*` tests, including the existing ones).

- [ ] **Step 5: Commit**

```bash
git add src/model.rs
git commit -m "feat: carry raw every/on recurrence strings on Task for display"
```

---

### Task 2: `Service::show` returning `TaskDetail`

**Files:**
- Modify: `src/service.rs` (add `TaskDetail` struct + `show` method to `impl Service`)
- Test: `src/service.rs` (the `#[cfg(test)] mod tests` block)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/service.rs` (the fixtures `service()`, `TODAY`, sample data are already defined there):

```rust
#[test]
fn show_returns_detail_with_resolved_vendor() {
    let (_dir, svc) = service();
    let detail = svc.show(TODAY, "clean-drains").unwrap();
    assert_eq!(detail.status.task.id, "clean-drains");
    assert_eq!(detail.status.bucket, Bucket::Overdue);
    let vendor = detail.vendor.expect("vendor resolved");
    assert_eq!(vendor.id, "roto-rooter");
    assert_eq!(vendor.name, "Roto-Rooter");
}

#[test]
fn show_task_without_vendor_has_none() {
    let (_dir, svc) = service();
    let detail = svc.show(TODAY, "groceries").unwrap();
    assert!(detail.vendor.is_none());
    assert_eq!(detail.status.task.every, "weekly");
    assert_eq!(detail.status.task.on, None);
}

#[test]
fn show_fixed_task_carries_on_anchor() {
    let (_dir, svc) = service();
    let detail = svc.show(TODAY, "blow-out-sprinklers").unwrap();
    assert_eq!(detail.status.task.on, Some("10-15".to_string()));
}

#[test]
fn show_unknown_id_is_data_file_error() {
    let (_dir, svc) = service();
    match svc.show(TODAY, "nope") {
        Err(Error::DataFile(msg)) => assert!(msg.contains("nope"), "msg: {msg}"),
        other => panic!("expected DataFile, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib service::tests::show_`
Expected: FAIL to compile — no method `show`, no type `TaskDetail`.

- [ ] **Step 3: Add `TaskDetail` and `Service::show`**

In `src/service.rs`, add the struct after the `Service` struct definition (after line 31, before `impl Service`):

```rust
/// The full detail of a single task: its computed status plus the resolved
/// vendor (if the task references one).
#[derive(Debug, Clone)]
pub struct TaskDetail {
    /// The task's computed status (owns the full `Task`).
    pub status: TaskStatus,
    /// The vendor referenced by the task, resolved from the directory.
    pub vendor: Option<Vendor>,
}
```

Add the method inside `impl Service` (place it after `list_tasks`, before `due`):

```rust
/// The full detail of a single task for `today`: its status plus resolved
/// vendor.
///
/// # Errors
///
/// Returns [`Error::DataFile`] if `id` matches no task, and propagates any
/// error from the store loaders or the status engine.
pub fn show(&self, today: Date, id: &str) -> Result<TaskDetail> {
    let (tasks, vendors, _completions) = store::load_all(&self.data_dir)?;
    let events = store::load_events(&self.data_dir)?;
    let statuses = compute_status(&tasks, &events, today)?;
    let status = statuses
        .into_iter()
        .find(|s| s.task.id == id)
        .ok_or_else(|| Error::DataFile(format!("unknown task id {id}")))?;
    let vendor = status
        .task
        .vendor
        .as_deref()
        .and_then(|vid| vendors.iter().find(|v| v.id == vid).cloned());
    Ok(TaskDetail { status, vendor })
}
```

`Vendor` is already imported (`use crate::model::{Completion, Punt, Task, Vendor};`). `TaskStatus` is already imported.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib service::tests::show_`
Expected: PASS (all four `show_*` tests).

- [ ] **Step 5: Commit**

```bash
git add src/service.rs
git commit -m "feat: add Service::show returning TaskDetail for a single task"
```

---

### Task 3: `lm show <id>` CLI command (text + JSON)

**Files:**
- Modify: `src/cli.rs` (add `Command::Show` variant, dispatch arm, `cmd_show`, and `show_text` / `show_json` renderers)
- Test: `tests/cli.rs` (integration tests)

- [ ] **Step 1: Write the failing integration tests**

The shared fixture in `tests/cli.rs` has no task with `prep`/`notes`, so add a
dedicated fixture for the text test. Add these tests to `tests/cli.rs`:

```rust
const PREP_TASKS: &str = "\
- id: clean-drains
  name: Clean drains
  every: yearly
  on: \"10-15\"
  lead_time: 2 weeks
  vendor: roto-rooter
  notes: access panel is behind the dryer
  prep:
    - clear the area
    - move boxes
";

#[test]
fn show_text_includes_prep_notes_and_vendor() {
    let dir = tempdir().unwrap();
    write_dataset(dir.path(), PREP_TASKS);
    lm(dir.path())
        .args(["show", "clean-drains", "--today", "2026-06-06"])
        .assert()
        .success()
        .stdout(predicate::str::contains("clean-drains"))
        .stdout(predicate::str::contains("yearly (on 10-15)"))
        .stdout(predicate::str::contains("clear the area"))
        .stdout(predicate::str::contains("access panel is behind the dryer"))
        .stdout(predicate::str::contains("Roto-Rooter"))
        .stdout(predicate::str::contains("lm history --id clean-drains"));
}

#[test]
fn show_json_has_documented_shape() {
    let dir = sample_dir();
    let value = stdout_json(
        lm(dir.path()).args(["show", "clean-drains", "--today", "2026-06-06", "--json"]),
    );
    assert_eq!(value["id"], "clean-drains");
    assert_eq!(value["bucket"], "overdue");
    assert_eq!(value["recurrence"]["every"], "yearly");
    assert_eq!(value["vendor"]["name"], "Roto-Rooter");
    assert!(value["prep"].is_array());
}

#[test]
fn show_unknown_id_fails_with_message() {
    let dir = sample_dir();
    lm(dir.path())
        .args(["show", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown task id nope"));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --test cli show_`
Expected: FAIL — `lm show` is an unrecognized subcommand (clap error / non-zero where success expected).

- [ ] **Step 3: Add the `Show` command variant**

In `src/cli.rs`, add to the `Command` enum (after the `List` variant, before `Due`):

```rust
/// Show the full detail of a single task.
Show {
    /// The task id to inspect.
    id: String,
    /// Treat this date as today (YYYY-MM-DD).
    #[arg(long)]
    today: Option<String>,
    /// Emit JSON instead of text.
    #[arg(long)]
    json: bool,
},
```

Add the dispatch arm in `run` (after the `Command::List { .. }` arm):

```rust
Command::Show { id, today, json } => cmd_show(&service()?, id, today.as_deref(), *json),
```

Import `TaskDetail` — update the service import line:

```rust
use crate::service::{ReportKind, Service, TaskDetail};
```

- [ ] **Step 4: Add the handler and renderers**

In `src/cli.rs`, add `cmd_show` near the other `cmd_*` handlers (e.g. after `cmd_due`):

```rust
/// Handle `show`.
fn cmd_show(
    service: &Service,
    id: &str,
    today: Option<&str>,
    json: bool,
) -> Result<(), CliError> {
    let today = resolve_today(today)?;
    let detail = service.show(today, id)?;
    if json {
        show_json(&detail);
    } else {
        show_text(&detail);
    }
    Ok(())
}
```

Add a bucket-label helper near `bucket_marker`:

```rust
/// The lowercase label of a bucket (e.g. "overdue"), via its serde encoding.
fn bucket_label(bucket: Bucket) -> String {
    serde_json::to_value(bucket)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}
```

Add the JSON renderer near the other `*_json` helpers:

```rust
/// Render a `TaskDetail` as the documented single-object JSON.
fn show_json(detail: &TaskDetail) {
    let status = &detail.status;
    let task = &status.task;
    let vendor = detail.vendor.as_ref().map_or(Value::Null, |v| {
        json!({
            "id": v.id,
            "name": v.name,
            "phone": v.phone,
            "email": v.email,
            "notes": v.notes,
        })
    });
    print_pretty(&json!({
        "id": task.id,
        "name": task.name,
        "bucket": status.bucket,
        "last_done": status.last_done.map(|d| d.to_string()),
        "next_due": status.next_due.to_string(),
        "prep_due": status.prep_due.map(|d| d.to_string()),
        "recurrence": { "every": task.every, "on": task.on },
        "prep": task.prep,
        "notes": task.notes,
        "vendor": vendor,
    }));
}
```

Add the text renderer near the other `print_*` helpers:

```rust
/// Render a `TaskDetail` as the human-readable text layout.
fn show_text(detail: &TaskDetail) {
    let status = &detail.status;
    let task = &status.task;

    println!("{}  {}", task.id, task.name);

    let last = status
        .last_done
        .map_or(String::new(), |d| format!(", last done {d}"));
    println!(
        "  status:     {} (due {}{last})",
        bucket_label(status.bucket),
        status.next_due,
    );

    let on = task
        .on
        .as_deref()
        .map_or(String::new(), |o| format!(" (on {o})"));
    println!("  recurrence: {}{on}", task.every);

    if let Some(prep_due) = status.prep_due {
        println!("  prep opens: {prep_due}");
    }

    if !task.prep.is_empty() {
        println!("  prep:");
        for step in &task.prep {
            println!("    - {step}");
        }
    }

    if let Some(notes) = &task.notes {
        println!("  notes:      {notes}");
    }

    if let Some(v) = &detail.vendor {
        let mut line = format!("  vendor:     {}", v.name);
        if let Some(phone) = &v.phone {
            line.push_str(&format!("  {phone}"));
        }
        if let Some(email) = &v.email {
            line.push_str(&format!("  {email}"));
        }
        println!("{line}");
        if let Some(notes) = &v.notes {
            println!("              {notes}");
        }
    }

    println!("  (history: lm history --id {})", task.id);
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --test cli show_`
Expected: PASS (all three `show_*` integration tests).

- [ ] **Step 6: Run the full check suite**

Run: `cargo test && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all tests pass, no clippy warnings, formatting clean.

- [ ] **Step 7: Commit**

```bash
git add src/cli.rs tests/cli.rs
git commit -m "feat: add lm show <id> command with text and --json output"
```

---

### Task 4: Document `lm show` in the README

**Files:**
- Modify: `README.md` (the `## Commands` list)

- [ ] **Step 1: Add the command to the list**

In `README.md`, insert this bullet in the `## Commands` section, immediately
after the `lm list ...` bullet:

```markdown
- `lm show <id>` — show one task's full detail: status, schedule, prep
  checklist, notes, and vendor contact. (Use `lm history --id <id>` for its
  completion history.)
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: document lm show in the README commands list"
```

---

## Self-Review Notes

- **Spec coverage:** status/schedule (Task 2 status + Task 3 renderers), prep & notes (Task 3 text/json), vendor contact (Task 2 resolution + Task 3 render), recurrence `{every, on}` (Task 1 model + Task 3 json), `on`-anchor text rule (Task 3 `show_text`), unknown-id error (Task 2 + Task 3 tests), `--today`/`--json` flags (Task 3 variant), README (Task 4). All covered.
- **Type consistency:** `TaskDetail { status: TaskStatus, vendor: Option<Vendor> }` defined in Task 2, imported and consumed in Task 3. `bucket_label` / `show_text` / `show_json` / `cmd_show` names are consistent across steps. `Task.every: String` / `Task.on: Option<String>` defined in Task 1, read in Tasks 2-3.
- **No placeholders:** every code step shows complete code; every test step shows the assertion and the exact `cargo` command with expected outcome.
