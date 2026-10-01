//! Presentation and preview pacing never replace game-state observations.

pub const FRAME_INTERVAL_MS: u64 = 200;
pub const REPEATED_PREVIEW_INTERVAL_MS: u64 = 1000;

pub struct WindowPresentation<T> {
    pub position: Option<(i32, i32)>,
    pub size: Option<(u32, u32)>,
    pub visible: Option<bool>,
    pub payload: Option<T>,
}

impl<T> Default for WindowPresentation<T> {
    fn default() -> Self {
        Self { position: None, size: None, visible: None, payload: None }
    }
}

impl<T: PartialEq> WindowPresentation<T> {
    pub fn position_changed(&self, position: (i32, i32)) -> bool {
        self.position != Some(position)
    }

    pub fn size_changed(&self, size: (u32, u32)) -> bool {
        self.size != Some(size)
    }

    pub fn visibility_changed(&self, visible: bool) -> bool {
        self.visible != Some(visible)
    }

    pub fn payload_changed(&self, payload: &T) -> bool {
        self.payload.as_ref() != Some(payload)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewKey {
    pub revision: u64,
    pub dimensions: (u32, u32),
    pub content: Option<[u64; 4]>,
    pub board: Option<[u64; 4]>,
    pub status: String,
    pub message: String,
}

#[derive(Default)]
pub struct PreviewThrottle {
    last_ms: Option<u64>,
    last_key: Option<PreviewKey>,
}

impl PreviewThrottle {
    pub fn needs_preview(&self, now: u64, key: &PreviewKey, force: bool) -> bool {
        force || self.last_key.as_ref() != Some(key)
            || self.last_ms.is_none_or(|last| now.saturating_sub(last) >= REPEATED_PREVIEW_INTERVAL_MS)
    }

    // Call only after the frame still passes the session/version checks.
    pub fn published(&mut self, now: u64, key: PreviewKey) {
        self.last_ms = Some(now);
        self.last_key = Some(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_preview() -> PreviewKey {
        PreviewKey {
            revision: 3,
            dimensions: (1920, 1080),
            content: Some([0, 0, 1920, 1080]),
            board: Some([900, 300, 900, 600]),
            status: "ready".into(),
            message: "棋盘已同步".into(),
        }
    }

    #[test]
    fn unchanged_watchdog_ticks_need_no_native_updates() {
        let window = WindowPresentation {
            position: Some((12, 34)), size: Some((500, 300)), visible: Some(true),
            payload: Some((3, false)),
        };
        for _ in 0..100 {
            assert!(!window.position_changed((12, 34)));
            assert!(!window.size_changed((500, 300)));
            assert!(!window.visibility_changed(true));
            assert!(!window.payload_changed(&(3, false)));
        }
    }

    #[test]
    fn movement_resize_and_same_version_style_changes_are_independent() {
        let window = WindowPresentation {
            position: Some((12, 34)), size: Some((500, 300)), visible: Some(true),
            payload: Some((3, false)),
        };
        assert!(window.position_changed((22, 34)));
        assert!(window.size_changed((1000, 600)));
        assert!(window.payload_changed(&(3, true)));
        assert!(window.payload_changed(&(4, false)));
        assert!(!window.visibility_changed(true));
    }

    #[test]
    fn hide_and_restore_do_not_require_a_new_result() {
        let mut window = WindowPresentation {
            position: Some((12, 34)), size: Some((500, 300)), visible: Some(true),
            payload: Some((3, false)),
        };
        assert!(window.visibility_changed(false));
        window.visible = Some(false);
        assert!(!window.visibility_changed(false));
        assert!(window.visibility_changed(true));
        assert!(!window.payload_changed(&(3, false)));
    }

    #[test]
    fn unsuccessful_native_updates_remain_retryable() {
        let window = WindowPresentation::<(u64, bool)>::default();
        // A failed native call must not populate the corresponding cache slot.
        assert!(window.position_changed((12, 34)));
        assert!(window.position_changed((12, 34)));
        assert!(window.size_changed((500, 300)));
        assert!(window.payload_changed(&(3, false)));
    }

    #[test]
    fn repeating_auto_previews_refresh_once_per_second() {
        let mut throttle = PreviewThrottle::default();
        let key = ready_preview();
        assert!(throttle.needs_preview(1000, &key, false));
        throttle.published(1000, key.clone());
        for now in [1200, 1400, 1600, 1800, 1999] {
            assert!(!throttle.needs_preview(now, &key, false));
        }
        assert!(throttle.needs_preview(2000, &key, false));
    }

    #[test]
    fn manual_refresh_and_resume_bypass_preview_pacing() {
        let mut throttle = PreviewThrottle::default();
        let key = ready_preview();
        throttle.published(1000, key.clone());
        assert!(throttle.needs_preview(1001, &key, true));
    }

    #[test]
    fn changed_status_version_or_geometry_gets_an_immediate_preview() {
        let mut throttle = PreviewThrottle::default();
        let key = ready_preview();
        throttle.published(1000, key.clone());
        let changes = [
            PreviewKey { revision: 4, ..key.clone() },
            PreviewKey { status: "uncertain".into(), ..key.clone() },
            PreviewKey { message: "需要校正".into(), ..key.clone() },
            PreviewKey { dimensions: (3840, 2160), ..key.clone() },
            PreviewKey { content: Some([0, 60, 1920, 1080]), ..key.clone() },
            PreviewKey { board: Some([1000, 300, 900, 600]), ..key.clone() },
        ];
        for changed in changes {
            assert!(throttle.needs_preview(1001, &changed, false));
        }
    }
}
