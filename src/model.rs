//! Internal domain types, converted from the serde boundary types.

use jiff::civil::Date;
use jiff::Span;

use crate::error::Result;
use crate::schedule::{parse_interval, parse_schedule, Schedule};
use crate::schema::{RawCompletion, RawPunt, RawTask, RawVendor};

/// A service vendor/contact referenced by tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vendor {
    pub id: String,
    pub name: String,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub notes: Option<String>,
}

impl Vendor {
    /// Build a `Vendor` from its raw boundary form.
    #[must_use]
    pub fn from_raw(raw: RawVendor) -> Self {
        Self {
            id: raw.id,
            name: raw.name,
            phone: raw.phone,
            email: raw.email,
            notes: raw.notes,
        }
    }
}

/// A maintenance task definition.
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

impl Task {
    /// Build a `Task` from its raw form, parsing the schedule and lead time.
    ///
    /// # Errors
    /// Returns [`crate::error::Error::Schedule`] if `every`/`on`/`lead_time`
    /// cannot be parsed.
    pub fn from_raw(raw: RawTask) -> Result<Self> {
        let lead_time = match raw.lead_time.as_deref() {
            Some(s) => Some(parse_interval(s)?),
            None => None,
        };
        let schedule = parse_schedule(&raw.every, raw.on.as_ref())?;
        Ok(Self {
            id: raw.id,
            name: raw.name,
            schedule,
            lead_time,
            prep: raw.prep,
            vendor: raw.vendor,
            notes: raw.notes,
            start: raw.start,
        })
    }
}

/// A completion event: a task was done on a date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub id: String,
    pub done: Date,
    pub via: String,
    pub by: Option<String>,
    pub cost_cents: Option<i64>,
    pub note: Option<String>,
}

impl Completion {
    /// Build a `Completion` from its raw form.
    #[must_use]
    pub fn from_raw(raw: RawCompletion) -> Self {
        Self {
            id: raw.id,
            done: raw.done,
            via: raw.via,
            by: raw.by,
            cost_cents: raw.cost_cents,
            note: raw.note,
        }
    }
}

/// A punt event: defer a task's next occurrence to an explicit date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Punt {
    pub id: String,
    pub punt_to: Date,
    pub via: String,
}

impl Punt {
    /// Build a `Punt` from its raw form.
    #[must_use]
    pub fn from_raw(raw: RawPunt) -> Self {
        Self {
            id: raw.id,
            punt_to: raw.punt_to,
            via: raw.via,
        }
    }
}

/// An entry in the append-only event log.
#[derive(Debug, Clone)]
pub enum Event {
    /// A completion event.
    Completion(Completion),
    /// A punt event.
    Punt(Punt),
}

#[cfg(test)]
mod tests {
    use super::{Completion, Punt, Task, Vendor};
    use crate::schedule::Schedule;
    use crate::schema::{RawCompletion, RawPunt, RawTask, RawVendor};
    use jiff::civil::date;

    #[test]
    fn task_from_raw_relative_with_lead_time() {
        let raw: RawTask = serde_yaml::from_str(
            "id: g\nname: G\nevery: 6 months\nlead_time: 2 weeks\nprep:\n  - call vendor\n  - clear area\n",
        )
        .unwrap();
        let task = Task::from_raw(raw).unwrap();
        assert!(matches!(task.schedule, Schedule::Relative(_)));
        assert!(task.lead_time.is_some());
        assert_eq!(
            task.prep,
            vec!["call vendor".to_string(), "clear area".to_string()]
        );
    }

    #[test]
    fn task_from_raw_fixed() {
        let raw: RawTask =
            serde_yaml::from_str("id: s\nname: S\nevery: yearly\non: \"10-15\"\n").unwrap();
        let task = Task::from_raw(raw).unwrap();
        assert!(matches!(task.schedule, Schedule::Fixed(_)));
    }

    #[test]
    fn task_from_raw_no_lead_time_is_none() {
        let raw: RawTask = serde_yaml::from_str("id: x\nname: X\nevery: weekly\n").unwrap();
        let task = Task::from_raw(raw).unwrap();
        assert!(task.lead_time.is_none());
    }

    #[test]
    fn task_from_raw_bad_interval_is_err() {
        let raw: RawTask = serde_yaml::from_str("id: x\nname: X\nevery: fortnightly\n").unwrap();
        assert!(Task::from_raw(raw).is_err());
    }

    #[test]
    fn completion_from_raw_carries_fields() {
        let json = r#"{"id":"g","done":"2026-06-05","by":"Acme","cost_cents":28500}"#;
        let raw: RawCompletion = serde_json::from_str(json).unwrap();
        let completion = Completion::from_raw(raw);
        assert_eq!(completion.done, date(2026, 6, 5));
        assert_eq!(completion.cost_cents, Some(28500));
        assert_eq!(completion.by, Some("Acme".to_string()));
    }

    #[test]
    fn vendor_from_raw_carries_fields() {
        let raw: RawVendor = serde_yaml::from_str("id: acme\nname: Acme\n").unwrap();
        let vendor = Vendor::from_raw(raw);
        assert_eq!(vendor.name, "Acme");
        assert_eq!(vendor.phone, None);
    }

    #[test]
    fn punt_from_raw_carries_fields() {
        let json = r#"{"id":"mow","punt_to":"2027-04-01","via":"cli"}"#;
        let raw: RawPunt = serde_json::from_str(json).unwrap();
        let punt = Punt::from_raw(raw);
        assert_eq!(punt.punt_to, date(2027, 4, 1));
        assert_eq!(punt.via, "cli");
    }
}
