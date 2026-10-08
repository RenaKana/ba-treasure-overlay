//! Local Windows OCR. Failed or ambiguous readings remain None.
use image::{imageops::FilterType, Rgba, RgbaImage};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};
use windows::{
    Globalization::Language,
    Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::DataWriter,
};

#[derive(Default, Clone, Debug)]
pub struct Hud {
    pub remaining: Option<u32>,
    pub round: Option<String>,
    pub counts: [Option<u32>; 3],
    pub count_errors: [Option<String>; 3],
    pub error: Option<String>,
}

const TEXT_HEIGHT: f64 = 80.0;
// Shared by all fields; return before the 5-second capture watchdog so the
// caller can report the OCR failure instead of a missing-frame error.
const READ_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, PartialEq, Eq)]
enum ReadError {
    Empty,
    Service,
    Timeout,
}
impl ReadError {
    fn message(&self) -> &'static str {
        match self {
            Self::Empty => "数量裁剪区域无效",
            Self::Service => "OCR服务读取失败",
            Self::Timeout => "识别超时，请重试或校正",
        }
    }
}
fn time_left(deadline: Instant) -> Result<Duration, ReadError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or(ReadError::Timeout)
}
fn wait_for_completion<T>(
    receiver: mpsc::Receiver<Result<T, ReadError>>,
    deadline: Instant,
    cancel: impl FnOnce(),
) -> Result<T, ReadError> {
    let result = time_left(deadline).and_then(|left| {
        receiver.recv_timeout(left).map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => ReadError::Timeout,
            mpsc::RecvTimeoutError::Disconnected => ReadError::Service,
        })?
    });
    if matches!(result, Err(ReadError::Timeout)) {
        // A cancellation request is cleanup, not a completion acknowledgement.
        // Never join the operation after this request.
        cancel();
    }
    result
}

fn otsu_threshold(histogram: &[u64; 256]) -> u8 {
    let total = histogram.iter().sum::<u64>();
    let sum = histogram
        .iter()
        .enumerate()
        .map(|(v, n)| v as f64 * *n as f64)
        .sum::<f64>();
    let (mut below, mut below_sum, mut best, mut threshold) = (0_u64, 0.0, 0.0, 0);
    for (level, count) in histogram.iter().enumerate() {
        below += count;
        below_sum += level as f64 * *count as f64;
        if below == 0 || below == total {
            continue;
        }
        let above = total - below;
        let delta = below_sum / below as f64 - (sum - below_sum) / above as f64;
        let variance = below as f64 * above as f64 * delta * delta;
        if variance > best {
            best = variance;
            threshold = level as u8;
        }
    }
    threshold
}

fn text_at_scale(
    engine: &OcrEngine,
    frame: &RgbaImage,
    content: [f64; 4],
    r: [f64; 4],
    target_height: Option<f64>,
    contrast: bool,
    deadline: Instant,
) -> Result<String, ReadError> {
    time_left(deadline)?;
    let [x, y, w, h] = content;
    let rx = (x + r[0] * w).floor() as u32;
    let ry = (y + r[1] * h).floor() as u32;
    let right = (x + (r[0] + r[2]) * w).ceil() as u32;
    let bottom = (y + (r[1] + r[3]) * h).ceil() as u32;
    if right > frame.width() || bottom > frame.height() || right <= rx || bottom <= ry {
        return Err(ReadError::Empty);
    }
    let crop = image::imageops::crop_imm(frame, rx, ry, right - rx, bottom - ry).to_image();
    let (rw, rh) = crop.dimensions();
    // Normalize both small and large text, bounded by the OCR engine's real
    // image limit rather than an arbitrary maximum enlargement factor.
    let max_dimension = OcrEngine::MaxImageDimension().map_err(|_| ReadError::Service)?;
    let source_height = rh as f64;
    let requested_scale = target_height.map_or(1.0, |height| height / source_height);
    let requested_padding = target_height.unwrap_or(rh as f64) / 10.0;
    let scale_limit = max_dimension as f64
        / (rw as f64 + 2.0 * requested_padding / requested_scale)
            .max(rh as f64 + 2.0 * requested_padding / requested_scale);
    let scale = requested_scale.min(scale_limit);
    let padding = (requested_padding * scale / requested_scale).floor() as u32;
    let mut data = image::imageops::resize(
        &crop,
        ((rw as f64 * scale).floor() as u32).max(1),
        ((rh as f64 * scale).floor() as u32).max(1),
        FilterType::CatmullRom,
    );
    if contrast {
        let mut histogram = [0_u64; 256];
        for p in data.pixels_mut() {
            let gray =
                (p[0] as f64 * 0.299 + p[1] as f64 * 0.587 + p[2] as f64 * 0.114).round() as u8;
            histogram[gray as usize] += 1;
            *p = Rgba([gray, gray, gray, 255]);
        }
        let threshold = otsu_threshold(&histogram);
        for p in data.pixels_mut() {
            let value = if p[0] > threshold { 255 } else { 0 };
            *p = Rgba([value, value, value, 255]);
        }
    }
    // Leave a background-colored margin so a digit touching a crop edge is
    // still separated from the OCR image boundary after enlargement.
    let mut background_bins = std::collections::BTreeMap::<[u8; 3], (usize, [u64; 3])>::new();
    for p in data.pixels() {
        let key = [p[0] / 32, p[1] / 32, p[2] / 32];
        let entry = background_bins.entry(key).or_default();
        entry.0 += 1;
        for channel in 0..3 {
            entry.1[channel] += p[channel] as u64;
        }
    }
    let (count, sum) = background_bins
        .into_values()
        .max_by_key(|(count, _)| *count)
        .ok_or(ReadError::Empty)?;
    let background = Rgba([
        (sum[0] / count as u64) as u8,
        (sum[1] / count as u64) as u8,
        (sum[2] / count as u64) as u8,
        255,
    ]);
    let mut padded = RgbaImage::from_pixel(
        data.width() + 2 * padding,
        data.height() + 2 * padding,
        background,
    );
    image::imageops::overlay(&mut padded, &data, padding as i64, padding as i64);
    data = padded;
    for p in data.pixels_mut() {
        p.0.swap(0, 2);
    }
    let writer = DataWriter::new().map_err(|_| ReadError::Service)?;
    writer
        .WriteBytes(data.as_raw())
        .map_err(|_| ReadError::Service)?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &writer.DetachBuffer().map_err(|_| ReadError::Service)?,
        BitmapPixelFormat::Bgra8,
        data.width() as i32,
        data.height() as i32,
        BitmapAlphaMode::Ignore,
    )
    .map_err(|_| ReadError::Service)?;
    time_left(deadline)?;
    let operation = engine
        .RecognizeAsync(&bitmap)
        .map_err(|_| ReadError::Service)?;
    let (sender, receiver) = mpsc::channel();
    // Each callback owns only this read's resources and completion channel.
    // Timeout discards the receiver; a late completion cannot publish a Hud.
    // Cancellation cannot authorize reuse of this engine or channel.
    let keep_engine = engine.clone();
    operation
        .when(move |result| {
            let _resources = (keep_engine, bitmap);
            let text = result
                .and_then(|result| result.Text())
                .map(|text| text.to_string())
                .map_err(|_| ReadError::Service);
            let _ = sender.send(text);
        })
        .map_err(|_| ReadError::Service)?;
    wait_for_completion(receiver, deadline, || {
        let _ = operation.Cancel();
    })
}

fn consensus(readings: &[Option<u32>]) -> Option<u32> {
    let first = readings.iter().flatten().next()?;
    readings
        .iter()
        .flatten()
        .all(|value| value == first)
        .then_some(*first)
}

fn number_from_views(
    engine: &OcrEngine,
    frame: &RgbaImage,
    content: [f64; 4],
    views: &[[f64; 4]],
    parse: fn(&str, bool) -> Option<u32>,
    deadline: Instant,
) -> Result<Option<u32>, ReadError> {
    let read_views = |height, contrast, retry| -> Result<Vec<Option<u32>>, ReadError> {
        views
            .iter()
            .map(
                |r| match text_at_scale(engine, frame, content, *r, height, contrast, deadline) {
                    Ok(text) => Ok(parse(&text, retry)),
                    Err(ReadError::Empty) => Ok(None),
                    Err(error) => Err(error),
                },
            )
            .collect()
    };
    let readings = read_views(Some(TEXT_HEIGHT), false, false)?;
    if readings.iter().any(Option::is_some) {
        return Ok(consensus(&readings));
    }
    // Keep the existing round/remaining retries, including native large text.
    if views.iter().any(|r| r[3] * content[3] > TEXT_HEIGHT) {
        let readings = read_views(None, false, false)?;
        if readings.iter().any(Option::is_some) {
            return Ok(consensus(&readings));
        }
    }
    for (height, contrast) in [(60.0, false), (120.0, false), (TEXT_HEIGHT, true)] {
        let readings = read_views(Some(height), contrast, true)?;
        if readings.iter().any(Option::is_some) {
            return Ok(consensus(&readings));
        }
    }
    // Small fractions such as 44/45 need the established 40px final view.
    Ok(consensus(&read_views(Some(40.0), false, true)?))
}

fn count_number(text: &str) -> Option<u32> {
    let field = text.trim();
    let field = field.strip_prefix(['×', 'x', 'X']).unwrap_or(field).trim();
    let digits: String = field
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| if matches!(c, 'L' | 'l') { '1' } else { c })
        .collect();
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

pub fn numbers(s: &str) -> Vec<u32> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect()
}

fn remaining_number(s: &str) -> Option<u32> {
    // Windows OCR may split one number into words, e.g. "1 5 / 4 5".
    // Join whitespace only inside the fraction payload, after its label.
    // A malformed or competing fraction stays unread instead of selecting
    // the last numeric word (which would silently turn 15 into 5).
    if s.matches(['/', '／']).count() != 1 {
        return None;
    }
    let payload = s.rsplit([':', '：']).next()?;
    let (numerator, denominator) = payload.split_once(['/', '／'])?;
    let parse_field = |field: &str| {
        let digits = field
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        if !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        digits.parse::<u32>().ok()
    };
    if parse_field(denominator)? != 45 {
        return None;
    }
    parse_field(numerator).filter(|n| *n <= 45)
}

pub fn read(frame: &RgbaImage) -> Hud {
    crate::vision::locate_content(frame)
        .map_or_else(Hud::default, |content| read_in_content(frame, content))
}

pub fn read_in_content(frame: &RgbaImage, content: [f64; 4]) -> Hud {
    let [x, y, w, h] = content;
    if content.iter().any(|n| !n.is_finite())
        || x < 0.0
        || y < 0.0
        || w <= 0.0
        || h <= 0.0
        || x + w > frame.width() as f64
        || y + h > frame.height() as f64
    {
        return Hud::default();
    }
    let deadline = Instant::now() + READ_TIMEOUT;
    // New engine per read: a previously timed-out operation owns no shared
    // engine or mutex needed by this capture.
    let engine = Language::CreateLanguage(&"zh-Hans".into())
        .ok()
        .and_then(|l| OcrEngine::TryCreateFromLanguage(&l).ok())
        .or_else(|| OcrEngine::TryCreateFromUserProfileLanguages().ok());
    let Some(engine) = engine else {
        return Hud {
            error: Some(ReadError::Service.message().into()),
            ..Hud::default()
        };
    };
    let mut hud = Hud::default();
    let result = read_fields(&engine, frame, content, deadline, &mut hud);
    if let Err(error) = result {
        hud.error = Some(error.message().into());
    }
    hud
}

fn read_fields(
    engine: &OcrEngine,
    frame: &RgbaImage,
    content: [f64; 4],
    deadline: Instant,
    hud: &mut Hud,
) -> Result<(), ReadError> {
    let centered = crate::vision::layout_region(content, crate::vision::LayoutAnchor::Center);
    let bottom = crate::vision::layout_region(content, crate::vision::LayoutAnchor::Bottom);
    hud.remaining = number_from_views(
        engine,
        frame,
        centered,
        &[[0.649, 0.222, 0.153, 0.035], [0.736, 0.222, 0.055, 0.035]],
        |text, _| remaining_number(text),
        deadline,
    )?;
    hud.round = number_from_views(
        engine,
        frame,
        centered,
        &[[0.651, 0.18, 0.156, 0.046], [0.729, 0.18, 0.050, 0.046]],
        |text, _| {
            numbers(text)
                .last()
                .copied()
                .filter(|n| *n > 0 && *n < 10000)
        },
        deadline,
    )?
    .map(|n| n.to_string());
    for (index, x) in [0.153, 0.264, 0.371].into_iter().enumerate() {
        // One fixed count-plate field, containing the entire number. Its optional
        // multiplication prefix is stripped once; no digits are extracted from labels.
        let text = match text_at_scale(
            engine,
            frame,
            bottom,
            [x + 0.002, 0.931, 0.030, 0.037],
            Some(TEXT_HEIGHT),
            false,
            deadline,
        ) {
            Ok(text) => text,
            Err(ReadError::Empty) => {
                hud.count_errors[index] = Some(ReadError::Empty.message().into());
                continue;
            }
            Err(error) => return Err(error),
        };
        hud.counts[index] = count_number(&text);
        if hud.counts[index].is_none() {
            hud.count_errors[index] = Some(
                if text.trim().is_empty() {
                    "OCR返回空文本"
                } else {
                    "识别结果不是有效数字"
                }
                .into(),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pixel_test_engine() -> OcrEngine {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
        Language::CreateLanguage(&"zh-Hans".into())
            .ok()
            .and_then(|language| OcrEngine::TryCreateFromLanguage(&language).ok())
            .or_else(|| OcrEngine::TryCreateFromUserProfileLanguages().ok())
            .expect("local Windows OCR engine")
    }
    #[test]
    fn count_field_requires_the_complete_numeric_payload() {
        for (text, expected) in [
            ("1", 1),
            ("L", 1),
            ("xl", 1),
            ("× 12", 12),
            ("X1 2", 12),
            ("0", 0),
            ("4294967295", u32::MAX),
        ] {
            assert_eq!(count_number(text), Some(expected), "{text}");
        }
        for text in [
            "",
            "x",
            "xx1",
            "第7轮 ×1",
            "1/45",
            "-1",
            "+1",
            "1?",
            "1 ×2",
            "4294967296",
        ] {
            assert_eq!(count_number(text), None, "{text}");
        }
    }
    #[test]
    fn timeout_releases_caller_and_late_results_are_isolated() {
        let (old_sender, old_receiver) = mpsc::channel::<Result<u32, ReadError>>();
        let start = Instant::now();
        let cancelled = std::cell::Cell::new(false);
        assert_eq!(
            wait_for_completion(old_receiver, start + Duration::from_millis(20), || {
                cancelled.set(true)
            }),
            Err(ReadError::Timeout)
        );
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(cancelled.get()); // Model Cancel acknowledged while old operation stays pending.
        let (new_sender, new_receiver) = mpsc::channel();
        new_sender.send(Ok(4)).unwrap();
        assert!(old_sender.send(Ok(1)).is_err());
        assert_eq!(
            wait_for_completion(
                new_receiver,
                Instant::now() + Duration::from_secs(1),
                || panic!("successful read cancelled")
            ),
            Ok(4)
        );
        assert_eq!(time_left(start), Err(ReadError::Timeout));
    }
    #[test]
    fn expired_read_stops_fields_and_a_new_read_succeeds() {
        let engine = pixel_test_engine();
        let frame = image::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vision-pc-user-20261008-remaining22-third-one.png"
        ))
        .unwrap()
        .to_rgba8();
        let content = crate::vision::locate_content(&frame).unwrap();
        let mut expired = Hud::default();
        assert_eq!(
            read_fields(&engine, &frame, content, Instant::now(), &mut expired),
            Err(ReadError::Timeout)
        );
        assert_eq!(expired.remaining, None);
        assert_eq!(expired.round, None);
        assert_eq!(expired.counts, [None; 3]);
        let (old_sender, old_receiver) = mpsc::channel::<Result<String, ReadError>>();
        assert_eq!(
            wait_for_completion(
                old_receiver,
                Instant::now() + Duration::from_millis(10),
                || {}
            ),
            Err(ReadError::Timeout)
        );
        // The simulated operation ignores cancellation and completes only after
        // another real Windows OCR read. It cannot reach that read's Hud.
        let current = read(&frame);
        assert!(old_sender.send(Ok("99".into())).is_err());
        assert_eq!(current.remaining, Some(22));
        assert_eq!(current.round.as_deref(), Some("6"));
        assert_eq!(current.counts, [Some(0), Some(4), Some(1)]);
        assert_eq!(current.error, None);
    }
    #[test]
    fn complete_multi_digit_field_and_empty_field_keep_other_hud_values() {
        let _engine = pixel_test_engine();
        let mut frame = image::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vision-pc-user-20261008-remaining22-third-one.png"
        ))
        .unwrap()
        .to_rgba8();
        // Derived-pixel fixture: form x11 from the real third card's x1. This
        // tests a complete multi-digit field, not a real captured inventory of 11.
        let prefix = image::imageops::crop_imm(&frame, 487, 722, 9, 16).to_image();
        let digit = image::imageops::crop_imm(&frame, 496, 722, 10, 16).to_image();
        let background = *frame.get_pixel(510, 722);
        for y in 720..740 {
            for x in 481..515 {
                frame.put_pixel(x, y, background);
            }
        }
        image::imageops::overlay(&mut frame, &prefix, 482, 722);
        image::imageops::overlay(&mut frame, &digit, 492, 722);
        image::imageops::overlay(&mut frame, &digit, 502, 722);
        let hud = read(&frame);
        assert_eq!(hud.remaining, Some(22), "{hud:?}");
        assert_eq!(hud.round.as_deref(), Some("6"), "{hud:?}");
        assert_eq!(hud.counts, [Some(0), Some(4), Some(11)], "{hud:?}");
        let bottom = crate::vision::layout_region(
            crate::vision::locate_content(&frame).unwrap(),
            crate::vision::LayoutAnchor::Bottom,
        );
        for y in (bottom[1] + 0.931 * bottom[3]).floor() as u32
            ..(bottom[1] + 0.968 * bottom[3]).ceil() as u32
        {
            for x in (bottom[0] + 0.155 * bottom[2]).floor() as u32
                ..(bottom[0] + 0.185 * bottom[2]).ceil() as u32
            {
                frame.put_pixel(x, y, background);
            }
        }
        let hud = read(&frame);
        assert_eq!(hud.remaining, Some(22), "{hud:?}");
        assert_eq!(hud.round.as_deref(), Some("6"), "{hud:?}");
        assert_eq!(hud.counts, [None, Some(4), Some(11)], "{hud:?}");
        assert_eq!(hud.count_errors[0].as_deref(), Some("OCR返回空文本"));
        assert_eq!(hud.error, None);
    }
    #[test]
    fn real_pc_failed_quantity_frames_read_complete_hud() {
        let _engine = pixel_test_engine();
        for (name, round, remaining, counts) in [
            (
                "vision-pc-user-20261006-count-one-window.png",
                "1",
                19,
                [0, 1, 2],
            ),
            (
                "vision-pc-user-20261008-count-one-round6.png",
                "6",
                45,
                [1, 4, 3],
            ),
            (
                "vision-pc-user-20261008-remaining22-third-one.png",
                "6",
                22,
                [0, 4, 1],
            ),
            (
                "vision-pc-global-fullscreen-initial.png",
                "2",
                45,
                [1, 2, 5],
            ),
        ] {
            let frame = image::open(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(name),
            )
            .unwrap()
            .to_rgba8();
            let hud = read(&frame);
            eprintln!("{name}: {hud:?}");
            assert_eq!(hud.remaining, Some(remaining), "{name}: {hud:?}");
            assert_eq!(hud.round.as_deref(), Some(round), "{name}: {hud:?}");
            assert_eq!(hud.counts, counts.map(Some), "{name}: {hud:?}");
            assert_eq!(hud.count_errors, [None, None, None], "{name}");
            assert_eq!(hud.error, None, "{name}");
        }
    }
    #[test]
    fn round_seven_quantity_one_survives_window_scaling() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
        let frame = image::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vision-resolution-round-seven.png"
        ))
        .unwrap()
        .to_rgba8();
        for width in [1960, 960, 1440, 2560, 3840] {
            let height =
                (frame.height() as f64 * width as f64 / frame.width() as f64).round() as u32;
            let resized = image::imageops::resize(&frame, width, height, FilterType::CatmullRom);
            let hud = read(&resized);
            assert_eq!(hud.remaining, Some(32), "width={width}: {hud:?}");
            assert_eq!(hud.round.as_deref(), Some("7"), "width={width}: {hud:?}");
            assert_eq!(
                hud.counts,
                [Some(1), Some(2), Some(5)],
                "width={width}: {hud:?}"
            );
        }
    }
    #[test]
    fn numeric_fields_are_separate() {
        assert_eq!(numbers("剩余格子 44 / 45"), vec![44, 45]);
        assert_eq!(numbers("×2"), vec![2]);
    }
    #[test]
    fn remaining_fraction_joins_only_its_ocr_spaced_digits() {
        assert_eq!(remaining_number("剩 余 格 子 数 量 ： 1 5 / 45"), Some(15));
        assert_eq!(remaining_number("1 5 / 4 5"), Some(15));
        assert_eq!(
            remaining_number("第 3 轮 剩余格子数量：1 5 / 4 5"),
            Some(15)
        );
        assert_eq!(remaining_number("0 / 45"), Some(0));
        assert_eq!(remaining_number("45 / 45"), Some(45));
        assert_eq!(remaining_number("： 44 ／ 45"), Some(44));
    }
    #[test]
    fn remaining_fraction_rejects_invalid_or_ambiguous_fields() {
        for text in [
            "15 45",
            "15 / 44",
            "46 / 45",
            "15 / 45 / 45",
            "15 / 45：5 / 45",
            "15 / 45：5 ／ 45",
            "： ／ 45",
            "1?5 / 45",
            "15 / 4?5",
            "+15 / 45",
            "1 5 4 5 / 45",
            "第3轮 1 5 / 45",
            "： / 45",
            "15 / 45 3",
        ] {
            assert_eq!(remaining_number(text), None, "{text}");
        }
    }
    #[test]
    fn conflicting_numeric_views_stay_unread() {
        assert_eq!(consensus(&[Some(15), None]), Some(15));
        assert_eq!(consensus(&[Some(15), Some(15)]), Some(15));
        assert_eq!(consensus(&[Some(15), Some(5)]), None);
        assert_eq!(consensus(&[None, None]), None);
    }
    #[test]
    fn real_round6_remaining44_keeps_fraction_and_partial_board_consistent() {
        let _engine = pixel_test_engine();
        let frame = image::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vision-pc-user-20261008-remaining44-partial.png"
        ))
        .unwrap()
        .to_rgba8();
        let hud = read(&frame);
        assert_eq!(hud.remaining, Some(44));
        assert_eq!(hud.round.as_deref(), Some("6"));
        assert_eq!(hud.counts, [Some(1), Some(4), Some(3)]);
    }
    #[test]
    fn invalid_content_does_not_read_or_guess_hud() {
        let frame = RgbaImage::new(960, 570);
        for content in [
            [0.0, 0.0, 0.0, 0.0],
            [-1.0, 0.0, 960.0, 540.0],
            [0.0, 0.0, 961.0, 540.0],
            [0.0, 31.0, 960.0, 540.0],
            [0.0, 0.0, f64::NAN, 540.0],
        ] {
            let hud = read_in_content(&frame, content);
            assert_eq!(hud.remaining, None);
            assert_eq!(hud.round, None);
            assert_eq!(hud.counts, [None; 3]);
        }
    }
    #[test]
    fn real_mid_round_card_counts_are_remaining_counts() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
        let frame = image::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vision-phones-completed.png"
        ))
        .unwrap()
        .to_rgba8();
        let hud = read(&frame);
        assert_eq!(hud.remaining, Some(13));
        assert_eq!(hud.round.as_deref(), Some("1"));
        assert_eq!(hud.counts, [Some(0), Some(1), Some(1)]);
    }
    #[test]
    fn real_round_three_remaining_keeps_the_split_tens_digit() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
        let frame = image::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vision-ocr-round-three-remaining-15.png"
        ))
        .unwrap()
        .to_rgba8();
        let hud = read(&frame);
        assert_eq!(hud.remaining, Some(15));
        assert_eq!(hud.round.as_deref(), Some("3"));
        assert_eq!(hud.counts, [Some(0), Some(0), Some(2)]);
    }
    #[test]
    fn small_round_three_hud_reads_all_current_numbers() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
        let frame = image::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vision-ocr-round-three-remaining-15.png"
        ))
        .unwrap()
        .to_rgba8();
        for (width, height) in [(1440, 855), (960, 570)] {
            let resized = image::imageops::resize(&frame, width, height, FilterType::CatmullRom);
            let hud = read(&resized);
            assert_eq!(hud.remaining, Some(15), "{width}x{height}: {hud:?}");
            assert_eq!(hud.round.as_deref(), Some("3"), "{width}x{height}: {hud:?}");
            assert_eq!(
                hud.counts,
                [Some(0), Some(0), Some(2)],
                "{width}x{height}: {hud:?}"
            );
        }
    }
    #[test]
    fn real_fullscreen_hud_is_normalized_to_window_ocr_scale() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
        let frame = image::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vision-ocr-fullscreen-3840.png"
        ))
        .unwrap()
        .to_rgba8();
        let hud = read(&frame);
        assert_eq!(hud.remaining, Some(13));
        assert_eq!(hud.round.as_deref(), Some("1"));
        assert_eq!(hud.counts, [Some(0), Some(1), Some(1)]);
    }
    #[test]
    fn real_round_two_hud_reads_window_and_fullscreen_counts() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
        for (filename, remaining, counts) in [
            ("vision-round-two-initial.png", 45, [1, 2, 5]),
            ("vision-ocr-round-two-fullscreen-3840.png", 42, [1, 2, 4]),
        ] {
            let frame = image::open(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures")
                    .join(filename),
            )
            .unwrap()
            .to_rgba8();
            let hud = read(&frame);
            assert_eq!(hud.remaining, Some(remaining), "{filename}");
            assert_eq!(hud.round.as_deref(), Some("2"), "{filename}");
            assert_eq!(hud.counts, counts.map(Some), "{filename}");
        }
    }
    #[test]
    fn window_hud_survives_larger_viewport_scales() {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
        let frame = image::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vision-phones-completed.png"
        ))
        .unwrap()
        .to_rgba8();
        for width in [2560, 3840] {
            let height =
                (frame.height() as f64 * width as f64 / frame.width() as f64).round() as u32;
            let resized = image::imageops::resize(&frame, width, height, FilterType::CatmullRom);
            let hud = read(&resized);
            assert_eq!(hud.remaining, Some(13), "width={width}");
            assert_eq!(hud.round.as_deref(), Some("1"), "width={width}");
            assert_eq!(hud.counts, [Some(0), Some(1), Some(1)], "width={width}");
        }
    }
}
