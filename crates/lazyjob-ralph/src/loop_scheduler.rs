use std::str::FromStr;

use chrono::{DateTime, Utc};

use crate::error::{RalphError, Result};

pub struct LoopScheduler {
    schedule: cron::Schedule,
    last_checked: DateTime<Utc>,
}

impl LoopScheduler {
    pub fn new(expr: &str) -> Result<Self> {
        let schedule =
            cron::Schedule::from_str(expr).map_err(|e| RalphError::CronParse(e.to_string()))?;
        let last_checked = Utc::now() - chrono::Duration::milliseconds(1);
        Ok(Self {
            schedule,
            last_checked,
        })
    }

    pub fn should_run(&mut self, now: DateTime<Utc>) -> bool {
        let next = self.schedule.after(&self.last_checked).next();
        if let Some(next_tick) = next
            && next_tick <= now
        {
            self.last_checked = now;
            return true;
        }
        false
    }

    pub fn next_run_after(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.schedule.after(&now).next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // learning test: verifies cron::Schedule parses a standard 6-field expression
    #[test]
    fn cron_schedule_parses_standard_expr() {
        let result = cron::Schedule::from_str("0 * * * * *");
        assert!(
            result.is_ok(),
            "standard 6-field cron expression must parse"
        );
    }

    // learning test: verifies schedule.upcoming(Utc) returns a future DateTime
    #[test]
    fn cron_schedule_upcoming_iterator() {
        let schedule = cron::Schedule::from_str("0 * * * * *").unwrap();
        let now = Utc::now();
        let next = schedule.upcoming(Utc).next();
        assert!(next.is_some(), "must have an upcoming tick");
        assert!(next.unwrap() > now, "next tick must be in the future");
    }

    #[test]
    fn scheduler_rejects_invalid_cron() {
        let result = LoopScheduler::new("not a cron expression @@@@");
        assert!(matches!(result, Err(RalphError::CronParse(_))));
    }

    #[test]
    fn scheduler_fires_on_matching_tick() {
        let mut scheduler = LoopScheduler::new("0 * * * * *").unwrap();
        let past = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        scheduler.last_checked = past;

        let future = Utc.with_ymd_and_hms(2020, 1, 1, 0, 1, 0).unwrap();
        assert!(
            scheduler.should_run(future),
            "should fire when past a scheduled tick"
        );
    }

    #[test]
    fn scheduler_silent_before_tick() {
        let mut scheduler = LoopScheduler::new("0 0 * * * *").unwrap();
        let just_before_hour = Utc.with_ymd_and_hms(2020, 6, 15, 10, 59, 59).unwrap();
        scheduler.last_checked = just_before_hour;

        let still_before = Utc.with_ymd_and_hms(2020, 6, 15, 10, 59, 59).unwrap();
        assert!(
            !scheduler.should_run(still_before),
            "should not fire when no tick has occurred"
        );
    }

    #[test]
    fn scheduler_next_run_after_returns_future_time() {
        let scheduler = LoopScheduler::new("0 * * * * *").unwrap();
        let now = Utc::now();
        let next = scheduler.next_run_after(now);
        assert!(next.is_some());
        assert!(next.unwrap() > now);
    }

    #[test]
    fn scheduler_does_not_fire_twice_for_same_tick() {
        let mut scheduler = LoopScheduler::new("0 * * * * *").unwrap();
        let past = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        scheduler.last_checked = past;

        let at_tick = Utc.with_ymd_and_hms(2020, 1, 1, 0, 1, 0).unwrap();
        let fired_first = scheduler.should_run(at_tick);
        let fired_second = scheduler.should_run(at_tick);

        assert!(fired_first);
        assert!(
            !fired_second,
            "should not fire again for same now value after advancing last_checked"
        );
    }
}
