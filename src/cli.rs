//! The command-line transport layer.
//!
//! This module is responsible only for parsing arguments, calling into the
//! [`crate::service::Service`], and serializing results to stdout. All
//! diagnostics and errors go to stderr; stdout is the machine-parseable
//! channel for `--json` consumers.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use jiff::civil::Date;
use serde_json::{json, Value};

use crate::error::Error;
use crate::model::{Completion, Vendor};
use crate::service::{ReportKind, Service};
use crate::status::{Bucket, TaskStatus};
use crate::store::DataDir;

/// Track and complete recurring home/life maintenance tasks.
#[derive(Debug, Parser)]
#[command(name = "lm", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// The set of top-level subcommands.
#[derive(Debug, Subcommand)]
enum Command {
    /// List tasks and their status.
    List {
        /// Filter to tasks matching this query (id, name, notes, vendor).
        #[arg(short, long)]
        query: Option<String>,
        /// Only show tasks that are overdue or due.
        #[arg(long)]
        due: bool,
        /// Only show tasks that are overdue.
        #[arg(long)]
        overdue: bool,
        /// Treat this date as today (YYYY-MM-DD).
        #[arg(long)]
        today: Option<String>,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// List tasks that are overdue, due, or in their prep window.
    Due {
        /// Treat this date as today (YYYY-MM-DD).
        #[arg(long)]
        today: Option<String>,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Record a task as completed.
    Done {
        /// The task id to complete.
        id: String,
        /// The completion date (YYYY-MM-DD); defaults to today.
        #[arg(long)]
        on: Option<String>,
        /// Who performed the task.
        #[arg(long, default_value = "self")]
        by: String,
        /// The cost in dollars (e.g. 42.00).
        #[arg(long)]
        cost: Option<String>,
        /// A free-form note.
        #[arg(long)]
        note: Option<String>,
        /// Treat this date as today (YYYY-MM-DD).
        #[arg(long)]
        today: Option<String>,
        /// Do not commit the change to git.
        #[arg(long)]
        no_commit: bool,
    },
    /// Defer a task's next occurrence to an explicit date.
    Punt {
        /// The task id to punt.
        id: String,
        /// The date to defer to (YYYY-MM-DD).
        to_date: String,
        /// Treat this date as today (YYYY-MM-DD).
        #[arg(long)]
        today: Option<String>,
        /// Do not commit the change to git.
        #[arg(long)]
        no_commit: bool,
    },
    /// Show completion history.
    History {
        /// Filter to a single task id.
        #[arg(long)]
        id: Option<String>,
        /// Only show completions on or after this date (YYYY-MM-DD).
        #[arg(long)]
        since: Option<String>,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Show the vendor directory.
    Vendors {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Print the full denormalized dataset as JSON.
    Export {
        /// Treat this date as today (YYYY-MM-DD).
        #[arg(long)]
        today: Option<String>,
    },
    /// Print a built-in summary report.
    Report {
        /// Which report to produce.
        kind: ReportKindArg,
        /// Treat this date as today (YYYY-MM-DD).
        #[arg(long)]
        today: Option<String>,
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}

/// The CLI form of [`ReportKind`], rendered in kebab-case.
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum ReportKindArg {
    /// Total recorded spend per task id.
    SpendByTask,
    /// Total recorded spend per completion year.
    PerYear,
    /// Counts of tasks by urgency bucket.
    OverdueCount,
}

impl From<ReportKindArg> for ReportKind {
    fn from(arg: ReportKindArg) -> Self {
        match arg {
            ReportKindArg::SpendByTask => ReportKind::SpendByTask,
            ReportKindArg::PerYear => ReportKind::PerYear,
            ReportKindArg::OverdueCount => ReportKind::OverdueCount,
        }
    }
}

/// A transport-layer error: either a bad CLI argument or a library failure.
#[derive(Debug)]
enum CliError {
    /// An argument could not be parsed (e.g. a malformed date or cost).
    Arg(String),
    /// An error propagated from the library.
    Lib(Error),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Arg(msg) => write!(f, "{msg}"),
            CliError::Lib(err) => write!(f, "{err}"),
        }
    }
}

impl From<Error> for CliError {
    fn from(err: Error) -> Self {
        CliError::Lib(err)
    }
}

/// Parse, dispatch, and map the result to a process exit code.
///
/// This is the binary entry point. On error it prints the message to stderr
/// and returns [`ExitCode::FAILURE`]; on success it returns
/// [`ExitCode::SUCCESS`].
#[must_use]
pub fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

/// Resolve the data directory from `LM_DATA_DIR`, defaulting to `./data`.
fn data_dir() -> DataDir {
    let path = std::env::var("LM_DATA_DIR").map_or_else(|_| PathBuf::from("data"), PathBuf::from);
    DataDir::new(path)
}

/// The current civil date in the system timezone.
///
/// This is the only clock read in the program and lives in the transport
/// layer, where non-determinism is acceptable. `Zoned::now()` is infallible.
fn current_today() -> Date {
    jiff::Zoned::now().date()
}

/// Resolve a `--today` override, defaulting to the current date.
fn resolve_today(today: Option<&str>) -> Result<Date, CliError> {
    match today {
        Some(s) => parse_date(s),
        None => Ok(current_today()),
    }
}

/// Parse a `YYYY-MM-DD` date, reporting a clear error on failure.
fn parse_date(s: &str) -> Result<Date, CliError> {
    s.parse::<Date>()
        .map_err(|_| CliError::Arg(format!("invalid date {s:?}: expected YYYY-MM-DD")))
}

/// Parse a dollar amount into integer cents without using floating point.
///
/// Accepts an optional integer part and up to two fractional digits, e.g.
/// `42`, `42.5`, `42.50`. Negative amounts are rejected. Returns a
/// human-readable error message on any malformed input.
fn parse_cents(s: &str) -> Result<i64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("cost must not be empty".to_string());
    }
    if s.starts_with('-') {
        return Err("cost must not be negative".to_string());
    }
    let mut parts = s.split('.');
    let dollars_str = parts.next().unwrap_or("");
    let frac_str = parts.next().unwrap_or("0");
    if parts.next().is_some() {
        return Err(format!("invalid cost {s:?}: too many decimal points"));
    }
    let dollars: i64 = dollars_str
        .parse()
        .map_err(|_| format!("invalid cost {s:?}: bad dollar amount"))?;
    if frac_str.is_empty() || frac_str.len() > 2 || !frac_str.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("invalid cost {s:?}: cents must be 1 or 2 digits"));
    }
    let frac_padded = format!("{frac_str:0<2}");
    let frac: i64 = frac_padded
        .parse()
        .map_err(|_| format!("invalid cost {s:?}: bad cents"))?;
    Ok(dollars * 100 + frac)
}

/// Dispatch a parsed command to its handler.
fn run(command: &Command) -> Result<(), CliError> {
    let service = Service::new(data_dir());
    match command {
        Command::List {
            query,
            due,
            overdue,
            today,
            json,
        } => cmd_list(
            &service,
            query.as_deref(),
            *due,
            *overdue,
            today.as_deref(),
            *json,
        ),
        Command::Due { today, json } => cmd_due(&service, today.as_deref(), *json),
        Command::Done {
            id,
            on,
            by,
            cost,
            note,
            today,
            no_commit,
        } => cmd_done(
            &service,
            id,
            on.as_deref(),
            by,
            cost.as_deref(),
            note.as_deref(),
            today.as_deref(),
            *no_commit,
        ),
        Command::Punt {
            id,
            to_date,
            today,
            no_commit,
        } => cmd_punt(&service, id, to_date, today.as_deref(), *no_commit),
        Command::History { id, since, json } => {
            cmd_history(&service, id.as_deref(), since.as_deref(), *json)
        }
        Command::Vendors { json } => cmd_vendors(&service, *json),
        Command::Export { today } => cmd_export(&service, today.as_deref()),
        Command::Report { kind, today, json } => {
            cmd_report(&service, *kind, today.as_deref(), *json)
        }
    }
}

/// Handle `list`.
fn cmd_list(
    service: &Service,
    query: Option<&str>,
    due: bool,
    overdue: bool,
    today: Option<&str>,
    json: bool,
) -> Result<(), CliError> {
    let today = resolve_today(today)?;
    let only: Option<&[Bucket]> = if overdue {
        Some(&[Bucket::Overdue])
    } else if due {
        Some(&[Bucket::Overdue, Bucket::Due])
    } else {
        None
    };
    let statuses = service.list_tasks(today, query, only)?;
    print_statuses(&statuses, json);
    Ok(())
}

/// Handle `due`.
fn cmd_due(service: &Service, today: Option<&str>, json: bool) -> Result<(), CliError> {
    let today = resolve_today(today)?;
    let statuses = service.due(today)?;
    print_statuses(&statuses, json);
    Ok(())
}

/// Handle `done`.
#[allow(clippy::too_many_arguments)] // each argument is a distinct completion field
fn cmd_done(
    service: &Service,
    id: &str,
    on: Option<&str>,
    by: &str,
    cost: Option<&str>,
    note: Option<&str>,
    today: Option<&str>,
    no_commit: bool,
) -> Result<(), CliError> {
    let today = resolve_today(today)?;
    let done = match on {
        Some(s) => parse_date(s)?,
        None => today,
    };
    let cost_cents = match cost {
        Some(s) => Some(parse_cents(s).map_err(CliError::Arg)?),
        None => None,
    };
    let completion = service.complete(id, done, "cli", by, cost_cents, note, !no_commit)?;
    println!("done {} on {}", completion.id, completion.done);
    Ok(())
}

/// Handle `punt`.
fn cmd_punt(
    service: &Service,
    id: &str,
    to_date: &str,
    today: Option<&str>,
    no_commit: bool,
) -> Result<(), CliError> {
    // `today` is parsed for validation/consistency even though punt does not
    // need it, so a bad `--today` is reported rather than silently ignored.
    let _ = resolve_today(today)?;
    let to = parse_date(to_date)?;
    let punt = service.punt(id, to, "cli", !no_commit)?;
    println!("punted {} to {}", punt.id, punt.punt_to);
    Ok(())
}

/// Handle `history`.
fn cmd_history(
    service: &Service,
    id: Option<&str>,
    since: Option<&str>,
    json: bool,
) -> Result<(), CliError> {
    let since = match since {
        Some(s) => Some(parse_date(s)?),
        None => None,
    };
    let rows = service.history(id, since)?;
    print_history(&rows, json);
    Ok(())
}

/// Handle `vendors`.
fn cmd_vendors(service: &Service, json: bool) -> Result<(), CliError> {
    let vendors = service.vendors()?;
    print_vendors(&vendors, json);
    Ok(())
}

/// Handle `export`.
fn cmd_export(service: &Service, today: Option<&str>) -> Result<(), CliError> {
    let today = resolve_today(today)?;
    let payload = service.export(today)?;
    print_pretty(&payload);
    Ok(())
}

/// Handle `report`.
fn cmd_report(
    service: &Service,
    kind: ReportKindArg,
    today: Option<&str>,
    json: bool,
) -> Result<(), CliError> {
    let today = resolve_today(today)?;
    let payload = service.report(kind.into(), today)?;
    if json {
        println!("{payload}");
    } else {
        print_report_text(&payload);
    }
    Ok(())
}

/// The text marker for a bucket.
fn bucket_marker(bucket: Bucket) -> &'static str {
    match bucket {
        Bucket::Overdue => "!!",
        Bucket::Due => "->",
        Bucket::Prep => "..",
        Bucket::Ok => "  ",
    }
}

/// Render a task status as a JSON object.
fn status_json(status: &TaskStatus) -> Value {
    json!({
        "id": status.task.id,
        "name": status.task.name,
        "bucket": status.bucket,
        "last_done": status.last_done.map(|d| d.to_string()),
        "next_due": status.next_due.to_string(),
        "prep_due": status.prep_due.map(|d| d.to_string()),
    })
}

/// Print task statuses, as JSON array or one text line each.
fn print_statuses(statuses: &[TaskStatus], json: bool) {
    if json {
        let rows: Vec<Value> = statuses.iter().map(status_json).collect();
        println!("{}", Value::Array(rows));
    } else {
        for status in statuses {
            let bucket = serde_json::to_value(status.bucket)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            println!(
                "{} {}  {}  {}",
                bucket_marker(status.bucket),
                status.task.id,
                bucket,
                status.next_due,
            );
        }
    }
}

/// Render a completion as a JSON object.
fn completion_json(c: &Completion) -> Value {
    json!({
        "id": c.id,
        "done": c.done.to_string(),
        "via": c.via,
        "by": c.by,
        "cost_cents": c.cost_cents,
        "note": c.note,
    })
}

/// Print completion history, as JSON array or one text line each.
fn print_history(rows: &[Completion], json: bool) {
    if json {
        let rows: Vec<Value> = rows.iter().map(completion_json).collect();
        println!("{}", Value::Array(rows));
    } else {
        for c in rows {
            let by = c.by.as_deref().unwrap_or("");
            let note = c.note.as_deref().unwrap_or("");
            println!("{}  {}  by {by}  {note}", c.done, c.id);
        }
    }
}

/// Print the vendor directory, as JSON array or one text line each.
fn print_vendors(vendors: &[Vendor], json: bool) {
    if json {
        let rows: Vec<Value> = vendors
            .iter()
            .map(|v| {
                json!({
                    "id": v.id,
                    "name": v.name,
                    "phone": v.phone,
                    "email": v.email,
                    "notes": v.notes,
                })
            })
            .collect();
        println!("{}", Value::Array(rows));
    } else {
        for v in vendors {
            let phone = v.phone.as_deref().unwrap_or("");
            println!("{}  {}  {phone}", v.id, v.name);
        }
    }
}

/// Print a JSON value pretty-printed to stdout.
fn print_pretty(value: &Value) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        // Falls back to the compact form; a Value always serializes, so this
        // branch is effectively unreachable but avoids an unwrap.
        Err(_) => println!("{value}"),
    }
}

/// Print a report Value as `key: value` text lines.
fn print_report_text(value: &Value) {
    if let Some(map) = value.as_object() {
        for (key, val) in map {
            if let Some(inner) = val.as_object() {
                for (k, v) in inner {
                    println!("{key}.{k}: {v}");
                }
            } else {
                println!("{key}: {val}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_cents;

    #[test]
    fn parse_cents_table() {
        let ok_cases = [
            ("42.00", 4200),
            ("42", 4200),
            ("42.5", 4250),
            ("42.50", 4250),
            ("0", 0),
            ("0.01", 1),
            (" 42.00 ", 4200),
        ];
        for (input, expected) in ok_cases {
            assert_eq!(parse_cents(input), Ok(expected), "input: {input:?}");
        }

        let err_cases = [
            ("-5.00", "negative"),
            ("abc", "bad dollar amount"),
            ("1.234", "1 or 2 digits"),
            ("", "must not be empty"),
            ("1.2.3", "too many decimal points"),
            ("4.2a", "1 or 2 digits"),
        ];
        for (input, expected_substring) in err_cases {
            let err = parse_cents(input).expect_err(&format!("expected Err for {input:?}"));
            assert!(
                err.contains(expected_substring),
                "input {input:?}: expected message to contain {expected_substring:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn parse_cents_rejects_negative_with_message() {
        let err = parse_cents("-5.00").unwrap_err();
        assert!(err.contains("negative"), "msg: {err}");
    }
}
