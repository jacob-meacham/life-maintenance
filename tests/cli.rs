//! Black-box integration tests for the `lm` binary.
//!
//! Each test builds an isolated data directory in a tempdir and points the
//! binary at it via `LM_DATA_DIR`. Commands that write use `--no-commit` so
//! they do not require a git repository.

use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
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
  phone: \"555-1234\"
";

const SAMPLE_EVENTS: &str = "\
{\"id\":\"groceries\",\"done\":\"2026-06-01\"}
{\"id\":\"clean-drains\",\"done\":\"2025-05-01\",\"cost_cents\":28500}
";

/// Write the standard sample dataset into a fresh tempdir.
fn sample_dir() -> TempDir {
    let dir = tempdir().unwrap();
    write_dataset(dir.path(), SAMPLE_TASKS);
    dir
}

/// Write a dataset with the given `tasks.yaml` contents.
fn write_dataset(root: &Path, tasks: &str) {
    std::fs::write(root.join("tasks.yaml"), tasks).unwrap();
    std::fs::write(root.join("vendors.yaml"), SAMPLE_VENDORS).unwrap();
    std::fs::write(root.join("completions.jsonl"), SAMPLE_EVENTS).unwrap();
}

/// A command bound to the binary and the given data dir.
fn lm(root: &Path) -> Command {
    let mut cmd = Command::cargo_bin("lm").unwrap();
    cmd.env("LM_DATA_DIR", root);
    cmd
}

/// Run a command expected to succeed and return its stdout as a JSON value.
fn stdout_json(cmd: &mut Command) -> Value {
    let output = cmd.assert().success().get_output().stdout.clone();
    serde_json::from_slice(&output).expect("stdout is valid JSON")
}

/// The set of task ids in a JSON array of status objects.
fn ids(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn list_json_returns_all_three_ids() {
    let dir = sample_dir();
    let value = stdout_json(lm(dir.path()).args(["list", "--today", "2026-06-06", "--json"]));
    let got = ids(&value);
    assert_eq!(got.len(), 3, "ids: {got:?}");
    for id in ["groceries", "clean-drains", "blow-out-sprinklers"] {
        assert!(got.iter().any(|g| g == id), "missing {id} in {got:?}");
    }
}

#[test]
fn list_query_filters_to_drains() {
    let dir = sample_dir();
    let value = stdout_json(lm(dir.path()).args([
        "list",
        "-q",
        "drain",
        "--today",
        "2026-06-06",
        "--json",
    ]));
    assert_eq!(ids(&value), ["clean-drains"]);
}

#[test]
fn due_includes_drains_excludes_groceries() {
    let dir = sample_dir();
    let value = stdout_json(lm(dir.path()).args(["due", "--today", "2026-06-06", "--json"]));
    let got = ids(&value);
    assert!(got.iter().any(|g| g == "clean-drains"), "got: {got:?}");
    assert!(!got.iter().any(|g| g == "groceries"), "got: {got:?}");
}

#[test]
fn done_then_history_records_cost() {
    let dir = sample_dir();
    lm(dir.path())
        .args([
            "done",
            "groceries",
            "--cost",
            "42.00",
            "--today",
            "2026-06-06",
            "--no-commit",
        ])
        .assert()
        .success();
    let value = stdout_json(lm(dir.path()).args(["history", "--id", "groceries", "--json"]));
    let rows = value.as_array().unwrap();
    let last = rows.last().expect("at least one groceries completion");
    assert_eq!(last["cost_cents"], 4200);
}

#[test]
fn done_unknown_id_fails_with_stderr() {
    let dir = sample_dir();
    lm(dir.path())
        .args(["done", "nope", "--today", "2026-06-06", "--no-commit"])
        .assert()
        .failure()
        .stderr(predicate::str::is_empty().not());
}

#[test]
fn punt_defers_next_due_and_marks_ok() {
    let dir = sample_dir();
    lm(dir.path())
        .args(["punt", "clean-drains", "2026-09-01", "--no-commit"])
        .assert()
        .success();
    let value = stdout_json(lm(dir.path()).args([
        "list",
        "-q",
        "clean-drains",
        "--today",
        "2026-06-06",
        "--json",
    ]));
    let rows = value.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["next_due"], "2026-09-01");
    assert_eq!(rows[0]["bucket"], "ok");
}

#[test]
fn punt_unknown_id_fails() {
    let dir = sample_dir();
    lm(dir.path())
        .args(["punt", "nope", "2026-09-01", "--no-commit"])
        .assert()
        .failure();
}

#[test]
fn export_has_schema_version() {
    let dir = sample_dir();
    let value = stdout_json(lm(dir.path()).args(["export", "--today", "2026-06-06"]));
    assert_eq!(value["schema_version"], 1);
}

#[test]
fn report_spend_by_task_sums_cost() {
    let dir = sample_dir();
    let value = stdout_json(lm(dir.path()).args([
        "report",
        "spend-by-task",
        "--today",
        "2026-06-06",
        "--json",
    ]));
    assert_eq!(value["spend_by_task"]["clean-drains"], 28500);
}

#[test]
fn vendors_json_lists_roto_rooter_first() {
    let dir = sample_dir();
    let value = stdout_json(lm(dir.path()).args(["vendors", "--json"]));
    let rows = value.as_array().unwrap();
    assert_eq!(rows[0]["id"], "roto-rooter");
}

#[test]
fn malformed_tasks_yaml_fails_with_stderr_mentioning_file() {
    let dir = tempdir().unwrap();
    write_dataset(
        dir.path(),
        "- id: groceries\n  name: Groceries\n  every: fortnightly\n",
    );
    lm(dir.path())
        .args(["list", "--today", "2026-06-06"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("tasks.yaml"));
}

#[test]
fn list_text_mode_shows_drains_and_overdue() {
    let dir = sample_dir();
    lm(dir.path())
        .args(["list", "--today", "2026-06-06"])
        .assert()
        .success()
        .stdout(predicate::str::contains("clean-drains").and(predicate::str::contains("overdue")));
}

#[test]
fn done_negative_cost_fails() {
    let dir = sample_dir();
    lm(dir.path())
        .args([
            "done",
            "groceries",
            "--cost",
            "-5.00",
            "--today",
            "2026-06-06",
            "--no-commit",
        ])
        .assert()
        .failure();
}

/// A bare command with neither `LM_DATA_DIR` nor an ambient `XDG_CONFIG_HOME`,
/// using `home` as an isolated `HOME`. This makes the resolved config base
/// `<home>/.config` and the config file `<home>/.config/life-maintenance/
/// config.toml`, so the developer's (and CI's) real config is never touched.
fn lm_unconfigured(home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("lm").unwrap();
    cmd.env_remove("LM_DATA_DIR")
        .env_remove("XDG_CONFIG_HOME")
        .env("HOME", home);
    cmd
}

#[test]
fn list_unconfigured_fails_with_hint() {
    let home = tempdir().unwrap();
    lm_unconfigured(home.path())
        .args(["list", "--today", "2026-06-06"])
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("not configured").and(predicate::str::contains("LM_DATA_DIR")),
        );
}

#[test]
fn config_set_then_show_json_reports_config_source() {
    let home = tempdir().unwrap();
    let data = tempdir().unwrap();
    lm_unconfigured(home.path())
        .args(["config", "set", data.path().to_str().unwrap()])
        .assert()
        .success();
    let config_file = home.path().join(".config/life-maintenance/config.toml");
    assert!(
        config_file.exists(),
        "expected config at {}",
        config_file.display()
    );
    let value = stdout_json(lm_unconfigured(home.path()).args(["config", "show", "--json"]));
    assert_eq!(value["configured"], true);
    assert_eq!(value["data_dir"], data.path().to_str().unwrap());
    assert_eq!(value["source"], "config");
    assert_eq!(
        value["config_path"],
        config_file.display().to_string().as_str()
    );
}

#[test]
fn config_show_json_reports_env_source() {
    let home = tempdir().unwrap();
    let data = tempdir().unwrap();
    let value = stdout_json(
        Command::cargo_bin("lm")
            .unwrap()
            .env("HOME", home.path())
            .env("LM_DATA_DIR", data.path())
            .args(["config", "show", "--json"]),
    );
    assert_eq!(value["configured"], true);
    assert_eq!(value["source"], "env");
    assert_eq!(value["data_dir"], data.path().to_str().unwrap());
}

#[test]
fn config_show_json_unconfigured_reports_uniform_shape() {
    let home = tempdir().unwrap();
    let value = stdout_json(lm_unconfigured(home.path()).args(["config", "show", "--json"]));
    assert_eq!(value["configured"], false);
    assert!(value["data_dir"].is_null());
    assert!(value["source"].is_null());
    let config_path = value["config_path"].as_str().expect("config_path string");
    let expected = home
        .path()
        .join(".config/life-maintenance/config.toml")
        .display()
        .to_string();
    assert_eq!(config_path, expected);
}

#[test]
fn config_show_text_unconfigured_succeeds_and_shows_path() {
    let home = tempdir().unwrap();
    lm_unconfigured(home.path())
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("not configured").and(predicate::str::contains("config.toml")),
        );
}

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
        .stdout(predicate::str::contains("prep opens:"))
        .stdout(predicate::str::contains("clear the area"))
        .stdout(predicate::str::contains("access panel is behind the dryer"))
        .stdout(predicate::str::contains("Roto-Rooter"))
        .stdout(predicate::str::contains("lm history --id clean-drains"));
}

#[test]
fn show_json_has_documented_shape() {
    let dir = sample_dir();
    let value = stdout_json(lm(dir.path()).args([
        "show",
        "clean-drains",
        "--today",
        "2026-06-06",
        "--json",
    ]));
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

#[test]
fn config_set_data_dir_loads_tasks() {
    let home = tempdir().unwrap();
    let data = tempdir().unwrap();
    write_dataset(data.path(), SAMPLE_TASKS);
    lm_unconfigured(home.path())
        .args(["config", "set", data.path().to_str().unwrap()])
        .assert()
        .success();
    let value =
        stdout_json(lm_unconfigured(home.path()).args(["list", "--today", "2026-06-06", "--json"]));
    let got = ids(&value);
    assert_eq!(got.len(), 3, "ids: {got:?}");
    for id in ["groceries", "clean-drains", "blow-out-sprinklers"] {
        assert!(got.iter().any(|g| g == id), "missing {id} in {got:?}");
    }
}

#[test]
fn completions_emit_bash_registration_script() {
    let dir = sample_dir();
    lm(dir.path())
        .env("COMPLETE", "bash")
        .assert()
        .success()
        .stdout(predicate::str::contains("complete").and(predicate::str::contains("lm")));
}

/// Run a dynamic-completion request the way a shell's completer does and
/// return the candidate strings. `words` are the full command-line words
/// (including the `lm` binary at index 0); `cword` is the index being
/// completed.
fn completions(root: &Path, cword: usize, words: &[&str]) -> Vec<String> {
    let mut cmd = lm(root);
    cmd.env("COMPLETE", "bash")
        .env("_CLAP_COMPLETE_INDEX", cword.to_string())
        .arg("--")
        .args(words);
    let out = cmd.assert().success().get_output().stdout.clone();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn completions_offer_task_ids_for_show() {
    let dir = sample_dir();
    let got = completions(dir.path(), 2, &["lm", "show", ""]);
    for id in ["groceries", "clean-drains", "blow-out-sprinklers"] {
        assert!(got.iter().any(|g| g == id), "missing {id} in {got:?}");
    }
}

#[test]
fn completions_offer_task_ids_for_done() {
    let dir = sample_dir();
    let got = completions(dir.path(), 2, &["lm", "done", ""]);
    assert!(got.iter().any(|g| g == "groceries"), "got: {got:?}");
}

#[test]
fn completions_offer_task_ids_for_punt() {
    let dir = sample_dir();
    let got = completions(dir.path(), 2, &["lm", "punt", ""]);
    assert!(got.iter().any(|g| g == "groceries"), "got: {got:?}");
}

#[test]
fn completions_offer_task_ids_for_history_id_flag() {
    let dir = sample_dir();
    let got = completions(dir.path(), 3, &["lm", "history", "--id", ""]);
    assert!(got.iter().any(|g| g == "groceries"), "got: {got:?}");
}

#[test]
fn completions_offer_vendor_names_for_done_by() {
    let dir = sample_dir();
    let got = completions(dir.path(), 4, &["lm", "done", "groceries", "--by", ""]);
    assert!(got.iter().any(|g| g == "roto-rooter"), "got: {got:?}");
}
