//! Geometry observations are separate from game rounds and pixel references.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct WindowGeometry {
    pub width: i32,
    pub height: i32,
    pub dpi: u32,
}

#[derive(Default, Debug)]
pub struct GeometryState {
    pub window: Option<WindowGeometry>,
    pub frame_size: Option<(u32, u32)>,
    pub content: Option<[f64; 4]>,
    frame_reads: u8,
    content_reads: u8,
}

impl GeometryState {
    pub fn observe_window(&mut self, window: WindowGeometry) -> bool {
        if self.window == Some(window) {
            return false;
        }
        self.window = Some(window);
        self.reset_frames();
        true
    }

    pub fn reset_frames(&mut self) {
        self.frame_size = None;
        self.frame_reads = 0;
        self.content = None;
        self.content_reads = 0;
    }

    pub fn observe_frame(&mut self, width: u32, height: u32) -> bool {
        if self.frame_size == Some((width, height)) {
            self.frame_reads = self.frame_reads.saturating_add(1);
            return false;
        }
        self.frame_size = Some((width, height));
        self.frame_reads = 1;
        self.content = None;
        self.content_reads = 0;
        true
    }

    pub fn frame_stable(&self) -> bool {
        self.frame_reads >= 2 && self.frame_size.is_some_and(|(w, h)| w > 0 && h > 0)
    }

    pub fn observe_content(&mut self, content: Option<[f64; 4]>) -> bool {
        // Ignore subpixel raster rounding, not motion of the game layout.
        let same = match (self.content, content) {
            (Some(a), Some(b)) => a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 1.0),
            (None, None) => true,
            _ => false,
        };
        if same {
            self.content_reads = self.content_reads.saturating_add(1);
        } else {
            self.content = content;
            self.content_reads = 1;
        }
        !same
    }

    pub fn ready(&self) -> bool {
        self.frame_stable() && self.content.is_some() && self.content_reads >= 2
    }

    pub fn content_settled(&self) -> bool {
        self.content_reads >= 2
    }
}

/// The application connection and game round outlive native WGC sessions.
/// A resize retires callbacks immediately; one worker may rebuild only after
/// window metadata settles. Its ownership survives subsequent resize events.
#[derive(Default, Debug)]
pub struct CaptureLifecycle {
    pub generation: u64,
    pub window: Option<WindowGeometry>,
    pub pending: bool,
    pub rebuilding: Option<u64>,
    pub failed: bool,
    changed_ms: u64,
}

impl CaptureLifecycle {
    pub const SETTLE_MS: u64 = 300;

    pub fn new(window: WindowGeometry, now: u64) -> Self {
        Self { generation: 1, window: Some(window), rebuilding: Some(1), changed_ms: now, ..Self::default() }
    }

    pub fn observe_window(&mut self, window: WindowGeometry, now: u64) -> bool {
        if self.window == Some(window) { return false; }
        let changed = self.window.is_some();
        self.window = Some(window);
        if changed { self.retire(now); }
        changed
    }

    pub fn retire(&mut self, now: u64) {
        self.generation += 1;
        self.changed_ms = now;
        self.pending = !self.failed;
    }

    pub fn claim(&mut self, now: u64) -> Option<(u64, WindowGeometry)> {
        if !self.pending || self.failed || self.rebuilding.is_some()
            || now.saturating_sub(self.changed_ms) < Self::SETTLE_MS {
            return None;
        }
        let window = self.window.filter(|w| w.width > 0 && w.height > 0)?;
        self.rebuilding = Some(self.generation);
        Some((self.generation, window))
    }

    pub fn release(&mut self, generation: u64) {
        if self.rebuilding == Some(generation) { self.rebuilding = None; }
    }

    pub fn installed(&mut self, generation: u64) -> bool {
        if self.generation != generation || self.failed { return false; }
        self.pending = false;
        self.release(generation);
        true
    }

    pub fn fail(&mut self, generation: u64) -> bool {
        if self.rebuilding != Some(generation) { return false; }
        // A failed stop may leave native resources alive. Do not start another
        // capture using this round's shared Recognizer until the user reconnects.
        self.failed = true;
        self.pending = false;
        self.rebuilding = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn window(width: i32, height: i32) -> WindowGeometry {
        WindowGeometry { width, height, dpi: 144 }
    }
    fn stable(g: &mut GeometryState, w: u32, h: u32, rect: [f64; 4]) {
        g.observe_frame(w, h);
        g.observe_frame(w, h);
        g.observe_content(Some(rect));
        g.observe_content(Some(rect));
        assert!(g.ready());
    }
    #[test]
    fn lifecycle_initial_start_and_unchanged_geometry_do_not_schedule_rebuilds() {
        let initial = window(1960, 1162);
        let mut capture = CaptureLifecycle::new(initial, 0);
        assert!(capture.claim(1000).is_none());
        assert!(capture.installed(1));
        assert!(!capture.observe_window(initial, 1100));
        assert!(capture.claim(2000).is_none());
        assert_eq!(capture.generation, 1);
        assert!(capture.observe_window(WindowGeometry { dpi: 192, ..initial }, 2100));
        assert!(capture.claim(2399).is_none());
        assert_eq!(capture.claim(2400).unwrap().0, 2);
    }

    #[test]
    fn resize_during_initial_start_waits_for_that_start_to_release_ownership() {
        let mut capture = CaptureLifecycle::new(window(1960, 1162), 0);
        capture.observe_window(window(3840, 2094), 100);
        assert!(capture.claim(1000).is_none());
        assert!(!capture.installed(1));
        capture.release(1);
        let (generation, _) = capture.claim(1000).unwrap();
        assert_eq!(generation, 2);
        assert!(capture.installed(generation));
        assert!(capture.claim(2000).is_none());
    }

    #[test]
    fn resize_storm_keeps_one_worker_until_the_old_session_has_joined() {
        let mut capture = CaptureLifecycle::new(window(1960, 1162), 0);
        capture.installed(1);
        capture.observe_window(window(2000, 1200), 100);
        let (first, _) = capture.claim(400).unwrap();
        capture.observe_window(window(2200, 1300), 450);
        capture.observe_window(window(2400, 1400), 500);
        assert!(capture.claim(900).is_none());
        assert!(!capture.installed(first));
        assert_eq!(capture.rebuilding, Some(first));
        capture.release(first);
        let (last, geometry) = capture.claim(900).unwrap();
        assert_eq!(geometry, window(2400, 1400));
        assert!(last > first);
        assert!(!capture.fail(first));
        assert!(capture.installed(last));
        assert!(capture.claim(5000).is_none());
    }

    #[test]
    fn native_failure_is_terminal_until_an_explicit_new_connection() {
        let mut capture = CaptureLifecycle::new(window(1960, 1162), 0);
        capture.installed(1);
        capture.retire(100);
        let (generation, _) = capture.claim(400).unwrap();
        assert!(capture.fail(generation));
        assert!(capture.failed);
        assert!(capture.claim(1000).is_none());
        capture.observe_window(window(3840, 2094), 1100);
        assert!(capture.claim(2000).is_none());
        assert!(!capture.installed(capture.generation));
        let mut fresh = CaptureLifecycle::new(window(3840, 2094), 2000);
        assert!(fresh.installed(1));
        assert!(!fresh.failed);
    }

    #[test]
    fn resize_and_dpi_changes_require_new_stable_frames() {
        let mut g = GeometryState::default();
        g.observe_window(window(1924, 1142));
        stable(&mut g, 1924, 1142, [2.0, 60.0, 1920.0, 1080.0]);
        assert!(g.observe_window(window(964, 602)));
        assert!(!g.ready());
        g.observe_frame(964, 602);
        assert!(!g.frame_stable());
        stable(&mut g, 964, 602, [2.0, 60.0, 960.0, 540.0]);
        assert!(g.observe_window(WindowGeometry { dpi: 192, ..window(964, 602) }));
        assert!(!g.ready());
    }
    #[test]
    fn same_frame_size_with_content_shift_invalidates_geometry() {
        let mut g = GeometryState::default();
        stable(&mut g, 1600, 1000, [100.0, 100.0, 1280.0, 720.0]);
        assert!(g.observe_content(Some([150.0, 100.0, 1280.0, 720.0])));
        assert!(!g.ready());
        assert!(!g.observe_content(Some([150.0, 100.0, 1280.0, 720.0])));
        assert!(g.ready());
        assert!(g.observe_content(None));
        assert!(!g.ready());
    }
    #[test]
    fn screen_position_is_not_part_of_geometry() {
        let mut g = GeometryState::default();
        g.observe_window(window(1924, 1142));
        stable(&mut g, 1924, 1142, [2.0, 60.0, 1920.0, 1080.0]);
        assert!(!g.observe_window(window(1924, 1142)));
        assert!(g.ready());
    }
}
