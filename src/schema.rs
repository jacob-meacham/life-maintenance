//! serde boundary types for the data files.

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
