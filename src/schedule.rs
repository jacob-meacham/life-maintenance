//! Scheduling engine: relative-interval schedules and interval parsing.

use jiff::civil::Date;
use jiff::Span;

use crate::error::{Error, Result};

/// Number of months in a quarter, used for the `quarterly` interval.
const MONTHS_PER_QUARTER: i64 = 3;
/// Number of weeks in a `weekly` interval.
const WEEKS_PER_WEEKLY: i64 = 1;
/// Number of months in a `monthly` interval.
const MONTHS_PER_MONTHLY: i64 = 1;
/// Number of years in a `yearly` interval.
const YEARS_PER_YEARLY: i64 = 1;
/// Minimum count accepted for an `N <unit>` interval.
const MIN_COUNT: i64 = 1;
/// Exact number of whitespace-separated tokens in an `N <unit>` interval.
const COUNT_UNIT_TOKENS: usize = 2;

/// Maps a `jiff` error onto the library's schedule error.
fn schedule_err(err: &jiff::Error) -> Error {
    Error::Schedule(err.to_string())
}

/// Parses a recurrence interval into a [`jiff::Span`].
///
/// Accepts the named intervals `"weekly"`, `"monthly"`, `"quarterly"`, and
/// `"yearly"`, as well as the form `"N days|weeks|months|years"` where `N` is
/// an integer greater than or equal to one. Input is trimmed and lowercased
/// before matching.
///
/// # Errors
///
/// Returns [`Error::Schedule`] if the text does not match a known named
/// interval or a valid `"N <unit>"` form (for example, an empty string, an
/// unknown unit, a non-positive count, or a non-numeric count).
pub fn parse_interval(text: &str) -> Result<Span> {
    let normalized = text.trim().to_lowercase();

    match normalized.as_str() {
        "weekly" => {
            return Span::new()
                .try_weeks(WEEKS_PER_WEEKLY)
                .map_err(|e| schedule_err(&e))
        }
        "monthly" => {
            return Span::new()
                .try_months(MONTHS_PER_MONTHLY)
                .map_err(|e| schedule_err(&e));
        }
        "quarterly" => {
            return Span::new()
                .try_months(MONTHS_PER_QUARTER)
                .map_err(|e| schedule_err(&e));
        }
        "yearly" => {
            return Span::new()
                .try_years(YEARS_PER_YEARLY)
                .map_err(|e| schedule_err(&e))
        }
        _ => {}
    }

    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    if tokens.len() != COUNT_UNIT_TOKENS {
        return Err(Error::Schedule(format!("unrecognized interval: {text:?}")));
    }

    let count: i64 = tokens[0]
        .parse()
        .map_err(|_| Error::Schedule(format!("invalid interval count: {:?}", tokens[0])))?;
    if count < MIN_COUNT {
        return Err(Error::Schedule(format!(
            "interval count must be >= {MIN_COUNT}, got {count}"
        )));
    }

    match tokens[1] {
        "day" | "days" => Span::new().try_days(count).map_err(|e| schedule_err(&e)),
        "week" | "weeks" => Span::new().try_weeks(count).map_err(|e| schedule_err(&e)),
        "month" | "months" => Span::new().try_months(count).map_err(|e| schedule_err(&e)),
        "year" | "years" => Span::new().try_years(count).map_err(|e| schedule_err(&e)),
        unit => Err(Error::Schedule(format!("unknown interval unit: {unit:?}"))),
    }
}

/// A recurrence schedule for a maintenance task.
#[derive(Debug, Clone)]
pub enum Schedule {
    /// A schedule defined by a relative interval applied to an anchor date.
    Relative(Span),
}

impl Schedule {
    /// Returns the next due date strictly after `anchor`.
    ///
    /// For a [`Schedule::Relative`] this is `anchor` plus the interval span.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Schedule`] if the underlying date arithmetic fails
    /// (for example, on overflow of the representable date range).
    pub fn next_due(&self, anchor: Date) -> Result<Date> {
        match self {
            Schedule::Relative(span) => anchor.checked_add(*span).map_err(|e| schedule_err(&e)),
        }
    }

    /// Returns the first due date on or after `on_or_after`.
    ///
    /// For a [`Schedule::Relative`] schedule any date satisfies the interval,
    /// so the boundary date itself is returned unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Schedule`] only if a future variant requires fallible
    /// computation; the relative variant is currently infallible.
    pub fn first_due(&self, on_or_after: Date) -> Result<Date> {
        match self {
            Schedule::Relative(_) => Ok(on_or_after),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_interval, Schedule};
    use jiff::civil::date;

    fn relative(text: &str) -> Schedule {
        Schedule::Relative(parse_interval(text).unwrap())
    }

    #[test]
    fn next_due_table() {
        let cases = [
            ("weekly", date(2026, 1, 1), date(2026, 1, 8)),
            ("monthly", date(2026, 1, 15), date(2026, 2, 15)),
            ("quarterly", date(2026, 1, 15), date(2026, 4, 15)),
            ("yearly", date(2026, 1, 15), date(2027, 1, 15)),
            ("6 months", date(2026, 1, 15), date(2026, 7, 15)),
            ("2 weeks", date(2026, 1, 1), date(2026, 1, 15)),
            ("10 days", date(2026, 1, 1), date(2026, 1, 11)),
            ("3 years", date(2026, 1, 1), date(2029, 1, 1)),
        ];

        for (text, anchor, expected) in cases {
            let got = relative(text).next_due(anchor).unwrap();
            assert_eq!(got, expected, "next_due({text}, {anchor})");
        }
    }

    #[test]
    fn monthly_clamps_end_of_month() {
        let got = relative("monthly").next_due(date(2026, 1, 31)).unwrap();
        assert_eq!(got, date(2026, 2, 28));
    }

    #[test]
    fn first_due_is_inclusive() {
        let got = relative("monthly").first_due(date(2026, 3, 1)).unwrap();
        assert_eq!(got, date(2026, 3, 1));
    }

    #[test]
    fn rejects_invalid_intervals() {
        let bad = [
            "",
            "fortnightly",
            "2 fortnights",
            "monthlyy",
            "-1 days",
            "5",
            "0 days",
            "0 months",
        ];
        for input in bad {
            assert!(
                parse_interval(input).is_err(),
                "expected error for {input:?}"
            );
        }
    }

    #[test]
    fn parse_is_case_and_whitespace_insensitive() {
        let anchor = date(2026, 1, 1);

        assert_eq!(
            relative("  Weekly ").next_due(anchor).unwrap(),
            relative("weekly").next_due(anchor).unwrap(),
        );
        assert_eq!(
            relative("3  Months").next_due(anchor).unwrap(),
            relative("3 months").next_due(anchor).unwrap(),
        );
    }
}
