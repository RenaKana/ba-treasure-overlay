#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod ocr;
mod vision;
use vision::cover_reference::{self, CoverSample, SAMPLE_SIZE};
mod geometry_state;
use geometry_state::{CaptureLifecycle, GeometryState, WindowGeometry};
mod performance_state;
use performance_state::{PreviewKey, PreviewThrottle, WindowPresentation, FRAME_INTERVAL_MS};

use base64::Engine;
use image::{codecs::jpeg::JpegEncoder, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager, PhysicalPosition, PhysicalSize};
use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
    UI::HiDpi::GetDpiForWindow,
    UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, IsIconic, IsWindow,
        SetWindowLongPtrW, ShowWindow, GWL_EXSTYLE, SW_HIDE, SW_SHOWNOACTIVATE, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
    },
};
use windows_capture::{
    capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
    frame::Frame,
    graphics_capture_api::InternalCaptureControl,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
    window::Window,
};

type NativeError = Box<dyn std::error::Error + Send + Sync>;
type Control = CaptureControl<Capturer, NativeError>;
static COVER_TOKENS: AtomicU64 = AtomicU64::new(1);
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
impl Rect {
    fn array(self) -> [f64; 4] {
        [self.x, self.y, self.width, self.height]
    }
    fn from(a: [f64; 4]) -> Self {
        Self {
            x: a[0],
            y: a[1],
            width: a[2],
            height: a[3],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ItemSpec {
    width: u32,
    height: u32,
    remaining_count: i32,
}
#[derive(Clone, Serialize, Deserialize)]
struct Profile {
    // Legacy initial item totals are deliberately ignored during deserialization.
    board: Option<Rect>,
    #[serde(default)]
    update_mode: UpdateMode,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum UpdateMode {
    #[default]
    Manual,
    Auto,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            board: None,
            update_mode: UpdateMode::Manual,
        }
    }
}
#[derive(Clone, Serialize)]
struct CaptureState {
    session_id: u64,
    round_epoch: u64,
    revision: u64,
    status: String,
    message: String,
    frame_url: String,
    width: u32,
    height: u32,
    /// Absolute pixels in this captured frame; all image readers share it.
    content_rect_px: Option<[f64; 4]>,
    board: Option<Rect>,
    cells: Vec<String>,
    items: Vec<ItemSpec>,
    remaining: Option<u32>,
    round: Option<String>,
    confirmed: bool,
    update_mode: UpdateMode,
    refreshing: bool,
    captured_at_ms: Option<u64>,
    completed_objects: Vec<vision::CompletedObject>,
    candidate_constraints: Vec<vision::PlacementConstraint>,
    reference_ready: [bool; 3],
    card_fingerprints: [Option<String>; 3],
    finish: [bool; 3],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct InferredPlacement {
    item_index: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}
#[derive(Clone, Serialize, Deserialize)]
struct OverlayResult {
    session_id: u64,
    round_epoch: u64,
    revision: u64,
    probabilities: Vec<f64>,
    cells: Vec<String>,
    precision: String,
    message: String,
    inferred_placements: Vec<InferredPlacement>,
    #[serde(default)]
    emphasize_best: bool,
}
#[derive(Clone, PartialEq, Serialize)]
struct OverlayState {
    session_id: u64,
    round_epoch: u64,
    revision: u64,
    probabilities: Vec<f64>,
    cells: Vec<String>,
    precision: String,
    message: String,
    visible: bool,
    inferred_placements: Vec<InferredPlacement>,
    emphasize_best: bool,
}
#[derive(Clone, PartialEq, Serialize)]
struct RefreshControlState {
    session_id: u64,
    round_epoch: u64,
    revision: u64,
    refreshing: bool,
    enabled: bool,
    attention: bool,
    message: String,
}
struct Session {
    id: u64,
    round_epoch: u64,
    revision: u64,
    hwnd: i64,
    paused: bool,
    visible: bool,
    confirmed: bool,
    profile: Profile,
    items: Vec<ItemSpec>,
    item_correction: Option<(String, Vec<ItemSpec>)>,
    card_reading: String,
    recognizer: Arc<Mutex<vision::Recognizer>>,
    cover_samples: Vec<CoverSample>,
    cover_candidates: Option<CoverCandidates>,
    cover_selection: Option<CoverCandidates>,
    latest: Option<CaptureState>,
    result: Option<OverlayResult>,
    calculating: bool,
    pending: String,
    pending_frames: u32,
    published: String,
    round: Option<String>,
    pending_round: Option<String>,
    round_reads: u32,
    remaining: Option<u32>,
    started_ms: u64,
    last_frame_ms: u64,
    latencies: Vec<u64>,
    bootstrap_latencies: Vec<u64>,
    refresh_bootstrap: bool,
    last_measured_revision: u64,
    refresh_deadline_ms: Option<u64>,
    full_cover_reads: u32,
    seen_opened: bool,
    analysis_ms: u64,
    reference_initializing: bool,
    geometry: GeometryState,
    capture: CaptureLifecycle,
    no_frame_error_reported: bool,
}
fn unread_items() -> Vec<ItemSpec> {
    vec![
        ItemSpec {
            width: 0,
            height: 0,
            remaining_count: -1
        };
        3
    ]
}
impl Default for Session {
    fn default() -> Self {
        Self {
            id: 0,
            round_epoch: 0,
            revision: 0,
            hwnd: 0,
            paused: false,
            visible: true,
            confirmed: false,
            profile: Profile::default(),
            items: unread_items(),
            item_correction: None,
            card_reading: String::new(),
            recognizer: Arc::new(Mutex::new(vision::Recognizer::new())),
            cover_samples: Vec::new(),
            cover_candidates: None,
            cover_selection: None,
            latest: None,
            result: None,
            calculating: false,
            pending: String::new(),
            pending_frames: 0,
            published: String::new(),
            round: None,
            pending_round: None,
            round_reads: 0,
            remaining: None,
            started_ms: 0,
            last_frame_ms: 0,
            latencies: vec![],
            bootstrap_latencies: vec![],
            refresh_bootstrap: true,
            last_measured_revision: 0,
            refresh_deadline_ms: None,
            full_cover_reads: 0,
            seen_opened: false,
            analysis_ms: 0,
            reference_initializing: true,
            geometry: GeometryState::default(),
            capture: CaptureLifecycle::default(),
            no_frame_error_reported: false,
        }
    }
}
#[derive(Default)]
struct AppState {
    session: Mutex<Session>,
    // If both locks are needed, always acquire session before control. Never
    // call native start/stop/join while either lock is held.
    control: Mutex<Option<ActiveControl>>,
    presentation: Mutex<NativePresentation>,
    quitting: AtomicBool,
}
#[derive(Default)]
struct NativePresentation {
    overlay: WindowPresentation<OverlayState>,
    refresh: WindowPresentation<RefreshControlState>,
}
struct ActiveControl {
    session_id: u64,
    generation: u64,
    control: Control,
}

#[derive(Clone, Copy, Debug)]
struct CaptureTicket {
    session_id: u64,
    generation: u64,
    hwnd: i64,
    window: WindowGeometry,
}

impl CaptureTicket {
    fn current(self, s: &Session) -> bool {
        capture_current(s, self.session_id, self.generation, self.hwnd)
            && s.capture.window == Some(self.window)
    }

    fn can_install(self, s: &Session, occupied: bool) -> bool {
        self.current(s) && !occupied
    }

    fn owns_retired_control(self, session_id: u64, generation: u64) -> bool {
        session_id == self.session_id && generation < self.generation
    }
}

fn capture_current(s: &Session, id: u64, generation: u64, hwnd: i64) -> bool {
    s.id == id && s.hwnd == hwnd && hwnd != 0
        && s.capture.generation == generation && !s.capture.failed
}

fn observe_capture_window(s: &mut Session, window: WindowGeometry, now: u64) -> bool {
    let retired = s.capture.observe_window(window, now);
    s.geometry.observe_window(window) || retired
}

fn observe_capture_frame(s: &mut Session, width: u32, height: u32, now: u64) -> bool {
    if s.geometry.frame_size.is_some_and(|size| size != (width, height)) {
        // A surface-size change may precede the DWM metadata notification.
        s.capture.retire(now);
        s.geometry.reset_frames();
        true
    } else {
        s.geometry.observe_frame(width, height)
    }
}

fn release_capture_worker(s: &mut Session, ticket: CaptureTicket) {
    if s.id == ticket.session_id && s.hwnd == ticket.hwnd {
        s.capture.release(ticket.generation);
    }
}

#[derive(Serialize)]
struct WindowChoice {
    hwnd: i64,
    title: String,
}

fn foreground(hwnd: i64) -> bool {
    unsafe {
        IsWindow(Some(HWND(hwnd as *mut _))).as_bool()
            && !IsIconic(HWND(hwnd as *mut _)).as_bool()
            && GetForegroundWindow().0 as i64 == hwnd
    }
}
fn frame_bounds(hwnd: i64) -> RECT {
    unsafe {
        let mut r = RECT::default();
        if DwmGetWindowAttribute(
            HWND(hwnd as *mut _),
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut _ as _,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_err()
        {
            let _ = GetWindowRect(HWND(hwnd as *mut _), &mut r);
        }
        r
    }
}
fn window_geometry(hwnd: i64) -> WindowGeometry {
    let bounds = frame_bounds(hwnd);
    WindowGeometry {
        width: bounds.right - bounds.left,
        height: bounds.bottom - bounds.top,
        dpi: unsafe { GetDpiForWindow(HWND(hwnd as *mut _)) },
    }
}
fn geometry_current(s: &Session) -> bool {
    !s.capture.pending && !s.capture.failed && s.capture.rebuilding.is_none()
        && s.geometry.ready() && s.geometry.window == Some(window_geometry(s.hwnd))
}
fn window_minimized(hwnd: i64) -> bool {
    unsafe { IsIconic(HWND(hwnd as *mut _)).as_bool() }
}
fn hide_window(app: &tauri::AppHandle, label: &str) {
    let state = app.state::<AppState>();
    let mut presentation = state.presentation.lock().unwrap();
    let visible = match label {
        "overlay" => &mut presentation.overlay.visible,
        "refresh" => &mut presentation.refresh.visible,
        _ => return,
    };
    if *visible == Some(false) { return; }
    if let Some(w) = app.get_webview_window(label) {
        if let Ok(hwnd) = w.hwnd() {
            unsafe {
                let _ = ShowWindow(HWND(hwnd.0), SW_HIDE);
            }
            *visible = Some(false);
        }
    }
}
fn hide_overlay(app: &tauri::AppHandle) {
    hide_window(app, "overlay");
}
fn refresh_control_state(s: &Session) -> RefreshControlState {
    let refreshing = s.refresh_deadline_ms.is_some() || s.calculating;
    let attention = !refreshing
        && s.latest
            .as_ref()
            .is_some_and(|c| matches!(c.status.as_str(), "error" | "uncertain" | "waiting_item"));
    RefreshControlState {
        session_id: s.id,
        round_epoch: s.round_epoch,
        revision: s.revision,
        refreshing,
        enabled: s.hwnd != 0 && s.profile.update_mode == UpdateMode::Manual && !refreshing,
        attention,
        message: if refreshing {
            "正在刷新棋盘概率".into()
        } else if attention {
            format!("{}；点击重新识别", s.latest.as_ref().unwrap().message)
        } else if !s.confirmed {
            "刷新棋盘并读取本轮物品".into()
        } else {
            "刷新棋盘概率（不翻格、不换轮）".into()
        },
    }
}
fn refresh_button_offset(
    width: i32,
    height: i32,
    board: Option<Rect>,
    side: i32,
    gap: i32,
) -> (i32, i32) {
    // Place beside the grid, away from the game's own change-round button.
    // A first manual capture has no pixel-derived board yet, so use an
    // in-window fallback until the user explicitly requests that first frame.
    let left = board.map_or(0.473, |r| r.x);
    let top = board.map_or(0.303, |r| r.y);
    (
        ((width as f64 * left).round() as i32 - side - gap).clamp(0, (width - side).max(0)),
        (height as f64 * top)
            .round()
            .clamp(0.0, (height - side).max(0) as f64) as i32,
    )
}
fn present_refresh_control(app: &tauri::AppHandle, s: &Session) {
    let Some(w) = app.get_webview_window("refresh") else {
        return;
    };
    if !s.visible || s.profile.update_mode != UpdateMode::Manual || !foreground(s.hwnd) {
        hide_window(app, "refresh");
        return;
    }
    let bounds = frame_bounds(s.hwnd);
    let width = bounds.right - bounds.left;
    let height = bounds.bottom - bounds.top;
    let scale = w.scale_factor().unwrap_or(1.0);
    let side = (40.0 * scale).round() as i32;
    if width < side || height < side {
        hide_window(app, "refresh");
        return;
    }
    let board = s.latest.as_ref().and_then(|c| c.board);
    let (x, y) = refresh_button_offset(width, height, board, side, (10.0 * scale).round() as i32);
    let position = (bounds.left + x, bounds.top + y);
    let size = (side as u32, side as u32);
    let payload = refresh_control_state(s);
    let state = app.state::<AppState>();
    let mut presentation = state.presentation.lock().unwrap();
    let previous = &mut presentation.refresh;
    if previous.position_changed(position)
        && w.set_position(PhysicalPosition::new(position.0, position.1)).is_ok() {
        previous.position = Some(position);
    }
    if previous.size_changed(size) && w.set_size(tauri::LogicalSize::new(40.0, 40.0)).is_ok() {
        previous.size = Some(size);
    }
    if previous.payload_changed(&payload) && app.emit_to("refresh", "refresh-state", &payload).is_ok() {
        previous.payload = Some(payload);
    }
    if previous.visibility_changed(true) {
        if let Ok(hwnd) = w.hwnd() {
            unsafe {
                let _ = ShowWindow(HWND(hwnd.0), SW_SHOWNOACTIVATE);
            }
            previous.visible = Some(true);
        }
    }
}
fn refresh_overlay(app: &tauri::AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || present_overlay(&handle));
}
fn present_overlay(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    // Window calls run on the UI thread while the session remains locked. This
    // serializes validation + show with a capture invalidating that version.
    let guard = state.session.lock().unwrap();
    // This small interactive window remains available even when recognition
    // fails. The probability window itself stays entirely click-through.
    present_refresh_control(app, &guard);
    let s = &*guard;
    let valid = s.visible
        && !s.paused
        && geometry_current(s)
        && foreground(s.hwnd)
        && s.latest.as_ref().is_some_and(|c| c.status == "ready")
        && s.result.as_ref().is_some_and(|r| {
            r.session_id == s.id && r.round_epoch == s.round_epoch && r.revision == s.revision
        });
    if !valid {
        hide_overlay(app);
        return;
    }
    let (Some(result), Some(snap), Some(w)) = (s.result.as_ref(), s.latest.as_ref(), app.get_webview_window("overlay"))
    else {
        return;
    };
    let Some(board) = snap.board else {
        hide_overlay(app);
        return;
    };
    let bounds = frame_bounds(s.hwnd);
    let width = (bounds.right - bounds.left) as f64;
    let height = (bounds.bottom - bounds.top) as f64;
    let position = (
        bounds.left + (board.x * width).round() as i32,
        bounds.top + (board.y * height).round() as i32,
    );
    let size = (
        (board.width * width).round().max(1.0) as u32,
        (board.height * height).round().max(1.0) as u32,
    );
    let payload = OverlayState {
            session_id: result.session_id,
            round_epoch: result.round_epoch,
            revision: result.revision,
            probabilities: result.probabilities.clone(),
            cells: result.cells.clone(),
            precision: result.precision.clone(),
            message: if guard.profile.update_mode == UpdateMode::Manual {
                format!("手动快照 · {}", result.message)
            } else {
                result.message.clone()
            },
            visible: true,
            inferred_placements: result.inferred_placements.clone(),
            emphasize_best: result.emphasize_best,
        };
    let mut presentation = state.presentation.lock().unwrap();
    let previous = &mut presentation.overlay;
    if previous.position_changed(position)
        && w.set_position(PhysicalPosition::new(position.0, position.1)).is_ok() {
        previous.position = Some(position);
    }
    if previous.size_changed(size) && w.set_size(PhysicalSize::new(size.0, size.1)).is_ok() {
        previous.size = Some(size);
    }
    if previous.payload_changed(&payload) && app.emit_to("overlay", "overlay-state", &payload).is_ok() {
        previous.payload = Some(payload);
    }
    // Tao's normal show() uses SW_SHOW after its initial window creation and
    // can steal focus even though the overlay has WS_EX_NOACTIVATE.
    if previous.visibility_changed(true) {
        if let Ok(hwnd) = w.hwnd() {
            unsafe {
                let _ = ShowWindow(HWND(hwnd.0), SW_SHOWNOACTIVATE);
            }
            previous.visible = Some(true);
        }
    }
}
fn invalidate(s: &mut Session) {
    s.result = None;
    s.calculating = false;
    s.pending.clear();
    s.pending_frames = 0;
    s.published.clear();
    s.revision += 1;
}
fn changed_snapshot(s: &mut Session, status: &str, message: &str) -> CaptureState {
    invalidate(s);
    s.refresh_deadline_ms = None;
    let mut snapshot = s.latest.clone().unwrap_or_else(|| CaptureState {
        session_id: s.id,
        round_epoch: s.round_epoch,
        revision: s.revision,
        status: String::new(),
        message: String::new(),
        frame_url: String::new(),
        width: 0,
        height: 0,
        content_rect_px: None,
        board: None,
        cells: vec!["uncertain".into(); 45],
        items: s.items.clone(),
        remaining: None,
        round: None,
        confirmed: s.confirmed,
        update_mode: s.profile.update_mode,
        refreshing: false,
        captured_at_ms: None,
        completed_objects: vec![],
        candidate_constraints: vec![],
        reference_ready: [false; 3],
        card_fingerprints: [None, None, None],
        finish: [false; 3],
    });
    snapshot.revision = s.revision;
    snapshot.round_epoch = s.round_epoch;
    snapshot.status = status.into();
    snapshot.message = message.into();
    snapshot.items = s.items.clone();
    snapshot.confirmed = s.confirmed;
    snapshot.update_mode = s.profile.update_mode;
    snapshot.refreshing = false;
    s.latest = Some(snapshot.clone());
    snapshot
}
fn geometry_changed(s: &mut Session) -> CaptureState {
    s.cover_candidates = None;
    // Resizing cancels results, not a user-requested refresh or this game round.
    let deadline = s.refresh_deadline_ms.or_else(|| {
        (s.profile.update_mode == UpdateMode::Manual && s.calculating)
            .then_some(s.started_ms + 15000)
    });
    let idle_manual = s.profile.update_mode == UpdateMode::Manual && deadline.is_none();
    let (status, message) = if s.paused {
        ("paused", "已暂停；继续后重新定位游戏区域")
    } else if idle_manual {
        ("manual", "窗口大小已变化，请点击刷新重新定位")
    } else {
        ("searching", "窗口或游戏区域变化，等待画面稳定")
    };
    let mut snapshot = changed_snapshot(
        s,
        status,
        message,
    );
    s.refresh_deadline_ms = deadline;
    snapshot.refreshing = deadline.is_some();
    snapshot.board = None;
    snapshot.content_rect_px = None;
    snapshot.cells = vec!["uncertain".into(); 45];
    snapshot.remaining = None;
    snapshot.confirmed = false;
    snapshot.completed_objects.clear();
    snapshot.candidate_constraints.clear();
    snapshot.frame_url.clear();
    snapshot.captured_at_ms = None;
    s.latest = Some(snapshot.clone());
    snapshot
}
fn settings_changed(s: &mut Session) -> CaptureState {
    if s.profile.update_mode == UpdateMode::Manual {
        changed_snapshot(s, "manual", "设置已保存，点击刷新读取棋盘")
    } else {
        changed_snapshot(s, "searching", "等待棋盘更新")
    }
}
fn frame_requested(s: &Session) -> bool {
    s.cover_selection.is_none()
        && (s.profile.update_mode == UpdateMode::Auto || s.refresh_deadline_ms.is_some())
}
fn begin_manual_refresh(s: &mut Session, now: u64) -> Result<CaptureState, String> {
    if s.cover_selection.is_some() { return Err("正在选择未翻开样本".into()); }
    if s.profile.update_mode != UpdateMode::Manual {
        return Err("请先切换到手动模式".into());
    }
    if refresh_control_state(s).refreshing {
        return Err("正在刷新，请等待本次完成".into());
    }
    s.paused = false;
    s.started_ms = now;
    s.refresh_bootstrap = s.reference_initializing;
    let mut snapshot = changed_snapshot(s, "searching", "正在读取当前棋盘");
    s.refresh_deadline_ms = Some(now + 15000);
    snapshot.refreshing = true;
    s.latest = Some(snapshot.clone());
    Ok(snapshot)
}
fn expire_manual_refresh(s: &mut Session, now: u64) -> Option<CaptureState> {
    if s.profile.update_mode == UpdateMode::Manual
        && s.refresh_deadline_ms
            .is_some_and(|deadline| now >= deadline)
    {
        Some(changed_snapshot(
            s,
            "error",
            "刷新超时，请恢复游戏窗口后重试",
        ))
    } else {
        None
    }
}
fn missing_frame_snapshot(s: &mut Session, now: u64) -> Option<CaptureState> {
    if s.no_frame_error_reported || now.saturating_sub(s.last_frame_ms) <= 5000
        || s.latest.as_ref().is_some_and(|c| c.captured_at_ms.is_some()) {
        return None;
    }
    s.no_frame_error_reported = true;
    invalidate(s);
    let snap = CaptureState {
        session_id: s.id,
        round_epoch: s.round_epoch,
        revision: s.revision,
        status: "error".into(),
        message: "捕获尚未返回画面，请切回游戏或重新连接".into(),
        frame_url: String::new(),
        width: 0,
        height: 0,
        content_rect_px: None,
        board: None,
        cells: vec!["uncertain".into(); 45],
        items: s.items.clone(),
        remaining: None,
        round: None,
        confirmed: false,
        update_mode: s.profile.update_mode,
        refreshing: false,
        captured_at_ms: None,
        completed_objects: vec![],
        candidate_constraints: vec![],
        reference_ready: [false; 3],
        card_fingerprints: [None, None, None],
        finish: [false; 3],
    };
    s.latest = Some(snap.clone());
    Some(snap)
}
fn save_profile(app: &tauri::AppHandle, profile: &Profile) {
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(data) = serde_json::to_vec_pretty(profile) {
            let _ = std::fs::write(dir.join("profile.json"), data);
        }
    }
}
fn saved_profile(app: &tauri::AppHandle) -> Profile {
    app.path()
        .app_data_dir()
        .ok()
        .and_then(|p| std::fs::read(p.join("profile.json")).ok())
        .and_then(|b| serde_json::from_slice::<Profile>(&b).ok())
        .filter(|p| p.board.is_none_or(valid_rect))
        .unwrap_or_default()
}

#[derive(Clone)]
struct CoverCandidates {
    token: u64,
    session_id: u64,
    generation: u64,
    round_epoch: u64,
    samples: Vec<CoverSample>,
    images: Vec<String>,
}
#[derive(Serialize)]
struct CoverSelection {
    token: u64,
    images: Vec<String>,
}
#[derive(Serialize)]
struct CoverReferenceInfo {
    count: usize,
    images: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}
fn cover_reference_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path().app_data_dir().map(|dir| dir.join("cover-reference.json"))
        .map_err(|_| "未翻开样本文件无法读取，请清除或重新选择".into())
}
fn cover_images(samples: &[CoverSample]) -> Result<Vec<String>, String> {
    use image::ImageEncoder;
    samples.iter().map(|sample| {
        if !sample.valid() { return Err("未翻开样本文件无法读取，请清除或重新选择".into()); }
        let rgb: Vec<u8> = sample.rgb.iter().flatten().copied().collect();
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(&rgb, SAMPLE_SIZE as u32, SAMPLE_SIZE as u32, image::ExtendedColorType::Rgb8)
            .map_err(|_| "未翻开样本文件无法读取，请清除或重新选择".to_owned())?;
        Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
    }).collect()
}
fn cover_info(samples: &[CoverSample]) -> Result<CoverReferenceInfo, String> {
    Ok(CoverReferenceInfo { count: samples.len(), images: cover_images(samples)?, error: None })
}
#[tauri::command]
fn get_cover_reference(app: tauri::AppHandle) -> CoverReferenceInfo {
    let result = cover_reference_path(&app).and_then(|path| cover_reference::load(&path))
        .and_then(|samples| cover_info(&samples));
    result.unwrap_or_else(|error| CoverReferenceInfo { count: 0, images: Vec::new(), error: Some(error) })
}
fn freeze_cover_selection(s: &mut Session) -> Result<CoverSelection, String> {
    let candidate = s.cover_candidates.as_ref()
        .filter(|c| s.hwnd != 0 && c.session_id == s.id && c.generation == s.capture.generation
            && c.round_epoch == s.round_epoch)
        .ok_or("请先刷新游戏画面，再选择未翻开样本")?.clone();
    let response = CoverSelection { token: candidate.token, images: candidate.images.clone() };
    s.cover_selection = Some(candidate);
    Ok(response)
}
#[tauri::command]
fn begin_cover_selection(
    app: tauri::AppHandle,
    expected_session: u64,
    expected_round_epoch: u64,
    expected_revision: u64,
) -> Result<CoverSelection, String> {
    let state = app.state::<AppState>();
    let (selection, snapshot) = {
        let mut s = state.session.lock().unwrap();
        require_version(&s, expected_session, expected_round_epoch, expected_revision)?;
        let selection = freeze_cover_selection(&mut s)?;
        let snapshot = changed_snapshot(&mut s, "manual", "正在选择未翻开样本");
        (selection, snapshot)
    };
    hide_overlay(&app);
    let _ = app.emit_to("main", "capture-state", snapshot);
    refresh_overlay(&app);
    Ok(selection)
}
fn selected_cover_samples(s: &Session, token: u64, indices: &[usize]) -> Result<Vec<CoverSample>, String> {
    // Frozen appearance samples remain valid if capture restarts after a resize.
    // They no longer depend on the current frame's geometry or generation.
    let candidate = s.cover_selection.as_ref()
        .filter(|c| c.token == token && c.session_id == s.id
            && c.round_epoch == s.round_epoch && s.hwnd != 0)
        .ok_or("样本选择已失效，请重新选择")?;
    if indices.is_empty() { return Err("请选择至少一个未翻开的格子".into()); }
    let mut unique = indices.to_vec();
    unique.sort_unstable();
    unique.dedup();
    unique.into_iter().map(|index| candidate.samples.get(index).cloned()
        .ok_or_else(|| "样本选择已失效，请重新选择".to_owned())).collect()
}
fn replace_cover_samples(s: &mut Session, samples: Vec<CoverSample>) {
    s.recognizer = Arc::new(Mutex::new(vision::Recognizer::with_cover_samples(&samples)));
    s.cover_samples = samples;
    s.cover_selection = None;
    s.cover_candidates = None;
    s.reference_initializing = true;
}
fn cover_selection_finished(s: &mut Session, message: &str) -> CaptureState {
    let status = if s.paused { "paused" } else if s.profile.update_mode == UpdateMode::Manual { "manual" } else { "searching" };
    changed_snapshot(s, status, message)
}
#[tauri::command]
fn save_cover_selection(app: tauri::AppHandle, token: u64, indices: Vec<usize>) -> Result<CoverReferenceInfo, String> {
    let state = app.state::<AppState>();
    let (info, snapshot) = {
        let mut s = state.session.lock().unwrap();
        let samples = selected_cover_samples(&s, token, &indices)?;
        let info = cover_info(&samples)?;
        // Commit to disk before changing active references. A failed save
        // leaves both the old reference and the frozen selection available.
        cover_reference::save(&cover_reference_path(&app)?, &samples)?;
        replace_cover_samples(&mut s, samples);
        let snapshot = cover_selection_finished(&mut s, "未翻开样本已保存，请刷新棋盘");
        (info, snapshot)
    };
    hide_overlay(&app);
    let _ = app.emit_to("main", "capture-state", snapshot);
    refresh_overlay(&app);
    Ok(info)
}
#[tauri::command]
fn cancel_cover_selection(app: tauri::AppHandle, token: u64) -> Result<(), String> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut s = state.session.lock().unwrap();
        if s.cover_selection.is_none() { return Ok(()); }
        if s.cover_selection.as_ref().is_some_and(|c| c.token != token) {
            return Err("样本选择已失效，请重新选择".into());
        }
        s.cover_selection = None;
        cover_selection_finished(&mut s, "已取消样本选择，请刷新棋盘")
    };
    let _ = app.emit_to("main", "capture-state", snapshot);
    refresh_overlay(&app);
    Ok(())
}
#[tauri::command]
fn clear_cover_reference(app: tauri::AppHandle) -> Result<CoverReferenceInfo, String> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut s = state.session.lock().unwrap();
        cover_reference::save(&cover_reference_path(&app)?, &[])?;
        replace_cover_samples(&mut s, Vec::new());
        if s.hwnd != 0 { Some(cover_selection_finished(&mut s, "已清除固定样本，请刷新棋盘")) } else { None }
    };
    hide_overlay(&app);
    if let Some(snapshot) = snapshot { let _ = app.emit_to("main", "capture-state", snapshot); }
    refresh_overlay(&app);
    cover_info(&[])
}
fn item_fits_board(width: u32, height: u32) -> bool {
    width > 0 && height > 0
        && ((width <= vision::COLS as u32 && height <= vision::ROWS as u32)
            || (height <= vision::COLS as u32 && width <= vision::ROWS as u32))
}
fn valid_items(items: &[ItemSpec]) -> bool {
    items.len() == 3
        && items.iter().all(|i| {
            item_fits_board(i.width, i.height)
                && (0..=7).contains(&i.remaining_count)
        })
        && items
            .iter()
            .map(|i| i.width * i.height * i.remaining_count as u32)
            .sum::<u32>()
            <= (vision::COLS * vision::ROWS) as u32
}
// All covers must be accounted for before treating unresolved pixels as an
// opened region awaiting completion. This never invents an item's identity.
fn awaiting_item_completion(cells: &[String], remaining: Option<u32>) -> bool {
    cells.len() == vision::COLS * vision::ROWS
        && remaining.is_some_and(|n| {
            n as usize == cells.iter().filter(|cell| cell.as_str() == "unknown").count()
        })
        && cells.iter().any(|cell| matches!(cell.as_str(), "item0" | "item1" | "item2" | "uncertain"))
}
fn item_reading_error(items: &[ItemSpec]) -> String {
    if items.len() != 3 {
        return "未读全三张物品卡片，请刷新或校正".into();
    }
    let slots = |matches: fn(&ItemSpec) -> bool| {
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches(item))
            .map(|(index, _)| (index + 1).to_string())
            .collect::<Vec<_>>()
            .join("、")
    };
    let mut errors = Vec::new();
    let sizes = slots(|i| i.width == 0 || i.height == 0);
    if !sizes.is_empty() {
        errors.push(format!("未读到物品 {sizes} 的尺寸"));
    }
    let counts = slots(|i| i.remaining_count < 0);
    if !counts.is_empty() {
        errors.push(format!("未读到物品 {counts} 的剩余件数"));
    }
    let large = slots(|i| i.width > 0 && i.height > 0 && !item_fits_board(i.width, i.height));
    if !large.is_empty() {
        errors.push(format!("物品 {large} 的尺寸无法放入 9×5 棋盘（含旋转）"));
    }
    let excess = slots(|i| i.remaining_count > 7);
    if !excess.is_empty() {
        errors.push(format!("物品 {excess} 的剩余件数超出支持范围（0–7 件）"));
    }
    if errors.is_empty() {
        errors.push("物品总占格数超过 45 格".into());
    }
    format!("{}，请刷新或校正", errors.join("；"))
}
fn valid_rect(r: Rect) -> bool {
    r.array().iter().all(|v| v.is_finite())
        && r.x >= 0.0
        && r.y >= 0.0
        && r.width >= 0.1
        && r.height >= 0.1
        && r.x + r.width <= 1.001
        && r.y + r.height <= 1.001
}
#[tauri::command]
fn list_windows() -> Result<Vec<WindowChoice>, String> {
    Ok(Window::enumerate()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter_map(|w| {
            let title = w.title().ok()?;
            if title.is_empty() || title.starts_with("BA ") {
                None
            } else {
                Some(WindowChoice {
                    hwnd: w.as_raw_hwnd() as i64,
                    title,
                })
            }
        })
        .collect())
}
fn end_capture(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let old = {
        let mut s = state.session.lock().unwrap();
        s.id += 1;
        s.hwnd = 0;
        s.result = None;
        s.cover_candidates = None;
        s.cover_selection = None;
        // Detach atomically with invalidation; a late worker cannot take the
        // control of a subsequent user connection.
        state.control.lock().unwrap().take()
    };
    hide_overlay(app);
    hide_window(app, "refresh");
    if let Some(old) = old {
        std::thread::spawn(move || { let _ = old.control.stop(); });
    }
}
#[tauri::command]
fn stop_capture(app: tauri::AppHandle) {
    end_capture(&app)
}

fn create_native_capture(app: &tauri::AppHandle, ticket: CaptureTicket) -> Result<Control, String> {
    let settings = Settings::new(
        Window::from_raw_hwnd(ticket.hwnd as *mut _),
        CursorCaptureSettings::WithoutCursor,
        DrawBorderSettings::Default,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Rgba8,
        (app.clone(), ticket.session_id, ticket.generation, ticket.hwnd),
    );
    Capturer::start_free_threaded(settings).map_err(|e| format!("WGC 捕获失败：{e}"))
}

fn install_native_capture(app: &tauri::AppHandle, ticket: CaptureTicket, control: Control) -> bool {
    let state = app.state::<AppState>();
    let mut control = Some(control);
    let mut changed = None;
    let installed = {
        let mut s = state.session.lock().unwrap();
        if ticket.current(&s) && !state.quitting.load(Ordering::Relaxed) {
            if !window_minimized(ticket.hwnd)
                && observe_capture_window(&mut s, window_geometry(ticket.hwnd), now_ms()) {
                changed = Some(geometry_changed(&mut s));
            }
            if ticket.current(&s) {
                let mut slot = state.control.lock().unwrap();
                // No worker may overwrite another installed control, even if
                // a stale startup returns after stop or a newer connection.
                if ticket.can_install(&s, slot.is_some()) && s.capture.installed(ticket.generation) {
                    *slot = Some(ActiveControl {
                        session_id: ticket.session_id,
                        generation: ticket.generation,
                        control: control.take().unwrap(),
                    });
                    s.last_frame_ms = now_ms();
                    s.no_frame_error_reported = false;
                    true
                } else { false }
            } else { false }
        } else { false }
    };
    // Even a rejected start owns live resources. Explicitly stop and join it;
    // dropping CaptureControl alone detaches the native thread.
    if let Some(control) = control {
        if let Err(error) = control.stop() {
            rebuild_failure(app, ticket, error.to_string(), true);
        }
    }
    if let Some(snapshot) = changed { let _ = app.emit_to("main", "capture-state", snapshot); }
    installed
}

#[tauri::command]
fn start_capture(app: tauri::AppHandle, hwnd: i64) -> Result<u64, String> {
    if !unsafe { IsWindow(Some(HWND(hwnd as *mut _))).as_bool() } {
        return Err("目标窗口已关闭，请刷新窗口列表".into());
    }
    let cover_samples = cover_reference::load(&cover_reference_path(&app)?)?;
    let state = app.state::<AppState>();
    let (ticket, old) = {
        let mut s = state.session.lock().unwrap();
        let id = s.id + 1;
        let window = window_geometry(hwnd);
        let old = state.control.lock().unwrap().take();
        *s = Session {
            id, hwnd, last_frame_ms: now_ms(), profile: saved_profile(&app),
            recognizer: Arc::new(Mutex::new(vision::Recognizer::with_cover_samples(&cover_samples))),
            cover_samples,
            capture: CaptureLifecycle::new(window, now_ms()),
            ..Session::default()
        };
        (CaptureTicket { session_id: id, generation: s.capture.generation, hwnd, window }, old)
    };
    refresh_overlay(&app);
    if let Some(old) = old { let _ = old.control.stop(); }
    {
        let mut s = state.session.lock().unwrap();
        if !ticket.current(&s) {
            release_capture_worker(&mut s, ticket);
            return Err("连接请求已取消或窗口尺寸已变化".into());
        }
    }
    let control = create_native_capture(&app, ticket).map_err(|error| {
        let mut s = state.session.lock().unwrap();
        if ticket.current(&s) { s.hwnd = 0; }
        release_capture_worker(&mut s, ticket);
        error
    })?;
    let installed = install_native_capture(&app, ticket, control);
    let snapshot = {
        let mut s = state.session.lock().unwrap();
        release_capture_worker(&mut s, ticket);
        if installed && ticket.current(&s) && s.profile.update_mode == UpdateMode::Manual {
            Some(changed_snapshot(&mut s, "manual", "点击刷新读取当前棋盘"))
        } else { None }
    };
    if let Some(snapshot) = snapshot { let _ = app.emit_to("main", "capture-state", snapshot); }
    Ok(ticket.session_id)
}

fn capture_failure(
    s: &mut Session, ticket: CaptureTicket, error: &str, teardown_failed: bool,
) -> Option<CaptureState> {
    if s.id != ticket.session_id || s.hwnd != ticket.hwnd { return None; }
    if !teardown_failed && !ticket.current(s) {
        release_capture_worker(s, ticket);
        return None;
    }
    if !s.capture.fail(ticket.generation) { return None; }
    // A terminal failure finishes an explicit refresh with this error;
    // successful rebinds never change its original deadline.
    Some(changed_snapshot(s, "error", &format!("重新建立捕获失败，请重新连接：{error}")))
}

fn rebuild_failure(app: &tauri::AppHandle, ticket: CaptureTicket, error: String, teardown_failed: bool) {
    let state = app.state::<AppState>();
    let snapshot = capture_failure(&mut state.session.lock().unwrap(), ticket, &error, teardown_failed);
    if let Some(snapshot) = snapshot {
        let _ = app.emit_to("main", "capture-state", snapshot);
        refresh_overlay(app);
    }
}

fn rebuild_capture(app: tauri::AppHandle, ticket: CaptureTicket, old: Option<ActiveControl>) {
    // Join the retired callback thread before reusing the shared Recognizer.
    // A version check alone cannot undo mutations by already-running OCR/vision.
    if let Some(old) = old {
        if let Err(error) = old.control.stop() {
            rebuild_failure(&app, ticket, error.to_string(), true);
            return;
        }
    }
    let state = app.state::<AppState>();
    let proceed = {
        let mut s = state.session.lock().unwrap();
        if ticket.current(&s) && !window_minimized(ticket.hwnd)
            && observe_capture_window(&mut s, window_geometry(ticket.hwnd), now_ms()) {
            geometry_changed(&mut s);
        }
        ticket.current(&s) && s.capture.rebuilding == Some(ticket.generation)
            && !window_minimized(ticket.hwnd) && !state.quitting.load(Ordering::Relaxed)
    };
    if proceed {
        match create_native_capture(&app, ticket) {
            Ok(control) => { install_native_capture(&app, ticket, control); }
            Err(error) => {
                rebuild_failure(&app, ticket, error, false);
                return;
            }
        }
    }
    let mut s = state.session.lock().unwrap();
    // Later resizes keep pending=true. Release only after every local native
    // control has been stopped and joined, including a rejected startup.
    release_capture_worker(&mut s, ticket);
}

fn schedule_capture_rebuild(app: &tauri::AppHandle) {
    let state = app.state::<AppState>();
    let work = {
        let mut s = state.session.lock().unwrap();
        if s.hwnd == 0 || window_minimized(s.hwnd) || state.quitting.load(Ordering::Relaxed) { return; }
        let Some((generation, window)) = s.capture.claim(now_ms()) else { return; };
        let ticket = CaptureTicket { session_id: s.id, generation, hwnd: s.hwnd, window };
        let mut slot = state.control.lock().unwrap();
        // Only detach this connection's retired generation.
        let old = if slot.as_ref().is_some_and(|c| ticket.owns_retired_control(c.session_id, c.generation)) {
            slot.take()
        } else { None };
        s.geometry.reset_frames();
        (ticket, old)
    };
    let app = app.clone();
    std::thread::spawn(move || rebuild_capture(app, work.0, work.1));
}

#[tauri::command]
fn set_update_mode(
    app: tauri::AppHandle,
    mode: UpdateMode,
    expected_session: u64,
    expected_round_epoch: u64,
    expected_revision: u64,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let (profile, snapshot) = {
        let mut s = state.session.lock().unwrap();
        require_version(
            &s,
            expected_session,
            expected_round_epoch,
            expected_revision,
        )?;
        s.profile.update_mode = mode;
        s.paused = false;
        let snapshot = settings_changed(&mut s);
        (s.profile.clone(), snapshot)
    };
    save_profile(&app, &profile);
    hide_overlay(&app);
    let _ = app.emit_to("main", "capture-state", snapshot);
    Ok(())
}
#[tauri::command]
fn refresh_capture(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    expected_session: u64,
    expected_round_epoch: u64,
    expected_revision: u64,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut s = state.session.lock().unwrap();
        require_version(
            &s,
            expected_session,
            expected_round_epoch,
            expected_revision,
        )?;
        if !unsafe { IsWindow(Some(HWND(s.hwnd as *mut _))).as_bool() }
            || unsafe { IsIconic(HWND(s.hwnd as *mut _)).as_bool() }
        {
            return Err("请先恢复游戏窗口，再点击刷新".into());
        }
        let snapshot = begin_manual_refresh(&mut s, now_ms())?;
        if std::env::var_os("BA_CAPTURE_TRACE").is_some() {
            eprintln!(
                "{}",
                serde_json::json!({
                    "event":"manual-refresh", "at_ms":now_ms(), "source":window.label(),
                    "session_id":s.id, "revision":s.revision, "target_hwnd":s.hwnd,
                    "foreground_hwnd":unsafe { GetForegroundWindow().0 as i64 }
                })
            );
        }
        snapshot
    };
    hide_overlay(&app);
    let _ = app.emit_to("main", "capture-state", snapshot);
    refresh_overlay(&app);
    Ok(())
}
#[tauri::command]
fn set_paused(app: tauri::AppHandle, paused: bool) {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut s = state.session.lock().unwrap();
        s.paused = paused;
        if paused {
            changed_snapshot(&mut s, "paused", "已暂停")
        } else {
            settings_changed(&mut s)
        }
    };
    hide_overlay(&app);
    let _ = app.emit_to("main", "capture-state", snapshot);
}
#[tauri::command]
fn set_overlay_visible(app: tauri::AppHandle, visible: bool) {
    app.state::<AppState>().session.lock().unwrap().visible = visible;
    refresh_overlay(&app)
}
#[tauri::command]
fn clear_overlay(app: tauri::AppHandle) {
    {
        let state = app.state::<AppState>();
        let mut s = state.session.lock().unwrap();
        s.result = None;
        s.calculating = false;
    }
    hide_overlay(&app);
    refresh_overlay(&app);
}
#[tauri::command]
fn solver_started(
    app: tauri::AppHandle,
    session_id: u64,
    round_epoch: u64,
    revision: u64,
) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        let mut s = state.session.lock().unwrap();
        require_version(&s, session_id, round_epoch, revision)?;
        if s.latest.as_ref().is_none_or(|c| c.status != "ready") {
            return Err("棋盘尚未就绪".into());
        }
        s.calculating = true;
    }
    refresh_overlay(&app);
    Ok(())
}
#[tauri::command]
fn calibrate(
    app: tauri::AppHandle,
    rect: Rect,
    expected_session: u64,
    expected_round_epoch: u64,
    expected_revision: u64,
) -> Result<(), String> {
    if rect.array().iter().any(|v| !v.is_finite())
        || rect.x < 0.0
        || rect.y < 0.0
        || rect.width < 0.1
        || rect.height < 0.1
        || rect.x + rect.width > 1.001
        || rect.y + rect.height > 1.001
    {
        return Err("请在画面内框选完整棋盘".into());
    }
    let state = app.state::<AppState>();
    let (profile, snapshot) = {
        let mut s = state.session.lock().unwrap();
        require_version(
            &s,
            expected_session,
            expected_round_epoch,
            expected_revision,
        )?;
        let snap = s.latest.as_ref().ok_or("尚无捕获画面")?;
        if snap.width == 0 || snap.height == 0 {
            return Err("请先刷新取得画面，再框选棋盘".into());
        }
        let content = snap.content_rect_px
            .ok_or("尚未定位游戏内容区，请恢复清晰画面后刷新")?;
        // Keep existing 16:9 profiles in the board's center-anchored canvas.
        let [x, y, w, h] = vision::layout_region(content, vision::LayoutAnchor::Center);
        let r = Rect {
            x: (rect.x * snap.width as f64 - x) / w,
            y: (rect.y * snap.height as f64 - y) / h,
            width: rect.width * snap.width as f64 / w,
            height: rect.height * snap.height as f64 / h,
        };
        if r.x < 0.0 || r.y < 0.0 || r.x + r.width > 1.01 || r.y + r.height > 1.01 {
            return Err("棋盘必须位于游戏内容区域内".into());
        }
        s.profile.board = Some(r);
        s.recognizer = Arc::new(Mutex::new(vision::Recognizer::with_cover_samples(&s.cover_samples)));
        s.cover_candidates = None;
        s.cover_selection = None;
        s.reference_initializing = true;
        let snapshot = settings_changed(&mut s);
        (s.profile.clone(), snapshot)
    };
    save_profile(&app, &profile);
    hide_overlay(&app);
    let _ = app.emit_to("main", "capture-state", snapshot);
    Ok(())
}
#[tauri::command]
fn confirm_items(
    app: tauri::AppHandle,
    items: Vec<ItemSpec>,
    expected_session: u64,
    expected_round_epoch: u64,
    expected_revision: u64,
) -> Result<(), String> {
    if !valid_items(&items) {
        return Err("需要三类物品：尺寸为正整数且旋转后能放入9×5棋盘、剩余件数0–7，剩余面积合计不超过45格".into());
    }
    let state = app.state::<AppState>();
    let (profile, snapshot) = {
        let mut s = state.session.lock().unwrap();
        require_version(
            &s,
            expected_session,
            expected_round_epoch,
            expected_revision,
        )?;
        s.item_correction = Some((s.card_reading.clone(), items.clone()));
        s.items = items;
        s.confirmed = true;
        let snapshot = settings_changed(&mut s);
        (s.profile.clone(), snapshot)
    };
    save_profile(&app, &profile);
    hide_overlay(&app);
    let _ = app.emit_to("main", "capture-state", snapshot);
    Ok(())
}
#[tauri::command]
async fn correct_cell(
    app: tauri::AppHandle,
    index: usize,
    cell: String,
    expected_session: u64,
    expected_round_epoch: u64,
    expected_revision: u64,
) -> Result<(), String> {
    let work_app = app.clone();
    let snapshot = tauri::async_runtime::spawn_blocking(move || -> Result<CaptureState, String> {
        let state = work_app.state::<AppState>();
        let recognizer = {
            let s = state.session.lock().unwrap();
            require_version(
                &s,
                expected_session,
                expected_round_epoch,
                expected_revision,
            )?;
            Arc::clone(&s.recognizer)
        };
        let mut recognizer = recognizer.lock().unwrap();
        let mut s = state.session.lock().unwrap();
        require_version(
            &s,
            expected_session,
            expected_round_epoch,
            expected_revision,
        )?;
        recognizer.correct(index, &cell)?;
        if let Some(latest) = s.latest.as_mut() {
            if let Some(observed) = latest.cells.get_mut(index) {
                *observed = cell;
            }
        }
        Ok(settings_changed(&mut s))
    })
    .await
    .map_err(|e| e.to_string())??;
    hide_overlay(&app);
    let _ = app.emit_to("main", "capture-state", snapshot);
    Ok(())
}
fn clear_round(s: &mut Session) {
    s.round_epoch += 1;
    s.recognizer = Arc::new(Mutex::new(vision::Recognizer::with_cover_samples(&s.cover_samples)));
    s.cover_candidates = None;
    s.cover_selection = None;
    s.items = unread_items();
    s.item_correction = None;
    s.card_reading.clear();
    s.confirmed = false;
    s.remaining = None;
    s.seen_opened = false;
    s.full_cover_reads = 0;
    s.reference_initializing = true;
    s.refresh_bootstrap = true;
    s.latest = None;
    invalidate(s);
}
fn observe_round(s: &mut Session, round: Option<String>) {
    let Some(round) = round else {
        s.pending_round = None;
        s.round_reads = 0;
        return;
    };
    if s.round.is_none() {
        s.round = Some(round);
    } else if s.round.as_ref() != Some(&round) {
        if s.pending_round.as_ref() == Some(&round) {
            s.round_reads += 1;
        } else {
            s.pending_round = Some(round.clone());
            s.round_reads = 1;
        }
        if s.round_reads >= 2 {
            clear_round(s);
            s.round = Some(round);
            s.pending_round = None;
            s.round_reads = 0;
        }
    } else {
        s.pending_round = None;
        s.round_reads = 0;
    }
}
fn observe_full_cover(s: &mut Session, all_covered: bool) -> bool {
    if all_covered && s.seen_opened {
        s.full_cover_reads += 1;
    } else {
        s.full_cover_reads = 0;
    }
    if s.full_cover_reads >= 2 {
        clear_round(s);
        true
    } else {
        false
    }
}
fn full_cover_observed(analysis: &vision::Analysis, remaining: Option<u32>) -> bool {
    remaining == Some(45)
        && analysis.finish.iter().all(|f| !f)
        && (analysis.fresh_initial_grid
            || (analysis.present && analysis.cells.iter().all(|c| c == "unknown")))
}
fn update_card_items(
    s: &mut Session,
    shapes: &[[u32; 2]],
    counts: [Option<u32>; 3],
    finish: [bool; 3],
) {
    let signature = format!("{shapes:?}|{counts:?}|{finish:?}");
    if s.item_correction
        .as_ref()
        .is_some_and(|(raw, _)| raw != &signature)
    {
        s.item_correction = None;
    }
    s.card_reading = signature;
    s.items = (0..3)
        .map(|i| {
            let shape = shapes.get(i).copied().unwrap_or([0, 0]);
            ItemSpec {
                width: shape[0],
                height: shape[1],
                remaining_count: if finish[i] {
                    0
                } else {
                    counts[i].map_or(-1, |n| n as i32)
                },
            }
        })
        .collect();
    if let Some((_, items)) = &s.item_correction {
        s.items = items.clone();
    }
    s.confirmed = valid_items(&s.items);
}
fn require_version(s: &Session, id: u64, round_epoch: u64, revision: u64) -> Result<(), String> {
    if s.id != id || s.round_epoch != round_epoch || s.revision != revision || s.latest.is_none() {
        Err("棋盘已变化，请核对当前画面后重试".into())
    } else {
        Ok(())
    }
}
#[tauri::command]
fn reset_round(app: tauri::AppHandle) {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut s = state.session.lock().unwrap();
        clear_round(&mut s);
        s.round = None;
        s.pending_round = None;
        s.round_reads = 0;
        s.remaining = None;
        if let Some(latest) = s.latest.as_mut() {
            latest.cells = vec!["uncertain".into(); 45];
            latest.remaining = None;
            latest.round = None;
        }
        settings_changed(&mut s)
    };
    hide_overlay(&app);
    let _ = app.emit_to("main", "capture-state", snapshot);
}
#[tauri::command]
fn render_overlay(app: tauri::AppHandle, result: OverlayResult) -> Result<(), String> {
    let state = app.state::<AppState>();
    {
        let mut s = state.session.lock().unwrap();
        if result.session_id != s.id
            || !geometry_current(&s)
            || result.round_epoch != s.round_epoch
            || result.revision != s.revision
            || s.latest.as_ref().is_none_or(|c| c.status != "ready")
        {
            return Err("计算结果已过期".into());
        }
        if result.probabilities.len() != 45
            || result.cells.len() != 45
            || result
                .probabilities
                .iter()
                .any(|p| !p.is_finite() || *p < 0.0 || *p > 1.0)
            || result.cells != s.latest.as_ref().unwrap().cells
            || result.inferred_placements.iter().any(|p| {
                !s.latest
                    .as_ref()
                    .unwrap()
                    .candidate_constraints
                    .iter()
                    .any(|c| {
                        c.item_index == p.item_index
                            && c.placements.len() == 1
                            && c.placements[0].x == p.x
                            && c.placements[0].y == p.y
                            && c.placements[0].width == p.width
                            && c.placements[0].height == p.height
                    })
            })
        {
            return Err("概率或棋盘数据不一致".into());
        }
        s.result = Some(result);
        s.calculating = false;
    }
    refresh_overlay(&app);
    Ok(())
}
#[tauri::command]
fn solver_failed(
    app: tauri::AppHandle,
    session_id: u64,
    round_epoch: u64,
    revision: u64,
    message: String,
) {
    let snapshot = {
        let state = app.state::<AppState>();
        let mut s = state.session.lock().unwrap();
        if require_version(&s, session_id, round_epoch, revision).is_err() {
            return;
        }
        changed_snapshot(
            &mut s,
            "error",
            &format!("计算失败：{message}；可校正或重试"),
        )
    };
    let _ = app.emit_to("main", "capture-state", snapshot);
    refresh_overlay(&app);
}
#[tauri::command]
fn overlay_painted(app: tauri::AppHandle, session_id: u64, round_epoch: u64, revision: u64) {
    let state = app.state::<AppState>();
    let diagnostics = {
        let mut s = state.session.lock().unwrap();
        if session_id != s.id
            || round_epoch != s.round_epoch
            || revision != s.revision
            || !s.visible
            || !foreground(s.hwnd)
            || !geometry_current(&s)
            || s.result.is_none()
            || s.latest.as_ref().is_none_or(|c| c.status != "ready")
        {
            return;
        }
        if s.last_measured_revision != revision {
            let elapsed = now_ms().saturating_sub(s.started_ms);
            let bootstrap = s.refresh_bootstrap;
            let samples = if bootstrap {
                &mut s.bootstrap_latencies
            } else {
                &mut s.latencies
            };
            samples.push(elapsed);
            if samples.len() > 100 {
                samples.remove(0);
            }
            s.last_measured_revision = s.revision;
        }
        let mut ordered = s.latencies.clone();
        ordered.sort_unstable();
        let p95 = ordered
            .get((ordered.len() * 95).div_ceil(100).saturating_sub(1))
            .copied();
        serde_json::json!({"backend":"Windows.Graphics.Capture","session_id":s.id,"capture_generation":s.capture.generation,"revision":s.revision,"round_epoch":s.round_epoch,"analysis_ms":s.analysis_ms,"update_mode":s.profile.update_mode,"measurement":"manual: refresh request to paint; auto: first changed frame to paint; includes focus waiting when game is not foreground","bootstrap_to_paint_ms":s.bootstrap_latencies,"observed_to_paint_ms":s.latencies,"p95_ms":p95,"sample_count":ordered.len(),"round":s.round,"remaining":s.remaining,"cells":s.latest.as_ref().map(|c|&c.cells),"items":s.items,"frame_size":s.geometry.frame_size,"window_geometry":s.geometry.window,"content_rect_px":s.latest.as_ref().and_then(|c|c.content_rect_px),"geometry_stable":s.geometry.ready(),"true_exclusive_fullscreen_verified":false})
    };
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::write(
            dir.join("session-diagnostics.json"),
            serde_json::to_vec_pretty(&diagnostics).unwrap(),
        );
    }
}

fn record_capture_diagnostics(app: &tauri::AppHandle, s: &Session, snap: &CaptureState, stage: &str) {
    let data = serde_json::json!({
        "at_ms": now_ms(), "session_id": s.id, "capture_generation": s.capture.generation,
        "capture_pending": s.capture.pending, "capture_rebuilding": s.capture.rebuilding,
        "capture_failed": s.capture.failed, "round_epoch": s.round_epoch,
        "revision": s.revision, "status": snap.status, "message": snap.message,
        "frame_size": [snap.width, snap.height], "window_geometry": s.geometry.window,
        "content_rect_px": snap.content_rect_px, "geometry_stable": s.geometry.ready(),
        "failure_stage": if snap.status == "ready" { None } else { Some(stage) },
        "board": snap.board, "remaining": snap.remaining, "items": snap.items,
        "update_mode": snap.update_mode, "analysis_ms": s.analysis_ms,
    });
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = std::fs::write(dir.join("capture-diagnostics.json"), serde_json::to_vec_pretty(&data).unwrap());
    }
}

struct Capturer {
    app: tauri::AppHandle,
    id: u64,
    generation: u64,
    hwnd: i64,
    last: Instant,
    last_ocr: Instant,
    hud: ocr::Hud,
    active: bool,
    preview: PreviewThrottle,
}
fn encode_preview(img: &RgbaImage) -> Result<String, NativeError> {
    let ratio = (960.0 / img.width() as f64).min(600.0 / img.height() as f64);
    let preview = image::imageops::resize(
        img,
        (img.width() as f64 * ratio).round().max(1.0) as u32,
        (img.height() as f64 * ratio).round().max(1.0) as u32,
        image::imageops::FilterType::Triangle,
    );
    let preview = image::DynamicImage::ImageRgba8(preview).to_rgb8();
    let mut jpg = vec![];
    JpegEncoder::new_with_quality(&mut jpg, 72).encode_image(&preview)?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpg)
    ))
}

impl Capturer {
    fn publish_snapshot(
        &mut self,
        mut snap: CaptureState,
        img: &RgbaImage,
        force_preview: bool,
        observed_ms: u64,
        work_started: Instant,
        stage: &str,
    ) -> Result<(), NativeError> {
        let key = PreviewKey {
            revision: snap.revision,
            dimensions: (snap.width, snap.height),
            content: snap.content_rect_px.map(|r| r.map(f64::to_bits)),
            board: snap.board.map(|r| [r.x, r.y, r.width, r.height].map(f64::to_bits)),
            status: snap.status.clone(),
            message: snap.message.clone(),
        };
        let update_preview = self.preview.needs_preview(
            observed_ms, &key, force_preview || snap.frame_url.is_empty(),
        );
        let profile_board = {
            let state = self.app.state::<AppState>();
            let s = state.session.lock().unwrap();
            s.profile.board.map(Rect::array)
        };
        // Build lossless normalized tiles from this same raw frame, never
        // from the downscaled JPEG preview. An unrecognized board can still
        // provide header/card-validated proposals for the player to inspect.
        let cover_candidates = if update_preview {
            vision::sample_cover_candidates(img, snap.content_rect_px, profile_board)
                .map(|samples| cover_images(&samples).map(|images| (samples, images)))
                .transpose()?
        } else { None };
        if update_preview {
            // Resize from the source reference; avoid cloning a full WGC frame.
            snap.frame_url = encode_preview(img)?;
        }
        let state = self.app.state::<AppState>();
        {
            let mut s = state.session.lock().unwrap();
            // Encoding runs outside the session lock. Never publish after an
            // edit, pause, resize, stop, or a newer user connection invalidates it.
            if !capture_current(&s, self.id, self.generation, self.hwnd)
                || s.revision != snap.revision || s.round_epoch != snap.round_epoch
                || s.paused || s.cover_selection.is_some()
                || s.geometry.window != Some(window_geometry(self.hwnd)) {
                return Ok(());
            }
            s.analysis_ms = work_started.elapsed().as_millis() as u64;
            if std::env::var_os("BA_CAPTURE_TRACE").is_some() {
                eprintln!(
                    "{}",
                    serde_json::json!({"event":"capture-state", "at_ms":now_ms(),
                        "session_id":s.id,"capture_generation":s.capture.generation,
                        "capture_pending":s.capture.pending,"capture_rebuilding":s.capture.rebuilding,
                        "round_epoch":s.round_epoch,"revision":s.revision,
                        "status":snap.status,"message":snap.message,"remaining":snap.remaining,
                        "items":snap.items,"round":snap.round,"cells":snap.cells,
                        "analysis_ms":s.analysis_ms,"reference_initializing":s.reference_initializing,
                        "frame_size":[snap.width,snap.height],"content_rect_px":snap.content_rect_px,
                        "window_geometry":s.geometry.window,"geometry_stable":s.geometry.ready(),
                        "update_mode":snap.update_mode,"refreshing":snap.refreshing,
                        "target_hwnd":self.hwnd,"foreground_hwnd":unsafe {GetForegroundWindow().0 as i64}})
                );
            }
            record_capture_diagnostics(&self.app, &s, &snap, stage);
            if update_preview {
                s.cover_candidates = cover_candidates.map(|(samples, images)| CoverCandidates {
                    token: COVER_TOKENS.fetch_add(1, Ordering::Relaxed),
                    session_id: s.id,
                    generation: s.capture.generation,
                    round_epoch: s.round_epoch,
                    samples,
                    images,
                });
            }
            s.latest = Some(snap.clone());
        }
        if update_preview { self.preview.published(observed_ms, key); }
        let _ = self.app.emit_to("main", "capture-state", snap);
        refresh_overlay(&self.app);
        Ok(())
    }
}
impl GraphicsCaptureApiHandler for Capturer {
    type Flags = (tauri::AppHandle, u64, u64, i64);
    type Error = NativeError;
    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            app: ctx.flags.0,
            id: ctx.flags.1,
            generation: ctx.flags.2,
            hwnd: ctx.flags.3,
            last: Instant::now() - Duration::from_secs(2),
            last_ocr: Instant::now() - Duration::from_secs(2),
            hud: ocr::Hud::default(),
            active: false,
            preview: PreviewThrottle::default(),
        })
    }
    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.last.elapsed() < Duration::from_millis(FRAME_INTERVAL_MS) {
            return Ok(());
        }
        self.last = Instant::now();
        let observed_ms = now_ms();
        let state = self.app.state::<AppState>();
        let (width, height) = (frame.width(), frame.height());
        let (manual, expected_revision, active) = {
            let mut s = state.session.lock().unwrap();
            if !capture_current(&s, self.id, self.generation, self.hwnd) {
                control.stop();
                return Ok(());
            }
            s.last_frame_ms = observed_ms;
            s.no_frame_error_reported = false;
            // Minimize is a visibility change, not a new game resolution.
            if window_minimized(self.hwnd) {
                self.active = false;
                return Ok(());
            }
            let window_changed = observe_capture_window(&mut s, window_geometry(self.hwnd), observed_ms);
            let frame_changed = if capture_current(&s, self.id, self.generation, self.hwnd) {
                observe_capture_frame(&mut s, width, height, observed_ms)
            } else { false };
            if window_changed || frame_changed {
                let snapshot = geometry_changed(&mut s);
                record_capture_diagnostics(&self.app, &s, &snapshot, "geometry");
                let _ = self.app.emit_to("main", "capture-state", snapshot);
                refresh_overlay(&self.app);
            }
            if !capture_current(&s, self.id, self.generation, self.hwnd) {
                self.active = false;
                return Ok(());
            }
            if !frame_requested(&s) {
                // Manual idle frames never enter pixel copy, OCR or recognition.
                self.active = false;
                return Ok(());
            }
            if !s.geometry.frame_stable() {
                return Ok(());
            }
            let manual = s.profile.update_mode == UpdateMode::Manual;
            (
                manual,
                s.revision,
                !s.paused && (manual || foreground(self.hwnd)),
            )
        };
        let resumed = active && !self.active;
        self.active = active;
        if !active {
            return Ok(());
        }
        let buffer = frame.buffer()?;
        let mut scratch = Vec::new();
        let raw = buffer.as_nopadding_buffer(&mut scratch).to_vec();
        let Some(img) = RgbaImage::from_raw(width, height, raw) else {
            return Err("无效的 WGC RGBA 帧".into());
        };
        let work_started = Instant::now();
        let locator = {
            let s = state.session.lock().unwrap();
            if !capture_current(&s, self.id, self.generation, self.hwnd)
                || s.revision != expected_revision
                || !frame_requested(&s)
            {
                return Ok(());
            }
            Arc::clone(&s.recognizer)
        };
        let content = locator.lock().unwrap().locate_content(&img);
        let (content, expected_revision, work_window) = {
            let mut s = state.session.lock().unwrap();
            if !capture_current(&s, self.id, self.generation, self.hwnd) || s.revision != expected_revision || !frame_requested(&s) {
                return Ok(());
            }
            if !window_minimized(self.hwnd)
                && observe_capture_window(&mut s, window_geometry(self.hwnd), now_ms()) {
                let snapshot = geometry_changed(&mut s);
                drop(s);
                let _ = self.app.emit_to("main", "capture-state", snapshot);
                refresh_overlay(&self.app);
                return Ok(());
            }
            if s.geometry.observe_content(content) {
                geometry_changed(&mut s);
            }
            if !s.geometry.ready() {
                let mut snap = s.latest.clone().unwrap();
                snap.width = width;
                snap.height = height;
                snap.content_rect_px = content;
                snap.captured_at_ms = Some(observed_ms);
                if content.is_none() && s.geometry.content_settled() {
                    snap.status = "uncertain".into();
                    snap.message = if s.cover_samples.is_empty() {
                        "未定位到清晰的游戏内容区，请检查遮挡或窗口尺寸"
                    } else {
                        "未翻开样本不匹配，请检查画面或更新样本"
                    }.into();
                    snap.refreshing = false;
                    s.refresh_deadline_ms = None;
                }
                drop(s);
                return self.publish_snapshot(snap, &img, manual || resumed, observed_ms, work_started, "geometry");
            }
            // Use this frame's measured transform for every reader.
            (content.unwrap(), s.revision, s.geometry.window)
        };
        // Numbers and pixels must come from the same frame, including immediately
        // after finishing an object. No old OCR count is paired with fresh art.
        self.hud = ocr::read_in_content(&img, content);
        self.last_ocr = Instant::now();
        let (recognizer, calibration, work_revision, work_epoch) = {
            let mut s = state.session.lock().unwrap();
            if !capture_current(&s, self.id, self.generation, self.hwnd) || s.revision != expected_revision || !frame_requested(&s) {
                return Ok(());
            }
            if !window_minimized(self.hwnd)
                && observe_capture_window(&mut s, window_geometry(self.hwnd), now_ms()) {
                let snapshot = geometry_changed(&mut s);
                drop(s);
                let _ = self.app.emit_to("main", "capture-state", snapshot);
                refresh_overlay(&self.app);
                return Ok(());
            }
            observe_round(&mut s, self.hud.round.clone());
            let calibration = s.profile.board.map(|r| {
                let [x, y, w, h] = vision::layout_region(content, vision::LayoutAnchor::Center);
                [
                    (x + r.x * w) / width as f64,
                    (y + r.y * h) / height as f64,
                    r.width * w / width as f64,
                    r.height * h / height as f64,
                ]
            });
            (
                Arc::clone(&s.recognizer),
                calibration,
                s.revision,
                s.round_epoch,
            )
        };
        // Expensive image work never holds the UI/session lock. Edits, stop,
        // refresh timeouts and new sessions can invalidate the result meanwhile.
        let analysis = recognizer.lock().unwrap().analyze_completed_in_content(
            &img,
            content,
            calibration,
            self.hud.remaining,
            self.hud.counts,
        );
        let snapshot = {
            let mut s = state.session.lock().unwrap();
            if !capture_current(&s, self.id, self.generation, self.hwnd)
                || s.revision != work_revision
                || s.round_epoch != work_epoch
                || !frame_requested(&s)
                || s.paused
                || s.geometry.window != work_window
                || work_window != Some(window_geometry(self.hwnd))
                || !s.geometry.ready()
            {
                return Ok(());
            }
            s.last_frame_ms = now_ms();
            s.analysis_ms = work_started.elapsed().as_millis() as u64;
            let all_covered = full_cover_observed(&analysis, self.hud.remaining);
            if observe_full_cover(&mut s, all_covered) {
                // Do not publish analysis built with the previous round's cache.
                let mut snap = changed_snapshot(&mut s, "searching", "新轮次，重新读取卡片");
                if manual {
                    s.refresh_deadline_ms = Some(now_ms() + 15000);
                    snap.refreshing = true;
                    s.latest = Some(snap.clone());
                }
                drop(s);
                let _ = self.app.emit_to("main", "capture-state", snap);
                refresh_overlay(&self.app);
                return Ok(());
            }
            if analysis.present && self.hud.remaining.is_some_and(|n| n < 45) {
                s.seen_opened = true;
            }
            update_card_items(&mut s, &analysis.shapes, self.hud.counts, analysis.finish);
            let candidate = format!(
                "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{width}x{height}",
                analysis.cells,
                s.items,
                self.hud.remaining,
                self.hud.round,
                analysis.completed_objects,
                analysis.candidate_constraints
            );
            if resumed {
                s.pending.clear();
                s.pending_frames = 0;
            }
            if s.pending != candidate {
                s.pending = candidate.clone();
                s.pending_frames = 1;
                if !manual {
                    s.started_ms = observed_ms;
                    s.refresh_bootstrap = s.reference_initializing;
                }
            } else {
                s.pending_frames = s.pending_frames.saturating_add(1);
            }
            let opened = analysis
                .cells
                .iter()
                .filter(|c| c.as_str() != "unknown")
                .count() as u32;
            let (mut status, mut message) = if !analysis.present {
                ("uncertain", analysis.message.clone())
            } else if !s.confirmed {
                ("uncertain", item_reading_error(&s.items))
            } else if s.pending_round.is_some() {
                ("uncertain", "正在确认轮次，暂停推荐".into())
            } else if self.hud.remaining.is_none() {
                ("uncertain", "未读到剩余格数，等待清晰画面".into())
            } else if awaiting_item_completion(&analysis.cells, self.hud.remaining) {
                ("waiting_item", if manual {
                    "已翻区域尚未确认完整，暂停概率；翻完物品后点击刷新".into()
                } else {
                    "已翻区域尚未确认完整，暂停概率；翻完物品后自动恢复".into()
                })
            } else if analysis.cells.iter().any(|c| c == "uncertain") {
                ("uncertain", if analysis.manual_cover_mismatch {
                    "未翻开样本不匹配，请检查画面或更新样本".into()
                } else {
                    "有格子待确认，请在预览中校正".into()
                })
            } else if opened != 45 - self.hud.remaining.unwrap() {
                ("uncertain", "翻格数量与画面尚未一致".into())
            } else if s.remaining.is_some_and(|n| self.hud.remaining.unwrap() > n) {
                ("uncertain", "棋盘恢复过程中，等待新轮次稳定".into())
            } else if s.pending_frames < 2 {
                ("searching", "等待画面稳定".into())
            } else {
                ("ready", "棋盘已同步".into())
            };
            if manual {
                if s.refresh_deadline_ms.is_some_and(|d| now_ms() >= d) {
                    s.refresh_deadline_ms = None;
                    status = "error";
                    message = "画面未能稳定，请等动画结束后重新刷新".into();
                } else if s.pending_frames >= 2 {
                    s.refresh_deadline_ms = None;
                    if status == "ready" {
                        message = "手动快照已更新；翻格后请再次刷新".into();
                    }
                }
            }
            if status == "ready" {
                s.remaining = self.hud.remaining;
                if opened > 0 {
                    s.seen_opened = true;
                }
            }
            let signature = format!("{status}|{candidate}");
            if s.published != signature {
                s.published = signature;
                s.revision += 1;
                s.result = None;
            }
            let snap = CaptureState {
                session_id: s.id,
                round_epoch: s.round_epoch,
                revision: s.revision,
                status: status.into(),
                message,
                frame_url: s.latest.as_ref().map_or_else(String::new, |c| c.frame_url.clone()),
                width,
                height,
                content_rect_px: Some(content),
                board: analysis.board.map(Rect::from),
                cells: analysis.cells,
                items: s.items.clone(),
                remaining: self.hud.remaining,
                round: s.round.clone(),
                confirmed: s.confirmed,
                update_mode: s.profile.update_mode,
                refreshing: s.refresh_deadline_ms.is_some(),
                captured_at_ms: Some(observed_ms),
                completed_objects: analysis.completed_objects,
                candidate_constraints: analysis.candidate_constraints,
                reference_ready: analysis.reference_ready,
                card_fingerprints: analysis.card_fingerprints,
                finish: analysis.finish,
            };
            s.reference_initializing = !snap
                .reference_ready
                .iter()
                .zip(snap.finish)
                .all(|(r, f)| *r || f);
            let stage = if !analysis.present { "vision" }
                else if self.hud.remaining.is_none() || !s.confirmed { "ocr_or_cards" }
                else { "observation" };
            (snap, stage)
        };
        self.publish_snapshot(snapshot.0, &img, manual || resumed, observed_ms, work_started, snapshot.1)
    }
    fn on_closed(&mut self) -> Result<(), Self::Error> {
        let state = self.app.state::<AppState>();
        let snap = {
            let mut s = state.session.lock().unwrap();
            if !capture_current(&s, self.id, self.generation, self.hwnd) {
                return Ok(());
            }
            s.capture.failed = true;
            s.capture.pending = false;
            invalidate(&mut s);
            s.refresh_deadline_ms = None;
            let revision = s.revision;
            s.latest.as_mut().map(|c| {
                c.revision = revision;
                c.status = "error".into();
                c.message = "捕获窗口已关闭，请重新选择".into();
                c.refreshing = false;
                c.clone()
            })
        };
        if let Some(c) = snap {
            let _ = self.app.emit_to("main", "capture-state", c);
        }
        refresh_overlay(&self.app);
        Ok(())
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;

    #[test]
    fn no_frame_timeout_publishes_once_until_native_frames_resume() {
        let mut session = Session::default();
        session.profile.update_mode = UpdateMode::Auto;
        session.last_frame_ms = 1000;
        assert!(missing_frame_snapshot(&mut session, 6000).is_none());
        let first = missing_frame_snapshot(&mut session, 6001).unwrap();
        assert_eq!(first.status, "error");
        let revision = session.revision;
        for now in (6100..10000).step_by(100) {
            assert!(missing_frame_snapshot(&mut session, now).is_none());
        }
        assert_eq!(session.revision, revision);
        // The first native callback/rebind resets this latch and watchdog time.
        session.last_frame_ms = 10000;
        session.no_frame_error_reported = false;
        assert!(missing_frame_snapshot(&mut session, 15000).is_none());
        assert!(missing_frame_snapshot(&mut session, 15001).is_some());
    }

    #[test]
    fn older_solver_payloads_keep_normal_emphasis_by_default() {
        let mut body = serde_json::json!({
            "session_id":1,"round_epoch":2,"revision":3,
            "probabilities":[],"cells":[],"precision":"exact",
            "message":"ready","inferred_placements":[],
        });
        let legacy: OverlayResult = serde_json::from_value(body.clone()).unwrap();
        assert!(!legacy.emphasize_best);
        body["emphasize_best"] = true.into();
        let changed: OverlayResult = serde_json::from_value(body).unwrap();
        assert!(changed.emphasize_best);
        assert_eq!(legacy.revision, changed.revision);
    }

    // Same 10-second static 5-Hz workload, with no capture, OCR, solver or UI.
    // Run optimized: cargo test --release preview_workload_benchmark -- --ignored --nocapture
    #[test]
    #[ignore]
    fn preview_workload_benchmark() {
        let path = std::env::var_os("BA_PERF_IMAGE").map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/vision-ocr-fullscreen-3840.png"));
        let img = image::open(&path).unwrap().to_rgba8();
        let legacy_encode = |img: &RgbaImage| {
            let preview = image::DynamicImage::ImageRgba8(img.clone())
                .resize(960, 600, image::imageops::FilterType::Triangle).to_rgb8();
            let mut jpg = vec![];
            JpegEncoder::new_with_quality(&mut jpg, 72).encode_image(&preview).unwrap();
            format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(jpg))
        };
        // Warm up both paths and verify that the direct-source resize preserves output.
        assert_eq!(legacy_encode(&img), encode_preview(&img).unwrap());
        let frames = 50_u64;
        let before = Instant::now();
        for _ in 0..frames { std::hint::black_box(legacy_encode(&img)); }
        let before_ms = before.elapsed().as_secs_f64() * 1000.0;
        let key = PreviewKey {
            revision: 1, dimensions: img.dimensions(), content: None, board: None,
            status: "ready".into(), message: "ready".into(),
        };
        let after = Instant::now();
        let mut throttle = PreviewThrottle::default();
        let mut encoded = 0;
        for frame in 0..frames {
            let now = frame * FRAME_INTERVAL_MS;
            if throttle.needs_preview(now, &key, false) {
                std::hint::black_box(encode_preview(&img).unwrap());
                throttle.published(now, key.clone());
                encoded += 1;
            }
        }
        let after_ms = after.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(encoded, 10);
        println!("{}", serde_json::json!({
            "scope":"preview encoding only; same static 10-second 5-Hz workload",
            "image":path,"frame_size":img.dimensions(),"frames":frames,
            "before_encodes":frames,"after_encodes":encoded,
            "before_ms":before_ms,"after_ms":after_ms,
        }));
    }

    fn connected_capture() -> (Session, CaptureTicket) {
        let window = WindowGeometry { width: 1960, height: 1162, dpi: 144 };
        let mut s = Session {
            id: 41, hwnd: 123, round_epoch: 7, round: Some("7".into()),
            capture: CaptureLifecycle::new(window, 0),
            ..Session::default()
        };
        assert!(s.capture.installed(1));
        s.geometry.observe_window(window);
        let ticket = CaptureTicket { session_id: s.id, generation: 1, hwnd: s.hwnd, window };
        (s, ticket)
    }

    #[test]
    fn native_rebind_preserves_connection_round_references_and_manual_intent() {
        let (mut s, old) = connected_capture();
        begin_manual_refresh(&mut s, 1000).unwrap();
        s.paused = true;
        s.profile.board = Some(Rect { x: 0.4, y: 0.2, width: 0.5, height: 0.5 });
        let refs = Arc::clone(&s.recognizer);
        let deadline = s.refresh_deadline_ms;
        let revision = s.revision;
        let next_window = WindowGeometry { width: 3840, height: 2094, ..old.window };
        assert!(observe_capture_window(&mut s, next_window, 1100));
        let snapshot = geometry_changed(&mut s);
        assert!(!old.current(&s));
        assert_eq!((s.id, s.round_epoch, s.round.as_deref()), (41, 7, Some("7")));
        assert!(Arc::ptr_eq(&refs, &s.recognizer));
        assert_eq!(s.refresh_deadline_ms, deadline);
        assert!(s.paused && snapshot.refreshing && s.revision > revision);
        assert_eq!(s.profile.board.unwrap().x, 0.4);
        assert!(s.capture.claim(1399).is_none());
        let (generation, _) = s.capture.claim(1400).unwrap();
        assert!(s.capture.installed(generation));
        assert_eq!(s.refresh_deadline_ms, deadline);
        assert!(s.paused);
    }

    #[test]
    fn retired_callback_and_late_start_cannot_affect_stop_or_new_connection() {
        let (mut s, ticket) = connected_capture();
        assert!(ticket.can_install(&s, false));
        assert!(!ticket.can_install(&s, true), "never overwrite an installed Control");
        s.id += 1;
        s.hwnd = 0;
        assert!(!capture_current(&s, ticket.session_id, ticket.generation, ticket.hwnd));
        assert!(!ticket.can_install(&s, false));
        let replacement = CaptureTicket { session_id: s.id + 1, ..ticket };
        s.id = replacement.session_id;
        s.hwnd = replacement.hwnd;
        s.capture = CaptureLifecycle::new(replacement.window, 2000);
        assert_eq!(s.capture.rebuilding, Some(1));
        release_capture_worker(&mut s, ticket);
        assert_eq!(s.capture.rebuilding, Some(1), "old worker cannot release a new startup");
        assert!(replacement.current(&s));
        assert!(!ticket.current(&s));
        assert!(!ticket.owns_retired_control(replacement.session_id, 0));
    }

    #[test]
    fn resize_during_start_or_join_cannot_install_stale_control() {
        let (mut s, old) = connected_capture();
        s.capture.retire(1000);
        let (generation, window) = s.capture.claim(1300).unwrap();
        let rebuilding = CaptureTicket { generation, window, ..old };
        assert!(rebuilding.owns_retired_control(old.session_id, old.generation));
        assert!(!rebuilding.owns_retired_control(old.session_id, generation));
        let latest_window = WindowGeometry { width: 2560, height: 1520, ..window };
        observe_capture_window(&mut s, latest_window, 1400);
        geometry_changed(&mut s);
        assert!(!rebuilding.can_install(&s, false));
        assert!(s.capture.claim(2000).is_none(), "old join/start still owns the worker");
        release_capture_worker(&mut s, rebuilding);
        let (generation, window) = s.capture.claim(2000).unwrap();
        let newest = CaptureTicket { generation, window, ..old };
        assert!(newest.can_install(&s, false));
        release_capture_worker(&mut s, rebuilding);
        assert_eq!(s.capture.rebuilding, Some(newest.generation));
    }

    #[test]
    fn stale_start_error_does_not_fail_a_later_resize_or_new_connection() {
        let (mut s, old) = connected_capture();
        s.capture.retire(1000);
        let (generation, window) = s.capture.claim(1300).unwrap();
        let ticket = CaptureTicket { generation, window, ..old };
        s.capture.retire(1400);
        assert!(capture_failure(&mut s, ticket, "old start failed", false).is_none());
        assert!(!s.capture.failed);
        assert!(s.capture.claim(1700).is_some());
        s.id += 1;
        let owner = s.capture.rebuilding;
        assert!(capture_failure(&mut s, ticket, "old stop failed", true).is_none());
        assert_eq!(s.capture.rebuilding, owner);
        assert!(!s.capture.failed);
    }

    #[test]
    fn failed_teardown_prevents_reusing_recognizer_with_an_unjoined_callback() {
        let (mut s, old) = connected_capture();
        begin_manual_refresh(&mut s, 1000).unwrap();
        s.capture.retire(1100);
        let (generation, window) = s.capture.claim(1400).unwrap();
        let ticket = CaptureTicket { generation, window, ..old };
        s.capture.retire(1500);
        let failure = capture_failure(&mut s, ticket, "native join failed", true).unwrap();
        assert_eq!(failure.status, "error");
        assert!(failure.message.contains("native join failed"));
        assert!(!failure.refreshing);
        assert!(!frame_requested(&s));
        assert!(s.capture.failed && s.capture.claim(5000).is_none());
    }

    #[test]
    fn first_frames_do_not_rebuild_but_later_surface_size_changes_do() {
        let (mut s, old) = connected_capture();
        assert!(observe_capture_frame(&mut s, 1960, 1162, 1));
        assert!(!observe_capture_frame(&mut s, 1960, 1162, 2));
        assert!(old.current(&s));
        assert!(s.capture.claim(1000).is_none());
        assert!(observe_capture_frame(&mut s, 3840, 2094, 1100));
        geometry_changed(&mut s);
        assert!(!old.current(&s));
        assert!(s.geometry.frame_size.is_none());
        assert!(s.capture.claim(1400).is_some());
        assert!(s.capture.installed(s.capture.generation));
        assert!(observe_capture_frame(&mut s, 3840, 2094, 1401));
        assert!(!s.geometry.frame_stable());
        assert!(!observe_capture_frame(&mut s, 3840, 2094, 1402));
        assert!(s.geometry.frame_stable());
    }

    #[test]
    fn manual_timeout_during_rebind_is_not_resurrected_after_install() {
        let (mut s, _) = connected_capture();
        begin_manual_refresh(&mut s, 1000).unwrap();
        s.capture.retire(1100);
        geometry_changed(&mut s);
        let (generation, _) = s.capture.claim(1400).unwrap();
        assert_eq!(s.refresh_deadline_ms, Some(16000));
        assert!(expire_manual_refresh(&mut s, 16000).is_some());
        assert!(s.capture.installed(generation));
        assert!(s.refresh_deadline_ms.is_none());
        assert!(!frame_requested(&s));
    }

    #[test]
    fn actual_nine_item_inventory_is_valid() {
        let items = vec![
            ItemSpec {
                width: 3,
                height: 2,
                remaining_count: 2,
            },
            ItemSpec {
                width: 3,
                height: 1,
                remaining_count: 5,
            },
            ItemSpec {
                width: 2,
                height: 1,
                remaining_count: 2,
            },
        ];
        assert!(valid_items(&items));
        assert!(!valid_items(&items[..2]));
        assert!(!valid_items(&vec![
            ItemSpec {
                width: u32::MAX,
                height: 1,
                remaining_count: 1
            };
            3
        ]));
    }

    #[test]
    fn dimensions_follow_rotatable_board_geometry() {
        let mut items = vec![ItemSpec { width: 1, height: 1, remaining_count: 0 }; 3];
        for (width, height) in [(5, 1), (1, 5), (9, 1), (1, 9), (5, 5), (9, 5), (5, 9)] {
            items[0] = ItemSpec { width, height, remaining_count: 1 };
            assert!(valid_items(&items), "{width}x{height}");
        }
        for (width, height) in [(6, 6), (10, 1), (1, 10), (u32::MAX, 1)] {
            items[0] = ItemSpec { width, height, remaining_count: 1 };
            assert!(!valid_items(&items), "{width}x{height}");
            assert!(item_reading_error(&items).contains("无法放入 9×5 棋盘"));
        }
    }

    #[test]
    fn unread_counts_do_not_report_a_size_error() {
        let mut items = vec![
            ItemSpec {
                width: 3,
                height: 1,
                remaining_count: 0,
            };
            3
        ];
        items[1].remaining_count = -1;
        items[2].remaining_count = -1;
        assert_eq!(
            item_reading_error(&items),
            "未读到物品 2、3 的剩余件数，请刷新或校正"
        );
        items[1].width = 0;
        assert!(item_reading_error(&items).contains("未读到物品 2 的尺寸"));
    }

    #[test]
    fn item_completion_waits_for_opened_regions_without_guessing_hidden_cells() {
        let mut cells = vec!["unknown".to_owned(); 45];
        for observation in ["item0", "item1", "item2", "uncertain"] {
            cells[8] = observation.into();
            assert!(awaiting_item_completion(&cells, Some(44)));
            assert!(!awaiting_item_completion(&cells, Some(45)), "missing cover must remain a recognition error");
            assert!(!awaiting_item_completion(&cells, None));
        }
        for observation in ["completed", "empty"] {
            cells[8] = observation.into();
            assert!(!awaiting_item_completion(&cells, Some(44)));
        }
    }

    #[test]
    fn stale_edits_cannot_confirm_or_correct_another_board() {
        let mut session = Session::default();
        session.id = 12;
        session.revision = 4;
        assert!(require_version(&session, 12, 0, 4).is_err());
        session.latest = Some(CaptureState {
            session_id: 12,
            round_epoch: 0,
            revision: 4,
            status: "ready".into(),
            message: String::new(),
            frame_url: String::new(),
            width: 1920,
            height: 1080,
            content_rect_px: Some([0.0, 0.0, 1920.0, 1080.0]),
            board: None,
            cells: vec!["unknown".into(); 45],
            items: session.items.clone(),
            remaining: Some(45),
            round: Some("1".into()),
            confirmed: true,
            update_mode: UpdateMode::Manual,
            refreshing: false,
            captured_at_ms: Some(1000),
            completed_objects: vec![],
            candidate_constraints: vec![],
            reference_ready: [false; 3],
            card_fingerprints: [None, None, None],
            finish: [false; 3],
        });
        assert!(require_version(&session, 12, 0, 4).is_ok());
        assert!(require_version(&session, 12, 1, 4).is_err());
        assert!(require_version(&session, 11, 0, 4).is_err());
        invalidate(&mut session);
        assert!(require_version(&session, 12, 0, 4).is_err());
    }

    #[test]
    fn invalidation_drops_stability_and_previous_probability() {
        let mut session = Session::default();
        session.pending = "board".into();
        session.pending_frames = 8;
        session.published = "ready".into();
        session.result = Some(OverlayResult {
            session_id: 0,
            round_epoch: 0,
            revision: 0,
            probabilities: vec![0.5; 45],
            cells: vec!["unknown".into(); 45],
            precision: "sampled".into(),
            inferred_placements: vec![],
            message: String::new(),
            emphasize_best: false,
        });
        invalidate(&mut session);
        assert_eq!(session.revision, 1);
        assert_eq!(session.pending_frames, 0);
        assert!(
            session.pending.is_empty() && session.published.is_empty() && session.result.is_none()
        );
    }

    #[test]
    fn manual_requests_are_explicit_and_do_not_queue() {
        let mut session = Session::default();
        assert!(!frame_requested(&session));
        let requested = begin_manual_refresh(&mut session, 1000).unwrap();
        assert!(requested.refreshing && frame_requested(&session));
        let revision = session.revision;
        assert!(begin_manual_refresh(&mut session, 1100).is_err());
        assert_eq!(session.revision, revision);
        assert_eq!(session.refresh_deadline_ms, Some(16000));
        // A user edit cancels the request; it must not start background work.
        let edited = settings_changed(&mut session);
        assert_eq!(edited.status, "manual");
        assert!(!edited.refreshing && !frame_requested(&session));
        assert!(require_version(&session, session.id, session.round_epoch, revision).is_err());
    }

    #[test]
    fn resize_invalidates_inflight_results_without_erasing_round_references() {
        let mut session = Session::default();
        session.profile.update_mode = UpdateMode::Auto;
        session.round_epoch = 7;
        changed_snapshot(&mut session, "ready", "棋盘已同步");
        let revision = session.revision;
        let recognizer = Arc::clone(&session.recognizer);
        session.calculating = true;
        let snap = geometry_changed(&mut session);
        assert_eq!(snap.status, "searching");
        assert!(session.revision > revision);
        assert!(require_version(&session, session.id, 7, revision).is_err());
        assert!(!session.calculating);
        assert!(session.result.is_none());
        assert_eq!(session.round_epoch, 7);
        assert!(Arc::ptr_eq(&recognizer, &session.recognizer));
        assert!(snap.board.is_none() && snap.content_rect_px.is_none());
        assert!(!snap.confirmed && snap.remaining.is_none());
        assert!(snap.cells.iter().all(|c| c == "uncertain"));
    }

    #[test]
    fn resize_does_not_request_idle_manual_work_or_cancel_an_explicit_refresh() {
        let mut session = Session::default();
        changed_snapshot(&mut session, "ready", "手动快照");
        let snap = geometry_changed(&mut session);
        assert_eq!(snap.status, "manual");
        assert!(!frame_requested(&session));
        begin_manual_refresh(&mut session, 1000).unwrap();
        let deadline = session.refresh_deadline_ms;
        let snap = geometry_changed(&mut session);
        assert_eq!(snap.status, "searching");
        assert!(snap.refreshing && frame_requested(&session));
        assert_eq!(session.refresh_deadline_ms, deadline);
        // The explicit request also covers a Worker currently solving it.
        session.refresh_deadline_ms = None;
        session.calculating = true;
        let snap = geometry_changed(&mut session);
        assert!(snap.refreshing && frame_requested(&session));
        assert_eq!(session.refresh_deadline_ms, deadline);
    }

    #[test]
    fn resize_preserves_the_paused_state_and_resume_action() {
        let mut session = Session::default();
        session.profile.update_mode = UpdateMode::Auto;
        session.paused = true;
        let snap = geometry_changed(&mut session);
        assert_eq!(snap.status, "paused");
        assert!(session.paused);
        assert!(!snap.refreshing);
    }

    #[test]
    fn manual_timeout_finishes_even_without_capture_frames() {
        let mut session = Session::default();
        begin_manual_refresh(&mut session, 1000).unwrap();
        assert!(expire_manual_refresh(&mut session, 15999).is_none());
        let snapshot = expire_manual_refresh(&mut session, 16000).unwrap();
        assert_eq!(snapshot.status, "error");
        assert!(!snapshot.refreshing && !frame_requested(&session));
        assert!(expire_manual_refresh(&mut session, 17000).is_none());
        assert!(begin_manual_refresh(&mut session, 17001).is_ok());
    }

    #[test]
    fn in_game_refresh_is_available_after_uncertainty_and_timeout() {
        let mut session = Session::default();
        session.hwnd = 123;
        changed_snapshot(&mut session, "uncertain", "需要校正");
        assert!(refresh_control_state(&session).enabled);
        assert!(refresh_control_state(&session).attention);
        changed_snapshot(&mut session, "waiting_item", "等待物品翻完");
        assert!(refresh_control_state(&session).enabled);
        assert!(refresh_control_state(&session).attention);
        session.calculating = true;
        assert!(!refresh_control_state(&session).enabled);
        assert!(begin_manual_refresh(&mut session, 999).is_err());
        session.calculating = false;
        begin_manual_refresh(&mut session, 1000).unwrap();
        let pending = refresh_control_state(&session);
        assert!(pending.refreshing && !pending.enabled);
        assert!(!pending.attention);
        assert_eq!(pending.revision, session.revision);
        expire_manual_refresh(&mut session, 16000).unwrap();
        assert!(refresh_control_state(&session).enabled);
        session.profile.update_mode = UpdateMode::Auto;
        assert!(!refresh_control_state(&session).enabled);
        session.profile.update_mode = UpdateMode::Manual;
        session.hwnd = 0;
        assert!(!refresh_control_state(&session).enabled);
    }

    #[test]
    fn refresh_button_is_beside_grid_and_inside_window_across_scales() {
        let board = Rect {
            x: 0.473,
            y: 0.303,
            width: 0.485,
            height: 0.454,
        };
        for (width, height, side, gap) in [
            (1924, 1142, 40, 10),
            (2560, 1440, 60, 15),
            (960, 570, 40, 10),
        ] {
            let (x, y) = refresh_button_offset(width, height, Some(board), side, gap);
            assert!(x >= 0 && y >= 0 && x + side <= width && y + side <= height);
            assert!(x + side <= (board.x * width as f64).round() as i32 - gap);
        }
        let (x, y) = refresh_button_offset(960, 570, None, 40, 10);
        assert!(x >= 0 && y >= 0 && x + 40 <= 960 && y + 40 <= 570);
        assert_eq!(refresh_button_offset(20, 20, None, 40, 10), (0, 0));
    }

    #[test]
    fn saved_mode_is_explicit_and_legacy_profile_defaults_to_manual() {
        let legacy: Profile = serde_json::from_str(r#"{"items":[],"board":null}"#).unwrap();
        assert_eq!(legacy.update_mode, UpdateMode::Manual);
        let mut session = Session::default();
        session.profile.update_mode = UpdateMode::Auto;
        assert!(frame_requested(&session));
        assert!(begin_manual_refresh(&mut session, 1).is_err());
        let saved = serde_json::to_string(&session.profile).unwrap();
        let restored: Profile = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored.update_mode, UpdateMode::Auto);
        session.profile.update_mode = UpdateMode::Manual;
        settings_changed(&mut session);
        assert!(!frame_requested(&session));
    }

    #[test]
    fn legacy_initial_totals_never_become_remaining_counts() {
        let old =
            r#"{"items":[{"width":3,"height":2,"count":2}],"board":null,"update_mode":"auto"}"#;
        let profile: Profile = serde_json::from_str(old).unwrap();
        assert_eq!(profile.update_mode, UpdateMode::Auto);
        assert!(!serde_json::to_string(&profile).unwrap().contains("items"));
        assert!(!valid_items(&unread_items()));
    }

    #[test]
    fn remaining_counts_are_read_even_mid_round_and_finish_is_zero() {
        let mut s = Session::default();
        s.remaining = Some(13);
        update_card_items(
            &mut s,
            &[[3, 2], [3, 1], [2, 1]],
            [None, Some(1), Some(1)],
            [true, false, false],
        );
        assert!(s.confirmed);
        assert_eq!(
            s.items
                .iter()
                .map(|i| i.remaining_count)
                .collect::<Vec<_>>(),
            vec![0, 1, 1]
        );
        update_card_items(
            &mut s,
            &[[3, 2], [3, 1], [2, 1]],
            [None, None, Some(1)],
            [true, false, false],
        );
        assert!(!s.confirmed);
        assert_eq!(s.items[1].remaining_count, -1);
        update_card_items(
            &mut s,
            &[[3, 2], [3, 1], [2, 1]],
            [None, Some(8), Some(1)],
            [true, false, false],
        );
        assert!(!s.confirmed, "out-of-range counts must not be clamped");
    }

    #[test]
    fn corrections_expire_when_card_reading_changes() {
        let mut s = Session::default();
        let shapes = [[3, 2], [3, 1], [2, 1]];
        update_card_items(
            &mut s,
            &shapes,
            [Some(0), None, Some(1)],
            [true, false, false],
        );
        let mut corrected = s.items.clone();
        corrected[1].remaining_count = 1;
        s.item_correction = Some((s.card_reading.clone(), corrected));
        update_card_items(
            &mut s,
            &shapes,
            [Some(0), None, Some(1)],
            [true, false, false],
        );
        assert!(s.confirmed);
        update_card_items(
            &mut s,
            &shapes,
            [Some(0), Some(0), Some(1)],
            [true, true, false],
        );
        assert!(s.item_correction.is_none());
        assert_eq!(s.items[1].remaining_count, 0);
    }

    #[test]
    fn manual_cover_selection_freezes_pixels_and_rejects_stale_tokens() {
        let mut s = Session::default();
        s.id = 7;
        s.hwnd = 42;
        s.profile.update_mode = UpdateMode::Auto;
        let samples: Vec<_> = (0..45).map(|i| CoverSample { rgb: vec![[i, 10, 20]; SAMPLE_SIZE * SAMPLE_SIZE] }).collect();
        s.cover_candidates = Some(CoverCandidates {
            token: 17, session_id: s.id, generation: s.capture.generation,
            round_epoch: s.round_epoch, samples: samples.clone(), images: vec!["preview".into(); 45],
        });
        assert!(frame_requested(&s));
        let selection = freeze_cover_selection(&mut s).unwrap();
        assert!(!frame_requested(&s));
        assert_eq!(selection.images.len(), 45);
        s.cover_candidates.as_mut().unwrap().samples[3].rgb.fill([255, 0, 0]);
        assert_eq!(selected_cover_samples(&s, selection.token, &[3, 3, 8]).unwrap(), vec![samples[3].clone(), samples[8].clone()]);
        assert!(selected_cover_samples(&s, selection.token, &[]).is_err());
        assert!(selected_cover_samples(&s, selection.token, &[45]).is_err());
        assert!(selected_cover_samples(&s, selection.token + 1, &[3]).is_err());
        s.capture.generation += 1;
        geometry_changed(&mut s);
        assert_eq!(selected_cover_samples(&s, selection.token, &[3]).unwrap(), vec![samples[3].clone()]);
        s.round_epoch += 1;
        assert!(selected_cover_samples(&s, selection.token, &[3]).is_err());
        s.round_epoch -= 1;
        s.id += 1;
        assert!(selected_cover_samples(&s, selection.token, &[3]).is_err());
        assert!(s.cover_samples.is_empty());
    }

    #[test]
    fn round_reset_preserves_player_samples_and_invalidates_selection() {
        let mut frame = image::load_from_memory(include_bytes!("../tests/fixtures/vision-initial.png"))
            .unwrap().to_rgba8();
        for y in 346..866 { for x in 910..1846 { frame.put_pixel(x, y, image::Rgba([30, 15, 160, 255])); } }
        let samples = vision::sample_cover_candidates(&frame, None, None).unwrap();
        let mut s = Session::default();
        replace_cover_samples(&mut s, vec![samples[0].clone()]);
        clear_round(&mut s);
        assert_eq!(s.cover_samples, vec![samples[0].clone()]);
        assert!(s.cover_selection.is_none() && s.cover_candidates.is_none());
        let analysis = s.recognizer.lock().unwrap().analyze(&frame, None, Some(45));
        assert!(analysis.present, "{}", analysis.message);
        assert!(analysis.cells.iter().all(|c| c == "unknown"));
    }

    #[test]
    fn changed_initial_covers_reset_the_round_when_round_ocr_is_missing() {
        let frame = image::load_from_memory(include_bytes!("../tests/fixtures/vision-initial.png"))
            .unwrap().to_rgba8();
        let mut s = Session::default();
        for _ in 0..2 {
            assert!(s.recognizer.lock().unwrap().analyze(&frame, None, Some(45)).present);
        }
        s.seen_opened = true;
        s.remaining = Some(44);
        let old_recognizer = Arc::clone(&s.recognizer);
        let mut next = frame.clone();
        for y in 346..866 {
            for x in 910..1846 {
                let p = next.get_pixel_mut(x, y);
                for channel in 0..3 {
                    p[channel] = 255 - p[channel];
                }
            }
        }
        for expected_reset in [false, true] {
            observe_round(&mut s, None);
            let analysis = old_recognizer.lock().unwrap().analyze(&next, None, Some(45));
            assert!(analysis.fresh_initial_grid);
            assert!(!full_cover_observed(&analysis, None));
            assert!(!full_cover_observed(&analysis, Some(44)));
            assert_eq!(observe_full_cover(&mut s, full_cover_observed(&analysis, Some(45))), expected_reset);
        }
        assert_eq!(s.round_epoch, 1);
        assert!(!Arc::ptr_eq(&old_recognizer, &s.recognizer));
        assert!(!s.seen_opened);
        assert_eq!(s.remaining, None);
    }

    #[test]
    fn round_reset_needs_two_reads_and_clears_round_scoped_state() {
        let mut s = Session::default();
        observe_round(&mut s, Some("1".into()));
        s.item_correction = Some(("old".into(), unread_items()));
        observe_round(&mut s, Some("2".into()));
        assert_eq!(s.round_epoch, 0);
        observe_round(&mut s, Some("2".into()));
        assert_eq!(s.round_epoch, 1);
        assert!(s.item_correction.is_none() && s.latest.is_none());
        observe_round(&mut s, Some("2".into()));
        assert_eq!(s.round_epoch, 1);
        s.seen_opened = true;
        assert!(!observe_full_cover(&mut s, true));
        assert!(observe_full_cover(&mut s, true));
        assert_eq!(s.round_epoch, 2);
        for _ in 0..10 {
            assert!(!observe_full_cover(&mut s, true));
        }
        assert_eq!(s.round_epoch, 2);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--inspect-image") {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        };
        let img = image::open(&args[2]).expect("image").to_rgba8();
        let content = vision::locate_content(&img);
        let hud = content.map(|r| ocr::read_in_content(&img, r)).unwrap_or_default();
        let timer = Instant::now();
        let mut recognizer = vision::Recognizer::new();
        let a = if let Some(r) = content {
            recognizer.analyze_completed_in_content(&img, r, None, hud.remaining, hud.counts)
        } else {
            recognizer.analyze_completed_snapshot(&img, None, hud.remaining, hud.counts)
        };
        println!(
            "{}",
            serde_json::json!({"frame_size":img.dimensions(),"content_rect_px":content,"remaining":hud.remaining,"round":hud.round,"counts":hud.counts,"board":a.board,"cells":a.cells,"shapes":a.shapes,"present":a.present,"message":a.message,"completed_objects":a.completed_objects,"candidate_constraints":a.candidate_constraints,"reference_ready":a.reference_ready,"card_fingerprints":a.card_fingerprints,"finish":a.finish,"analysis_ms":timer.elapsed().as_millis()})
        );
        return;
    }
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            list_windows,
            start_capture,
            stop_capture,
            set_paused,
            set_update_mode,
            refresh_capture,
            set_overlay_visible,
            clear_overlay,
            solver_started,
            calibrate,
            confirm_items,
            correct_cell,
            reset_round,
            get_cover_reference,
            begin_cover_selection,
            save_cover_selection,
            cancel_cover_selection,
            clear_cover_reference,
            render_overlay,
            overlay_painted,
            solver_failed
        ])
        .setup(|app| {
            let overlay = app.get_webview_window("overlay").unwrap();
            overlay.set_ignore_cursor_events(true)?;
            for label in ["overlay", "refresh"] {
                let window = app.get_webview_window(label).unwrap();
                window.set_content_protected(true)?;
                window.set_focusable(false)?;
                unsafe {
                    let hwnd = HWND(window.hwnd()?.0);
                    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                    SetWindowLongPtrW(
                        hwnd,
                        GWL_EXSTYLE,
                        ex | WS_EX_NOACTIVATE.0 as isize | WS_EX_TOOLWINDOW.0 as isize,
                    );
                }
            }
            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_millis(100));
                let state = handle.state::<AppState>();
                if state.quitting.load(Ordering::Relaxed) {
                    break;
                }
                let update = {
                    let mut s = state.session.lock().unwrap();
                    let hwnd = s.hwnd;
                    if s.hwnd == 0 {
                        None
                    } else if s.capture.failed {
                        None
                    } else if !window_minimized(hwnd)
                        && observe_capture_window(&mut s, window_geometry(hwnd), now_ms()) {
                        // Pixel work may be blocked in OCR. Window metadata is
                        // enough to retire its version and hide the old overlay.
                        Some(geometry_changed(&mut s))
                    } else if let Some(snapshot) = expire_manual_refresh(&mut s, now_ms()) {
                        Some(snapshot)
                    } else if s.capture.pending || s.capture.rebuilding.is_some() {
                        // Rebinding has its own terminal errors. Keep the
                        // refresh deadline watchdog alive while native join runs.
                        None
                    } else if s.profile.update_mode == UpdateMode::Manual {
                        // Focus/minimize only changes visibility of a manual snapshot.
                        // Returning to the game must never start a new calculation.
                        None
                    } else if let Some(snap) = missing_frame_snapshot(&mut s, now_ms()) {
                        Some(snap)
                    } else if (!foreground(s.hwnd)
                        || now_ms().saturating_sub(s.last_frame_ms) > 2500)
                        && s.latest.as_ref().is_some_and(|c| c.status == "ready")
                    {
                        invalidate(&mut s);
                        let revision = s.revision;
                        s.latest.as_mut().map(|c| {
                            c.revision = revision;
                            c.status = "away".into();
                            c.message = "游戏失焦或画面暂未更新".into();
                            c.clone()
                        })
                    } else {
                        None
                    }
                };
                if let Some(c) = update {
                    record_capture_diagnostics(&handle, &state.session.lock().unwrap(), &c, "window");
                    let _ = handle.emit_to("main", "capture-state", c);
                }
                schedule_capture_rebuild(&handle);
                refresh_overlay(&handle);
            });
            Ok(())
        })
        .on_window_event(|w, e| {
            if w.label() == "main" && matches!(e, tauri::WindowEvent::CloseRequested { .. }) {
                w.app_handle()
                    .state::<AppState>()
                    .quitting
                    .store(true, Ordering::Relaxed);
                end_capture(w.app_handle());
                w.app_handle().exit(0);
            }
        })
        .run(tauri::generate_context!())
        .expect("BA desktop runtime failed");
}
