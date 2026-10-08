#[path = "../src/vision.rs"]
mod vision;

use image::{imageops, Rgba, RgbaImage};
use vision::Recognizer;

fn fullscreen() -> RgbaImage {
    // Native international-client WGC capture, 2026-10-06: round 2, all 45
    // covers closed. Window sizes below are derived, not native captures.
    image::open("tests/fixtures/vision-pc-global-fullscreen-initial.png")
        .unwrap()
        .to_rgba8()
}

fn analyze(recognizer: &mut Recognizer, frame: &RgbaImage) -> vision::Analysis {
    recognizer.analyze_completed_in_content(
        frame,
        [0.0, 0.0, frame.width() as f64, frame.height() as f64],
        None,
        Some(45),
        [Some(1), Some(2), Some(5)],
    )
}

fn assert_closed(result: &vision::Analysis, context: &str) {
    assert!(result.present, "{context}: {}", result.message);
    assert_eq!(result.cells, vec!["unknown"; 45], "{context}");
    assert_eq!(result.shapes, vec![[4, 2], [4, 1], [3, 1]], "{context}");
    assert!(result.candidate_constraints.is_empty());
}

#[test]
fn native_fullscreen_initial_board_is_closed_from_cold_start() {
    let frame = fullscreen();
    let mut recognizer = Recognizer::new();
    assert_eq!(recognizer.locate_content(&frame), Some([0.0, 0.0, 3840.0, 2160.0]));
    for _ in 0..3 {
        assert_closed(&analyze(&mut recognizer, &frame), "native fullscreen");
    }
}

#[test]
fn learned_covers_survive_window_fullscreen_sampling_in_both_directions() {
    let full = fullscreen();
    for width in [1280, 1920, 2560] {
        let window = imageops::resize(&full, width, width * 9 / 16, imageops::FilterType::Triangle);
        for (seed, target) in [(&window, &full), (&full, &window)] {
            let mut recognizer = Recognizer::new();
            for _ in 0..2 {
                assert_closed(&analyze(&mut recognizer, seed), "learned initial board");
            }
            for _ in 0..2 {
                assert_closed(&analyze(&mut recognizer, target), &format!("{width}: {:?} -> {:?}", seed.dimensions(), target.dimensions()));
            }
        }
    }
}

#[test]
fn resized_reference_keeps_small_center_and_edge_fragments_unconfirmed() {
    let full = fullscreen();
    let window = imageops::resize(&full, 1280, 720, imageops::FilterType::Triangle);
    for (x, y) in [(1910, 664), (1830, 664)] {
        let mut recognizer = Recognizer::new();
        for _ in 0..2 {
            assert_closed(&analyze(&mut recognizer, &window), "learned window");
        }
        let mut partial = full.clone();
        // A 10px fragment at the 1920px layout scale, at the center or beside
        // the bevel. The remaining counter cannot convert it back to a cover.
        for yy in y..y + 20 {
            for xx in x..x + 20 {
                partial.put_pixel(xx, yy, Rgba([245, 20, 235, 255]));
            }
        }
        for remaining in [Some(44), Some(45)] {
            let result = recognizer.analyze_completed_snapshot(&partial, None, remaining, [None; 3]);
            assert!(result.present, "{}", result.message);
            assert_eq!(result.cells[0], "uncertain");
            assert_eq!(result.cells.iter().filter(|c| *c == "unknown").count(), 44);
            assert!(result.candidate_constraints.is_empty());
        }
    }
}



#[test]
fn native_window_and_fullscreen_preserve_the_same_closed_round() {
    let full = fullscreen();
    // Same round and tile arrangement, captured after F11 switched to window.
    let window = image::open("tests/fixtures/vision-pc-global-window-initial.png")
        .unwrap()
        .to_rgba8();
    for frames in [[&window, &full, &window], [&full, &window, &full]] {
        let mut recognizer = Recognizer::new();
        for frame in frames {
            let expected = if frame.width() == 3840 {
                [0.0, 0.0, 3840.0, 2160.0]
            } else {
                [2.0, 45.0, 1280.0, 720.0]
            };
            assert_eq!(recognizer.locate_content(frame), Some(expected));
            for _ in 0..2 {
                let result = recognizer.analyze_completed_snapshot(frame, None, Some(45), [Some(1), Some(2), Some(5)]);
                assert_closed(&result, &format!("native {:?}", frame.dimensions()));
            }
        }
    }
}
