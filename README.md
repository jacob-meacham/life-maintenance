# lifemaint

Track and complete recurring home/life maintenance tasks.

Git files are the source of truth and the `lm` CLI is the canonical interface to
them. The data lives in plain YAML/JSONL files that you can read, edit, and
version-control directly; `lm` reads and mutates those same files (and commits
the mutations for you). Every read command supports `--json` so the tool can
drive scripts and other programs as easily as a terminal.

For the full design and rationale, see
[`docs/specs/2026-06-06-life-maintenance-tracker-design.md`](docs/specs/2026-06-06-life-maintenance-tracker-design.md).

## Build

```sh
cargo build --release
```

The release binary is produced at `target/release/lm`.

### Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt
```

## Data

This repository ships only the sample dataset under `examples/`; point
`LM_DATA_DIR` at it to try the tool out without configuring anything:

```sh
LM_DATA_DIR=examples lm list
```

Your real maintenance log lives in a separate, user-configured directory. `lm`
resolves which directory to use with the precedence:

1. the `LM_DATA_DIR` environment variable (if set and non-empty), then
2. the `data_dir` key of the TOML config file at
   `~/.config/life-maintenance/config.toml` (honoring `XDG_CONFIG_HOME`; the
   file contains `data_dir = "..."`, set it with `lm config set <path>`), then
3. an error if neither is configured.

Set it once with:

```sh
lm config set ~/life-log
lm config show          # confirm the resolved dir and where it came from
```

The data directory may be its own git repository, or a subdirectory inside an
existing one. `lm done` and `lm punt` commit only the data files they touch, so
either layout works.

The directory holds three files:

- `tasks.yaml` — the recurring tasks.
- `vendors.yaml` — the vendor directory referenced by tasks.
- `completions.jsonl` — an append-only event log.

### Recurrence

A task recurs either *relatively* or on a *fixed* calendar date:

- **Relative** — `every: 6 months` (also `weekly`, `monthly`, `yearly`,
  `2 weeks`, etc.). The next occurrence is computed from the last completion.
- **Fixed** — `every: yearly` plus `on: "10-15"` (month-day). The task is due
  on that calendar date each year regardless of when it was last done.

Example `tasks.yaml` entry:

```yaml
- id: blow-out-sprinklers
  name: Blow out sprinkler lines
  every: yearly
  on: "10-15"
  lead_time: 2 weeks
  vendor: green-lawn
```

Optional fields include `lead_time` (a prep window before the due date),
`prep` (a checklist), `notes`, and `vendor` (an id from `vendors.yaml`).

### The event log

`completions.jsonl` is an append-only log with one JSON event per line. There
are two event kinds:

- A completion (`done`) line, written by `lm done`.
- A punt (`punt_to`) line, written by `lm punt`.

`lm done` and `lm punt` append the corresponding line and git-commit the change
automatically (pass `--no-commit` to skip the commit). Status is derived by
folding this log over the task definitions, so the log is the durable record of
what actually happened.

## Commands

All read commands accept `--today <YYYY-MM-DD>` to evaluate status as of a given
date, and `--json` to emit machine-readable output on stdout.

- `lm list [-q <query>] [--due] [--overdue]` — list tasks and their status.
- `lm due` — list tasks that are overdue, due, or inside their prep window.
- `lm done <id> [--on <date>] [--by <who>] [--cost <n>] [--note <text>]` —
  record a task as completed.
- `lm punt <id> <YYYY-MM-DD>` — defer a task's next occurrence to an explicit
  date.
- `lm history [--id <id>] [--since <date>]` — show completion history.
- `lm vendors` — show the vendor directory.
- `lm export` — print the full denormalized dataset as JSON.
- `lm report <spend-by-task|per-year|overdue-count>` — print a built-in summary
  report.
- `lm config show [--json]` — print the resolved data directory and its source
  (the `LM_DATA_DIR` env, or the config file path).
- `lm config set <path>` — store `<path>` as the `data_dir` (TOML) in
  `~/.config/life-maintenance/config.toml` (honors `XDG_CONFIG_HOME`).

## Seasonal tasks

There is no dedicated "season" type. For tasks that only apply part of the year
(for example, `mow-lawn`), punt them to the start of the next active season when
they go dormant:

```sh
lm punt mow-lawn 2026-11-01
```

This shifts the task's next occurrence forward so it stops surfacing as due
until the date you chose.

## Output conventions

Errors are written to stderr. Machine-readable output (`--json`, `export`) is
written to stdout, so you can pipe it without contamination.
