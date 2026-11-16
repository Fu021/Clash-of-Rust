#[derive(Default)]
pub(crate) struct Delivery {
    observed: u64,
    presented: u64,
}

impl Delivery {
    pub(crate) fn pending(&self, sequence: u64) -> bool {
        sequence > self.presented
    }

    pub(crate) fn observe(&mut self, sequence: u64) -> bool {
        if sequence <= self.observed || !self.pending(sequence) {
            return false;
        }
        self.observed = sequence;
        true
    }

    pub(crate) fn can_confirm(&self, sequence: u64, visible: bool) -> bool {
        visible && self.pending(sequence) && sequence <= self.observed
    }

    pub(crate) fn confirm(&mut self, sequence: u64, visible: bool) -> bool {
        if !self.can_confirm(sequence, visible) {
            return false;
        }
        self.presented = sequence;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::Delivery;

    #[test]
    fn an_observed_but_blocked_notice_remains_pending_without_repeating_history() {
        let mut delivery = Delivery::default();
        assert!(delivery.observe(41));
        for _ in 0..3 {
            assert!(delivery.pending(41));
            assert!(!delivery.observe(41));
        }
        assert!(delivery.confirm(41, true));
        assert!(!delivery.pending(41));
        assert!(!delivery.observe(41));
        assert!(!delivery.confirm(41, true));
    }

    #[test]
    fn old_callbacks_do_not_confirm_a_newer_notice_or_rewind_presentation() {
        let mut delivery = Delivery::default();
        assert!(delivery.observe(10));
        assert!(delivery.observe(11));
        assert!(delivery.confirm(10, true));
        assert!(delivery.pending(11));
        assert!(!delivery.confirm(12, true));
        assert!(delivery.confirm(11, true));
        assert!(!delivery.confirm(10, true));
        assert!(!delivery.pending(10));
        assert!(!delivery.pending(11));
        assert!(delivery.observe(12));
        assert!(delivery.pending(12));
    }

    #[test]
    fn an_empty_native_receipt_does_not_create_or_confirm_a_notice() {
        let mut delivery = Delivery::default();
        assert!(!delivery.pending(0));
        assert!(!delivery.observe(0));
        assert!(!delivery.confirm(0, true));
    }

    #[test]
    fn a_background_frame_cannot_consume_the_reminder_before_resuming() {
        let mut delivery = Delivery::default();
        assert!(delivery.observe(17));
        assert!(!delivery.confirm(17, false));
        assert!(delivery.pending(17));
        assert!(!delivery.observe(17));
        assert!(delivery.confirm(17, true));
        assert!(!delivery.pending(17));
    }

    #[test]
    fn a_failed_native_acknowledgement_can_be_retried_without_repeating_history() {
        let mut delivery = Delivery::default();
        assert!(delivery.observe(23));
        for _ in 0..3 {
            assert!(delivery.can_confirm(23, true));
            assert!(!delivery.observe(23));
            assert!(!delivery.can_confirm(24, true));
        }
        assert!(delivery.confirm(23, true));
        assert!(!delivery.can_confirm(23, true));
        assert!(!delivery.pending(23));
    }
}
