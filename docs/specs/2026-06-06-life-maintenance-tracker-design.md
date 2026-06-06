# Life Maintenance Tracker — Design

**Date:** 2026-06-06
**Status:** Approved design, pending spec review

## Problem

Track recurring home/life maintenance tasks across many cadences (weekly,
monthly, quarterly, yearly, and multi-year — e.g. groceries, clean drains,
clean gutters, replace roof). Previous attempts (spreadsheets, apps) haven't
stuck. The two things that make it stick:

1. **Low-friction completion** — marking something done must be trivial.
2. **Proactive notification** — the system pings a channel the user already
   lives in, rather than relying on memory.

Additional needs:
- Schedule **lead-time prep reminders** for bigger jobs (e.g. "two weeks
  before cleaning drains, make sure you have the tools / know who to call").
- Record **who services each task and their contact info** (vendor directory),
  and **who did it / what it cost** per completion — this is a maintenance
  *record*, not just a checklist.
- Produce **reports** from the data.

## Goals / Non-goals

**Goals**
- One-tap / one-command completion across multiple interfaces.
- Git files as the single source of truth; everything else is a view.
- A great CLI: every piece of data reachable as stable JSON, so dashboards and
  external agents are just consumers.
- Capture vendor/contact info and per-completion cost for reporting.

**Non-goals (v1)**
- Multi-user / sharing.
- A hosted SaaS. This runs on the user's own always-on VPS.
- Building around any specific external agent platform (e.g. "OpenClaw"). The
  CLI is the universal integration surface; such tools plug in later as clients.

## Architecture

A **pure computation engine** with thin interfaces around it. Git files are the
database; CLI, chat bot, cron sender, and (later) web are views/controllers.

```
         git repo (source of truth)
         ├── tasks.yaml          ← task definitions (hand-edit, or CLI/bot append)
         ├── completions.jsonl   ← append-only log of what was done & when
         ├── vendors.yaml        ← service/contact directory
         └── config.yaml         ← telegram token, chat id, schedule, timezone
                    │
                    ▼
            core engine (pure Python)
   "given defs + completion log + today → what's due / overdue / prep-due"
                    │
        ┌───────────┼────────────┬─────────────┐
       CLI      Telegram bot   cron sender    web (later)
   (lm done…)  (Done buttons) (daily digest)  (dashboard)
```

### Key design decisions

1. **Append-only completion log, separate from definitions.** Definitions live
   in `tasks.yaml`; each "done" appends one line to `completions.jsonl`.
   Multiple writers (hand edits, CLI, bot) never contend on the same lines, git
   merges cleanly, and full history is free. A double-tap just adds a redundant
   line; the engine only reads the latest. Self-healing.

2. **Two recurrence types, one downstream pipeline.** Tasks are either
   **relative** (anchored to last completion — a 6-month task done 3 weeks late
   is next due 6 months from when it was *actually* done) or **fixed** (anchored
   to the calendar — sprinkler blowout is due mid-October regardless of when it
   was last done). Both compute a single `next_due`; all bucketing, prep, and
   lead-time logic is shared. Relative matches consumables/wear; fixed matches
   seasonal jobs.

3. **Storage is git files, not a database.** YAML defs + append-only JSONL log.
   The data is small (dozens of tasks, a few thousand log lines over a decade),
   so search and reports are instant in-memory. This keeps defs hand-editable,
   gives free history via `git log`, and makes the CLI/agent integration
   trivial. If query needs ever outgrow this, the escape hatch is a *derived,
   rebuildable* SQLite read-index — never the system of record.

4. **CLI is the canonical interface; everything supports `--json`.** This is the
   single hook any future consumer (dashboards, external agents) needs. We don't
   design around any specific external tool, but nothing blocks one.

## Data model

### `tasks.yaml`

A task has exactly one recurrence type:

- **Relative** — `every: <interval>`, anchored to last completion.
  Interval forms: `weekly | monthly | quarterly | yearly | "N days/weeks/months/years"`.
- **Fixed** — `every: yearly|monthly` **plus** `on:`, anchored to the calendar.
  - `every: yearly` + `on: "10-15"` (MM-DD) → due Oct 15 each year.
  - `every: monthly` + `on: 1` (day-of-month) → due the 1st each month.
  - An `on` day past a month's length is **clamped** to the last day (e.g.
    `on: 31` → Feb 28/29); `on: "02-29"` resolves to Feb 28 in non-leap years.

```yaml
# Relative: due 6 months after it was last actually done
- id: clean-gutters
  name: Clean out gutters
  every: 6 months
  lead_time: 2 weeks       # optional — when to send the prep nudge
  prep:                    # optional — shown in the prep nudge
    - Check ladder is sound
    - Buy gutter scoop if missing
  vendor: null             # optional — reference to vendors.yaml id; omit/null for DIY
  notes: Back side clogs worst.   # optional
  start: 2026-01-15        # optional seed anchor for a never-done relative task

# Fixed: due mid-October every year, no matter when last done
- id: blow-out-sprinklers
  name: Blow out sprinkler lines
  every: yearly
  on: "10-15"
  lead_time: 2 weeks       # nudge in early Oct to book it before a freeze
  vendor: green-lawn

- id: clean-drains
  name: Clean out drains
  every: yearly
  lead_time: 2 weeks
  vendor: roto-rooter

- id: groceries
  name: Grocery shopping
  every: weekly
```

### `vendors.yaml`

```yaml
- id: roto-rooter
  name: Roto-Rooter
  phone: "555-123-4567"
  email: dispatch@example.com
  notes: Ask for Dave; flat rate for main line.
```

Vendors are referenced by id so contact info is updated once and every task +
past record points at current info.

### `completions.jsonl` (append-only)

```json
{"id": "clean-drains", "done": "2026-05-01", "via": "telegram", "by": "roto-rooter", "cost_cents": 28500, "note": "Roots in main line"}
{"id": "groceries", "done": "2026-06-05", "via": "cli", "by": "self"}
```

- `id`, `done` (ISO date), `via` (cli|telegram|web|manual) — required.
- `by` (`self` or a vendor id), `cost_cents` (integer cents — money is never a
  float), `note` (string) — optional. The CLI accepts `--cost` in dollars and
  converts to cents.

### Status computation (the engine's one job)

For each task: `last_done` = latest matching log line, else the `start` date (or
none).

`next_due` is computed by a small **schedule abstraction** with two
implementations, so downstream logic is identical:
- **Relative:** `next_due = last_done + every`. Never done & no `start` → due now.
- **Fixed:** `next_due` = the first calendar occurrence of `on` strictly after
  `last_done` (after completion it advances to the next occurrence). Never done →
  the first occurrence on/after `start` (or on/after today if no `start`).

Then, shared for both:
- `prep_due = next_due − lead_time` (only if `lead_time` set)
- Bucket: **overdue** (`next_due < today`), **due** (`today ≥ next_due`),
  **prep** (`today ≥ prep_due` and not yet due), **upcoming/ok** otherwise.

Everything else is presentation.

## Components

```
lifemaint/
  core/
    models.py      # Task, Vendor, Completion dataclasses + YAML/JSONL parsing
    schedule.py    # Relative + Fixed schedules → next_due(last_done, today); interval/date parsing
    status.py      # the engine: defs + log + today → bucketed status (PURE, no I/O)
    store.py       # read tasks/vendors, append completions, git commit
  cli.py           # `lm` commands, all with --json
  telegram/        # Phase 2 example only — NOT built in current scope
    sender.py      # build digest, send message w/ inline "✅ Done" buttons (cron)
    bot.py         # long-running; handles button taps & replies → store.complete()
  web/             # Phase 3
config.yaml        # token, chat_id, timezone, quiet hours, digest schedule
tasks.yaml
vendors.yaml
completions.jsonl
```

### Data flow

- **Status (current scope):** `lm due` → `status.py` (due/overdue/prep) →
  printed as text or `--json`. A future notifier (Phase 2) consumes the same
  `lm due --json` and surfaces it on a channel.
- **Complete:** `lm done <id> [--by --cost --note]` (or hand-edit the log) →
  `store.py` appends a line + git-commits → next status recompute reflects it.
  `--by` defaults to `self`; `--cost`/`--note` optional, useful for vendor jobs.

## CLI surface

Principle: **every bit of data is reachable as stable JSON.**

```
lm list [-q TERM] [--due] [--overdue] [--json]   # tasks + last done + next due; -q searches id/name/notes/vendor
lm due [--json]                # current due/overdue/prep
lm done <id> [--by V] [--cost N] [--note "..."] [--date D] [--via S]
lm add                         # interactive add (or hand-edit tasks.yaml)
lm history [--id X] [--since D] [--json]   # completion records w/ by/cost/note
lm vendors [--json]            # contacts directory
lm export [--json]             # full denormalized dataset (tasks⋈completions⋈vendors)
lm report <kind> [--json]      # built-in summaries (see below)
```

- `export` emits the whole joined dataset with a documented, stable schema — the
  feed for any external dashboard. Dashboards never parse the raw files.
- `lm report` ships a few summaries: **spend-by-task**, **per-year totals**,
  **overdue-count**. Text by default, `--json` for machine use.

## Error handling

- Malformed `tasks.yaml`/`vendors.yaml` → CLI reports the offending entry and
  exits non-zero, rather than silently skipping.
- A `vendor` reference with no matching id → validation error surfaced by CLI.
- Bot catches per-task send failures so one bad task doesn't kill the digest.
- Git commit failures (e.g. nothing to commit) are non-fatal.

## Testing

- `status.py` is pure (inject `today`): recurrence math, overdue/prep bucketing,
  lead-time edges exhaustively unit-tested with zero mocking.
- `schedule.py` tested across all cadence forms — relative intervals AND fixed
  schedules (year boundaries, Feb 29 / month-end `on` values, advance-after-done).
- `store.py` tested against temp dirs (append + commit).
- Telegram layer kept thin so most logic is covered without network.
- TDD throughout.

## Phasing

Each phase is independently useful.

- **Phase 1 — Core + CLI (current scope).** Engine (relative + fixed schedules),
  data files (tasks/vendors/completions), full `lm` command incl.
  `list`(+search)/`due`/`done`/`history`/`vendors`/`export`/`report`, full tests.
  Usable by hand immediately; also the entire foundation any notifier or external
  client will use.
- **Phase 2 — Notification layer (deferred, mechanism TBD).** How the digest
  gets surfaced is intentionally undecided until the CLI exists. Leading
  candidate: a scheduled **agent** that calls `lm due --json`, decides what's
  worth surfacing, and messages the user — rather than a hand-built Telegram
  bot. Whatever the mechanism, it is a pure consumer of the CLI; completions it
  records use `via` to mark their source (e.g. `via: agent`). The CLI must
  therefore expose everything the notifier needs (it does: `due --json`).
- **Phase 3 — Web dashboard (deferred).** Read/complete overview page over the
  CLI/export. Lowest urgency.

Because the notification mechanism is deferred, the `telegram/` module in the
component layout is **not built in the current scope** — it documents one
possible Phase 2 shape, not a commitment.
