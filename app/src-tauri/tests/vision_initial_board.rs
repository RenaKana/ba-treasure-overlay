#[path = "../src/vision.rs"]
mod vision;

use image::{imageops, Rgba, RgbaImage};
use vision::Recognizer;

fn initial() -> RgbaImage {
    let frame = image::load_from_memory(include_bytes!("fixtures/vision-initial.png"))
        .unwrap()
        .to_rgba8();
    imageops::crop_imm(&frame, 2, 60, 1920, 1080).to_image()
}

// A different flat outlined skin with a distinct pattern in each position.
// None of the old captured cover pixels are reused in these 45 squares.
fn draw_covers(frame: &mut RgbaImage, variant: u8) {
    for index in 0..45 {
        let left = 908 + (index % 9) * 104;
        let top = 286 + (index / 9) * 104;
        for y in 0..104 {
            for x in 0..104 {
                let nx = x * 32 / 104;
                let ny = y * 32 / 104;
                let color = if !(3..29).contains(&nx) || !(3..29).contains(&ny) {
                    [15, 32, 48]
                } else if nx == 3 || nx == 28 || ny == 3 || ny == 28 {
                    [235, 200, 250]
                } else {
                    [
                        45 + (index % 9) as u8 * 17 + ((nx + ny) % 3) as u8,
                        25 + variant,
                        90 + (index / 9) as u8 * 30,
                    ]
                };
                frame.put_pixel(left + x, top + y, Rgba([color[0], color[1], color[2], 255]));
            }
        }
    }
}

fn novel_initial() -> RgbaImage {
    let mut frame = initial();
    draw_covers(&mut frame, 0);
    frame
}

fn learn(recognizer: &mut Recognizer, frame: &RgbaImage) {
    let first = recognizer.analyze_completed_snapshot(frame, None, Some(45), [None; 3]);
    assert!(!first.present, "new skin must not pass legacy matching");
    let learned = recognizer.analyze_completed_snapshot(frame, None, Some(45), [None; 3]);
    assert!(learned.present, "{}", learned.message);
    assert!(learned.cells.iter().all(|s| s == "unknown"));
}

#[test]
fn learns_unfamiliar_initial_covers_then_recognizes_the_partial_board() {
    let frame = novel_initial();
    let mut recognizer = Recognizer::new();
    assert!(recognizer.locate_content(&frame).is_some());
    learn(&mut recognizer, &frame);

    let opened = image::load_from_memory(include_bytes!("fixtures/vision-opened.png"))
        .unwrap()
        .to_rgba8();
    let tile = imageops::crop_imm(&opened, 910 + 6 * 104, 346 + 104, 104, 104).to_image();
    let mut partial = frame.clone();
    imageops::replace(&mut partial, &tile, 908 + 6 * 104, 286 + 104);
    assert!(recognizer.locate_content(&partial).is_some());
    let result = recognizer.analyze_completed_snapshot(&partial, None, Some(44), [None; 3]);
    assert!(result.present, "{}", result.message);
    assert_eq!(result.cells[15], "empty");
    assert_eq!(result.cells.iter().filter(|s| *s == "unknown").count(), 44);
}

#[test]
fn compares_only_the_corresponding_cell_and_rejects_small_new_artwork() {
    let frame = novel_initial();
    let mut recognizer = Recognizer::new();
    learn(&mut recognizer, &frame);
    let mut changed = frame.clone();
    let other = imageops::crop_imm(&frame, 908 + 8 * 104, 286 + 4 * 104, 104, 104).to_image();
    imageops::replace(&mut changed, &other, 908, 286);
    // Preserve almost the entire closed outline but expose a small fragment.
    for y in 332..342 {
        for x in 1058..1068 {
            changed.put_pixel(x, y, Rgba([10, 240, 30, 255]));
        }
    }
    let result = recognizer.analyze_completed_snapshot(&changed, None, Some(43), [None; 3]);
    assert_ne!(
        result.cells[0], "unknown",
        "another cell is not a reference for this position"
    );
    assert_ne!(
        result.cells[1], "unknown",
        "small revealed fragment must not be swallowed"
    );
    assert_eq!(result.cells.iter().filter(|s| *s == "unknown").count(), 43);
}

#[test]
fn missing_or_noninitial_count_and_occlusion_break_pending_learning() {
    let frame = novel_initial();
    for count in [None, Some(44)] {
        let mut recognizer = Recognizer::new();
        for _ in 0..2 {
            let result = recognizer.analyze_completed_snapshot(&frame, None, count, [None; 3]);
            assert!(!result.present);
        }
        learn(&mut recognizer, &frame);
    }
    let mut recognizer = Recognizer::new();
    assert!(!recognizer.analyze(&frame, None, Some(45)).present);
    let mut popup = frame.clone();
    for y in 210..930 {
        for x in 948..1758 {
            popup.put_pixel(x, y, Rgba([250, 250, 250, 255]));
        }
    }
    assert!(recognizer.locate_content(&popup).is_none());
    learn(&mut recognizer, &frame);
}

#[test]
fn stale_45_on_an_opened_board_cannot_seed_references() {
    let mut frame = novel_initial();
    for y in 286..390 {
        for x in 908..1012 {
            frame.put_pixel(x, y, Rgba([220, 226, 236, 255]));
        }
    }
    let mut recognizer = Recognizer::new();
    for _ in 0..3 {
        // Even a caller supplying a valid content rectangle cannot bypass the
        // per-cell evidence required for learning.
        let result = recognizer.analyze_completed_in_content(
            &frame,
            [0.0, 0.0, 1920.0, 1080.0],
            None,
            Some(45),
            [None; 3],
        );
        assert!(!result.present);
    }
    learn(&mut recognizer, &novel_initial());
}

#[test]
fn finish_card_cannot_seed_an_initial_reference() {
    let source = image::load_from_memory(include_bytes!("fixtures/vision-waterguns-finished.png"))
        .unwrap()
        .to_rgba8();
    let mut frame = imageops::crop_imm(&source, 2, 60, 1920, 1080).to_image();
    draw_covers(&mut frame, 0);
    let mut recognizer = Recognizer::new();
    for _ in 0..3 {
        let result = recognizer.analyze_completed_snapshot(&frame, None, Some(45), [None; 3]);
        assert!(!result.present);
    }
    learn(&mut recognizer, &novel_initial());
}

#[test]
fn resize_preserves_confirmed_reference_and_reset_requires_fresh_learning() {
    let frame = novel_initial();
    let mut recognizer = Recognizer::new();
    learn(&mut recognizer, &frame);
    for width in [1440, 960, 1920] {
        let resized = imageops::resize(
            &frame,
            width,
            width * 9 / 16,
            imageops::FilterType::Triangle,
        );
        let result = recognizer.analyze_completed_snapshot(&resized, None, Some(45), [None; 3]);
        assert!(result.present, "width={width}: {}", result.message);
        assert!(result.cells.iter().all(|s| s == "unknown"), "width={width}");
    }
    recognizer.reset();
    let mut next = frame;
    draw_covers(&mut next, 45);
    learn(&mut recognizer, &next);
}

#[test]
fn confirmed_reference_survives_a_popup_and_real_selection_still_works() {
    let frame = initial();
    let mut recognizer = Recognizer::new();
    for _ in 0..2 {
        assert!(recognizer.analyze(&frame, None, Some(45)).present);
    }
    assert!(recognizer
        .locate_content(&RgbaImage::new(1920, 1080))
        .is_none());
    let selected = image::load_from_memory(include_bytes!("fixtures/vision-blue-selected.png"))
        .unwrap()
        .to_rgba8();
    let result = recognizer.analyze_completed_snapshot(&selected, None, Some(24), [None; 3]);
    assert!(result.present, "{}", result.message);
    assert_eq!(result.cells[22], "unknown");
    assert_eq!(result.cells.iter().filter(|s| *s == "unknown").count(), 24);
}

#[test]
fn changed_initial_style_is_a_reset_cue_without_overwriting_old_references() {
    let frame = novel_initial();
    let mut recognizer = Recognizer::new();
    learn(&mut recognizer, &frame);
    let mut next = frame.clone();
    draw_covers(&mut next, 45);
    for _ in 0..2 {
        let result = recognizer.analyze_completed_snapshot(&next, None, Some(45), [None; 3]);
        assert!(
            result.fresh_initial_grid,
            "the old cache must not block the next round"
        );
        assert!(
            !result.present,
            "reset evidence must not relabel changed cells"
        );
    }
    assert!(recognizer.analyze(&frame, None, Some(45)).present);
    recognizer.reset();
    learn(&mut recognizer, &next);
}
