#[path = "../src/vision.rs"]
mod vision;

use image::{imageops, Rgba, RgbaImage};
use vision::{Analysis, Recognizer};

fn prior_thirteen() -> RgbaImage {
    image::load_from_memory(include_bytes!("fixtures/vision-phones-completed.png"))
        .unwrap()
        .to_rgba8()
}

fn real_ten() -> RgbaImage {
    image::load_from_memory(include_bytes!("fixtures/vision-sparse-ten-covers.png"))
        .unwrap()
        .to_rgba8()
}

fn expected_thirteen() -> Vec<String> {
    let mut cells = vec!["unknown".to_owned(); 45];
    for index in [11, 15, 17, 29, 31, 37] {
        cells[index] = "empty".to_owned();
    }
    for index in [
        4, 5, 13, 14, 22, 23, 24, 25, 33, 34, 42, 43, 1, 10, 19, 12, 21, 30, 26, 35, 44, 38, 39,
        40, 6, 7,
    ] {
        cells[index] = "completed".to_owned();
    }
    cells
}

fn expected_ten() -> Vec<String> {
    let mut cells = expected_thirteen();
    for index in [18, 27, 36] {
        cells[index] = "completed".to_owned();
    }
    cells
}

fn assert_ten_objects(observed: &Analysis) {
    assert_eq!(observed.completed_objects.len(), 8);
    let mut rectangles: Vec<_> = observed
        .completed_objects
        .iter()
        .map(|object| (object.x, object.y, object.width, object.height))
        .collect();
    rectangles.sort_unstable();
    let mut expected = vec![
        (4, 0, 2, 3),
        (6, 2, 2, 3),
        (1, 0, 1, 3),
        (3, 1, 1, 3),
        (8, 2, 1, 3),
        (2, 4, 3, 1),
        (0, 2, 1, 3),
        (6, 0, 2, 1),
    ];
    expected.sort_unstable();
    assert_eq!(rectangles, expected);
}

#[test]
fn real_ten_covers_are_observed_fresh_with_two_finished_cards() {
    let frame = real_ten();
    assert_eq!(frame.dimensions(), (1924, 1142));
    let expected = expected_ten();
    let mut recognizer = Recognizer::new();
    let observed = recognizer.analyze_snapshot(&frame, None, Some(10), [Some(0), Some(0), Some(1)]);
    assert!(observed.present, "{}", observed.message);
    assert_eq!(observed.cells, expected, "real 45-cell ground truth");
    assert_eq!(observed.finish, [true, true, false]);
    assert_eq!(observed.reference_ready, [false, false, true]);
    assert_ten_objects(&observed);
    assert!(observed
        .completed_objects
        .iter()
        .filter(|object| object.width * object.height != 2)
        .all(|object| object.item_index.is_none()));
    assert_eq!(
        observed
            .completed_objects
            .iter()
            .filter(|object| object.item_index == Some(2))
            .count(),
        1
    );
    let no_counter = Recognizer::new().analyze_snapshot(&frame, None, None, [Some(99); 3]);
    assert!(no_counter.present, "{}", no_counter.message);
    assert_eq!(
        no_counter.cells, expected,
        "counts cannot supply missing pixels"
    );
    assert_ten_objects(&no_counter);
}

#[test]
fn real_thirteen_to_ten_transition_keeps_pixel_labels_and_phone_reference() {
    let mut recognizer = Recognizer::new();
    let before = recognizer.analyze_snapshot(
        &prior_thirteen(),
        None,
        Some(13),
        [Some(0), Some(1), Some(1)],
    );
    assert!(before.present, "{}", before.message);
    assert_eq!(before.cells, expected_thirteen());
    let after =
        recognizer.analyze_snapshot(&real_ten(), None, Some(10), [Some(0), Some(0), Some(1)]);
    assert!(after.present, "{}", after.message);
    assert_eq!(after.cells, expected_ten());
    assert_eq!(after.finish, [true, true, false]);
    assert_eq!(after.reference_ready, [false, true, true]);
    assert_eq!(after.card_fingerprints[1], before.card_fingerprints[1]);
    assert_ten_objects(&after);
    assert_eq!(
        after
            .completed_objects
            .iter()
            .filter(|object| object.item_index == Some(1))
            .count(),
        5
    );
    let changed: Vec<_> = before
        .cells
        .iter()
        .zip(&after.cells)
        .enumerate()
        .filter_map(|(index, (old, new))| (old != new).then_some(index))
        .collect();
    assert_eq!(changed, [18, 27, 36]);
}

fn derived_sparse_covers(keep: &[usize]) -> (RgbaImage, Vec<String>) {
    let mut frame = prior_thirteen();
    let mut expected = expected_thirteen();
    // Synthetic geometry regressions, not captured gameplay: copy the real
    // confirmed empty tile at index 15 over covers, retaining complete gray
    // objects and the original covers at `keep`. All retained covers lie
    // outside the former fixed geometry sample [0,4,8,18,22,26,36,40,44].
    let empty = imageops::crop_imm(&frame, 1534, 450, 104, 104).to_image();
    for index in keep {
        assert_eq!(expected[*index], "unknown");
    }
    for (index, label) in expected.iter_mut().enumerate() {
        if label == "unknown" && !keep.contains(&index) {
            imageops::overlay(
                &mut frame,
                &empty,
                910 + (index % 9) as i64 * 104,
                346 + (index / 9) as i64 * 104,
            );
            *label = "empty".to_owned();
        }
    }
    (frame, expected)
}

fn assert_derived_sparse(keep: &[usize]) {
    let (frame, expected) = derived_sparse_covers(keep);
    let observed = Recognizer::new().analyze_snapshot(
        &frame,
        None,
        Some(keep.len() as u32),
        [Some(0), Some(1), Some(1)],
    );
    assert!(observed.present, "{}", observed.message);
    assert_eq!(observed.cells, expected, "derived full 45-cell labels");
    assert_eq!(
        observed
            .cells
            .iter()
            .filter(|cell| *cell == "unknown")
            .count(),
        keep.len()
    );
    assert_eq!(observed.completed_objects.len(), 7);
}

#[test]
fn one_real_cover_outside_fixed_samples_uses_opened_pixel_geometry() {
    assert_derived_sparse(&[16]);
}

#[test]
fn two_real_covers_outside_fixed_samples_use_opened_pixel_geometry() {
    assert_derived_sparse(&[16, 20]);
}

#[test]
fn one_selected_cover_outside_fixed_samples_keeps_sparse_geometry() {
    for (source, index) in [
        ("vision-after-realtime-flip.png", 23),
        ("vision-blue-selected.png", 22),
    ] {
        let selected = image::open(format!("tests/fixtures/{source}"))
            .unwrap()
            .to_rgba8();
        let selected_tile = imageops::crop_imm(
            &selected,
            910 + (index % 9) * 104,
            346 + (index / 9) * 104,
            104,
            104,
        )
        .to_image();
        let (mut frame, expected) = derived_sparse_covers(&[16]);
        imageops::overlay(&mut frame, &selected_tile, 1638, 450);
        let observed =
            Recognizer::new().analyze_snapshot(&frame, None, Some(1), [Some(0), Some(1), Some(1)]);
        assert!(observed.present, "{source}: {}", observed.message);
        assert_eq!(observed.cells, expected, "{source}: full 45-cell labels");
        assert_eq!(observed.cells[16], "unknown");
        assert_eq!(observed.completed_objects.len(), 7);
    }
}

fn assert_rejected(observed: &Analysis) {
    assert!(!observed.present, "{}", observed.message);
    assert!(observed.board.is_none());
    assert!(observed.cells.iter().all(|cell| cell == "uncertain"));
    assert!(observed.completed_objects.is_empty());
    assert!(observed.candidate_constraints.is_empty());
}

#[test]
fn sparse_board_with_a_half_cell_manual_shift_is_rejected() {
    let frame = real_ten();
    // This range remains inside manual_board's coarse tolerance, so visual
    // evidence must reject a half-cell shift rather than trust the range.
    let shifted = [
        962.0 / 1924.0,
        398.0 / 1142.0,
        936.0 / 1924.0,
        520.0 / 1142.0,
    ];
    let result = Recognizer::new().analyze_snapshot(
        &frame,
        Some(shifted),
        Some(10),
        [Some(0), Some(0), Some(1)],
    );
    assert_rejected(&result);
}

#[test]
fn sparse_board_hidden_by_large_popup_is_rejected_even_with_valid_counts() {
    let mut frame = real_ten();
    // Synthetic popup leaves the real header/cards unchanged and masks all
    // board cells; a valid layout and supplied OCR counts are insufficient.
    for y in 346..866 {
        for x in 910..1846 {
            frame.put_pixel(x, y, Rgba([247, 248, 250, 255]));
        }
    }
    let board = [
        910.0 / 1924.0,
        346.0 / 1142.0,
        936.0 / 1924.0,
        520.0 / 1142.0,
    ];
    let mut recognizer = Recognizer::new();
    for remaining in [Some(10), Some(0)] {
        let result = recognizer.analyze_snapshot(
            &frame,
            Some(board),
            remaining,
            [Some(0), Some(0), Some(1)],
        );
        assert_rejected(&result);
    }
}
