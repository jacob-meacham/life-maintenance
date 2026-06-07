//! The pure status engine: folds events over tasks to compute due dates and
//! urgency buckets. No I/O — `today` is the only time input, injected by the
//! caller, so results are deterministic and testable.

use jiff::civil::Date;

use crate::error::{Error, Result};
use crate::model::{Event, Task};

/// Urgency classification for a task at a given `today`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Bucket {
    /// The next occurrence is in the past.
    Overdue,
    /// The next occurrence is today.
    Due,
    /// Inside the lead-time prep window but not yet due.
    Prep,
    /// Not due and not yet in the prep window.
    Ok,
}

/// The computed status of a single task.
///
/// Owns a clone of the task so the result is self-contained and free of
/// lifetime ties to the input slices — the downstream service and CLI need
/// owned data, and cloning at this scale is cheap.
#[derive(Debug, Clone)]
pub struct TaskStatus {
    /// The task this status describes (owned clone).
    pub task: Task,
    /// The most recent completion date, if any.
    pub last_done: Option<Date>,
    /// The next date the task is due.
    pub next_due: Date,
    /// The date the prep window opens, if the task has a lead time.
    pub prep_due: Option<Date>,
    /// The urgency bucket relative to `today`.
    pub bucket: Bucket,
}

/// Resolve a task's event history into its latest completion and any pending
/// punt, by folding the events in order.
///
/// A completion both advances `last_done` (taking the latest date seen) and
/// clears any pending punt, since completing a task supersedes a deferral.
/// A later punt re-establishes a deferral.
fn resolve(task_id: &str, events: &[Event]) -> (Option<Date>, Option<Date>) {
    let mut last_done: Option<Date> = None;
    let mut punt_to: Option<Date> = None;
    for event in events {
        match event {
            Event::Completion(completion) if completion.id == task_id => {
                last_done = Some(match last_done {
                    Some(existing) => existing.max(completion.done),
                    None => completion.done,
                });
                punt_to = None;
            }
            Event::Punt(punt) if punt.id == task_id => {
                punt_to = Some(punt.punt_to);
            }
            Event::Completion(_) | Event::Punt(_) => {}
        }
    }
    (last_done, punt_to)
}

/// Classify a task into an urgency bucket relative to `today`.
fn bucket_for(next_due: Date, prep_due: Option<Date>, today: Date) -> Bucket {
    if next_due < today {
        Bucket::Overdue
    } else if next_due == today {
        Bucket::Due
    } else if prep_due.is_some_and(|prep| prep <= today && today < next_due) {
        Bucket::Prep
    } else {
        Bucket::Ok
    }
}

/// Compute the status of every task given the event log and the current date.
///
/// For each task the events are folded to find its latest completion and any
/// pending punt, the next due date is derived from the schedule (or the punt),
/// the prep window is computed from the lead time, and an urgency bucket is
/// assigned relative to `today`.
///
/// # Errors
/// Returns [`Error::Schedule`] if a schedule cannot project the next due date
/// or if subtracting a task's lead time from its next due date overflows.
pub fn compute_status(tasks: &[Task], events: &[Event], today: Date) -> Result<Vec<TaskStatus>> {
    let mut statuses = Vec::with_capacity(tasks.len());
    for task in tasks {
        let (last_done, punt_to) = resolve(&task.id, events);

        let next_due = match (punt_to, last_done) {
            (Some(punt), _) => punt,
            (None, Some(done)) => task.schedule.next_due(done)?,
            (None, None) => task.schedule.first_due(task.start.unwrap_or(today))?,
        };

        let prep_due = match task.lead_time {
            Some(span) => Some(
                next_due
                    .checked_sub(span)
                    .map_err(|err| Error::Schedule(err.to_string()))?,
            ),
            None => None,
        };

        let bucket = bucket_for(next_due, prep_due, today);

        statuses.push(TaskStatus {
            task: task.clone(),
            last_done,
            next_due,
            prep_due,
            bucket,
        });
    }
    Ok(statuses)
}

#[cfg(test)]
mod tests {
    use super::{compute_status, Bucket};
    use crate::model::{Completion, Event, Punt, Task};
    use jiff::civil::{date, Date};

    fn task(yaml: &str) -> Task {
        Task::from_raw(serde_yaml::from_str(yaml).unwrap()).unwrap()
    }

    fn parse_date(s: &str) -> Date {
        s.parse().unwrap()
    }

    fn done(id: &str, on: &str) -> Event {
        Event::Completion(Completion {
            id: id.to_string(),
            done: parse_date(on),
            via: "test".to_string(),
            by: None,
            cost_cents: None,
            note: None,
        })
    }

    fn punt(id: &str, to: &str) -> Event {
        Event::Punt(Punt {
            id: id.to_string(),
            punt_to: parse_date(to),
            via: "test".to_string(),
        })
    }

    const MONTHLY: &str = "id: t\nname: T\nevery: monthly\n";

    fn only(statuses: &[super::TaskStatus]) -> &super::TaskStatus {
        assert_eq!(statuses.len(), 1);
        &statuses[0]
    }

    #[test]
    fn never_done_no_start_is_due_today() {
        let tasks = [task(MONTHLY)];
        let statuses = compute_status(&tasks, &[], date(2026, 6, 6)).unwrap();
        let status = only(&statuses);
        assert_eq!(status.next_due, date(2026, 6, 6));
        assert_eq!(status.bucket, Bucket::Due);
        assert_eq!(status.last_done, None);
    }

    #[test]
    fn relative_overdue() {
        let tasks = [task(MONTHLY)];
        let events = [done("t", "2026-01-01")];
        let status = compute_status(&tasks, &events, date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.next_due, date(2026, 2, 1));
        assert_eq!(status.bucket, Bucket::Overdue);
    }

    #[test]
    fn relative_ok() {
        let tasks = [task(MONTHLY)];
        let events = [done("t", "2026-06-01")];
        let status = compute_status(&tasks, &events, date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.next_due, date(2026, 7, 1));
        assert_eq!(status.bucket, Bucket::Ok);
    }

    #[test]
    fn max_done_wins() {
        let tasks = [task(MONTHLY)];
        let events = [done("t", "2026-01-01"), done("t", "2026-06-01")];
        let status = compute_status(&tasks, &events, date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.last_done, Some(date(2026, 6, 1)));
        assert_eq!(status.next_due, date(2026, 7, 1));
    }

    #[test]
    fn prep_window_is_prep() {
        let tasks = [task(
            "id: t\nname: T\nevery: yearly\non: \"10-15\"\nlead_time: 2 weeks\n",
        )];
        let status = compute_status(&tasks, &[], date(2026, 10, 5)).unwrap();
        let status = only(&status);
        assert_eq!(status.next_due, date(2026, 10, 15));
        assert_eq!(status.prep_due, Some(date(2026, 10, 1)));
        assert_eq!(status.bucket, Bucket::Prep);
    }

    #[test]
    fn prep_not_reached_is_ok() {
        let tasks = [task(
            "id: t\nname: T\nevery: yearly\non: \"10-15\"\nlead_time: 2 weeks\n",
        )];
        let status = compute_status(&tasks, &[], date(2026, 9, 1)).unwrap();
        let status = only(&status);
        assert_eq!(status.next_due, date(2026, 10, 15));
        assert_eq!(status.prep_due, Some(date(2026, 10, 1)));
        assert_eq!(status.bucket, Bucket::Ok);
    }

    #[test]
    fn due_exactly() {
        let tasks = [task(MONTHLY)];
        let events = [done("t", "2026-05-06")];
        let status = compute_status(&tasks, &events, date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.next_due, date(2026, 6, 6));
        assert_eq!(status.bucket, Bucket::Due);
    }

    #[test]
    fn completion_for_other_id_ignored() {
        let tasks = [task(MONTHLY)];
        let events = [done("other", "2026-01-01")];
        let status = compute_status(&tasks, &events, date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.last_done, None);
        assert_eq!(status.next_due, date(2026, 6, 6));
        assert_eq!(status.bucket, Bucket::Due);
    }

    #[test]
    fn never_done_future_start_is_ok() {
        let tasks = [task("id: t\nname: T\nevery: monthly\nstart: 2026-12-01\n")];
        let status = compute_status(&tasks, &[], date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.next_due, date(2026, 12, 1));
        assert_eq!(status.bucket, Bucket::Ok);
    }

    #[test]
    fn never_done_past_start_is_overdue() {
        let tasks = [task("id: t\nname: T\nevery: monthly\nstart: 2026-01-01\n")];
        let status = compute_status(&tasks, &[], date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.next_due, date(2026, 1, 1));
        assert_eq!(status.bucket, Bucket::Overdue);
    }

    #[test]
    fn future_completion_honored() {
        let tasks = [task(MONTHLY)];
        let events = [done("t", "2026-08-01")];
        let status = compute_status(&tasks, &events, date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.last_done, Some(date(2026, 8, 1)));
        assert_eq!(status.next_due, date(2026, 9, 1));
        assert_eq!(status.bucket, Bucket::Ok);
    }

    #[test]
    fn punt_overrides_schedule() {
        let tasks = [task(MONTHLY)];
        let events = [punt("t", "2026-09-01")];
        let status = compute_status(&tasks, &events, date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.next_due, date(2026, 9, 1));
        assert_eq!(status.bucket, Bucket::Ok);
    }

    #[test]
    fn completion_after_punt_clears_it() {
        let tasks = [task(MONTHLY)];
        let events = [punt("t", "2026-09-01"), done("t", "2026-06-01")];
        let status = compute_status(&tasks, &events, date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.next_due, date(2026, 7, 1));
        assert_eq!(status.bucket, Bucket::Ok);
    }

    #[test]
    fn punt_after_completion_wins() {
        let tasks = [task(MONTHLY)];
        let events = [done("t", "2026-06-01"), punt("t", "2026-12-01")];
        let status = compute_status(&tasks, &events, date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.next_due, date(2026, 12, 1));
    }

    #[test]
    fn interleaved_completion_after_punt_uses_max_done() {
        let tasks = [task(MONTHLY)];
        let events = [
            done("t", "2026-06-01"),
            punt("t", "2026-09-01"),
            done("t", "2026-05-01"),
        ];
        let status = compute_status(&tasks, &events, date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.last_done, Some(date(2026, 6, 1)));
        assert_eq!(status.next_due, date(2026, 7, 1));
        assert_eq!(status.bucket, Bucket::Ok);
    }

    #[test]
    fn punt_to_past_is_overdue() {
        let tasks = [task(MONTHLY)];
        let events = [punt("t", "2026-05-01")];
        let status = compute_status(&tasks, &events, date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.next_due, date(2026, 5, 1));
        assert_eq!(status.bucket, Bucket::Overdue);
    }

    #[test]
    fn punt_for_other_id_ignored() {
        let tasks = [task(MONTHLY)];
        let events = [punt("other", "2026-09-01")];
        let status = compute_status(&tasks, &events, date(2026, 6, 6)).unwrap();
        let status = only(&status);
        assert_eq!(status.next_due, date(2026, 6, 6));
        assert_eq!(status.bucket, Bucket::Due);
    }
}
