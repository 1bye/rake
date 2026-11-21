use std::time::{Duration, Instant};

const DURATION: Duration = Duration::from_millis(400);

#[derive(Default)]
pub(crate) struct Motion {
    from: f32,
    expanded: bool,
    started: Option<Instant>,
}

impl Motion {
    pub(crate) fn expanded(&self) -> bool {
        self.expanded
    }

    pub(crate) fn sample(&self, now: Instant) -> (f32, bool) {
        let target = f32::from(self.expanded);
        let Some(started) = self.started else {
            return (target, false);
        };

        let t = (now.duration_since(started).as_secs_f32() / DURATION.as_secs_f32()).min(1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        (self.from + (target - self.from) * eased, t < 1.0)
    }

    pub(crate) fn reverse(&mut self, now: Instant) {
        self.from = self.sample(now).0;
        self.expanded = !self.expanded;
        self.started = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reversal_starts_at_the_displayed_position_and_settles() {
        let start = Instant::now();
        let mut motion = Motion::default();
        assert_eq!(motion.sample(start), (0., false));

        motion.reverse(start);
        let middle = start + DURATION / 2;
        assert_eq!(motion.sample(middle), (0.5, true));

        motion.reverse(middle);
        assert_eq!(motion.sample(middle), (0.5, true));
        assert_eq!(motion.sample(middle + DURATION), (0., false));
    }
}
