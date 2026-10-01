#[path = "../src/vision.rs"]
mod vision;
use image::{imageops, Rgba, RgbaImage};
use vision::Recognizer;
fn fixture(name: &str) -> RgbaImage {
    image::open(format!("tests/fixtures/vision-{name}.png"))
        .unwrap()
        .to_rgba8()
}
fn expected_latest() -> Vec<String> {
    let mut cells = vec!["unknown".to_owned(); 45];
    for i in [11, 15, 17, 29, 31, 37] {
        cells[i] = "empty".to_owned();
    }
    for i in [
        4, 5, 13, 14, 22, 23, 24, 25, 33, 34, 42, 43, 1, 10, 19, 12, 21, 30, 26, 35, 44, 38, 39,
        40, 6, 7,
    ] {
        cells[i] = "completed".to_owned();
    }
    cells
}
#[test]
fn real_latest_has_seven_complete_objects_without_fragment_atlases() {
    let mut rec = Recognizer::new();
    let frame = fixture("phones-completed");
    let start = std::time::Instant::now();
    let fresh = rec.analyze_snapshot(&frame, None, Some(13), [Some(0), Some(1), Some(1)]);
    eprintln!("latest cold {:?}", start.elapsed());
    assert!(fresh.present, "{}", fresh.message);
    assert_eq!(fresh.cells, expected_latest());
    assert_eq!(fresh.completed_objects.len(), 7);
    assert_eq!(fresh.reference_ready, [false, true, true]);
    assert_eq!(fresh.finish, [true, false, false]);
    assert!(fresh
        .completed_objects
        .iter()
        .filter(|o| o.width * o.height == 6)
        .all(|o| o.item_index.is_none()));
    assert_eq!(
        fresh
            .completed_objects
            .iter()
            .filter(|o| o.item_index == Some(1))
            .count(),
        4
    );
    let start = std::time::Instant::now();
    let cached = rec.analyze_snapshot(&frame, None, Some(13), [Some(99), Some(99), Some(99)]);
    eprintln!("latest cached {:?}", start.elapsed());
    assert_eq!(
        cached.cells, fresh.cells,
        "OCR card counts cannot invent observations"
    );
    let game = imageops::crop_imm(&frame, 2, 60, 1920, 1080).to_image();
    for (w, h) in [(1440, 810), (960, 540)] {
        let scaled = imageops::resize(&game, w, h, imageops::FilterType::Triangle);
        assert_eq!(
            rec.analyze(&scaled, None, Some(13)).cells,
            expected_latest(),
            "derived {w}x{h}"
        );
    }
}
#[test]
fn finish_keeps_the_round_reference_and_reset_discards_it() {
    let mut rec = Recognizer::new();
    let before = rec.analyze(&fixture("other-items"), None, Some(25));
    let next = rec.analyze(&fixture("phones-completed"), None, Some(13));
    assert_eq!(next.card_fingerprints[0], before.card_fingerprints[0]);
    assert!(next.reference_ready[0]);
    assert_eq!(
        next.completed_objects
            .iter()
            .filter(|o| o.item_index == Some(0))
            .count(),
        2
    );
    rec.reset();
    let fresh = rec.analyze(&fixture("phones-completed"), None, Some(13));
    assert!(!fresh.reference_ready[0]);
}
#[test]
fn arbitrary_cover_color_keeps_the_same_structure() {
    let mut frame = fixture("initial");
    for y in 346..866 {
        for x in 910..1846 {
            let p = frame.get_pixel(x, y);
            let lum = (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0;
            frame.put_pixel(
                x,
                y,
                Rgba([
                    (lum * 0.5 + 70.0) as u8,
                    (lum * 0.5 + 5.0) as u8,
                    (lum * 0.5 + 50.0) as u8,
                    255,
                ]),
            );
        }
    }
    let a = Recognizer::new().analyze(&frame, None, Some(45));
    assert!(a.present, "{}", a.message);
    assert!(a.cells.iter().all(|s| s == "unknown"), "{:?}", a.cells);
}
#[test]
fn replacing_exposed_background_does_not_rotate_it_with_gray_objects() {
    let mut frame = fixture("phones-completed");
    let labels = expected_latest();
    for y in 346..866 {
        for x in 910..1846 {
            let idx = ((y - 346) / 104 * 9 + (x - 910) / 104) as usize;
            if labels[idx] == "unknown" {
                continue;
            }
            let p = *frame.get_pixel(x, y);
            let max = *p.0[..3].iter().max().unwrap();
            let min = *p.0[..3].iter().min().unwrap();
            if max > 195 && max - min < 35 {
                frame.put_pixel(
                    x,
                    y,
                    Rgba([p[0].saturating_sub(15), p[1].saturating_sub(5), p[2], 255]),
                );
            }
        }
    }
    let mut rec = Recognizer::new();
    rec.analyze(&fixture("other-items"), None, Some(25));
    let a = rec.analyze(&frame, None, Some(13));
    assert_eq!(a.cells, labels);
    assert_eq!(a.completed_objects.len(), 7);
}
#[test]
fn corrections_remove_conflicting_object_and_candidate_metadata() {
    let mut rec = Recognizer::new();
    let f = fixture("other-items");
    let a = rec.analyze(&f, None, Some(25));
    for c in &a.candidate_constraints {
        assert_eq!(a.cells[c.anchor], format!("item{}", c.item_index));
        for p in &c.placements {
            assert!(
                c.anchor % 9 >= p.x
                    && c.anchor % 9 < p.x + p.width
                    && c.anchor / 9 >= p.y
                    && c.anchor / 9 < p.y + p.height
            );
        }
    }
    rec.correct(1, "item2").unwrap();
    rec.correct(13, "item2").unwrap();
    let b = rec.analyze(&f, None, Some(25));
    assert_eq!(b.cells[1], "item2");
    assert!(!b.completed_objects.iter().any(|o| o.x == 1 && o.y == 0));
    assert!(!b
        .candidate_constraints
        .iter()
        .any(|c| c.anchor == 13 && c.item_index != 2));
}

#[test]
fn derived_all_finished_zero_counter_observes_pixels_then_new_round_resets() {
    let mut frame = fixture("phones-completed");
    let old = expected_latest();
    let empty = imageops::crop_imm(&frame, 1534, 450, 104, 104).to_image();
    let banner = imageops::crop_imm(&frame, 184, 982, 154, 46).to_image();
    for i in 0..45 {
        if old[i] == "unknown" {
            imageops::overlay(
                &mut frame,
                &empty,
                910 + (i % 9) as i64 * 104,
                346 + (i / 9) as i64 * 104,
            );
        }
    }
    // Synthetic terminal state: preserve each distinct shape icon, add only
    // the observed Finish stripe to all three cards. No gameplay is implied.
    for x in [391, 597] {
        imageops::overlay(&mut frame, &banner, x, 982);
    }
    let mut expected = old;
    for s in &mut expected {
        if s == "unknown" {
            *s = "empty".to_owned();
        }
    }
    let mut rec = Recognizer::new();
    let fresh = rec.analyze_snapshot(&frame, None, Some(0), [Some(0); 3]);
    assert!(fresh.present, "{}", fresh.message);
    assert_eq!(fresh.cells, expected);
    assert_eq!(fresh.finish, [true; 3]);
    assert_eq!(fresh.reference_ready, [false; 3]);
    assert_eq!(fresh.completed_objects.len(), 7);
    assert!(fresh
        .completed_objects
        .iter()
        .all(|o| o.item_index.is_none()));
    rec.reset();
    let start = rec.analyze(&fixture("initial"), None, Some(45));
    assert!(start.present);
    assert!(start.cells.iter().all(|s| s == "unknown"));
    assert_eq!(start.finish, [false; 3]);
    let cached = rec.analyze_snapshot(&frame, None, Some(0), [Some(0); 3]);
    assert!(cached.present);
    assert_eq!(cached.cells, expected);
    assert_eq!(cached.reference_ready, [true; 3]);
}
#[test]
fn zero_counter_does_not_turn_a_popup_or_covered_board_into_empty() {
    let original = fixture("initial");
    let mut rec = Recognizer::new();
    let contradicted = rec.analyze(&original, None, Some(0));
    assert!(!contradicted.present);
    assert!(contradicted.cells.iter().all(|s| s == "unknown"));
    let mut popup = original;
    for y in 346..866 {
        for x in 910..1846 {
            popup.put_pixel(x, y, Rgba([247, 248, 250, 255]));
        }
    }
    let blocked = rec.analyze(&popup, None, Some(0));
    assert!(!blocked.present);
    assert!(blocked.cells.iter().all(|s| s == "uncertain"));
    assert!(blocked.completed_objects.is_empty());
}
