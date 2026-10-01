#[path = "../src/vision.rs"]
mod vision;

use image::{imageops, Rgba, RgbaImage};
use vision::{locate_content, Recognizer};

#[test]
fn native_four_three_layout_survives_scale_and_surrounding_space() {
    let native = image::open("tests/fixtures/pc-global-zh-hant-four-three-initial.png")
        .unwrap()
        .to_rgba8();
    let content = imageops::crop_imm(&native, 2, 45, 1280, 960).to_image();
    for (width, margin) in [(960, 0), (1920, 0), (960, 37), (1440, 63)] {
        let height = width * 3 / 4;
        let scaled = imageops::resize(&content, width, height, imageops::FilterType::CatmullRom);
        let mut frame = RgbaImage::from_pixel(
            width + margin * 2,
            height + margin * 2,
            Rgba([24, 27, 31, 255]),
        );
        imageops::overlay(&mut frame, &scaled, margin as i64, margin as i64);
        let started = std::time::Instant::now();
        let region = locate_content(&frame).expect("derived 4:3 region");
        eprintln!(
            "derived 4:3 content {width}/{margin}: {:?}",
            started.elapsed()
        );
        assert_eq!(
            region,
            [margin as f64, margin as f64, width as f64, height as f64]
        );
        let result = Recognizer::new().analyze_completed_in_content(
            &frame,
            region,
            None,
            Some(45),
            [Some(2), Some(5), Some(2)],
        );
        assert!(result.present, "{width}/{margin}: {}", result.message);
        assert_eq!(result.cells, vec!["unknown"; 45], "{width}/{margin}");
        assert_eq!(
            result.shapes,
            vec![[3, 2], [3, 1], [2, 1]],
            "{width}/{margin}"
        );
        assert_eq!(result.reference_ready, [true; 3], "{width}/{margin}");
        let board = result.board.unwrap();
        let scale = width as f64 / 1920.0;
        assert!((board[0] * frame.width() as f64 - margin as f64 - 908.0 * scale).abs() < 0.01);
        assert!((board[1] * frame.height() as f64 - margin as f64 - 466.0 * scale).abs() < 0.01);
        assert!((board[2] * frame.width() as f64 - 936.0 * scale).abs() < 0.01);
        assert!((board[3] * frame.height() as f64 - 520.0 * scale).abs() < 0.01);
    }
}

#[test]
fn native_four_three_rejects_occluded_board() {
    let native = image::open("tests/fixtures/pc-global-zh-hant-four-three-initial.png")
        .unwrap()
        .to_rgba8();
    let mut frame = imageops::crop_imm(&native, 2, 45, 1280, 960).to_image();
    for y in 309..659 {
        for x in 604..1232 {
            frame.put_pixel(x, y, Rgba([247, 248, 250, 255]));
        }
    }
    assert!(locate_content(&frame).is_none());
}

fn game(name: &str) -> RgbaImage {
    let frame = image::open(format!("tests/fixtures/vision-{name}.png"))
        .unwrap()
        .to_rgba8();
    imageops::crop_imm(&frame, 2, 60, 1920, 1080).to_image()
}

fn scaled(game: &RgbaImage, width: u32) -> RgbaImage {
    imageops::resize(game, width, width * 9 / 16, imageops::FilterType::Triangle)
}

fn embedded(game: &RgbaImage, width: u32, frame_size: (u32, u32), offset: (u32, u32)) -> RgbaImage {
    let mut frame = RgbaImage::from_pixel(frame_size.0, frame_size.1, Rgba([18, 20, 24, 255]));
    // A toolbar is independently sized; it need not share the game's width.
    for y in 0..32 {
        for x in 0..frame_size.0 {
            frame.put_pixel(x, y, Rgba([55, 61, 68, 255]));
        }
    }
    imageops::overlay(
        &mut frame,
        &scaled(game, width),
        offset.0 as i64,
        offset.1 as i64,
    );
    frame
}

#[test]
fn completed_production_path_scales_without_a_resolution_ceiling() {
    let source = game("round-two-completed");
    let mut recognizer = Recognizer::new();
    let initial = recognizer.analyze_completed_snapshot(
        &game("round-two-initial"),
        None,
        Some(45),
        [Some(1), Some(2), Some(5)],
    );
    assert!(initial.present, "{}", initial.message);
    let baseline =
        recognizer.analyze_completed_snapshot(&source, None, Some(27), [Some(0), Some(1), Some(3)]);
    assert!(baseline.present, "{}", baseline.message);
    for width in [960, 1440, 1920, 2560, 3840] {
        let frame = scaled(&source, width);
        let content = locate_content(&frame).expect("content rectangle");
        assert_eq!(content, [0.0, 0.0, width as f64, (width * 9 / 16) as f64]);
        let result = recognizer.analyze_completed_in_content(
            &frame,
            content,
            None,
            Some(27),
            [Some(0), Some(1), Some(3)],
        );
        assert!(result.present, "{width}: {}", result.message);
        assert_eq!(result.cells, baseline.cells, "{width}");
        assert_eq!(result.shapes, baseline.shapes, "{width}");
        assert_eq!(result.finish, [true, false, false], "{width}");
        assert_eq!(
            result.card_fingerprints[0], initial.card_fingerprints[0],
            "Finish must retain this round's cached reference at {width}"
        );
        assert_eq!(
            result.completed_objects, baseline.completed_objects,
            "{width}"
        );
        assert!(result.candidate_constraints.is_empty());
    }
}

#[test]
fn toolbar_letterbox_and_same_size_content_translation_recompute_board() {
    let source = game("opened");
    let baseline = Recognizer::new().analyze_completed_snapshot(&source, None, Some(44), [None; 3]);
    assert!(baseline.present);
    let mut recognizer = Recognizer::new();
    for (width, frame_size, offset) in [
        (960, (1280, 800), (83, 91)),
        (960, (1280, 800), (149, 151)),
        (1440, (1800, 1200), (201, 133)),
        (2560, (3000, 1900), (137, 311)),
    ] {
        let frame = embedded(&source, width, frame_size, offset);
        let content = locate_content(&frame).expect("embedded content rectangle");
        for (actual, expected) in content.into_iter().zip([
            offset.0 as f64,
            offset.1 as f64,
            width as f64,
            (width * 9 / 16) as f64,
        ]) {
            assert!(
                (actual - expected).abs() < 1.0,
                "{width} at {offset:?}: {content:?}"
            );
        }
        let result =
            recognizer.analyze_completed_in_content(&frame, content, None, Some(44), [None; 3]);
        assert!(result.present, "{width} at {offset:?}: {}", result.message);
        assert_eq!(result.cells, baseline.cells);
        let board = result.board.unwrap();
        assert!(
            (board[0] * frame_size.0 as f64 - offset.0 as f64 - width as f64 * 908.0 / 1920.0)
                .abs()
                < 1.0
        );
        assert!(
            (board[1] * frame_size.1 as f64 - offset.1 as f64 - width as f64 * 286.0 / 1920.0)
                .abs()
                < 1.0
        );
    }
}

#[test]
fn margins_are_candidates_and_never_proof_of_a_game() {
    let blank = RgbaImage::from_pixel(1920, 1080, Rgba([220, 225, 232, 255]));
    assert!(locate_content(&embedded(&blank, 960, (1280, 800), (123, 97))).is_none());
    let mut occluded = game("round-two-initial");
    for y in 180..830 {
        for x in 900..1860 {
            occluded.put_pixel(x, y, Rgba([240, 240, 240, 255]));
        }
    }
    assert!(locate_content(&embedded(&occluded, 960, (1280, 800), (123, 97))).is_none());
}
#[test]
fn a_same_size_content_move_discards_pixel_corrections_but_not_card_templates() {
    let source = game("full-watergun");
    let mut recognizer = Recognizer::new();
    let first = embedded(&source, 960, (1280, 800), (70, 90));
    let result = recognizer.analyze_completed_snapshot(&first, None, Some(36), [None; 3]);
    assert!(result.present, "{}", result.message);
    assert_eq!(result.cells[24], "completed");
    recognizer.correct(24, "empty").unwrap();
    let corrected = recognizer.analyze_completed_snapshot(&first, None, Some(36), [None; 3]);
    assert_eq!(corrected.cells[24], "empty");
    let moved = embedded(&source, 960, (1280, 800), (170, 150));
    let relocated = recognizer.analyze_completed_snapshot(&moved, None, Some(36), [None; 3]);
    assert!(relocated.present, "{}", relocated.message);
    assert_eq!(relocated.cells[24], "completed");
    assert_eq!(relocated.card_fingerprints, result.card_fingerprints);
}
#[test]
fn locator_rejects_board_popup_with_visible_header_and_cards() {
    let mut source = game("round-two-initial");
    for y in 286..806 {
        for x in 908..1844 {
            source.put_pixel(x, y, Rgba([247, 248, 250, 255]));
        }
    }
    let frame = embedded(&source, 960, (1280, 800), (123, 97));
    let started = std::time::Instant::now();
    assert!(locate_content(&frame).is_none());
    eprintln!(
        "occluded embedded content location: {:?}",
        started.elapsed()
    );
}
#[test]
fn real_round_seven_whole_window_scaling_uses_measured_chrome() {
    let source = image::open("tests/fixtures/vision-resolution-round-seven.png")
        .unwrap()
        .to_rgba8();
    let mut expected = vec!["unknown"; 45];
    for index in [11, 12, 14, 23, 27, 28, 29, 30, 32, 36, 37, 38, 39] {
        expected[index] = "completed";
    }
    for width in [960, 1440, 1960, 2560, 3840] {
        let height = (source.height() as f64 * width as f64 / source.width() as f64).round() as u32;
        let frame = imageops::resize(&source, width, height, imageops::FilterType::CatmullRom);
        let content = locate_content(&frame).expect("whole-window content");
        let result = Recognizer::new().analyze_completed_in_content(
            &frame,
            content,
            None,
            Some(32),
            [Some(1), Some(2), Some(5)],
        );
        assert!(result.present, "{width}: {}", result.message);
        assert_eq!(
            result.shapes,
            vec![[4, 2], [3, 1], [2, 1]],
            "{width}: {content:?}"
        );
        assert_eq!(result.cells, expected, "{width}: {content:?}");
        assert_eq!(result.completed_objects.len(), 3, "{width}: {content:?}");
        if width == 960 {
            assert_eq!(content, [1.0, 29.125, 958.0, 538.875]);
        }
    }
}

#[test]
fn native_maximized_round_seven_ignores_artwork_above_the_shape_panel() {
    let frame = image::open("tests/fixtures/vision-resolution-round-seven-maximized.png")
        .unwrap()
        .to_rgba8();
    let content = locate_content(&frame).expect("maximized content");
    assert_eq!(content, [114.0, 56.25, 3612.0, 2031.75]);
    let result = Recognizer::new().analyze_completed_in_content(
        &frame,
        content,
        None,
        Some(32),
        [Some(1), Some(2), Some(5)],
    );
    let mut expected = vec!["unknown"; 45];
    for index in [11, 12, 14, 23, 27, 28, 29, 30, 32, 36, 37, 38, 39] {
        expected[index] = "completed";
    }
    assert!(result.present, "{}", result.message);
    assert_eq!(result.shapes, vec![[4, 2], [3, 1], [2, 1]]);
    assert_eq!(result.cells, expected);
    assert_eq!(result.completed_objects.len(), 3);
}

#[test]
fn native_window_resize_preserves_every_completed_round_seven_object() {
    let mut expected = vec!["completed"; 45];
    for index in [0, 1, 9, 10, 13, 15, 20, 21, 41, 43, 44] {
        expected[index] = "unknown";
    }
    expected[2] = "empty";
    let initial = image::open("tests/fixtures/vision-resolution-round-seven.png")
        .unwrap()
        .to_rgba8();
    let mut warm = Recognizer::new();
    let seeded =
        warm.analyze_completed_snapshot(&initial, None, Some(32), [Some(1), Some(2), Some(5)]);
    assert!(seeded.present);
    assert_eq!(seeded.reference_ready, [true; 3]);
    for name in ["maximized", "small", "internal720"] {
        let frame = image::open(format!(
            "tests/fixtures/vision-resolution-round-seven-eleven-{name}.png"
        ))
        .unwrap()
        .to_rgba8();
        let content = locate_content(&frame).expect("native content");
        let mut cold = Recognizer::new();
        for (mode, recognizer) in [("cold", &mut cold), ("warm", &mut warm)] {
            let result = recognizer.analyze_completed_in_content(
                &frame,
                content,
                None,
                Some(11),
                [Some(0), Some(0), Some(2)],
            );
            assert!(result.present, "{name}/{mode}: {}", result.message);
            assert_eq!(result.shapes, vec![[4, 2], [3, 1], [2, 1]], "{name}/{mode}");
            assert_eq!(result.cells, expected, "{name}/{mode}");
            assert_eq!(result.completed_objects.len(), 9, "{name}/{mode}");
            assert!(
                result.completed_objects.contains(&vision::CompletedObject {
                    item_index: Some(2),
                    x: 0,
                    y: 2,
                    width: 2,
                    height: 1,
                }),
                "{name}/{mode}"
            );
            assert_eq!(result.finish, [true, true, false], "{name}/{mode}");
            if mode == "warm" {
                assert_eq!(
                    &result.card_fingerprints[..2],
                    &seeded.card_fingerprints[..2],
                    "{name}: Finish retains this round's references"
                );
                assert_eq!(result.reference_ready, [true; 3], "{name}");
            }
        }
    }
}
