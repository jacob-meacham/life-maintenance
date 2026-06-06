# Life Maintenance Tracker — Rust CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Reimplement the `lm` maintenance-tracker CLI in Rust — a single fast static binary over git-backed YAML/JSONL files — matching the validated design (relative + fixed schedules, lead-time prep, vendors, completions with integer-cents cost, punt/seasonal deferral, search, due/overdue/prep buckets, export, reports).

**Architecture:** Library-first. All logic in `lib.rs` + modules (`error`, `schedule`, `schema`, `model`, `status`, `store`, `service`, `cli`); `main.rs` is a thin shell. The engine (`schedule`, `status`) is pure (no I/O; `today` injected). Boundary parsing uses `serde` with `#[serde(deny_unknown_fields)]`, converting to plain internal domain types. Money is `i64` cents. Dates use `jiff::civil::Date`.

**Tech Stack:** Rust (stable, edition 2021), cargo; `clap` (derive) CLI; `serde` + `serde_json` + `serde_yaml`; `jiff` (civil dates, with `serde` feature); `thiserror` (library errors); `tempfile` + `assert_cmd` (dev/test). git via `std::process::Command`.

**Constitution:** `agent-instructions/coding/constitution/rust.md` + `general.md`. In force: cargo + pinned `rust-toolchain.toml`; `cargo clippy --all-targets -- -D warnings` clean with `clippy::pedantic` on (allow-with-justification only); `cargo fmt --check` clean; **no `unwrap`/`expect`/`panic`/`unreachable`/`todo` in non-test code** (justified-invariant exceptions carry a `// SAFETY/INVARIANT:` comment); typed `thiserror` error enum; enums + exhaustive `match` (no catch-all `_` on domain enums); newtypes/`i64` for money; serde at the boundary with `deny_unknown_fields`; unit tests in `#[cfg(test)]` + black-box tests in `tests/`; exact assertions; TDD.

**Data dir:** `LM_DATA_DIR` env → else `./data`. Holds `tasks.yaml`, `vendors.yaml`, `completions.jsonl` (the append-only event log). Completions/punts are committed into that repo.

---

## File Structure

```
lifemaint/
├── Cargo.toml
├── rust-toolchain.toml
├── src/
│   ├── lib.rs          # crate root: module decls + pub use public surface; #![warn(clippy::pedantic)]
│   ├── error.rs        # Error enum (thiserror) + Result alias
│   ├── schedule.rs     # Span/interval parsing; Schedule { Relative | Fixed }; next_due/first_due (jiff)
│   ├── schema.rs       # serde boundary: RawTask/RawVendor/RawCompletion/RawPunt (deny_unknown_fields)
│   ├── model.rs        # domain: Task, Vendor, Completion, Punt, Event; From<Raw…> conversions
│   ├── status.rs       # PURE engine: Bucket, TaskStatus, compute_status(events) with punt fold
│   ├── store.rs        # DataDir; load_tasks/vendors/events/all; append_completion/punt; commit
│   ├── service.rs      # Service: list_tasks/due/history/vendors/export/complete/punt/report
│   ├── cli.rs          # clap Cli/Commands; run() dispatch; render text or --json
│   └── main.rs         # fn main() -> ExitCode { lifemaint::cli::main() }
├── tests/
│   └── cli.rs          # black-box tests via assert_cmd over the built binary
└── data/               # example data (final task)
    ├── tasks.yaml
    ├── vendors.yaml
    └── completions.jsonl
```

**Layer boundaries:** `cli` (transport: parse/serialize/exit) → `service` (controller) → `store` (I/O) + `status`/`schedule` (pure engine). `cli`/`main` only in the binary path; engine has zero I/O.

---

## Task 1: Cargo scaffold

**Files:** `Cargo.toml`, `rust-toolchain.toml`, `src/lib.rs`, `src/main.rs`.

- [ ] **Step 1: toolchain + Cargo.toml**

`rust-toolchain.toml`:
```toml
[toolchain]
channel = "stable"
components = ["clippy", "rustfmt"]
```

`Cargo.toml`:
```toml
[package]
name = "lifemaint"
version = "0.1.0"
edition = "2021"
rust-version = "1.80"
description = "Track and complete recurring home/life maintenance tasks."

[[bin]]
name = "lm"
path = "src/main.rs"

[lib]
name = "lifemaint"
path = "src/lib.rs"

[dependencies]
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
jiff = { version = "0.1", features = ["serde"] }
thiserror = "1"

[dev-dependencies]
tempfile = "3"
assert_cmd = "2"
predicates = "3"

[lints.clippy]
pedantic = { level = "warn", priority = -1 }
```

NOTE: pin exact current major versions via `cargo add` at implementation time; the above are floors. If `jiff` has moved to 1.x, use it and adjust APIs. `serde_yaml` is archived-but-stable; acceptable, or swap a maintained drop-in fork (`serde_norway`) keeping the same API — report if you swap.

- [ ] **Step 2: minimal lib + bin**

`src/lib.rs`:
```rust
#![warn(clippy::pedantic)]

pub mod error;
```
(We add modules + `pub use` as tasks land; start with `error` declared in Task 2. For Task 1, lib.rs may be empty of mod decls — just the crate attribute — and a trivial `pub fn version() -> &'static str { env!("CARGO_PKG_VERSION") }` to have something testable. Keep whichever compiles cleanly.)

For Task 1 specifically, make `src/lib.rs`:
```rust
#![warn(clippy::pedantic)]

/// The crate version, from Cargo.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn version_is_nonempty() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
        assert!(!version().is_empty());
    }
}
```

`src/main.rs`:
```rust
fn main() {
    println!("lm {}", lifemaint::version());
}
```
(main.rs becomes the real CLI entry in Task 12; this placeholder keeps the bin compiling.)

- [ ] **Step 3: verify**

Run:
```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
Expected: builds; 1 test passes; clippy clean (pedantic); fmt clean.

- [ ] **Step 4: commit**
```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml src/
git commit -m "chore: scaffold lifemaint Rust crate (cargo, clippy pedantic, rustfmt)"
```

---

## Task 2: Error enum

**Files:** `src/error.rs`; declare `pub mod error;` in `lib.rs`.

- [ ] **Step 1: failing test** — add to `error.rs` a `#[cfg(test)]` test asserting the `Display` output of the variants (write it before the enum so it fails to compile).

- [ ] **Step 2: implement**

`src/error.rs`:
```rust
//! The library error type.

/// Errors produced by the lifemaint library.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A recurrence spec (`every`/`on`) could not be parsed.
    #[error("cannot parse schedule: {0}")]
    Schedule(String),

    /// A data file is missing, unreadable, or malformed.
    #[error("{0}")]
    DataFile(String),

    /// A task references a vendor id that is not defined.
    #[error("task {task_id} references unknown vendor {vendor_id}")]
    UnknownVendor { task_id: String, vendor_id: String },

    /// An I/O failure reading or writing a data file.
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Convenience alias for library results.
pub type Result<T> = std::result::Result<T, Error>;
```

In `lib.rs` add `pub mod error;` and re-export later. Tests assert e.g.
`Error::UnknownVendor{task_id:"clean-drains".into(), vendor_id:"roto".into()}.to_string()` contains both ids; `Error::Schedule("bad".into()).to_string() == "cannot parse schedule: bad"`.

- [ ] **Step 3: verify** (`cargo test`, clippy, fmt) — [ ] **Step 4: commit** `feat: add library error enum`.

---

## Task 3: Relative schedules + interval parsing (jiff)

**Files:** `src/schedule.rs`; `pub mod schedule;` in lib.rs.

**Interface:**
- `pub fn parse_interval(text: &str) -> Result<jiff::Span>` — `weekly|monthly|quarterly|yearly` and `"N days|weeks|months|years"` (N ≥ 1). Anything else → `Error::Schedule`.
- `pub enum Schedule { Relative(jiff::Span), Fixed(Fixed) }` (Fixed added in Task 4).
- For Task 3, implement just `Schedule::Relative` arm behavior via helper methods that will be shared:
  - `Schedule::next_due(&self, anchor: Date) -> Result<Date>` — strictly after anchor (for Relative: `anchor + span`).
  - `Schedule::first_due(&self, on_or_after: Date) -> Result<Date>` — inclusive (for Relative: returns `on_or_after`).

Use `jiff::civil::Date` and `jiff::Span`. Build spans: `Span::new().weeks(1)`, `.months(1)`, `.months(3)` (quarterly), `.years(1)`, and counted forms `.days(n)` etc. Add via `anchor.checked_add(span).map_err(|e| Error::Schedule(e.to_string()))`.

**CRITICAL — verify jiff's month-add clamping:** jiff is expected to clamp day to month end when adding months/years (e.g. `2026-01-31 + 1 month = 2026-02-28`). The implementer MUST confirm this with a test. If jiff instead errors or balances differently, adjust: for relative month/year intervals, clamp manually (see Task 4's `clamp_day`). Report jiff's actual behavior.

- [ ] **Step 1: failing tests** (`#[cfg(test)] mod tests`):
  - table of `(text, anchor, expected)` for next_due: weekly 2026-01-01→2026-01-08; monthly 2026-01-15→2026-02-15; quarterly 2026-01-15→2026-04-15; yearly 2026-01-15→2027-01-15; "6 months" 2026-01-15→2026-07-15; "2 weeks" 2026-01-01→2026-01-15; "10 days"→2026-01-11; "3 years"→2029-01-01.
  - `monthly` next_due of 2026-01-31 == 2026-02-28 (clamp).
  - `first_due` of monthly at 2026-03-01 == 2026-03-01 (returns arg).
  - rejects: "", "fortnightly", "2 fortnights", "monthlyy", "-1 days", "5", "0 days", "0 months".
  Construct dates with `jiff::civil::date(y, m, d)` (the `date!`/`date()` helper) or `Date::new(y,m,d)?`.

- [ ] **Step 2: run → fail. Step 3: implement** `parse_interval`, `Schedule` enum with Relative arm, `next_due`/`first_due` (Relative branch; Fixed branch can `match` with a `Schedule::Fixed(_)` arm returning a placeholder ONLY if needed to compile — but prefer to introduce `Fixed` minimally in Task 4; for Task 3 you may define `Schedule` with only the `Relative` variant and add `Fixed` in Task 4). Parsing: named lookup, else regex-free manual split on whitespace into (count, unit) — parse count as `i64`, match unit; reject count < 1.

- [ ] **Step 4: verify; Step 5: commit** `feat: relative schedules + interval parsing`.

---

## Task 4: Fixed (calendar) schedules + parse_schedule

**Files:** `src/schedule.rs` (extend).

**Interface:**
- `pub enum Fixed { Yearly { month: i8, day: i8 }, Monthly { day: i8 } }`. Add `Schedule::Fixed(Fixed)` variant.
- `Schedule::next_due` / `first_due` Fixed arms:
  - **Yearly{month,day}:** occurrences are `(year, month, clamp_day(year,month,day))`. `next_due(anchor)` = first occurrence strictly after anchor; `first_due(x)` = first occurrence on/after x.
  - **Monthly{day}:** occurrences are `(year, mon, clamp_day(year,mon,day))` for each month; same strictly-after / inclusive search.
- `pub fn parse_schedule(every: &str, on: Option<&OnSpec>) -> Result<Schedule>`:
  - `on` is `None` → `Relative(parse_interval(every))`.
  - `every == "yearly"` + `on = MonthDay("MM-DD")` → parse month/day (1..=12, 1..=31), else `Error::Schedule` → `Fixed::Yearly`.
  - `every == "monthly"` + `on = Day(n)` (1..=31) → `Fixed::Monthly`.
  - other combinations (e.g. weekly + on, yearly + Day, monthly + MonthDay, out-of-range) → `Error::Schedule`.
  `OnSpec` is the boundary enum from `schema.rs` (Task 5). To avoid a cycle, define `OnSpec` in `schema.rs` and have `schedule::parse_schedule` accept it — `schema` depends on `schedule`? No: `model` calls `parse_schedule` with the raw `on`. Put `parse_schedule(every: &str, on: Option<&OnSpec>)` in `schedule.rs` and `use crate::schema::OnSpec;`. (schema.rs defines only data; schedule.rs defines parsing — schema does not depend on schedule, model depends on both. No cycle.)

- `fn clamp_day(year: i16, month: i8, day: i8) -> Result<Date>` — clamp `day` to the month's last day, then `Date::new`. Get last day via jiff: `Date::new(year, month, 1)?.last_of_month().day()` (confirm the method name; alternatively compute days-in-month). Then `Date::new(year, month, day.min(last))`.

Named consts for magic numbers (12 months, 1..=31 day range).

- [ ] **Step 1: failing tests:**
  - yearly {10,15}: next_due(2026-06-01)=2026-10-15; next_due(2026-10-15)=2027-10-15 (strict); next_due(2026-11-01)=2027-10-15; first_due(2026-10-15)=2026-10-15 (inclusive); first_due(2026-10-16)=2027-10-15.
  - yearly {2,29}: next_due(2025-01-01)=2025-02-28 (non-leap clamp); next_due(2027-03-01)=2028-02-29 (leap).
  - monthly {31}: next_due(2026-01-15)=2026-01-31; next_due(2026-01-31)=2026-02-28; next_due(2026-02-28)=2026-03-31.
  - parse_schedule: ("yearly", Some(MonthDay("10-15")))→Fixed::Yearly{10,15}; ("monthly", Some(Day(1)))→Fixed::Monthly{1}; ("6 months", None)→Relative; rejects ("weekly", MonthDay("10-15")), ("yearly", MonthDay("13-01")), ("yearly", MonthDay("10-40")), ("monthly", Day(0)), ("monthly", Day(32)).

- [ ] **Step 2-5:** run→fail; implement (monthly search bounded loop ≤ 13 iterations with a comment on the bound and a non-panicking fallthrough that returns `Error::Schedule` for the unreachable case — do NOT `unreachable!()`); verify; commit `feat: fixed calendar schedules + parse_schedule`.

(`OnSpec` must exist to compile — implement Task 5's `OnSpec` first or define it minimally here and move it to schema.rs in Task 5. Simplest: define `OnSpec` in schema.rs as part of Task 5 and order Task 5 before wiring parse_schedule into model. For Task 4, define `OnSpec` inline in schedule.rs tests or pull Task 5's enum forward. The implementer may implement `OnSpec` in schema.rs as a prerequisite — note it.)

---

## Task 5: serde boundary schema

**Files:** `src/schema.rs`; `pub mod schema;` (or `mod schema;` — internal) in lib.rs.

```rust
use jiff::civil::Date;
use serde::Deserialize;

/// The `on:` field: either a day-of-month int (monthly) or "MM-DD" (yearly).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum OnSpec {
    Day(i8),
    MonthDay(String),
}

fn default_via() -> String { "manual".to_string() }

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawTask {
    pub id: String,
    pub name: String,
    pub every: String,
    #[serde(default)]
    pub on: Option<OnSpec>,
    #[serde(default)]
    pub lead_time: Option<String>,
    #[serde(default)]
    pub prep: Vec<String>,
    #[serde(default)]
    pub vendor: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub start: Option<Date>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawVendor {
    pub id: String,
    pub name: String,
    #[serde(default)] pub phone: Option<String>,
    #[serde(default)] pub email: Option<String>,
    #[serde(default)] pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawCompletion {
    pub id: String,
    pub done: Date,
    #[serde(default = "default_via")] pub via: String,
    #[serde(default)] pub by: Option<String>,
    #[serde(default)] pub cost_cents: Option<i64>,
    #[serde(default)] pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPunt {
    pub id: String,
    pub punt_to: Date,
    #[serde(default = "default_via")] pub via: String,
}
```

- [ ] **Tests** (unit, using `serde_yaml::from_str` / `serde_json::from_str`):
  - RawTask minimal (`id/name/every`) → `on` None, `prep` empty.
  - **`on: "10-15"` YAML KEY/VALUE gotcha:** a task with `every: yearly` and `on: "10-15"` parses with `on == Some(MonthDay("10-15"))` — NOT a bool. (YAML 1.1 treats bare `on` specially; verify serde_yaml maps the struct field correctly. If it mishandles, add `#[serde(rename = "on")]` or handle — report.) Also `on: 1` → `Some(Day(1))`.
  - unknown field rejected (deny_unknown_fields).
  - missing required (`id`) rejected.
  - RawCompletion default via "manual", cost_cents None; `cost_cents` must be integer (a float `12.5` line fails).
  - RawPunt parses; RawVendor minimal.

- [ ] run→fail; implement; verify; commit `feat: serde boundary schema with deny_unknown_fields`.

---

## Task 6: domain model + conversions

**Files:** `src/model.rs`; `pub mod model;`.

```rust
use jiff::{civil::Date, Span};
use crate::error::Result;
use crate::schedule::{parse_interval, parse_schedule, Schedule};
use crate::schema::{RawCompletion, RawPunt, RawTask, RawVendor};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vendor { pub id: String, pub name: String, pub phone: Option<String>, pub email: Option<String>, pub notes: Option<String> }

#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub schedule: Schedule,
    pub lead_time: Option<Span>,
    pub prep: Vec<String>,
    pub vendor: Option<String>,
    pub notes: Option<String>,
    pub start: Option<Date>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion { pub id: String, pub done: Date, pub via: String, pub by: Option<String>, pub cost_cents: Option<i64>, pub note: Option<String> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Punt { pub id: String, pub punt_to: Date, pub via: String }

/// An entry in the append-only event log.
#[derive(Debug, Clone)]
pub enum Event { Completion(Completion), Punt(Punt) }
```

Conversions (use `TryFrom` since schedule parsing is fallible):
```rust
impl Vendor { pub fn from_raw(r: RawVendor) -> Self { ... } }   // infallible
impl Task {
    pub fn from_raw(r: RawTask) -> Result<Self> {
        let lead_time = r.lead_time.as_deref().map(parse_interval).transpose()?;
        let schedule = parse_schedule(&r.every, r.on.as_ref())?;
        Ok(Self { id: r.id, name: r.name, schedule, lead_time, prep: r.prep, vendor: r.vendor, notes: r.notes, start: r.start })
    }
}
impl Completion { pub fn from_raw(r: RawCompletion) -> Self { ... } }
impl Punt { pub fn from_raw(r: RawPunt) -> Self { ... } }
```
(`Schedule`/`Span` aren't `PartialEq`/`Eq`-friendly necessarily; that's why `Task` derives only `Debug, Clone`. Confirm `jiff::Span: Clone` — yes.)

- [ ] **Tests:** Task::from_raw relative (schedule is Relative, lead_time Some, prep vec); fixed (`on:"10-15"`→Fixed); no lead_time→None; bad interval→Err(Error::Schedule); Completion::from_raw carries cost_cents/by/done; Vendor::from_raw; Punt::from_raw. (Use `matches!(task.schedule, Schedule::Relative(_))` etc. — so `Schedule` may need to be matchable; it is, being a pub enum.)

- [ ] run→fail; implement; verify; commit `feat: domain model + raw conversions`.

---

## Task 7: pure status engine + punt fold

**Files:** `src/status.rs`; `pub mod status;`.

```rust
use jiff::civil::Date;
use crate::error::Result;
use crate::model::{Event, Task};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Bucket { Overdue, Due, Prep, Ok }

#[derive(Debug, Clone)]
pub struct TaskStatus<'a> {
    pub task: &'a Task,
    pub last_done: Option<Date>,
    pub next_due: Date,
    pub prep_due: Option<Date>,
    pub bucket: Bucket,
}
```
(Borrowing `&'a Task` avoids cloning tasks into statuses; if lifetimes complicate the service/CLI, store `task: Task` (Clone) instead — implementer's call, report which. Cloning is acceptable at this scale.)

`compute_status(tasks: &'a [Task], events: &[Event], today: Date) -> Result<Vec<TaskStatus<'a>>>`:
- per task, fold events in order via `resolve(task_id, events) -> (Option<Date> last_done, Option<Date> punt_to)`:
  - `Event::Completion(c)` if `c.id == id`: `last_done = Some(max(last_done, c.done))`; `punt_to = None`.
  - `Event::Punt(p)` if `p.id == id`: `punt_to = Some(p.punt_to)`.
- `next_due`: if `Some(p)=punt_to` → p; else if `Some(d)=last_done` → `task.schedule.next_due(d)?`; else → `task.schedule.first_due(task.start.unwrap_or(today))?`.
- `prep_due`: `task.lead_time.map(|s| next_due.checked_sub(s)).transpose()?` (subtract span; map err to Error::Schedule).
- `bucket`: `next_due < today` → Overdue; `== today` → Due; `prep_due.is_some_and(|pd| pd <= today && today < next_due)` → Prep; else Ok.

Exhaustive `match` on `Event` (no `_`). Bucket ladder mutually exclusive.

- [ ] **Tests** (build Tasks via `Task::from_raw(serde_yaml::from_str(...))` helper, Completions/Punts via constructors):
  - never-done no-start → Due, next_due == today.
  - relative overdue (done 2026-01-01 monthly, today 2026-06-06 → next 2026-02-01, Overdue).
  - relative ok (done 2026-06-01 monthly → 2026-07-01, Ok).
  - max done wins across two completions.
  - prep window (yearly on 10-15 lead 2 weeks; today 2026-10-05 → next 2026-10-15, prep_due 2026-10-01, Prep).
  - prep not reached (today 2026-09-01 → Ok).
  - due exactly today.
  - completion for other id ignored.
  - never-done with start (future start → Ok; past start → Overdue).
  - future completion honored as anchor.
  - punt overrides next_due (punt to future → Ok at that date).
  - completion AFTER punt clears it (reverts to schedule).
  - punt AFTER completion wins.
  - interleaved [done, punt, done] → punt cleared, max anchor kept.
  - punt to past → Overdue.
  - punt for other id ignored.

- [ ] run→fail; implement; verify; commit `feat: pure status engine with punt fold`.

---

## Task 8: store — load data files

**Files:** `src/store.rs`; `pub mod store;`.

```rust
pub struct DataDir { pub root: PathBuf }
impl DataDir {
    pub fn new(root: PathBuf) -> Self;
    fn tasks_path(&self) -> PathBuf;       // root/tasks.yaml
    fn vendors_path(&self) -> PathBuf;     // root/vendors.yaml
    fn events_path(&self) -> PathBuf;      // root/completions.jsonl
}
pub fn load_tasks(d: &DataDir) -> Result<Vec<Task>>;
pub fn load_vendors(d: &DataDir) -> Result<Vec<Vendor>>;
pub fn load_events(d: &DataDir) -> Result<Vec<Event>>;        // ordered; both shapes
pub fn load_completions(d: &DataDir) -> Result<Vec<Completion>>; // = events filtered to Completion
pub fn load_all(d: &DataDir) -> Result<(Vec<Task>, Vec<Vendor>, Vec<Completion>)>; // + vendor-ref check
```

- YAML lists: read file (missing → `Ok(vec![])`); `serde_yaml::from_str::<Vec<RawTask>>` (map serde err → `Error::DataFile(format!("tasks.yaml: {e}"))`); convert each via `Task::from_raw` (map err, mention `tasks.yaml`). Same for vendors.
- Event log (`completions.jsonl`): missing → `Ok(vec![])`. For each non-blank line (1-indexed):
  - `let value: serde_json::Value = serde_json::from_str(line).map_err(|e| DataFile(format!("completions.jsonl: line {n}: {e}")))?;`
  - require `value.is_object()` else `DataFile("... line {n}: expected a JSON object")`.
  - if object has key `"punt_to"` → `serde_json::from_value::<RawPunt>(value)` → `Event::Punt(Punt::from_raw(..))`; else if has `"done"` → `RawCompletion` → `Event::Completion`; else → `DataFile("... line {n}: unrecognized event (needs 'done' or 'punt_to')")`. Map `from_value` errors (incl. deny_unknown_fields catching both-keys lines) to `DataFile` with line number.
- `load_all`: load tasks, vendors, completions; `check_vendor_refs` → for any task whose `vendor` is `Some(v)` not in the vendor id set → `Error::UnknownVendor{task_id, vendor_id}`.

**YAML `on:` gotcha:** confirm `serde_yaml` parses `on: "10-15"` correctly into `RawTask.on` (Task 5 covers the unit-level check; here ensure the file-level load of the sample task works). Add a store test loading a tasks.yaml containing an `on:` task and asserting its schedule is Fixed.

- [ ] **Test fixtures:** create a `tests`-local helper or a `#[cfg(test)]` builder that writes sample `tasks.yaml` (groceries weekly; clean-drains yearly vendor roto-rooter; blow-out-sprinklers yearly on "10-15"), `vendors.yaml` (roto-rooter), `completions.jsonl` (groceries done 2026-06-01; clean-drains done 2025-05-01 cost_cents 28500) into a `tempfile::tempdir()`. (Black-box CLI tests in Task 12 will reuse a similar fixture.)
- [ ] **Tests:** load_tasks order/ids; load_vendors phone; load_events returns [Completion, Punt] for a mixed file + order; load_completions excludes punts; load_all lengths + unknown-vendor rejection (`Error::UnknownVendor` with vendor_id); malformed yaml names "tasks.yaml"; missing files → empty; malformed jsonl line names "line 2"; non-object line rejected; both-keys line rejected; `on:` task loads as Fixed.

- [ ] run→fail; implement; verify; commit `feat: store loading (tasks/vendors/event-log) + vendor-ref check`.

---

## Task 9: store — append + git commit

**Files:** `src/store.rs` (extend).

```rust
pub fn append_completion(d: &DataDir, c: &Completion) -> Result<()>;
pub fn append_punt(d: &DataDir, p: &Punt) -> Result<()>;
pub fn commit(d: &DataDir, message: &str) -> Result<bool>;  // false if not a repo / nothing to commit
```
- append: build a serde_json object (omit `None` fields for completion: by/cost_cents/note; punt always id/punt_to/via), serialize one line, create parent dir, open file append, write line + "\n". Map io errors → `Error::Io`. Date as ISO via `date.to_string()` (jiff Date Display is ISO) or serde.
  - Prefer: define a `#[derive(Serialize)] struct OutCompletion<'a>` / `OutPunt<'a>` with `#[serde(skip_serializing_if = "Option::is_none")]` on optionals, and `serde_json::to_string`. Cleaner than hand-building a Value.
- commit: if `!d.root.join(".git").exists()` → `Ok(false)`. Else `std::process::Command::new("git").args(["add","-A"]).current_dir(&d.root)` (check status; non-zero → `Error::DataFile`/`Error::Io`), then `git commit -m <message>` (capture; returncode 0 → Ok(true), nonzero → Ok(false) since "nothing to commit" is normal/non-fatal).

- [ ] **Tests** (tempdir; a `git_data_dir` helper that `git init`s + sets user + seeds a commit):
  - append_completion writes one line; load_completions count +1; fields back.
  - omits None fields (string-check the written line: no "cost_cents"/"note").
  - append creates file when missing.
  - append_punt round-trips via load_events as a Punt.
  - commit persists (git log contains message); commit no-op when nothing changed → false; commit false when not a repo.

- [ ] run→fail; implement; verify; commit `feat: append events + git commit`.

---

## Task 10: service — read operations

**Files:** `src/service.rs`; `pub mod service;`.

```rust
pub struct Service { pub data_dir: DataDir }
impl Service {
    pub fn list_tasks(&self, today: Date, query: Option<&str>, only: Option<&[Bucket]>) -> Result<Vec<OwnedStatus>>;
    pub fn due(&self, today: Date) -> Result<Vec<OwnedStatus>>;   // only Overdue/Due/Prep
    pub fn history(&self, task_id: Option<&str>, since: Option<Date>) -> Result<Vec<Completion>>;
    pub fn vendors(&self) -> Result<Vec<Vendor>>;
    pub fn export(&self, today: Date) -> Result<serde_json::Value>;  // schema_version + joined rows
}
```
NOTE on lifetimes: `compute_status` returns `TaskStatus<'a>` borrowing tasks owned locally in the method → can't return them. Resolve by either (a) `compute_status` borrows and the method maps each `TaskStatus` into an owned `OwnedStatus { task: Task, last_done, next_due, prep_due, bucket }` before returning, or (b) make `TaskStatus` own its `Task` (Clone). Pick the simplest that satisfies the borrow checker; **report the choice.** Recommended: define an owned status struct the service returns (the CLI only needs owned data to render).

- search: `query` matches (case-insensitive `contains`) any of id/name/notes/vendor.
- `only`: filter to buckets in the slice (single-element slice covers the scalar case).
- `due`: `only = &[Overdue, Due, Prep]`.
- status methods load events for the engine; vendor-ref validation via `load_all`; history/vendors use completions/vendors.
- `export`: `{ "schema_version": 1, "generated_for": today, "tasks": [ {id,name,bucket,last_done,next_due,prep_due, vendor: {id,name,phone}|null, completions: [{done,via,by,cost_cents,note}]} ] }` — build with `serde_json::json!` or typed Serialize structs. Completion rows from completions (NOT punts). bucket as lowercase string (Bucket Serialize).

- [ ] **Tests** (tempdir fixture like Task 8): list all 3 ids; search "drain"/"roto" → clean-drains; only=[Overdue] → clean-drains; due excludes Ok (groceries) includes clean-drains; history all (2)/by-task (cost_cents 28500)/since; vendors; export schema_version==1, drains row bucket "overdue", vendor name, next_due "2026-05-01", completions[0].cost_cents 28500.

- [ ] run→fail; implement; verify; commit `feat: service read operations`.

---

## Task 11: service — complete, punt, report

**Files:** `src/service.rs` (extend).

```rust
impl Service {
    pub fn complete(&self, task_id: &str, done: Date, via: &str, by: &str, cost_cents: Option<i64>, note: Option<&str>, do_commit: bool) -> Result<Completion>;
    pub fn punt(&self, task_id: &str, to: Date, via: &str, do_commit: bool) -> Result<Punt>;
    pub fn report(&self, kind: ReportKind, today: Date) -> Result<serde_json::Value>;
}
pub enum ReportKind { SpendByTask, PerYear, OverdueCount }
```
- complete/punt: load_all → if no task with id → `Error::DataFile(format!("unknown task id {task_id}"))` (validate before write); build Completion/Punt; append; if do_commit → commit; return it.
- report:
  - SpendByTask: sum `cost_cents` (only `Some`) per task id → `{"spend_by_task": {id: cents}}`.
  - PerYear: sum per `done.year()` (string key) → `{"per_year": {year: cents}}`.
  - OverdueCount: from `compute_status` over events → `{"overdue": n, "due": n, "prep": n}`.
  Build via `serde_json::json!`/`Map`.

- [ ] **Tests:** complete appends + visible in history (cost_cents); complete unknown id → Err; complete resets next_due (clean-drains Overdue→Ok, next 2027-06-06 after done 2026-06-06); complete `do_commit=false` makes no commit (git rev-list count unchanged); punt unknown → Err; punt defers next_due in list (clean-drains → 2026-09-01, Ok); complete-after-punt reverts to schedule; punt no-commit makes no commit; punt excluded from history; report spend-by-task {clean-drains:28500}; per-year {2025:28500}; overdue-count overdue==1 due==0; prep counted when today=2026-10-05 (overdue 2, prep 1); spend sums multiple same-task completions.

- [ ] run→fail; implement; verify; commit `feat: service complete/punt/report`.

---

## Task 12: clap CLI + main

**Files:** `src/cli.rs`; rewrite `src/main.rs`; `pub mod cli;`. Black-box tests `tests/cli.rs`.

```rust
#[derive(clap::Parser)]
#[command(name = "lm", about = "Track and complete recurring maintenance tasks.")]
struct Cli { #[command(subcommand)] command: Commands }

#[derive(clap::Subcommand)]
enum Commands {
    List { #[arg(short='q', long)] query: Option<String>, #[arg(long)] due: bool, #[arg(long)] overdue: bool, #[arg(long)] today: Option<String>, #[arg(long)] json: bool },
    Due { #[arg(long)] today: Option<String>, #[arg(long)] json: bool },
    Done { id: String, #[arg(long)] on: Option<String>, #[arg(long, default_value="self")] by: String, #[arg(long)] cost: Option<String>, #[arg(long)] note: Option<String>, #[arg(long)] today: Option<String>, #[arg(long, default_value_t=true, action=clap::ArgAction::Set)] commit: bool },
    Punt { id: String, to: String, #[arg(long)] today: Option<String>, #[arg(long, default_value_t=true, action=clap::ArgAction::Set)] commit: bool },
    History { #[arg(long="id")] task_id: Option<String>, #[arg(long)] since: Option<String>, #[arg(long)] json: bool },
    Vendors { #[arg(long)] json: bool },
    Export { #[arg(long)] today: Option<String> },
    Report { kind: ReportKindArg, #[arg(long)] today: Option<String>, #[arg(long)] json: bool },
}
```
- Use `--no-commit` ergonomics: simplest is `#[arg(long)] no_commit: bool` then `do_commit = !no_commit`. (clap's `--commit/--no-commit` bool pairing is fiddly; prefer a `--no-commit` flag.) Update the spec/README usage to `--no-commit`.
- `ReportKindArg`: `#[derive(clap::ValueEnum)] enum { SpendByTask, PerYear, OverdueCount }` with `#[value(rename_all="kebab-case")]` so `spend-by-task` works; map to `service::ReportKind`.
- Data dir: `LM_DATA_DIR` env → else `./data`.
- `today`/dates: parse `Option<String>` → `Date` via `"...".parse()` (jiff `Date: FromStr`, ISO). `today` default = `Date::today()`? jiff: `jiff::Zoned::now().date()` or `Date::from(...)`. Use `jiff::Zoned::now()` then `.date()`, or `Timestamp::now()`. Confirm jiff "now" API; `today` is the only clock use (transport layer — allowed).
- `cost` dollars→cents: parse string as decimal → cents. Without a decimal crate: parse `f64`? NO (money/float forbidden). Parse manually: split on '.', combine `dollars*100 + cents` as i64, validate ≤2 fractional digits, reject negative. Implement a small `parse_cents(&str) -> Result<i64>` (reject negative; reject >2 decimals). Report approach.
- Errors: every command maps `Error` → print to **stderr** (`eprintln!`) and return a nonzero `ExitCode`. `cli::main() -> ExitCode`.
- Rendering: `--json` → `serde_json` to stdout; text → human format. For statuses, a markers table (`!! / -> / .. / "  "`). `export` always JSON. Errors NEVER on stdout (keep stdout machine-parseable).
- `main.rs`:
```rust
use std::process::ExitCode;
fn main() -> ExitCode { lifemaint::cli::main() }
```

- [ ] **Black-box tests** (`tests/cli.rs`, `assert_cmd`): write a sample data dir to `tempdir`, set `LM_DATA_DIR`. Cover: list --json all; list -q drain; due excludes ok; done --cost 42.00 → history cost_cents 4200; done unknown → nonzero, message on stderr; punt defers (list shows next_due 2026-09-01 bucket ok); punt unknown → nonzero; export schema_version 1; report spend-by-task --json; vendors --json; malformed tasks.yaml → nonzero + "tasks.yaml" on stderr; list text mode shows id+bucket; done rejects negative cost.

- [ ] run→fail; implement; verify (`cargo test`, clippy, fmt); commit `feat: clap CLI (list/due/done/punt/history/vendors/export/report)`.

---

## Task 13: public surface, example data, README

**Files:** `src/lib.rs` (pub use), `data/*`, `README.md`.

- [ ] `lib.rs` re-exports the public surface:
```rust
#![warn(clippy::pedantic)]
pub mod error; pub mod schedule; pub mod schema; pub mod model; pub mod status; pub mod store; pub mod service; pub mod cli;
pub use error::{Error, Result};
pub use model::{Completion, Event, Punt, Task, Vendor};
pub use schedule::{Fixed, Schedule};
pub use service::{ReportKind, Service};
pub use status::{compute_status, Bucket, TaskStatus};
pub use store::DataDir;
```
(Keep `version()` or drop it — your call; if dropped, remove its test.)
- [ ] `data/tasks.yaml` (groceries weekly; clean-gutters 6 months lead 2 weeks prep; blow-out-sprinklers yearly on "10-15" vendor green-lawn; clean-drains yearly vendor roto-rooter; mow-lawn weekly + note "punt to spring when the season ends"); `data/vendors.yaml` (roto-rooter, green-lawn); empty `data/completions.jsonl`.
- [ ] Smoke test: `LM_DATA_DIR=$(pwd)/data cargo run --bin lm -- list --today 2026-06-06`; `... due --json`; `... punt mow-lawn 2026-11-01 --today 2026-06-06 --no-commit` then `... list -q mow --json` shows next_due 2026-11-01; then reset `data/completions.jsonl` to empty.
- [ ] `README.md`: setup (`cargo build --release`, binary at `target/release/lm`), `LM_DATA_DIR`, data file formats (relative vs fixed `on:`), all 8 commands incl `lm punt`, seasonal-via-punt note, errors-to-stderr, dev commands (`cargo test`/`clippy`/`fmt`).
- [ ] Final: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` all green; `data/completions.jsonl` empty + tracked. Commit `feat: public surface, example data, README`.

---

## Done — Rust Phase 1 complete

A single static `lm` binary: relative + fixed schedules, lead-time prep, vendors + refs, completions with integer-cents cost, punt/seasonal deferral, search, due/overdue/prep, export, reports — pure engine, exhaustively tested, clippy-pedantic clean. Deploys to the VPS as one file with no runtime; an agent calls `lm due --json` for the future notifier.

## Self-review checklist (run after writing tasks)
- Every spec requirement maps to a task: schedules (T3/T4), lead-time/prep (T7), vendors+refs (T8), completions+cost_cents (T5/T6/T9), punt+seasonal (T5/T6/T7/T9/T11/T12), search + buckets (T7/T10/T12), export (T10), reports (T11), git append-only (T9). ✓
- jiff API assumptions (month clamping, FromStr ISO, now, serde) flagged for verification in T3/T4/T12. ✓
- No `_` catch-alls on domain enums; no unwrap in non-test; serde deny_unknown_fields at boundary; money i64. ✓
