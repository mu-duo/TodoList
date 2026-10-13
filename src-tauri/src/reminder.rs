//! Pure reminder scheduling primitives. This module deliberately does not own a
//! timer, perform I/O, or invoke an operating-system notification API.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::{DateTime, Duration, Local, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// A task snapshot sufficient to make one deterministic reminder plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderTask {
    pub list_file: String,
    pub task_id: String,
    pub title: String,
    pub status: String,
    pub due: Option<String>,
    pub reminder_offset_days: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderEventType {
    Advance,
    Due,
}

/// Stable identity independent of a list's current file name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ReminderEventKey {
    pub task_id: String,
    pub canonical_due: String,
    pub event_type: ReminderEventType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderEvent {
    pub key: ReminderEventKey,
    pub task_title: String,
    pub scheduled_local: NaiveDateTime,
    pub scheduled_at: DateTime<Utc>,
}

/// Serde-friendly ledger state. Persisting this value is the adapter's
/// responsibility; `claim` only returns an updated in-memory copy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReminderLedger {
    /// Last uniquely observed task status (`true` means open). Missing on old
    /// ledgers, so the first observed snapshot keeps normal catch-up semantics.
    #[serde(default)]
    pub last_status: BTreeMap<String, bool>,
    #[serde(default)]
    pub attempted: BTreeSet<ReminderEventKey>,
}

impl ReminderLedger {
    pub fn claim(&mut self, keys: impl IntoIterator<Item = ReminderEventKey>) {
        self.attempted.extend(keys);
    }

    pub fn claim_plan(&mut self, plan: &ReminderPlan) {
        self.claim(plan.claimed_ledger_keys.iter().cloned());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReminderDiagnostic {
    DuplicateTaskIds(Vec<String>),
    InvalidDue {
        task_id: String,
        due: String,
    },
    UnsupportedOffset {
        task_id: String,
        offset: i64,
    },
    AdvanceDateOutOfRange {
        task_id: String,
        due: String,
    },
    LocalTimeUnresolvable {
        task_id: String,
        event_type: ReminderEventType,
        requested_local: NaiveDateTime,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyReminder {
    pub event: ReminderEvent,
    /// This task's overdue, previously unclaimed keys; future events are excluded.
    pub claim_keys: Vec<ReminderEventKey>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReminderPlan {
    /// At most one catch-up notification per open task in this reconciliation.
    pub ready_notifications: Vec<ReadyReminder>,
    /// Unattempted events whose resolved instant is still in the future.
    pub future_events: Vec<ReminderEvent>,
    pub next_due: Option<DateTime<Utc>>,
    /// All overdue, previously-unclaimed events covered by this reconciliation.
    pub claimed_ledger_keys: Vec<ReminderEventKey>,
    pub duplicate_task_ids: Vec<String>,
    pub diagnostics: Vec<ReminderDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalResolution {
    Single(DateTime<Utc>),
    None,
    Ambiguous(DateTime<Utc>, DateTime<Utc>),
}

/// Resolves a wall-clock time in the machine's local time zone.
pub trait LocalTimeResolver {
    fn resolve_local(&self, local: NaiveDateTime) -> LocalResolution;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemLocalTimeResolver;

impl LocalTimeResolver for SystemLocalTimeResolver {
    fn resolve_local(&self, local: NaiveDateTime) -> LocalResolution {
        match Local.from_local_datetime(&local) {
            LocalResult::Single(value) => LocalResolution::Single(value.with_timezone(&Utc)),
            LocalResult::None => LocalResolution::None,
            LocalResult::Ambiguous(first, second) => {
                LocalResolution::Ambiguous(first.with_timezone(&Utc), second.with_timezone(&Utc))
            }
        }
    }
}

/// Construct a schedule from a caller-provided immutable task snapshot and
/// ledger. `now` is an absolute UTC instant; all due values remain local wall
/// times and are resolved through `resolver`.
pub fn reconcile<R: LocalTimeResolver>(
    tasks: &[ReminderTask],
    ledger: &ReminderLedger,
    now: DateTime<Utc>,
    resolver: &R,
) -> ReminderPlan {
    let mut plan = ReminderPlan::default();
    let mut task_ids: HashMap<&str, usize> = HashMap::new();
    for task in tasks {
        *task_ids.entry(&task.task_id).or_default() += 1;
    }
    plan.duplicate_task_ids = task_ids
        .into_iter()
        .filter_map(|(task_id, count)| (count > 1).then_some(task_id.to_string()))
        .collect();
    plan.duplicate_task_ids.sort();
    if !plan.duplicate_task_ids.is_empty() {
        plan.diagnostics.push(ReminderDiagnostic::DuplicateTaskIds(
            plan.duplicate_task_ids.clone(),
        ));
    }
    let duplicates: HashSet<String> = plan.duplicate_task_ids.iter().cloned().collect();

    for task in tasks {
        if task.status != "open" || duplicates.contains(&task.task_id) {
            continue;
        }
        let Some(due_value) = task.due.as_deref() else {
            if let Some(offset) = task.reminder_offset_days {
                if offset != 1 {
                    plan.diagnostics
                        .push(ReminderDiagnostic::UnsupportedOffset {
                            task_id: task.task_id.clone(),
                            offset,
                        });
                }
            }
            continue;
        };
        let Some((canonical_due, due_local)) = parse_due(due_value) else {
            plan.diagnostics.push(ReminderDiagnostic::InvalidDue {
                task_id: task.task_id.clone(),
                due: due_value.to_string(),
            });
            continue;
        };

        let mut events = Vec::with_capacity(2);
        if task.reminder_offset_days == Some(1) {
            match advance_local_day(&task.task_id, &canonical_due, due_local) {
                Ok(advance_local) => {
                    events.push((ReminderEventType::Advance, advance_local));
                }
                Err(diagnostic) => plan.diagnostics.push(diagnostic),
            }
        } else if let Some(offset) = task.reminder_offset_days {
            plan.diagnostics
                .push(ReminderDiagnostic::UnsupportedOffset {
                    task_id: task.task_id.clone(),
                    offset,
                });
        }
        events.push((ReminderEventType::Due, due_local));
        append_events(
            task,
            &canonical_due,
            events,
            ledger,
            now,
            resolver,
            &mut plan,
        );
    }

    plan.ready_notifications.sort_by(|a, b| {
        a.event
            .scheduled_at
            .cmp(&b.event.scheduled_at)
            .then(a.event.key.cmp(&b.event.key))
    });
    plan.future_events
        .sort_by(|a, b| a.scheduled_at.cmp(&b.scheduled_at).then(a.key.cmp(&b.key)));
    plan.claimed_ledger_keys.sort();
    plan.next_due = plan
        .future_events
        .iter()
        .map(|event| event.scheduled_at)
        .min();
    plan
}

fn append_events<R: LocalTimeResolver>(
    task: &ReminderTask,
    canonical_due: &str,
    event_specs: Vec<(ReminderEventType, NaiveDateTime)>,
    ledger: &ReminderLedger,
    now: DateTime<Utc>,
    resolver: &R,
    plan: &mut ReminderPlan,
) {
    let mut overdue = Vec::new();
    for (event_type, requested_local) in event_specs {
        let Some(event) = make_event(task, canonical_due, event_type, requested_local, resolver)
        else {
            plan.diagnostics
                .push(ReminderDiagnostic::LocalTimeUnresolvable {
                    task_id: task.task_id.clone(),
                    event_type,
                    requested_local,
                });
            continue;
        };
        if ledger.attempted.contains(&event.key) {
            continue;
        }
        if event.scheduled_at <= now {
            overdue.push(event);
        } else {
            plan.future_events.push(event);
        }
    }
    if !overdue.is_empty() {
        overdue.sort_by_key(|event| match event.key.event_type {
            ReminderEventType::Due => 0,
            ReminderEventType::Advance => 1,
        });
        let claim_keys: Vec<_> = overdue.iter().map(|event| event.key.clone()).collect();
        plan.ready_notifications.push(ReadyReminder {
            event: overdue[0].clone(),
            claim_keys: claim_keys.clone(),
        });
        plan.claimed_ledger_keys.extend(claim_keys);
    }
}

fn advance_local_day(
    task_id: &str,
    canonical_due: &str,
    local: NaiveDateTime,
) -> Result<NaiveDateTime, ReminderDiagnostic> {
    local
        .date()
        .pred_opt()
        .map(|date| date.and_time(local.time()))
        .ok_or_else(|| ReminderDiagnostic::AdvanceDateOutOfRange {
            task_id: task_id.to_string(),
            due: canonical_due.to_string(),
        })
}

fn make_event<R: LocalTimeResolver>(
    task: &ReminderTask,
    canonical_due: &str,
    event_type: ReminderEventType,
    scheduled_local: NaiveDateTime,
    resolver: &R,
) -> Option<ReminderEvent> {
    let (scheduled_local, scheduled_at) = resolve_wall_time(scheduled_local, resolver)?;
    Some(ReminderEvent {
        key: ReminderEventKey {
            task_id: task.task_id.clone(),
            canonical_due: canonical_due.to_string(),
            event_type,
        },
        task_title: task.title.clone(),
        scheduled_local,
        scheduled_at,
    })
}

const MAX_LOCAL_RESOLUTION_MINUTES: usize = 48 * 60;

fn resolve_wall_time<R: LocalTimeResolver>(
    requested: NaiveDateTime,
    resolver: &R,
) -> Option<(NaiveDateTime, DateTime<Utc>)> {
    let mut local = requested;
    for _ in 0..=MAX_LOCAL_RESOLUTION_MINUTES {
        match resolver.resolve_local(local) {
            LocalResolution::Single(value) => return Some((local, value)),
            LocalResolution::Ambiguous(first, second) => return Some((local, first.min(second))),
            LocalResolution::None => {
                local = local.checked_add_signed(Duration::minutes(1))?;
            }
        }
    }
    None
}

fn parse_due(value: &str) -> Option<(String, NaiveDateTime)> {
    if value.len() == 10 {
        let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
        if date.format("%Y-%m-%d").to_string() != value {
            return None;
        }
        return Some((value.to_string(), date.and_hms_opt(9, 0, 0)?));
    }
    if value.len() == 16 {
        let datetime = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M").ok()?;
        if datetime.format("%Y-%m-%dT%H:%M").to_string() != value {
            return None;
        }
        return Some((value.to_string(), datetime));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, Timelike};
    use std::cell::Cell;

    #[derive(Default)]
    struct FakeResolver {
        answers: HashMap<NaiveDateTime, LocalResolution>,
    }

    impl FakeResolver {
        fn map(&mut self, local: NaiveDateTime, resolution: LocalResolution) {
            self.answers.insert(local, resolution);
        }
    }

    impl LocalTimeResolver for FakeResolver {
        fn resolve_local(&self, local: NaiveDateTime) -> LocalResolution {
            self.answers
                .get(&local)
                .cloned()
                .unwrap_or_else(|| LocalResolution::Single(local.and_utc()))
        }
    }

    struct NeverResolver {
        calls: Cell<usize>,
    }

    impl LocalTimeResolver for NeverResolver {
        fn resolve_local(&self, _local: NaiveDateTime) -> LocalResolution {
            self.calls.set(self.calls.get() + 1);
            LocalResolution::None
        }
    }

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    fn utc(value: NaiveDateTime) -> DateTime<Utc> {
        value.and_utc()
    }

    fn task(id: &str, due: &str, offset: Option<i64>) -> ReminderTask {
        ReminderTask {
            list_file: format!("{id}.yaml"),
            task_id: id.to_string(),
            title: format!("title {id}"),
            status: "open".to_string(),
            due: Some(due.to_string()),
            reminder_offset_days: offset,
        }
    }

    fn plan(task: &ReminderTask, now: NaiveDateTime) -> ReminderPlan {
        reconcile(
            std::slice::from_ref(task),
            &ReminderLedger::default(),
            utc(now),
            &FakeResolver::default(),
        )
    }

    #[test]
    fn date_due_is_nine_and_datetime_due_uses_exact_wall_time() {
        let date = task("date", "2026-10-13", None);
        let date_plan = plan(&date, dt(2026, 10, 12, 8, 0));
        assert_eq!(
            date_plan.future_events[0].scheduled_local,
            dt(2026, 10, 13, 9, 0)
        );
        assert_eq!(date_plan.future_events[0].key.canonical_due, "2026-10-13");

        let timed = task("timed", "2026-10-13T14:35", None);
        let timed_plan = plan(&timed, dt(2026, 10, 13, 10, 0));
        assert_eq!(
            timed_plan.future_events[0].scheduled_local,
            dt(2026, 10, 13, 14, 35)
        );
    }

    #[test]
    fn advance_uses_previous_local_calendar_day_and_preserves_wall_time() {
        let cases = [
            ("2025-01-01T14:45", dt(2024, 12, 31, 14, 45)),
            ("2024-03-01", dt(2024, 2, 29, 9, 0)),
        ];
        for (due, expected_advance) in cases {
            let task = task("t", due, Some(1));
            let due_local = parse_due(due).unwrap().1;
            let result = plan(&task, due_local - Duration::days(3));
            let advance = result
                .future_events
                .iter()
                .find(|event| event.key.event_type == ReminderEventType::Advance)
                .unwrap();
            assert_eq!(advance.scheduled_local, expected_advance);
        }
    }

    #[test]
    fn dst_gap_moves_to_next_valid_minute_and_ambiguous_uses_earliest() {
        let requested = dt(2026, 3, 8, 2, 30);
        let next = dt(2026, 3, 8, 3, 0);
        let mut resolver = FakeResolver::default();
        for minute in 30..60 {
            resolver.map(dt(2026, 3, 8, 2, minute), LocalResolution::None);
        }
        resolver.map(next, LocalResolution::Single(utc(dt(2026, 3, 8, 7, 0))));
        let event = make_event(
            &task("gap", "2026-03-08T02:30", None),
            "2026-03-08T02:30",
            ReminderEventType::Due,
            requested,
            &resolver,
        )
        .unwrap();
        assert_eq!(event.scheduled_local, next);
        assert_eq!(event.scheduled_at, utc(dt(2026, 3, 8, 7, 0)));

        let repeated = dt(2026, 11, 1, 1, 30);
        let early = utc(dt(2026, 11, 1, 5, 30));
        let late = utc(dt(2026, 11, 1, 6, 30));
        let mut resolver = FakeResolver::default();
        resolver.map(repeated, LocalResolution::Ambiguous(late, early));
        let event = make_event(
            &task("repeat", "2026-11-01T01:30", None),
            "2026-11-01T01:30",
            ReminderEventType::Due,
            repeated,
            &resolver,
        )
        .unwrap();
        assert_eq!(event.scheduled_at, early);
    }

    #[test]
    fn overdue_events_catch_up_once_and_claim_all_overdue_events() {
        let task = task("t", "2026-10-13T09:00", Some(1));
        let result = plan(&task, dt(2026, 10, 14, 10, 0));
        assert_eq!(result.ready_notifications.len(), 1);
        assert_eq!(
            result.ready_notifications[0].event.key.event_type,
            ReminderEventType::Due
        );
        assert_eq!(result.claimed_ledger_keys.len(), 2);
        assert_eq!(result.ready_notifications[0].claim_keys.len(), 2);
        assert!(result.ready_notifications[0]
            .claim_keys
            .iter()
            .any(|key| key.event_type == ReminderEventType::Due));
        assert!(result.ready_notifications[0]
            .claim_keys
            .iter()
            .any(|key| key.event_type == ReminderEventType::Advance));
        assert_eq!(result.ready_notifications[0].event.task_title, "title t");
        assert!(result.future_events.is_empty());

        let mut ledger = ReminderLedger::default();
        ledger.claim_plan(&result);
        let repeated = reconcile(
            &[task],
            &ledger,
            utc(dt(2026, 10, 14, 10, 0)),
            &FakeResolver::default(),
        );
        assert!(repeated.ready_notifications.is_empty());
        assert!(repeated.claimed_ledger_keys.is_empty());
    }

    #[test]
    fn advance_only_overdue_does_not_claim_future_due() {
        let task = task("t", "2026-10-13T09:00", Some(1));
        let result = plan(&task, dt(2026, 10, 12, 10, 0));
        assert_eq!(result.ready_notifications.len(), 1);
        assert_eq!(
            result.ready_notifications[0].event.key.event_type,
            ReminderEventType::Advance
        );
        assert_eq!(result.claimed_ledger_keys.len(), 1);
        assert_eq!(result.ready_notifications[0].claim_keys.len(), 1);
        assert_eq!(
            result.ready_notifications[0].claim_keys[0].event_type,
            ReminderEventType::Advance
        );
        assert_eq!(result.future_events.len(), 1);
        assert_eq!(
            result.future_events[0].key.event_type,
            ReminderEventType::Due
        );
        assert_eq!(result.next_due, Some(utc(dt(2026, 10, 13, 9, 0))));
    }

    #[test]
    fn due_only_overdue_produces_due_event() {
        let result = plan(&task("t", "2026-10-13", None), dt(2026, 10, 13, 9, 1));
        assert_eq!(
            result.ready_notifications[0].event.key.event_type,
            ReminderEventType::Due
        );
        assert_eq!(result.claimed_ledger_keys.len(), 1);
    }

    #[test]
    fn completed_or_unknown_status_is_skipped() {
        let mut completed = task("done", "2026-10-12", None);
        completed.status = "done".to_string();
        let mut unknown = task("unknown", "2026-10-12", None);
        unknown.status = "paused".to_string();
        let result = reconcile(
            &[completed, unknown],
            &ReminderLedger::default(),
            utc(dt(2026, 10, 13, 9, 0)),
            &FakeResolver::default(),
        );
        assert!(result.ready_notifications.is_empty());
        assert!(result.claimed_ledger_keys.is_empty());
    }

    #[test]
    fn changed_due_creates_new_key_but_changing_back_is_deduplicated() {
        let first = task("t", "2026-10-12", None);
        let first_plan = plan(&first, dt(2026, 10, 13, 9, 0));
        let mut ledger = ReminderLedger::default();
        ledger.claim_plan(&first_plan);

        let changed = task("t", "2026-10-14", None);
        let changed_plan = reconcile(
            &[changed],
            &ledger,
            utc(dt(2026, 10, 15, 9, 0)),
            &FakeResolver::default(),
        );
        assert_eq!(changed_plan.ready_notifications.len(), 1);
        assert_ne!(
            changed_plan.ready_notifications[0].event.key.canonical_due,
            first_plan.ready_notifications[0].event.key.canonical_due
        );
        ledger.claim_plan(&changed_plan);
        let back = reconcile(
            &[first],
            &ledger,
            utc(dt(2026, 10, 15, 9, 0)),
            &FakeResolver::default(),
        );
        assert!(back.ready_notifications.is_empty());
    }

    #[test]
    fn duplicate_task_ids_are_isolated_and_invalid_due_is_diagnosed() {
        let duplicate_a = ReminderTask {
            list_file: "a.yaml".into(),
            ..task("dup", "2026-10-12", None)
        };
        let duplicate_b = ReminderTask {
            list_file: "b.yaml".into(),
            ..task("dup", "2026-10-12", None)
        };
        let duplicate_same_list_a = task("same-list", "2026-10-12", None);
        let duplicate_same_list_b = task("same-list", "2026-10-12", None);
        let invalid = task("bad", "2026-02-30", None);
        let result = reconcile(
            &[
                duplicate_a,
                duplicate_b,
                duplicate_same_list_a,
                duplicate_same_list_b,
                invalid,
            ],
            &ReminderLedger::default(),
            utc(dt(2026, 10, 13, 10, 0)),
            &FakeResolver::default(),
        );
        assert_eq!(result.duplicate_task_ids, vec!["dup", "same-list"]);
        assert!(result.ready_notifications.is_empty());
        assert!(result.claimed_ledger_keys.is_empty());
        assert!(result.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            ReminderDiagnostic::InvalidDue { task_id, .. } if task_id == "bad"
        )));
    }

    #[test]
    fn leap_day_advance_stays_at_nine() {
        let task = task("leap", "2024-03-01", Some(1));
        let event = plan(&task, dt(2024, 2, 27, 10, 0)).future_events[0].clone();
        assert_eq!(event.scheduled_local.hour(), 9);
        assert_eq!(event.scheduled_local, dt(2024, 2, 29, 9, 0));
    }

    #[test]
    fn resolution_search_is_bounded_and_failure_is_diagnosed() {
        let requested = dt(2026, 3, 8, 2, 30);
        let resolver = NeverResolver {
            calls: Cell::new(0),
        };
        assert!(resolve_wall_time(requested, &resolver).is_none());
        assert_eq!(resolver.calls.get(), MAX_LOCAL_RESOLUTION_MINUTES + 1);

        let mut boundary_resolver = FakeResolver::default();
        for minute in 0..MAX_LOCAL_RESOLUTION_MINUTES {
            boundary_resolver.map(
                requested + Duration::minutes(minute as i64),
                LocalResolution::None,
            );
        }
        let boundary = requested + Duration::minutes(MAX_LOCAL_RESOLUTION_MINUTES as i64);
        boundary_resolver.map(boundary, LocalResolution::Single(utc(boundary)));
        assert_eq!(
            resolve_wall_time(requested, &boundary_resolver),
            Some((boundary, utc(boundary)))
        );

        let resolver = NeverResolver {
            calls: Cell::new(0),
        };
        let result = reconcile(
            &[task("stuck", "2026-03-08T02:30", None)],
            &ReminderLedger::default(),
            utc(dt(2026, 3, 8, 0, 0)),
            &resolver,
        );
        assert!(result.ready_notifications.is_empty());
        assert!(result.future_events.is_empty());
        assert!(result.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            ReminderDiagnostic::LocalTimeUnresolvable {
                task_id,
                event_type: ReminderEventType::Due,
                ..
            } if task_id == "stuck"
        )));
    }

    #[test]
    fn minimum_chrono_date_does_not_panic_when_computing_advance_date() {
        let minimum = NaiveDate::MIN.and_hms_opt(9, 0, 0).unwrap();
        assert!(matches!(
            advance_local_day("minimum", "due", minimum),
            Err(ReminderDiagnostic::AdvanceDateOutOfRange { task_id, .. }) if task_id == "minimum"
        ));

        let earliest_contract_date = task("year-zero", "0000-01-01", Some(1));
        let result = plan(&earliest_contract_date, dt(0, 1, 1, 8, 0));
        assert!(result.diagnostics.iter().all(|diagnostic| !matches!(
            diagnostic,
            ReminderDiagnostic::AdvanceDateOutOfRange { .. }
        )));
    }
}
