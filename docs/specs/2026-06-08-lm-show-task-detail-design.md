# `lm show <id>` — single-task detail view

## Problem

`lm list` shows only id / bucket / next-due. `lm export` dumps the entire
dataset as JSON. Neither lets you pull up one task and see what you actually
need to *do* it: the prep checklist, the free-form notes, who to call, and
where the task sits in its schedule. This adds a focused "I'm about to do
this — show me everything" view for a single task.

## Command

```
lm show <id> [--today <YYYY-MM-DD>] [--json]
```

- `<id>` — the task id to inspect (exact match, not a fuzzy query).
- `--today` — evaluate status as of a given date (matches every other read
  command).
- `--json` — emit machine-readable output on stdout.

An unknown id is reported as `unknown task id <id>` on stderr with a failure
exit code, consistent with `lm done` / `lm punt`.

## What it shows

- **Status & schedule** — bucket, last done, next due, prep-window open date
  (when a lead time is set), and the recurrence rule.
- **Prep checklist & notes** — the `prep` steps and `notes` field.
- **Vendor contact** — the resolved vendor's name, phone, email, and notes.

Completion history is intentionally *not* duplicated here; it is already
available via `lm history --id <id>`, and the text view points at it.

## Text output

```
clean-drains  Clean drains
  status:     overdue (due 2026-05-01, last done 2025-05-01)
  recurrence: every yearly
  prep opens: 2026-04-17
  prep:
    - clear the area
    - move boxes
  notes:      access panel is behind the dryer
  vendor:     Roto-Rooter  (555) 123-4567  hello@roto.example
              24hr emergency line
  (history: lm history --id clean-drains)
```

Rules:

- The `prep opens:` line appears only when the task has a lead time
  (`prep_due` is present).
- The `prep:`, `notes:`, and `vendor:` sections are omitted entirely when
  empty.
- For a vendor, optional fields (phone, email, notes) that are absent are
  skipped; the vendor's notes render on a continuation line.
- The trailing `(history: ...)` hint is always shown.

## JSON output

A single object (not an array):

```json
{
  "id": "clean-drains",
  "name": "Clean drains",
  "bucket": "overdue",
  "last_done": "2025-05-01",
  "next_due": "2026-05-01",
  "prep_due": null,
  "recurrence": { "every": "yearly", "on": "10-15" },
  "prep": ["clear the area", "move boxes"],
  "notes": "access panel is behind the dryer",
  "vendor": {
    "id": "roto-rooter",
    "name": "Roto-Rooter",
    "phone": "(555) 123-4567",
    "email": "hello@roto.example",
    "notes": "24hr emergency line"
  }
}
```

- `recurrence` is represented structurally as `{ "every", "on" }` rather than
  a prose string. `on` is `null` for relative schedules.
- `last_done`, `prep_due`, `notes`, and `vendor` are `null` when absent;
  `prep` is `[]` when empty.

## Architecture

Follows the existing layering: the service returns owned domain data, the CLI
formats it.

- **`Service::show(today, id) -> Result<TaskDetail>`** (new) in `service.rs`.
  Loads tasks/vendors via `store::load_all` and the full event log via
  `store::load_events`, computes statuses with `compute_status`, finds the
  status whose `task.id == id`, and resolves the task's vendor id against the
  vendor directory. Returns `Error::DataFile("unknown task id <id>")` when no
  task matches — no I/O beyond the loads, mirroring `complete`/`punt`'s
  id-validation error.

- **`TaskDetail`** (new struct in `service.rs`):

  ```rust
  pub struct TaskDetail {
      pub status: TaskStatus,    // carries the full Task + computed status
      pub vendor: Option<Vendor>, // resolved from task.vendor, if any
  }
  ```

  `TaskStatus` already owns a clone of the `Task` (with `prep`, `notes`,
  schedule, etc.), so `TaskDetail` only needs to add the resolved vendor.

- **CLI** in `cli.rs`: a new `Command::Show { id, today, json }` variant and a
  `cmd_show` handler that calls `Service::show` and renders either the text
  layout or the JSON object. New helpers `show_text` / `show_json` alongside
  the existing `print_*` functions.

### Model change

The parsed `Task` currently discards the human-authored cadence — `every: 6
months` is parsed into a `jiff::Span` that renders as ISO-8601 `P6M`, which is
not what we want to show. To render the recurrence rule faithfully, carry the
raw cadence through the domain model:

- Add `every: String` and `on: Option<String>` to `Task` (in `model.rs`),
  populated in `Task::from_raw` from the `RawTask` fields it already receives.
  The `OnSpec` enum (`Day(i8)` | `MonthDay(String)`) is stringified to its
  canonical form (`"15"` or `"10-15"`).

No change to the on-disk data files or to `schema.rs`.

## Testing

Service level (`service.rs` tests, using the existing sample fixtures):

- `show` returns the expected detail for a task **with** a vendor
  (`clean-drains` → vendor resolved to Roto-Rooter) and **without** one
  (`groceries` → `vendor` is `None`).
- `show` for an unknown id returns `Error::DataFile` whose message contains the
  id.
- The returned `status.bucket` / `next_due` match what `list_tasks` computes
  for the same `today`.
- Recurrence raw fields survive: a relative task reports `every` verbatim and
  `on == None`; a fixed task reports its `on` string.

CLI / integration (`tests/`, following the existing `assert_cmd` patterns):

- `lm show clean-drains` text output contains the prep and vendor lines.
- `lm show clean-drains --json` parses and has the documented shape
  (`recurrence.every`, `vendor.name`, `prep` array).
- `lm show nope` exits non-zero and writes the unknown-id message to stderr.

## Documentation

Add `lm show <id>` to the Commands section of `README.md`.
