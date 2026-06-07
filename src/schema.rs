//! serde boundary types for the data files.

use jiff::civil::Date;
use serde::Deserialize;

/// The `on:` field of a task: a day-of-month int (monthly) or "MM-DD" (yearly).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum OnSpec {
    /// Day of month, 1..=31 (monthly fixed schedule).
    Day(i8),
    /// "MM-DD" (yearly fixed schedule).
    MonthDay(String),
}

/// Default `via` value for completion/punt events when the field is absent.
fn default_via() -> String {
    "manual".to_string()
}

/// Raw task definition as it appears in `tasks.yaml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawTask {
    /// Stable task identifier.
    pub id: String,
    /// Human-readable task name.
    pub name: String,
    /// Recurrence cadence (e.g. "weekly", "yearly").
    pub every: String,
    /// Optional fixed anchor: day-of-month or "MM-DD".
    #[serde(default)]
    pub on: Option<OnSpec>,
    /// Optional lead time before the due date (e.g. "2w").
    #[serde(default)]
    pub lead_time: Option<String>,
    /// Optional preparation steps.
    #[serde(default)]
    pub prep: Vec<String>,
    /// Optional vendor id reference.
    #[serde(default)]
    pub vendor: Option<String>,
    /// Optional free-form notes.
    #[serde(default)]
    pub notes: Option<String>,
    /// Optional schedule start date.
    #[serde(default)]
    pub start: Option<Date>,
}

/// Raw vendor entry as it appears in `vendors.yaml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawVendor {
    /// Stable vendor identifier.
    pub id: String,
    /// Human-readable vendor name.
    pub name: String,
    /// Optional phone number.
    #[serde(default)]
    pub phone: Option<String>,
    /// Optional email address.
    #[serde(default)]
    pub email: Option<String>,
    /// Optional free-form notes.
    #[serde(default)]
    pub notes: Option<String>,
}

/// Raw completion event (a line in `completions.jsonl` with a `done` field).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawCompletion {
    /// Id of the task that was completed.
    pub id: String,
    /// Date the task was completed.
    pub done: Date,
    /// How the completion was recorded (defaults to "manual").
    #[serde(default = "default_via")]
    pub via: String,
    /// Optional person who completed the task.
    #[serde(default)]
    pub by: Option<String>,
    /// Optional cost in integer cents.
    #[serde(default)]
    pub cost_cents: Option<i64>,
    /// Optional free-form note.
    #[serde(default)]
    pub note: Option<String>,
}

/// Raw punt event (a line in `completions.jsonl` with a `punt_to` field).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPunt {
    /// Id of the task that was punted.
    pub id: String,
    /// New due date the task was punted to.
    pub punt_to: Date,
    /// How the punt was recorded (defaults to "manual").
    #[serde(default = "default_via")]
    pub via: String,
}

#[cfg(test)]
mod tests {
    use super::{OnSpec, RawCompletion, RawPunt, RawTask, RawVendor};
    use jiff::civil::date;

    #[test]
    fn raw_task_minimal_defaults_on_none_and_prep_empty() {
        let yaml = "id: x\nname: X\nevery: weekly\n";
        let task: RawTask = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(task.id, "x");
        assert_eq!(task.name, "X");
        assert_eq!(task.every, "weekly");
        assert_eq!(task.on, None);
        assert!(task.prep.is_empty());
        assert_eq!(task.vendor, None);
        assert_eq!(task.start, None);
    }

    #[test]
    fn raw_task_on_month_day_string_is_not_a_bool() {
        let yaml = "id: s\nname: S\nevery: yearly\non: \"10-15\"\n";
        let task: RawTask = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(task.on, Some(OnSpec::MonthDay("10-15".into())));
    }

    #[test]
    fn raw_task_on_integer_is_day() {
        let yaml = "id: s\nname: S\nevery: monthly\non: 1\n";
        let task: RawTask = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(task.on, Some(OnSpec::Day(1)));
    }

    #[test]
    fn raw_task_rejects_unknown_field() {
        let yaml = "id: x\nname: X\nevery: weekly\nfrequency: oops\n";
        let result = serde_yaml::from_str::<RawTask>(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn raw_task_missing_required_id_is_error() {
        let yaml = "name: X\nevery: weekly\n";
        let result = serde_yaml::from_str::<RawTask>(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn raw_completion_minimal_defaults() {
        let json = r#"{"id":"g","done":"2026-06-05"}"#;
        let completion: RawCompletion = serde_json::from_str(json).unwrap();
        assert_eq!(completion.id, "g");
        assert_eq!(completion.done, date(2026, 6, 5));
        assert_eq!(completion.via, "manual");
        assert_eq!(completion.cost_cents, None);
        assert_eq!(completion.by, None);
        assert_eq!(completion.note, None);
    }

    #[test]
    fn raw_completion_rejects_fractional_cost_cents() {
        let json = r#"{"id":"g","done":"2026-06-05","cost_cents":12.5}"#;
        let result = serde_json::from_str::<RawCompletion>(json);
        assert!(result.is_err());
    }

    #[test]
    fn raw_punt_parses_date_and_defaults_via() {
        let json = r#"{"id":"mow","punt_to":"2027-04-01"}"#;
        let punt: RawPunt = serde_json::from_str(json).unwrap();
        assert_eq!(punt.id, "mow");
        assert_eq!(punt.punt_to, date(2027, 4, 1));
        assert_eq!(punt.via, "manual");
    }

    #[test]
    fn raw_vendor_minimal_defaults_phone_none() {
        let yaml = "id: acme\nname: Acme\n";
        let vendor: RawVendor = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(vendor.id, "acme");
        assert_eq!(vendor.name, "Acme");
        assert_eq!(vendor.phone, None);
        assert_eq!(vendor.email, None);
        assert_eq!(vendor.notes, None);
    }
}
