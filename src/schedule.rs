//! Scheduling engine: relative-interval schedules and interval parsing.

use jiff::civil::Date;
use jiff::Span;

use crate::error::{Error, Result};
use crate::schema::OnSpec;

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
/// Number of months in a year (upper bound for a `MM-DD` month value).
const MONTHS_PER_YEAR: i8 = 12;
/// Largest day-of-month value any month can have.
const MAX_DAY_OF_MONTH: i8 = 31;
/// Exact number of `-`-separated parts in an `MM-DD` `on:` value.
const MONTH_DAY_PARTS: usize = 2;
/// Upper bound on the monthly fixed-schedule search. A valid monthly
/// occurrence is found within 13 month-steps from any reference date, so this
/// bounds the loop while leaving a margin over the worst case (12).
const MONTHLY_SEARCH_BOUND: i16 = 13;

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

/// A calendar-anchored recurrence: the occurrence lands on a fixed
/// day-of-month (and, for yearly, a fixed month) rather than a span past an
/// anchor date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fixed {
    /// Recurs once per year on `month`/`day`. A day past the month's length
    /// (for example `2`/`29` in a non-leap year) is clamped to the last valid
    /// day of that month.
    Yearly {
        /// Month of the occurrence, 1..=12.
        month: i8,
        /// Day of the occurrence, 1..=31 (clamped to the month's length).
        day: i8,
    },
    /// Recurs once per month on `day`, clamped to each month's length.
    Monthly {
        /// Day of the occurrence, 1..=31 (clamped to the month's length).
        day: i8,
    },
}

/// A recurrence schedule for a maintenance task.
#[derive(Debug, Clone)]
pub enum Schedule {
    /// A schedule defined by a relative interval applied to an anchor date.
    Relative(Span),
    /// A schedule anchored to fixed calendar positions.
    Fixed(Fixed),
}

/// Builds a [`Date`] for `year`/`month`/`day`, clamping `day` down to the last
/// valid day of that month (so `2`/`29` becomes `2`/`28` in a non-leap year).
///
/// # Errors
///
/// Returns [`Error::Schedule`] if `year`/`month` do not form a valid date.
fn clamp_day(year: i16, month: i8, day: i8) -> Result<Date> {
    let first = Date::new(year, month, 1).map_err(|e| schedule_err(&e))?;
    let last = first.days_in_month();
    Date::new(year, month, day.min(last)).map_err(|e| schedule_err(&e))
}

/// Finds the first occurrence of a [`Fixed`] schedule relative to `reference`.
///
/// When `inclusive` is true an occurrence falling exactly on `reference` is
/// accepted; otherwise the result is strictly after `reference`.
fn fixed_due(fixed: &Fixed, reference: Date, inclusive: bool) -> Result<Date> {
    let accepts = |candidate: Date| candidate > reference || (candidate == reference && inclusive);

    match fixed {
        Fixed::Yearly { month, day } => {
            let candidate = clamp_day(reference.year(), *month, *day)?;
            if accepts(candidate) {
                Ok(candidate)
            } else {
                clamp_day(reference.year() + 1, *month, *day)
            }
        }
        Fixed::Monthly { day } => {
            let mut year = reference.year();
            let mut month = reference.month();
            for _ in 0..MONTHLY_SEARCH_BOUND {
                let candidate = clamp_day(year, month, *day)?;
                if accepts(candidate) {
                    return Ok(candidate);
                }
                if month == MONTHS_PER_YEAR {
                    month = 1;
                    year += 1;
                } else {
                    month += 1;
                }
            }
            // The search advances one month per iteration; a matching
            // occurrence is guaranteed within 13 steps, so this is
            // unreachable in practice but returned instead of panicking.
            Err(Error::Schedule(format!(
                "no monthly occurrence found within {MONTHLY_SEARCH_BOUND} months of {reference}"
            )))
        }
    }
}

impl Schedule {
    /// Returns the next due date strictly after `anchor`.
    ///
    /// For a [`Schedule::Relative`] this is `anchor` plus the interval span.
    /// For a [`Schedule::Fixed`] it is the first scheduled occurrence strictly
    /// after `anchor`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Schedule`] if the underlying date arithmetic fails
    /// (for example, on overflow of the representable date range).
    pub fn next_due(&self, anchor: Date) -> Result<Date> {
        match self {
            Schedule::Relative(span) => anchor.checked_add(*span).map_err(|e| schedule_err(&e)),
            Schedule::Fixed(fixed) => fixed_due(fixed, anchor, false),
        }
    }

    /// Returns the first due date on or after `on_or_after`.
    ///
    /// For a [`Schedule::Relative`] schedule any date satisfies the interval,
    /// so the boundary date itself is returned unchanged. For a
    /// [`Schedule::Fixed`] schedule it is the first scheduled occurrence on or
    /// after `on_or_after`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Schedule`] if the underlying date arithmetic fails for
    /// a fixed schedule; the relative variant is infallible.
    pub fn first_due(&self, on_or_after: Date) -> Result<Date> {
        match self {
            Schedule::Relative(_) => Ok(on_or_after),
            Schedule::Fixed(fixed) => fixed_due(fixed, on_or_after, true),
        }
    }
}

/// Parses an `every`/`on` recurrence spec into a [`Schedule`].
///
/// With `on` absent the `every` text is parsed as a relative interval (see
/// [`parse_interval`]). With `on` present only two combinations are valid:
/// `every = "yearly"` with an `"MM-DD"` [`OnSpec::MonthDay`], and
/// `every = "monthly"` with a 1..=31 [`OnSpec::Day`].
///
/// # Errors
///
/// Returns [`Error::Schedule`] if `on` is combined with an incompatible
/// `every`, if a `"MM-DD"` value is malformed or out of range, or if a
/// monthly day is outside 1..=31.
pub fn parse_schedule(every: &str, on: Option<&OnSpec>) -> Result<Schedule> {
    let Some(on) = on else {
        return Ok(Schedule::Relative(parse_interval(every)?));
    };

    let normalized = every.trim().to_lowercase();
    match (normalized.as_str(), on) {
        ("yearly", OnSpec::MonthDay(s)) => {
            let (month, day) = parse_month_day(s)?;
            Ok(Schedule::Fixed(Fixed::Yearly { month, day }))
        }
        ("monthly", OnSpec::Day(d)) => {
            if (1..=MAX_DAY_OF_MONTH).contains(d) {
                Ok(Schedule::Fixed(Fixed::Monthly { day: *d }))
            } else {
                Err(Error::Schedule(format!(
                    "monthly day must be 1..={MAX_DAY_OF_MONTH}, got {d}"
                )))
            }
        }
        (other, _) => Err(Error::Schedule(format!(
            "`every: {other:?}` is not compatible with the given `on:` value"
        ))),
    }
}

/// Parses an `"MM-DD"` string into a validated `(month, day)` pair.
///
/// # Errors
///
/// Returns [`Error::Schedule`] if the value is not exactly two `-`-separated
/// numeric parts, or if month/day fall outside 1..=12 / 1..=31.
fn parse_month_day(s: &str) -> Result<(i8, i8)> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != MONTH_DAY_PARTS {
        return Err(Error::Schedule(format!("expected `MM-DD`, got {s:?}")));
    }
    let month: i8 = parts[0]
        .parse()
        .map_err(|_| Error::Schedule(format!("invalid month in {s:?}")))?;
    let day: i8 = parts[1]
        .parse()
        .map_err(|_| Error::Schedule(format!("invalid day in {s:?}")))?;
    if !(1..=MONTHS_PER_YEAR).contains(&month) {
        return Err(Error::Schedule(format!(
            "month must be 1..={MONTHS_PER_YEAR}, got {month}"
        )));
    }
    if !(1..=MAX_DAY_OF_MONTH).contains(&day) {
        return Err(Error::Schedule(format!(
            "day must be 1..={MAX_DAY_OF_MONTH}, got {day}"
        )));
    }
    Ok((month, day))
}

#[cfg(test)]
mod tests {
    use super::{parse_interval, parse_schedule, Fixed, Schedule};
    use crate::schema::OnSpec;
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
    fn fixed_yearly_next_and_first_due() {
        let s = Schedule::Fixed(Fixed::Yearly { month: 10, day: 15 });

        assert_eq!(s.next_due(date(2026, 6, 1)).unwrap(), date(2026, 10, 15));
        // Strictly after: on the day itself rolls to next year.
        assert_eq!(s.next_due(date(2026, 10, 15)).unwrap(), date(2027, 10, 15));
        assert_eq!(s.next_due(date(2026, 11, 1)).unwrap(), date(2027, 10, 15));
        // Inclusive first_due.
        assert_eq!(s.first_due(date(2026, 10, 15)).unwrap(), date(2026, 10, 15));
        assert_eq!(s.first_due(date(2026, 10, 16)).unwrap(), date(2027, 10, 15));
    }

    #[test]
    fn fixed_yearly_clamps_feb_29() {
        let s = Schedule::Fixed(Fixed::Yearly { month: 2, day: 29 });

        // Non-leap year clamps to Feb 28.
        assert_eq!(s.next_due(date(2025, 1, 1)).unwrap(), date(2025, 2, 28));
        // Leap year keeps Feb 29.
        assert_eq!(s.next_due(date(2027, 3, 1)).unwrap(), date(2028, 2, 29));
    }

    #[test]
    fn fixed_monthly_clamps_day_31() {
        let s = Schedule::Fixed(Fixed::Monthly { day: 31 });

        assert_eq!(s.next_due(date(2026, 1, 15)).unwrap(), date(2026, 1, 31));
        // Strictly after Jan 31 -> Feb, clamped to 28.
        assert_eq!(s.next_due(date(2026, 1, 31)).unwrap(), date(2026, 2, 28));
        // Strictly after Feb 28 -> Mar 31.
        assert_eq!(s.next_due(date(2026, 2, 28)).unwrap(), date(2026, 3, 31));
    }

    #[test]
    fn parse_schedule_builds_fixed_and_relative() {
        match parse_schedule("yearly", Some(&OnSpec::MonthDay("10-15".into()))).unwrap() {
            Schedule::Fixed(Fixed::Yearly { month, day }) => {
                assert_eq!((month, day), (10, 15));
            }
            other => panic!("expected yearly fixed, got {other:?}"),
        }

        match parse_schedule("monthly", Some(&OnSpec::Day(1))).unwrap() {
            Schedule::Fixed(Fixed::Monthly { day }) => assert_eq!(day, 1),
            other => panic!("expected monthly fixed, got {other:?}"),
        }

        match parse_schedule("6 months", None).unwrap() {
            Schedule::Relative(_) => {}
            Schedule::Fixed(fixed) => panic!("expected relative, got fixed {fixed:?}"),
        }
    }

    #[test]
    fn parse_schedule_rejects_bad_combinations() {
        let bad = [
            ("weekly", OnSpec::MonthDay("10-15".into())),
            ("yearly", OnSpec::MonthDay("13-01".into())),
            ("yearly", OnSpec::MonthDay("10-40".into())),
            ("monthly", OnSpec::Day(0)),
            ("monthly", OnSpec::Day(32)),
        ];
        for (every, on) in bad {
            assert!(
                parse_schedule(every, Some(&on)).is_err(),
                "expected error for ({every:?}, {on:?})"
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
