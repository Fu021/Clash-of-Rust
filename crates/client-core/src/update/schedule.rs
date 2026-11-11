use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Schedule {
    minutes: Option<u32>,
    last_check: Option<Instant>,
}

impl Schedule {
    pub fn due(&mut self, minutes: u32, now: Instant) -> bool {
        if self.minutes != Some(minutes) {
            if self.minutes.is_some() {
                self.last_check = Some(now);
            }
            self.minutes = Some(minutes);
        }
        minutes > 0
            && self.last_check.is_none_or(|last| {
                now.saturating_duration_since(last) >= Duration::from_secs(u64::from(minutes) * 60)
            })
    }

    pub fn checked(&mut self, now: Instant) {
        self.last_check = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_schedule_never_checks_even_on_startup() {
        let now = Instant::now();
        let mut schedule = Schedule::default();
        assert!(!schedule.due(0, now));
        assert!(!schedule.due(0, now + Duration::from_secs(604800)));
    }

    #[test]
    fn enabled_schedule_checks_on_startup_then_obeys_minutes() {
        let now = Instant::now();
        let mut schedule = Schedule::default();
        assert!(schedule.due(360, now));
        schedule.checked(now);
        assert!(!schedule.due(360, now + Duration::from_secs(21599)));
        let next = now + Duration::from_secs(21600);
        assert!(schedule.due(360, next));
        schedule.checked(next);
        assert!(!schedule.due(360, next + Duration::from_secs(1)));
    }

    #[test]
    fn changing_or_reenabling_an_interval_restarts_the_countdown() {
        let now = Instant::now();
        let mut schedule = Schedule::default();
        assert!(schedule.due(360, now));
        schedule.checked(now);
        assert!(!schedule.due(1, now));
        assert!(!schedule.due(1, now + Duration::from_secs(59)));
        assert!(schedule.due(1, now + Duration::from_secs(60)));
        assert!(!schedule.due(0, now + Duration::from_secs(61)));
        assert!(!schedule.due(1, now + Duration::from_secs(120)));
        assert!(!schedule.due(1, now + Duration::from_secs(179)));
        assert!(schedule.due(1, now + Duration::from_secs(180)));
    }

    #[test]
    fn a_manual_check_postpones_the_next_automatic_attempt() {
        let now = Instant::now();
        let mut schedule = Schedule::default();
        assert!(schedule.due(1, now));
        schedule.checked(now);
        schedule.checked(now + Duration::from_secs(50));
        assert!(!schedule.due(1, now + Duration::from_secs(60)));
        assert!(schedule.due(1, now + Duration::from_secs(110)));
    }

    #[test]
    fn an_overdue_check_remains_due_until_the_host_can_start_it() {
        let now = Instant::now();
        let mut schedule = Schedule::default();
        assert!(schedule.due(1, now));
        assert!(schedule.due(1, now + Duration::from_secs(120)));
        schedule.checked(now + Duration::from_secs(120));
        assert!(!schedule.due(1, now + Duration::from_secs(121)));
    }
}
