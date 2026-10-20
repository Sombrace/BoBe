use chrono::{DateTime, Duration, Local, NaiveTime, Utc};
use tracing::{debug, info, warn};

pub(crate) struct CheckinScheduler {
    times: Vec<NaiveTime>,
    interval_minutes: Option<u64>,
    jitter_minutes: i32,
    enabled: bool,
    fired_slot: Option<DateTime<Utc>>,
    next_slot: Option<DateTime<Utc>>,
    next_checkin: Option<DateTime<Utc>>,
    next_interval_checkin: Option<DateTime<Utc>>,
}

impl CheckinScheduler {
    pub(crate) fn new(
        times: &[String],
        interval_minutes: Option<u64>,
        jitter_minutes: u32,
        enabled: bool,
    ) -> Self {
        let parsed_times = Self::parse_times(times);
        info!(
            times = ?parsed_times.iter().map(|t| t.format("%H:%M").to_string()).collect::<Vec<_>>(),
            interval_minutes = ?interval_minutes,
            jitter_minutes,
            enabled,
            "checkin_scheduler.initialized"
        );
        Self {
            times: parsed_times,
            interval_minutes,
            jitter_minutes: jitter_minutes as i32,
            enabled,
            fired_slot: None,
            next_slot: None,
            next_checkin: None,
            next_interval_checkin: None,
        }
    }

    fn parse_times(times: &[String]) -> Vec<NaiveTime> {
        let mut parsed = Vec::new();
        for time_str in times {
            let parts: Vec<&str> = time_str.split(':').collect();
            if parts.len() == 2 {
                if let (Ok(hour), Ok(minute)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>())
                    && let Some(t) = NaiveTime::from_hms_opt(hour, minute, 0)
                {
                    parsed.push(t);
                }
            } else {
                warn!(time = %time_str, "checkin_scheduler.invalid_time_format");
            }
        }
        parsed.sort();
        parsed
    }

    pub(crate) fn reconfigure(
        &mut self,
        times: &[String],
        interval_minutes: Option<u64>,
        jitter_minutes: u32,
        enabled: bool,
    ) {
        self.times = Self::parse_times(times);
        self.interval_minutes = interval_minutes;
        self.jitter_minutes = jitter_minutes as i32;
        self.enabled = enabled;
        self.next_slot = None;
        self.next_checkin = None;
        self.next_interval_checkin = None;
        self.refresh_schedules(Utc::now());
        info!(enabled, "checkin_scheduler.reconfigured");
    }

    pub(crate) fn should_checkin(&mut self) -> bool {
        self.should_checkin_at(Utc::now())
    }

    fn should_checkin_at(&mut self, now: DateTime<Utc>) -> bool {
        if !self.enabled {
            return false;
        }
        if self.times.is_empty() && self.interval_minutes.is_none() {
            return false;
        }

        self.refresh_schedules(now);

        if !self.times.is_empty()
            && let Some(next) = self.next_checkin
            && now >= next
        {
            info!(
                trigger_type = "scheduled_time",
                scheduled = %next,
                "checkin_scheduler.triggered"
            );
            return true;
        }

        if self.interval_minutes.is_some()
            && let Some(next) = self.next_interval_checkin
            && now >= next
        {
            info!(
                trigger_type = "interval",
                scheduled = %next,
                "checkin_scheduler.triggered"
            );
            return true;
        }

        false
    }

    pub(crate) fn mark_checkin_done(&mut self) {
        self.mark_checkin_done_at(Utc::now());
    }

    fn mark_checkin_done_at(&mut self, now: DateTime<Utc>) {
        if !self.times.is_empty() && self.next_checkin.is_some_and(|next| now >= next) {
            self.fired_slot = self.next_slot;
            self.schedule_next_checkin(now);
        }
        if self.interval_minutes.is_some() {
            self.schedule_next_interval(now);
        }

        let next = self.get_next_checkin_time();
        info!(
            next = ?next.map(|t| t.format("%H:%M:%S").to_string()),
            "checkin_scheduler.done"
        );
    }

    pub(crate) fn get_next_checkin_time(&mut self) -> Option<DateTime<Utc>> {
        if !self.enabled {
            return None;
        }
        if self.times.is_empty() && self.interval_minutes.is_none() {
            return None;
        }

        let now = Utc::now();
        self.refresh_schedules(now);

        [self.next_checkin, self.next_interval_checkin]
            .into_iter()
            .flatten()
            .min()
    }

    fn refresh_schedules(&mut self, now: DateTime<Utc>) {
        if !self.times.is_empty() && self.next_checkin.is_none() {
            self.schedule_next_checkin(now);
        }
        if self.interval_minutes.is_some() && self.next_interval_checkin.is_none() {
            self.schedule_next_interval(now);
        }
    }

    fn schedule_next_checkin(&mut self, after: DateTime<Utc>) {
        if self.times.is_empty() {
            self.next_slot = None;
            self.next_checkin = None;
            return;
        }

        let after = self.fired_slot.map_or(after, |slot| slot.max(after));
        let local_now = after.with_timezone(&Local);
        let today = local_now.date_naive();
        let local_time = local_now.time();

        let mut next_time: Option<DateTime<Utc>> = None;
        for &t in &self.times {
            if t > local_time {
                let naive_dt = today.and_time(t);
                let local_dt = naive_dt.and_local_timezone(Local).single();
                if let Some(dt) = local_dt {
                    next_time = Some(dt.with_timezone(&Utc));
                    break;
                }
            }
        }

        if next_time.is_none() {
            let tomorrow = today + Duration::days(1);
            let naive_dt = tomorrow.and_time(self.times[0]);
            if let Some(dt) = naive_dt.and_local_timezone(Local).single() {
                next_time = Some(dt.with_timezone(&Utc));
            }
        }

        self.next_slot = next_time;
        if let Some(ref mut t) = next_time
            && self.jitter_minutes > 0
        {
            let jitter = rand::random_range(-self.jitter_minutes..=self.jitter_minutes);
            *t += Duration::minutes(jitter as i64);
        }

        self.next_checkin = next_time;
        if let Some(t) = next_time {
            debug!(next = %t.format("%Y-%m-%d %H:%M:%S"), "checkin_scheduler.scheduled_next_time");
        }
    }

    fn schedule_next_interval(&mut self, after: DateTime<Utc>) {
        let Some(interval) = self.interval_minutes else {
            self.next_interval_checkin = None;
            return;
        };

        let mut next_time = after + Duration::minutes(interval as i64);

        if self.jitter_minutes > 0 {
            let jitter = rand::random_range(-self.jitter_minutes..=self.jitter_minutes);
            next_time += Duration::minutes(jitter as i64);
        }

        self.next_interval_checkin = Some(next_time);
        debug!(next = %next_time.format("%Y-%m-%d %H:%M:%S"), "checkin_scheduler.scheduled_interval");
    }
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, TimeZone};

    use super::*;

    fn local(day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        let naive = NaiveDate::from_ymd_opt(2026, 1, day)
            .and_then(|date| date.and_hms_opt(hour, minute, 0))
            .unwrap();
        Local
            .from_local_datetime(&naive)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn an_early_jittered_checkin_fires_its_slot_once() {
        let mut scheduler = CheckinScheduler::new(&["09:00".into()], None, 0, true);
        scheduler.schedule_next_checkin(local(15, 8, 0));
        assert_eq!(scheduler.next_slot, Some(local(15, 9, 0)));
        scheduler.next_checkin = Some(local(15, 8, 57));

        assert!(scheduler.should_checkin_at(local(15, 8, 57)));
        scheduler.mark_checkin_done_at(local(15, 8, 57));

        assert_eq!(scheduler.next_slot, Some(local(16, 9, 0)));
        assert!(!scheduler.should_checkin_at(local(15, 9, 5)));
    }

    #[test]
    fn an_interval_checkin_keeps_the_pending_slot() {
        let mut scheduler = CheckinScheduler::new(&["14:00".into()], Some(60), 5, true);
        scheduler.schedule_next_checkin(local(15, 12, 0));
        scheduler.next_checkin = Some(local(15, 13, 57));
        scheduler.next_interval_checkin = Some(local(15, 13, 30));

        assert!(scheduler.should_checkin_at(local(15, 13, 30)));
        scheduler.mark_checkin_done_at(local(15, 13, 30));

        assert_eq!(scheduler.next_checkin, Some(local(15, 13, 57)));
        assert_eq!(scheduler.next_slot, Some(local(15, 14, 0)));
        assert!(!scheduler.should_checkin_at(local(15, 13, 31)));
        assert!(scheduler.should_checkin_at(local(15, 13, 57)));
    }

    #[test]
    fn a_rescheduled_slot_after_reconfiguration_does_not_fire_again() {
        let mut scheduler = CheckinScheduler::new(&["09:00".into(), "14:00".into()], None, 0, true);
        scheduler.schedule_next_checkin(local(15, 8, 0));
        scheduler.next_checkin = Some(local(15, 8, 56));
        assert!(scheduler.should_checkin_at(local(15, 8, 56)));
        scheduler.mark_checkin_done_at(local(15, 8, 56));

        scheduler.next_slot = None;
        scheduler.next_checkin = None;
        scheduler.refresh_schedules(local(15, 8, 58));

        assert_eq!(scheduler.next_slot, Some(local(15, 14, 0)));
    }
}
