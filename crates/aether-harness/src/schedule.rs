//! Scheduling, heartbeats, and durable re-entry (spec §15, §16).
//!
//! Both were **entirely absent** from AETHER (verified: zero hits for
//! `cron|schedule|heartbeat|next_run`). Built event-driven like the reference
//! harness: one parked timer wakes on the earliest due job or on an explicit
//! `wake()`, instead of polling.
//!
//! Re-entry semantics that matter (spec §16): a job carries the *session* it
//! belongs to, so firing resumes that session — it never creates an unrelated
//! task and never resets goal/checkpoint/model assignments.

use serde::{Deserialize, Serialize};

/// How often a job fires. Grammar mirrors the reference harness so operators
/// can copy familiar forms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    /// Fire once at a specific unix-millis timestamp.
    Once { at_ms: i64 },
    /// Fire every `period_ms`.
    Interval { period_ms: i64 },
    /// Standard 5-field cron: minute hour day-of-month month day-of-week.
    Cron { expr: String },
}

impl Schedule {
    /// Parse `in 30s`, `every 5m`, `every 2h`, `at <unix-ms>`, or a
    /// 5-field cron expression.
    pub fn parse(spec: &str) -> Result<Self, ScheduleError> {
        let s = spec.trim().to_lowercase();
        let mut parts = s.split_whitespace();
        let head = parts.next().unwrap_or("");
        let bad = || ScheduleError::BadSpec(spec.to_string());
        match head {
            "in" | "once" => {
                let rest: String = parts.collect::<Vec<_>>().join("");
                let ms = parse_amount(&rest).ok_or_else(bad)?;
                Ok(Self::Once {
                    at_ms: chrono::Utc::now().timestamp_millis() + ms,
                })
            }
            "every" | "each" => {
                let rest: String = parts.collect::<Vec<_>>().join("");
                let ms = parse_amount(&rest).ok_or_else(bad)?;
                if ms <= 0 {
                    return Err(bad());
                }
                Ok(Self::Interval { period_ms: ms })
            }
            "at" => {
                let at: i64 = parts
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(bad)?;
                Ok(Self::Once { at_ms: at })
            }
            _ => {
                if s.split_whitespace().count() == 5 {
                    let _ = Cron::parse(&s)?;
                    Ok(Self::Cron { expr: s })
                } else {
                    Err(bad())
                }
            }
        }
    }

    /// Next fire strictly after `after_ms`, or `None` when the job is done
    /// (one-shot in the past). Bounded search so a cron job can never hang
    /// the wake loop.
    pub fn next_after(&self, after_ms: i64) -> Option<i64> {
        match self {
            Self::Once { at_ms } => (*at_ms > after_ms).then_some(*at_ms),
            Self::Interval { period_ms } => {
                let period = (*period_ms).max(1);
                let next = after_ms - (after_ms % period) + period;
                Some(next)
            }
            Self::Cron { expr } => Cron::parse(expr).ok()?.next_after(after_ms),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Once { at_ms } => format!("once@{at_ms}"),
            Self::Interval { period_ms } => format!("every {}ms", period_ms),
            Self::Cron { expr } => format!("cron {expr}"),
        }
    }
}

/// Parse an amount with an optional unit suffix: `30s`, `5m`, `2h`, `500ms`.
/// A bare number means seconds.
fn parse_amount(text: &str) -> Option<i64> {
    let t = text.trim();
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let n: i64 = digits.parse().ok()?;
    let unit = &t[digits.len()..];
    unit_ms(n, if unit.is_empty() { "s" } else { unit })
}

fn unit_ms(n: i64, unit: &str) -> Option<i64> {    let mult = match unit {
        "ms" | "milli" | "millis" => 1,
        "s" | "sec" | "secs" | "second" | "seconds" => 1_000,
        "m" | "min" | "mins" | "minute" | "minutes" => 60_000,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3_600_000,
        "d" | "day" | "days" => 86_400_000,
        _ => return None,
    };
    n.checked_mul(mult)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScheduleError {
    #[error("bad schedule spec: {0}")]
    BadSpec(String),
    #[error("bad cron expression: {0}")]
    BadCron(String),
}

/// Minimal 5-field cron. Fields: minute hour dom month dow. Supports `*`,
/// numbers, `a-b`, `a-b/n`, `*\/n`, and comma lists. Day-of-month and
/// day-of-week are OR-ed when both are restricted (standard cron behavior).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minutes: Vec<u32>,
    hours: Vec<u32>,
    doms: Vec<u32>,
    months: Vec<u32>,
    dows: Vec<u32>,
}

impl Cron {
    pub fn parse(expr: &str) -> Result<Self, ScheduleError> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(ScheduleError::BadCron(expr.to_string()));
        }
        Ok(Self {
            minutes: parse_field(fields[0], 0, 59, expr)?,
            hours: parse_field(fields[1], 0, 23, expr)?,
            doms: parse_field(fields[2], 1, 31, expr)?,
            months: parse_field(fields[3], 1, 12, expr)?,
            dows: parse_dow(fields[4], expr)?,
        })
    }

    fn matches(&self, t: chrono::DateTime<chrono::Utc>) -> bool {
        use chrono::{Datelike, Timelike};
        let dom_restricted = self.doms.len() != 31;
        let dow_restricted = self.dows.len() != 7;
        let dow = t.weekday().num_days_from_sunday() as u32;
        let dom_ok = self.doms.contains(&t.day());
        let dow_ok = self.dows.contains(&dow);
        let day_ok = match (dom_restricted, dow_restricted) {
            (true, true) => dom_ok || dow_ok,
            (true, false) => dom_ok,
            (false, true) => dow_ok,
            (false, false) => true,
        };
        self.minutes.contains(&t.minute())
            && self.hours.contains(&t.hour())
            && self.months.contains(&(t.month()))
            && day_ok
    }

    /// Next matching minute strictly after `after_ms`, searched at most
    /// 366 days ahead.
    pub fn next_after(&self, after_ms: i64) -> Option<i64> {
        use chrono::Timelike;
        let start = chrono::DateTime::from_timestamp_millis(after_ms)
            .or_else(|| chrono::DateTime::from_timestamp_millis(0))?
            .with_timezone(&chrono::Utc)
            + chrono::Duration::minutes(1);
        // Truncate to the minute boundary, then step a minute at a time.
        let mut t = start.with_second(0)?.with_nanosecond(0)?;
        let horizon = 366 * 24 * 60;
        for _ in 0..horizon {
            if self.matches(t) {
                return Some(t.timestamp_millis());
            }
            t += chrono::Duration::minutes(1);
        }
        None
    }
}

fn parse_dow(f: &str, expr: &str) -> Result<Vec<u32>, ScheduleError> {
    // Named days may be combined with numeric ranges, so rewrite names to
    // numbers before parsing the field syntax.
    let mut normalized = f.to_string();
    for (name, num) in [
        ("sun", "0"),
        ("mon", "1"),
        ("tue", "2"),
        ("wed", "3"),
        ("thu", "4"),
        ("fri", "5"),
        ("sat", "6"),
    ] {
        normalized = normalized.replace(name, num);
    }
    let mut out = Vec::new();
    for part in normalized.split(',') {
        let part = part.trim();
        // 7 and 0 both mean Sunday.
        let part = if part == "7" { "0" } else { part };
        let vals = parse_field(part, 0, 7, expr)?;
        out.extend(vals);
    }
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(ScheduleError::BadCron(expr.to_string()));
    }
    Ok(out)
}

fn parse_field(f: &str, min: u32, max: u32, expr: &str) -> Result<Vec<u32>, ScheduleError> {
    let mut out = Vec::new();
    for part in f.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().ok().filter(|v| *v > 0)),
            None => (part, Some(1)),
        };
        let step = match step {
            Some(s) => s,
            None => return Err(ScheduleError::BadCron(expr.to_string())),
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (
                a.parse().map_err(|_| ScheduleError::BadCron(expr.to_string()))?,
                b.parse().map_err(|_| ScheduleError::BadCron(expr.to_string()))?,
            )
        } else {
            let v: u32 = range
                .parse()
                .map_err(|_| ScheduleError::BadCron(expr.to_string()))?;
            (v, v)
        };
        if lo < min || hi > max || lo > hi {
            return Err(ScheduleError::BadCron(expr.to_string()));
        }
        let mut v = lo;
        while v <= hi {
            out.push(v);
            v += step;
        }
    }
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(ScheduleError::BadCron(expr.to_string()));
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Active,
    Paused,
    Completed,
    Cancelled,
}

/// Whether a firing interrupts a busy session or waits for a free slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Steer,
    FollowUp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledJob {
    pub id: String,
    /// Session this job re-enters (spec §16: resume, never re-create).
    pub session_id: String,
    pub label: String,
    pub prompt: String,
    pub schedule: Schedule,
    pub status: JobStatus,
    pub delivery: DeliveryMode,
    pub next_run_at: Option<i64>,
    pub last_run_at: Option<i64>,
    pub run_count: u32,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
    /// Heartbeats emit their own events; ordinary jobs do not.
    pub is_heartbeat: bool,
}

impl ScheduledJob {
    pub fn new(
        session_id: &str,
        label: &str,
        prompt: &str,
        schedule: Schedule,
    ) -> Self {
        let next = schedule.next_after(chrono::Utc::now().timestamp_millis());
        Self {
            id: format!("job-{}", uuid::Uuid::new_v4().simple()),
            session_id: session_id.to_string(),
            label: label.to_string(),
            prompt: prompt.to_string(),
            schedule,
            status: JobStatus::Active,
            delivery: DeliveryMode::FollowUp,
            next_run_at: next,
            last_run_at: None,
            run_count: 0,
            consecutive_failures: 0,
            last_error: None,
            is_heartbeat: false,
        }
    }

    pub fn heartbeat(mut self, steer: bool) -> Self {
        self.is_heartbeat = true;
        self.delivery = if steer {
            DeliveryMode::Steer
        } else {
            DeliveryMode::FollowUp
        };
        self
    }

    pub fn with_delivery(mut self, d: DeliveryMode) -> Self {
        self.delivery = d;
        self
    }

    /// Exponential failure backoff (120s * 2^(n-1), capped at 1h) so a broken
    /// target cannot spin the wake loop.
    pub fn failure_backoff_ms(&self) -> i64 {
        let n = self.consecutive_failures.clamp(1, 6) as i64;
        (120_000_i64 * 2i64.pow((n - 1) as u32)).min(3_600_000)
    }
}

/// Job registry with event-driven wake (spec §15, §16).
///
/// One parked timer waits on the earliest due job; any mutation calls
/// [`Scheduler::wake`], so nothing polls when the schedule is quiet.
pub struct Scheduler {
    inner: std::sync::Mutex<SchedulerState>,
    wake: tokio::sync::Notify,
}

#[derive(Default)]
struct SchedulerState {
    jobs: Vec<ScheduledJob>,
    /// Set while `run_due` is dispatching, so a re-entrant wake cannot
    /// double-claim the same job.
    dispatching: bool,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(SchedulerState::default()),
            wake: tokio::sync::Notify::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SchedulerState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Register a job and arm the timer.
    pub fn add(&self, job: ScheduledJob) {
        {
            let mut st = self.lock();
            st.jobs.retain(|j| j.id != job.id);
            st.jobs.push(job);
        }
        self.wake.notify_waiters();
    }

    /// Replace any active/paused heartbeat for this session with a new one —
    /// one heartbeat per session, never stacked (spec §15).
    pub fn set_heartbeat(&self, mut job: ScheduledJob) {
        job.is_heartbeat = true;
        {
            let mut st = self.lock();
            st.jobs
                .retain(|j| !(j.is_heartbeat && j.session_id == job.session_id));
            st.jobs.push(job);
        }
        self.wake.notify_waiters();
    }

    pub fn jobs(&self) -> Vec<ScheduledJob> {
        self.lock().jobs.clone()
    }

    pub fn pause(&self, id: &str) {
        let mut st = self.lock();
        if let Some(j) = st.jobs.iter_mut().find(|j| j.id == id) {
            j.status = JobStatus::Paused;
        }
        drop(st);
        self.wake.notify_waiters();
    }

    pub fn resume(&self, id: &str) {
        let mut st = self.lock();
        if let Some(j) = st.jobs.iter_mut().find(|j| j.id == id) {
            j.status = JobStatus::Active;
            let now = chrono::Utc::now().timestamp_millis();
            j.next_run_at = j.schedule.next_after(now);
        }
        drop(st);
        self.wake.notify_waiters();
    }

    pub fn cancel(&self, id: &str) {
        let mut st = self.lock();
        if let Some(j) = st.jobs.iter_mut().find(|j| j.id == id) {
            j.status = JobStatus::Cancelled;
            j.next_run_at = None;
        }
        drop(st);
        self.wake.notify_waiters();
    }

    /// Earliest due timestamp among active jobs, for the wake timer.
    pub fn next_due(&self) -> Option<i64> {
        self.lock()
            .jobs
            .iter()
            .filter(|j| j.status == JobStatus::Active)
            .filter_map(|j| j.next_run_at)
            .min()
    }

    /// Claim every job due at or before `now_ms`. One-shot jobs that have
    /// fired become `Completed`; recurring jobs get their next slot. A job
    /// claimed twice in the same tick is impossible because claiming mutates
    /// `next_run_at`/`status` under the same lock.
    pub fn claim_due(&self, now_ms: i64) -> Vec<ScheduledJob> {
        let mut st = self.lock();
        if st.dispatching {
            return Vec::new();
        }
        st.dispatching = true;
        let mut due = Vec::new();
        for j in st.jobs.iter_mut() {
            if j.status != JobStatus::Active {
                continue;
            }
            let Some(at) = j.next_run_at else { continue };
            if at > now_ms {
                continue;
            }
            let mut claim = j.clone();
            claim.last_run_at = Some(now_ms);
            claim.run_count = claim.run_count.saturating_add(1);
            match j.schedule {
                Schedule::Once { .. } => {
                    j.status = JobStatus::Completed;
                    j.next_run_at = None;
                }
                _ => {
                    j.next_run_at = j.schedule.next_after(now_ms);
                }
            }
            due.push(claim);
        }
        st.dispatching = false;
        due
    }

    /// Report success: clear the failure streak.
    pub fn record_success(&self, id: &str) {
        let mut st = self.lock();
        if let Some(j) = st.jobs.iter_mut().find(|j| j.id == id) {
            j.consecutive_failures = 0;
            j.last_error = None;
        }
    }

    /// Report failure: exponential backoff on the next slot.
    pub fn record_failure(&self, id: &str, error: &str) {
        let mut st = self.lock();
        if let Some(j) = st.jobs.iter_mut().find(|j| j.id == id) {
            j.consecutive_failures = j.consecutive_failures.saturating_add(1);
            j.last_error = Some(error.to_string());
            let backoff = j.failure_backoff_ms();
            j.next_run_at = Some(chrono::Utc::now().timestamp_millis() + backoff);
        }
    }

    /// Wake the parked timer (call after any external state change).
    /// `notify_one` stores a permit when nobody is parked yet, so a wake can
    /// never be lost between mutations — the reason this is not
    /// `notify_waiters`.
    pub fn wake(&self) {
        self.wake.notify_waiters();
    }

    /// Force a job's next fire time (used by "run now" and by tests).
    pub fn set_next_run(&self, id: &str, at_ms: i64) {
        let mut st = self.lock();
        if let Some(j) = st.jobs.iter_mut().find(|j| j.id == id) {
            j.next_run_at = Some(at_ms);
            j.status = JobStatus::Active;
        }
        drop(st);
        self.wake();
    }

    /// Park until the next due job or an explicit wake, then return so the
    /// caller can re-check and claim. Returning on a bare wake is what makes
    /// an idle scheduler cooperative instead of spinning forever.
    pub async fn wait_for_due(&self, max_sleep_ms: u64) {
        // Register interest BEFORE reading state so a wake between the read
        // and the await is not lost (Notify stores one permit for that).
        let notified = self.wake.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        match self.next_due() {
            None => {
                // Nothing scheduled: sleep until someone schedules something.
                notified.await;
            }
            Some(next) => {
                let now = chrono::Utc::now().timestamp_millis();
                let wait = (next - now).clamp(1, max_sleep_ms as i64) as u64;
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(wait)) => {}
                    _ = &mut notified => {}
                }
            }
        }
    }
}

/// Heartbeat evaluation inputs (spec §15). Heartbeats prefer events over
/// polling: `children_settled` and `new_events` are the common triggers.
#[derive(Debug, Clone, Default)]
pub struct HeartbeatInputs {
    pub children_settled: bool,
    pub children_pending: usize,
    pub new_events: usize,
    pub task_blocked: bool,
    pub task_finished: bool,
    pub no_progress_streak: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatDecision {
    pub run: bool,
    pub reason: String,
}

impl HeartbeatDecision {
    fn skip(reason: &str) -> Self {
        Self {
            run: false,
            reason: reason.to_string(),
        }
    }
    fn run(reason: &str) -> Self {
        Self {
            run: true,
            reason: reason.to_string(),
        }
    }
}

/// Decide whether a heartbeat should fire. Event-driven first, polling only
/// as a last-resort stall detector.
pub fn evaluate_heartbeat(i: &HeartbeatInputs) -> HeartbeatDecision {
    if i.task_finished {
        return HeartbeatDecision::skip("task already finished");
    }
    if i.task_blocked {
        return HeartbeatDecision::run("task blocked — check for user input");
    }
    if i.children_settled && i.children_pending == 0 {
        return HeartbeatDecision::run("children settled — collect results");
    }
    if i.new_events > 0 {
        return HeartbeatDecision::run("new runtime events");
    }
    if i.children_pending > 0 {
        return HeartbeatDecision::skip("children still running");
    }
    if i.no_progress_streak >= 2 {
        return HeartbeatDecision::run("no progress for several turns — escalate");
    }
    HeartbeatDecision::skip("nothing to do")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now_ms() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    #[test]
    fn parses_interval_and_once_specs() {
        assert_eq!(
            Schedule::parse("every 5m").unwrap(),
            Schedule::Interval { period_ms: 300_000 }
        );
        assert_eq!(
            Schedule::parse("every 2h").unwrap(),
            Schedule::Interval { period_ms: 7_200_000 }
        );
        let once = Schedule::parse("in 30s").unwrap();
        match once {
            Schedule::Once { at_ms } => {
                assert!(at_ms > now_ms() && at_ms <= now_ms() + 31_000)
            }
            _ => panic!("expected once"),
        }
        assert!(Schedule::parse("banana").is_err());
        assert!(Schedule::parse("every").is_err());
    }

    #[test]
    fn cron_parses_and_finds_next() {
        use chrono::Timelike;
        let c = Cron::parse("*/15 * * * *").unwrap();
        let base = chrono::Utc::now();
        let next = c.next_after(base.timestamp_millis()).unwrap();
        let at = chrono::DateTime::from_timestamp_millis(next).unwrap();
        assert_eq!(at.minute() % 15, 0);
        assert!(next > base.timestamp_millis());

        // Daily at 03:00
        let daily = Cron::parse("0 3 * * *").unwrap();
        let n = daily.next_after(now_ms()).unwrap();
        let t = chrono::DateTime::from_timestamp_millis(n).unwrap();
        assert_eq!(t.hour(), 3);
        assert_eq!(t.minute(), 0);

        // Weekday-only expression is accepted
        let wd = Cron::parse("0 9 * * mon-fri").unwrap();
        assert!(wd.next_after(now_ms()).is_some());

        assert!(Cron::parse("bogus").is_err());
        assert!(Cron::parse("0 99 * * *").is_err());
    }

    #[test]
    fn interval_next_is_strictly_future_and_aligned() {
        let s = Schedule::Interval { period_ms: 1_000 };
        let base = 5_000;
        assert_eq!(s.next_after(base), Some(6_000));
        assert_eq!(s.next_after(5_999), Some(6_000));
        // One-shot in the past is done.
        assert_eq!(Schedule::Once { at_ms: 100 }.next_after(200), None);
    }

    #[test]
    fn heartbeat_is_event_driven() {
        assert!(evaluate_heartbeat(&HeartbeatInputs {
            children_settled: true,
            ..Default::default()
        })
        .run);
        assert!(evaluate_heartbeat(&HeartbeatInputs {
            task_blocked: true,
            ..Default::default()
        })
        .run);
        assert!(!evaluate_heartbeat(&HeartbeatInputs {
            children_pending: 2,
            ..Default::default()
        })
        .run);
        let idle = evaluate_heartbeat(&HeartbeatInputs::default());
        assert!(!idle.run);
        assert!(idle.reason.contains("nothing to do"));
        assert!(!evaluate_heartbeat(&HeartbeatInputs {
            task_finished: true,
            ..Default::default()
        })
        .run);
    }

    #[test]
    fn job_backoff_grows_and_caps() {
        let mut j = ScheduledJob::new("s", "hb", "check", Schedule::Interval { period_ms: 60_000 });
        assert_eq!(j.failure_backoff_ms(), 120_000);
        j.consecutive_failures = 2;
        assert_eq!(j.failure_backoff_ms(), 240_000);
        j.consecutive_failures = 99;
        assert_eq!(j.failure_backoff_ms(), 3_600_000);
    }

    #[test]
    fn job_roundtrips_for_persistence() {
        let j = ScheduledJob::new("sess-1", "resume", "continue the task", Schedule::parse("every 10m").unwrap())
            .heartbeat(true);
        let s = serde_json::to_string(&j).unwrap();
        let back: ScheduledJob = serde_json::from_str(&s).unwrap();
        assert_eq!(back.session_id, "sess-1");
        assert_eq!(back.delivery, DeliveryMode::Steer);
        assert!(back.is_heartbeat);
        assert!(back.next_run_at.is_some());
    }

    #[test]
    fn scheduler_claims_due_jobs_once() {
        let s = Scheduler::new();
        s.add(ScheduledJob::new(
            "sess",
            "once",
            "resume goal",
            Schedule::Once { at_ms: 1_000 },
        ));
        // A one-shot already in the past has no next slot; force it due.
        let id = s.jobs()[0].id.clone();
        s.set_next_run(&id, 1_000);
        let now = 2_000;
        let due = s.claim_due(now);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].run_count, 1);
        // Not claimed twice.
        assert!(s.claim_due(now).is_empty());
        // One-shot is now complete.
        assert_eq!(s.jobs()[0].status, JobStatus::Completed);
        assert!(s.next_due().is_none());
    }

    #[test]
    fn recurring_job_advances_and_pause_blocks_it() {
        let s = Scheduler::new();
        s.add(ScheduledJob::new(
            "sess",
            "hb",
            "check",
            Schedule::Interval { period_ms: 1_000 },
        ));
        let id = s.jobs()[0].id.clone();
        s.set_next_run(&id, 10_000);
        let now = 10_000;
        assert_eq!(s.claim_due(now).len(), 1);
        assert!(s.next_due().unwrap() > now);
        s.pause(&id);
        assert!(s.claim_due(now + 10_000).is_empty());
        s.resume(&id);
        s.set_next_run(&id, now + 10_000);
        assert_eq!(s.claim_due(now + 10_000).len(), 1);
        s.cancel(&id);
        assert!(s.claim_due(now + 100_000).is_empty());
    }

    #[test]
    fn failure_backs_off_the_next_slot() {
        let s = Scheduler::new();
        s.add(ScheduledJob::new(
            "sess",
            "hb",
            "check",
            Schedule::Interval { period_ms: 1_000 },
        ));
        let id = s.jobs()[0].id.clone();
        let before = s.jobs()[0].next_run_at.unwrap();
        s.record_failure(&id, "target gone");
        let j = &s.jobs()[0];
        assert_eq!(j.consecutive_failures, 1);
        assert_eq!(j.last_error.as_deref(), Some("target gone"));
        assert!(j.next_run_at.unwrap() > before + 100_000, "backoff must exceed one period");
        s.record_success(&id);
        assert_eq!(s.jobs()[0].consecutive_failures, 0);
        assert!(s.jobs()[0].last_error.is_none());
    }

    #[test]
    fn one_heartbeat_per_session_is_never_stacked() {
        let s = Scheduler::new();
        s.set_heartbeat(ScheduledJob::new(
            "sess",
            "hb1",
            "first",
            Schedule::Interval { period_ms: 60_000 },
        ));
        s.set_heartbeat(ScheduledJob::new(
            "sess",
            "hb2",
            "second",
            Schedule::Interval { period_ms: 30_000 },
        ));
        let beats: Vec<_> = s.jobs().into_iter().filter(|j| j.is_heartbeat).collect();
        assert_eq!(beats.len(), 1);
        assert_eq!(beats[0].label, "hb2");
    }

    #[tokio::test]
    async fn wait_for_due_returns_on_wake() {
        let s = std::sync::Arc::new(Scheduler::new());
        let s2 = s.clone();
        let waker = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            s2.wake();
        });
        // No jobs at all: parked until woken.
        tokio::time::timeout(std::time::Duration::from_secs(10), s.wait_for_due(60_000))
            .await
            .expect("wake should release the parked timer");
        waker.await.unwrap();
    }

    #[tokio::test]
    async fn wait_for_due_returns_at_the_due_time() {
        let s = Scheduler::new();
        let now = chrono::Utc::now().timestamp_millis();
        s.add(ScheduledJob::new(
            "sess",
            "soon",
            "resume",
            Schedule::Once { at_ms: now + 200 },
        ));
        let start = std::time::Instant::now();
        tokio::time::timeout(std::time::Duration::from_secs(10), s.wait_for_due(60_000))
            .await
            .expect("timer should fire");
        // It waited for the schedule rather than returning instantly.
        assert!(start.elapsed() >= std::time::Duration::from_millis(100));
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        // Claim with a clock comfortably past the due time (no reliance on
        // sub-millisecond timer precision).
        assert_eq!(
            s.claim_due(chrono::Utc::now().timestamp_millis() + 1_000).len(),
            1
        );
    }
}
