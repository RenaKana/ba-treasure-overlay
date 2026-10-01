//! Local Windows OCR. Failed or ambiguous readings remain None.
use image::{imageops::FilterType, Rgba, RgbaImage};
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
}

const TEXT_HEIGHT: f64 = 80.0;
const COUNT_COPIES: usize = 3;

fn isolate_count_plate(crop: &RgbaImage) -> Option<(RgbaImage, Rgba<u8>)> {
    // The count plate's connected pale background excludes the blue card and
    // decorative frame. Keep the enclosed glyphs, filling outside each row's
    // plate boundary with the observed background rather than those edges.
    let (width, height) = crop.dimensions();
    let light = |p: &Rgba<u8>| {
        let min = p[0].min(p[1]).min(p[2]);
        let max = p[0].max(p[1]).max(p[2]);
        min >= 235 && max - min <= 25
    };
    let mut seen = vec![false; width as usize * height as usize];
    let mut largest = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            if seen[index] || !light(crop.get_pixel(x, y)) {
                continue;
            }
            seen[index] = true;
            let mut component = vec![(x, y)];
            let mut next = 0;
            while next < component.len() {
                let (cx, cy) = component[next];
                next += 1;
                for (nx, ny) in [
                    (cx.wrapping_sub(1), cy),
                    (cx + 1, cy),
                    (cx, cy.wrapping_sub(1)),
                    (cx, cy + 1),
                ] {
                    if nx >= width || ny >= height {
                        continue;
                    }
                    let index = (ny * width + nx) as usize;
                    if !seen[index] && light(crop.get_pixel(nx, ny)) {
                        seen[index] = true;
                        component.push((nx, ny));
                    }
                }
            }
            if component.len() > largest.len() {
                largest = component;
            }
        }
    }
    let left = largest.iter().map(|(x, _)| *x).min()?;
    let right = largest.iter().map(|(x, _)| *x).max()?;
    let top = largest.iter().map(|(_, y)| *y).min()?;
    let bottom = largest.iter().map(|(_, y)| *y).max()?;
    let mut rows = vec![None::<(u32, u32)>; height as usize];
    let mut sum = [0_u64; 3];
    for (x, y) in &largest {
        let range = rows[*y as usize].get_or_insert((*x, *x));
        range.0 = range.0.min(*x);
        range.1 = range.1.max(*x);
        for channel in 0..3 {
            sum[channel] += crop.get_pixel(*x, *y)[channel] as u64;
        }
    }
    let count = largest.len() as u64;
    let background = Rgba([
        (sum[0] / count) as u8,
        (sum[1] / count) as u8,
        (sum[2] / count) as u8,
        255,
    ]);
    let mut plate = RgbaImage::from_pixel(right - left + 1, bottom - top + 1, background);
    for y in top..=bottom {
        if let Some((start, end)) = rows[y as usize] {
            for x in start..=end {
                plate.put_pixel(x - left, y - top, *crop.get_pixel(x, y));
            }
        }
    }
    Some((plate, background))
}

fn count_ink(crop: &RgbaImage) -> Option<RgbaImage> {
    let mut ink = crop.enumerate_pixels().filter(|(_, _, p)| {
        let min = p[0].min(p[1]).min(p[2]);
        let max = p[0].max(p[1]).max(p[2]);
        max < 235 && max - min <= 25
    });
    let (x, y, _) = ink.next()?;
    let (mut left, mut right, mut top, mut bottom) = (x, x, y, y);
    for (x, y, _) in ink {
        left = left.min(x);
        right = right.max(x);
        top = top.min(y);
        bottom = bottom.max(y);
    }
    Some(image::imageops::crop_imm(crop, left, top, right - left + 1, bottom - top + 1).to_image())
}

fn text_at_scale(
    engine: &OcrEngine,
    frame: &RgbaImage,
    content: [f64; 4],
    r: [f64; 4],
    target_height: Option<f64>,
    contrast: bool,
    plate: bool,
) -> Option<String> {
    let [x, y, w, h] = content;
    let rx = (x + r[0] * w).floor() as u32;
    let ry = (y + r[1] * h).floor() as u32;
    let right = (x + (r[0] + r[2]) * w).ceil() as u32;
    let bottom = (y + (r[1] + r[3]) * h).ceil() as u32;
    if right > frame.width() || bottom > frame.height() || right <= rx || bottom <= ry {
        return None;
    }
    let crop = image::imageops::crop_imm(frame, rx, ry, right - rx, bottom - ry).to_image();
    let (crop, plate_background) = if plate {
        let (crop, background) = isolate_count_plate(&crop)?;
        (count_ink(&crop)?, Some(background))
    } else {
        (crop, None)
    };
    let copies = if plate { COUNT_COPIES as u32 } else { 1 };
    let (rw, rh) = crop.dimensions();
    // Normalize both small and large text, bounded by the OCR engine's real
    // image limit rather than an arbitrary maximum enlargement factor.
    let max_dimension = OcrEngine::MaxImageDimension().ok()?;
    // Ordinary views retain their full-crop normalization. After plate/ink
    // isolation, keep the original ROI's geometric text scale rather than
    // enlarging its glyphs to the height of the removed background.
    let source_height = if plate { r[3] * h } else { rh as f64 };
    let requested_scale = target_height.map_or(1.0, |height| height / source_height);
    let requested_padding = target_height.unwrap_or(rh as f64) / 10.0;
    let scale_limit = max_dimension as f64
        / ((rw as f64 + 2.0 * requested_padding / requested_scale) * copies as f64)
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
        let total = data.width() as u64 * data.height() as u64;
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
        .max_by_key(|(count, _)| *count)?;
    let background = plate_background.unwrap_or(Rgba([
        (sum[0] / count as u64) as u8,
        (sum[1] / count as u64) as u8,
        (sum[2] / count as u64) as u8,
        255,
    ]));
    let mut padded = RgbaImage::from_pixel(
        data.width() + 2 * padding,
        data.height() + 2 * padding,
        background,
    );
    image::imageops::overlay(&mut padded, &data, padding as i64, padding as i64);
    data = padded;
    if copies > 1 {
        let mut line = RgbaImage::from_pixel(data.width() * copies, data.height(), background);
        for copy in 0..copies {
            image::imageops::overlay(&mut line, &data, (copy * data.width()) as i64, 0);
        }
        data = line;
    }
    for p in data.pixels_mut() {
        p.0.swap(0, 2);
    }
    let writer = DataWriter::new().ok()?;
    writer.WriteBytes(data.as_raw()).ok()?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &writer.DetachBuffer().ok()?,
        BitmapPixelFormat::Bgra8,
        data.width() as i32,
        data.height() as i32,
        BitmapAlphaMode::Ignore,
    )
    .ok()?;
    engine
        .RecognizeAsync(&bitmap)
        .ok()?
        .join()
        .ok()?
        .Text()
        .ok()
        .map(|s| s.to_string())
}

fn consensus(readings: &[Option<u32>]) -> Option<u32> {
    let first = readings.iter().flatten().next()?;
    readings
        .iter()
        .flatten()
        .all(|value| value == first)
        .then_some(*first)
}

fn repeated_counts(text: &str) -> Option<Vec<u32>> {
    let mut fields = text.split(['×', 'x', 'X']);
    if !fields.next()?.trim().is_empty() {
        return None;
    }
    let counts = fields
        .map(|field| {
            let digits = field
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            digits.parse::<u32>().ok()
        })
        .collect::<Option<Vec<_>>>()?;
    (counts.len() == COUNT_COPIES).then_some(counts)
}

fn number_from_views(
    engine: &OcrEngine,
    frame: &RgbaImage,
    content: [f64; 4],
    views: &[[f64; 4]],
    parse: fn(&str, bool) -> Option<u32>,
    count_plate: bool,
) -> Option<u32> {
    let read_views = |height, contrast, retry| {
        views
            .iter()
            .map(|r| {
                text_at_scale(engine, frame, content, *r, height, contrast, false)
                    .and_then(|t| parse(&t, retry))
            })
            .collect::<Vec<_>>()
    };
    let readings = read_views(Some(TEXT_HEIGHT), false, false);
    if readings.iter().any(Option::is_some) {
        return consensus(&readings);
    }
    // Preserve the native-size retry when a large capture's thin digits were
    // lost during downsampling. No retry can override conflicting readings.
    if views.iter().any(|r| r[3] * content[3] > TEXT_HEIGHT) {
        let readings = read_views(None, false, false);
        if readings.iter().any(Option::is_some) {
            return consensus(&readings);
        }
    }
    for (height, contrast) in [(60.0, false), (120.0, false), (TEXT_HEIGHT, true)] {
        let readings = read_views(Some(height), contrast, true);
        if readings.iter().any(Option::is_some) {
            return consensus(&readings);
        }
    }
    if count_plate {
        // Only after every normal view is unread, repeat the same observed
        // plate text to give the short token a line. Every copy must retain a
        // real multiplication prefix and valid digits; all copies and views
        // must agree. An earlier conflict never reaches this supplement.
        let readings = views
            .iter()
            .filter_map(|r| {
                text_at_scale(engine, frame, content, *r, Some(TEXT_HEIGHT), false, true)
                    .and_then(|text| repeated_counts(&text))
            })
            .flatten()
            .map(Some)
            .collect::<Vec<_>>();
        return consensus(&readings);
    }
    None
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
    if s.matches('/').count() != 1 {
        return None;
    }
    let payload = s.rsplit([':', '：']).next()?;
    let (numerator, denominator) = payload.split_once('/')?;
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
    let engine = Language::CreateLanguage(&"zh-Hans".into())
        .ok()
        .and_then(|l| OcrEngine::TryCreateFromLanguage(&l).ok())
        .or_else(|| OcrEngine::TryCreateFromUserProfileLanguages().ok());
    let Some(engine) = engine else {
        return Hud::default();
    };
    let centered = crate::vision::layout_region(content, crate::vision::LayoutAnchor::Center);
    let bottom = crate::vision::layout_region(content, crate::vision::LayoutAnchor::Bottom);
    let remaining = number_from_views(
        &engine,
        frame,
        centered,
        &[[0.649, 0.222, 0.153, 0.035], [0.736, 0.222, 0.055, 0.035]],
        |text, _| remaining_number(text),
        false,
    );
    let round = number_from_views(
        &engine,
        frame,
        centered,
        &[[0.651, 0.18, 0.156, 0.046], [0.729, 0.18, 0.050, 0.046]],
        |text, _| {
            numbers(text)
                .last()
                .copied()
                .filter(|n| *n > 0 && *n < 10000)
        },
        false,
    )
    .map(|n| n.to_string());
    let counts = [0.153, 0.264, 0.371].map(|x| {
        // Two bounded views of the same count plate. Windows OCR can miss ×1
        // when it touches a tight crop. Disagreement is never guessed away.
        let tight_rect = [x, 0.931, 0.035, 0.045];
        let roomy_rect = [x - 0.002, 0.928, 0.040, 0.040];
        number_from_views(
            &engine,
            frame,
            bottom,
            &[tight_rect, roomy_rect],
            |text, retry| {
                if retry && !matches!(text.trim_start().chars().next(), Some('×' | 'x' | 'X')) {
                    return None;
                }
                let ns = numbers(text);
                (ns.len() == 1).then(|| ns[0])
            },
            true,
        )
    });
    Hud {
        remaining,
        round,
        counts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_count_retry_requires_every_actual_prefix_and_digit() {
        for (text, expected) in [("× 1 × 1 × 1", 1), ("x2 X 2 ×2", 2), ("×12 ×1 2 ×12", 12)]
        {
            let readings = repeated_counts(text)
                .unwrap()
                .into_iter()
                .map(Some)
                .collect::<Vec<_>>();
            assert_eq!(consensus(&readings), Some(expected), "{text}");
        }
        let readings = repeated_counts("×1 ×2 ×1")
            .unwrap()
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        assert_eq!(consensus(&readings), None);
        for text in [
            "",
            "xl xl xl",
            "×1 xl ×1",
            "×1 ×1",
            "×1 ×1 ×1 ×1",
            "第7轮 ×1 ×1 ×1",
            "×1 ×1 ×?1",
            "×1 ×1 ×-1",
        ] {
            assert_eq!(repeated_counts(text), None, "{text}");
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
    }
    #[test]
    fn remaining_fraction_rejects_invalid_or_ambiguous_fields() {
        for text in [
            "15 45",
            "15 / 44",
            "46 / 45",
            "15 / 45 / 45",
            "15 / 45：5 / 45",
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
