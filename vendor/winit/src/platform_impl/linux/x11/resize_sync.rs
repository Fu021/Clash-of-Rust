#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Serial {
    pub low: u32,
    pub high: i32,
}

impl Serial {
    pub(super) fn from_words(low: i64, high: i64) -> Self {
        Self { low: low as u32, high: high as i32 }
    }
}

#[derive(Default)]
pub(super) struct ResizeSync {
    requested: Option<Serial>,
    configured: Option<Serial>,
    presented: Option<Serial>,
}

impl ResizeSync {
    pub(super) fn request(&mut self, serial: Serial) {
        if serial.low != 0 || serial.high != 0 {
            self.requested = Some(serial);
        }
    }

    pub(super) fn configure(&mut self) -> bool {
        if let Some(serial) = self.requested.take() {
            self.configured = Some(serial);
            self.presented = None;
            true
        } else {
            false
        }
    }

    pub(super) fn pre_present(&mut self) {
        self.presented = self.configured;
    }

    pub(super) fn finish_frame(&mut self) -> Option<Serial> {
        let serial = self.presented.take()?;
        if self.configured == Some(serial) {
            self.configured = None;
        }
        Some(serial)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serial(value: i64) -> Serial {
        Serial::from_words(value, 0)
    }

    #[test]
    fn an_unconfigured_request_cannot_acknowledge_an_older_frame() {
        let mut state = ResizeSync::default();
        state.request(serial(1));
        state.pre_present();
        assert_eq!(state.finish_frame(), None);
        assert!(state.configure());
        state.pre_present();
        assert_eq!(state.finish_frame(), Some(serial(1)));
        assert_eq!(state.finish_frame(), None);
    }

    #[test]
    fn receiving_or_configuring_does_not_acknowledge_before_rendering() {
        let mut state = ResizeSync::default();
        assert!(!state.configure());
        state.request(serial(1));
        assert_eq!(state.finish_frame(), None);
        assert!(state.configure());
        assert_eq!(state.finish_frame(), None);
        state.pre_present();
        assert_eq!(state.finish_frame(), Some(serial(1)));
    }

    #[test]
    fn a_new_request_waits_for_its_own_configure_and_frame() {
        let mut state = ResizeSync::default();
        state.request(serial(1));
        assert!(state.configure());
        state.request(serial(2));
        state.pre_present();
        assert_eq!(state.finish_frame(), Some(serial(1)));
        assert_eq!(state.finish_frame(), None);
        assert!(state.configure());
        state.pre_present();
        assert_eq!(state.finish_frame(), Some(serial(2)));
    }

    #[test]
    fn coalesced_resizes_acknowledge_the_latest_rendered_size() {
        let mut state = ResizeSync::default();
        for value in 1..=20 {
            state.request(serial(value));
            assert!(state.configure());
        }
        assert_eq!(state.finish_frame(), None);
        state.pre_present();
        assert_eq!(state.finish_frame(), Some(serial(20)));
        state.pre_present();
        assert_eq!(state.finish_frame(), None);
    }

    #[test]
    fn a_superseded_frame_cannot_acknowledge_newer_geometry() {
        let mut state = ResizeSync::default();
        state.request(serial(1));
        assert!(state.configure());
        state.pre_present();
        state.request(serial(2));
        assert!(state.configure());
        assert_eq!(state.finish_frame(), None);
        state.pre_present();
        assert_eq!(state.finish_frame(), Some(serial(2)));
    }

    #[test]
    fn protocol_words_preserve_sign_and_low_bits_on_xlib_abis() {
        assert_eq!(
            Serial::from_words(-1, -1),
            Serial { low: u32::MAX, high: -1 }
        );
        assert_eq!(
            Serial::from_words(0xffff_ffff, 0x8000_0000),
            Serial { low: u32::MAX, high: i32::MIN }
        );
        assert_eq!(
            Serial::from_words(0x1234_5678, 0x7fff_ffff),
            Serial { low: 0x1234_5678, high: i32::MAX }
        );
    }

    #[test]
    fn an_invalid_zero_request_does_not_replace_valid_pending_geometry() {
        let mut state = ResizeSync::default();
        state.request(serial(0));
        assert!(!state.configure());
        state.request(serial(1));
        state.request(serial(0));
        assert!(state.configure());
        state.pre_present();
        assert_eq!(state.finish_frame(), Some(serial(1)));
    }

    #[test]
    fn move_events_and_ordinary_redraws_do_not_reuse_an_acknowledgement() {
        let mut state = ResizeSync::default();
        for _ in 0..20 {
            assert!(!state.configure());
            state.pre_present();
            assert_eq!(state.finish_frame(), None);
        }
        state.request(serial(5));
        assert!(state.configure());
        state.pre_present();
        assert_eq!(state.finish_frame(), Some(serial(5)));
        assert!(!state.configure());
        state.pre_present();
        assert_eq!(state.finish_frame(), None);
    }
}
