#[path = "../src/vision.rs"]
mod vision;

use image::RgbaImage;
use vision::{CompletedObject, Recognizer};

fn fixture(name: &str) -> RgbaImage {
    image::open(format!("tests/fixtures/vision-{name}.png"))
        .unwrap()
        .to_rgba8()
}

#[test]
fn completed_only_partial_does_not_publish_typed_hits_or_candidates() {
    for (name, remaining, partial) in [
        ("partial-watergun", 41, vec![33]),
        ("other-items", 25, vec![13, 26, 39]),
        ("waterguns-finished", 19, vec![21, 26, 39]),
    ] {
        let result = Recognizer::new().analyze_completed_snapshot(
            &fixture(name),
            None,
            Some(remaining),
            [None; 3],
        );
        assert!(result.present, "{name}: {}", result.message);
        assert!(result.candidate_constraints.is_empty(), "{name}");
        assert!(
            result.cells.iter().all(|cell| !cell.starts_with("item")),
            "{name}: {:?}",
            result.cells
        );
        for index in partial {
            assert_eq!(result.cells[index], "uncertain", "{name} cell {index}");
        }
        assert_eq!(
            result
                .cells
                .iter()
                .filter(|cell| *cell == "unknown")
                .count(),
            remaining as usize,
            "{name}: covers remain pixel observations"
        );
        for index in [11, 15, 29] {
            assert_eq!(result.cells[index], "empty", "{name} empty cell {index}");
        }
    }
}

#[test]
fn completed_only_complete_frames_resume_pixel_observations() {
    let mut recognizer = Recognizer::new();
    let partial = recognizer.analyze_completed_snapshot(
        &fixture("partial-watergun"),
        None,
        Some(41),
        [Some(2), Some(5), Some(2)],
    );
    assert_eq!(partial.cells[33], "uncertain");
    let complete = recognizer.analyze_completed_snapshot(
        &fixture("full-watergun"),
        None,
        Some(36),
        [Some(1), Some(5), Some(2)],
    );
    assert!(complete.present, "{}", complete.message);
    assert!(complete.candidate_constraints.is_empty());
    assert!(complete
        .cells
        .iter()
        .all(|cell| matches!(cell.as_str(), "unknown" | "empty" | "completed")));
    for index in [24, 25, 33, 34, 42, 43] {
        assert_eq!(complete.cells[index], "completed", "cell {index}");
    }
    assert_eq!(complete.completed_objects.len(), 1);

    // A prior real round's completely revealed frame exercises the same
    // production path after the removal of the silhouette-size prefilter.
    recognizer.reset();
    let prior = recognizer.analyze_completed_snapshot(
        &fixture("phones-completed"),
        None,
        Some(13),
        [Some(0), Some(1), Some(1)],
    );
    assert!(prior.present, "{}", prior.message);
    assert!(prior.candidate_constraints.is_empty());
    assert!(prior
        .cells
        .iter()
        .all(|cell| matches!(cell.as_str(), "unknown" | "empty" | "completed")));
    assert_eq!(prior.completed_objects.len(), 7);
    assert_eq!(
        prior.cells.iter().filter(|cell| *cell == "unknown").count(),
        13
    );
    assert_eq!(
        prior.cells.iter().filter(|cell| *cell == "empty").count(),
        6
    );
    assert_eq!(
        prior
            .cells
            .iter()
            .filter(|cell| *cell == "completed")
            .count(),
        26
    );
}

#[test]
fn completed_only_round_two_complete_is_18_cells_cold_and_cached() {
    let frame = fixture("round-two-completed");
    let cold = Recognizer::new().analyze_completed_snapshot(
        &frame,
        None,
        Some(27),
        [Some(0), Some(1), Some(3)],
    );
    let mut expected = vec!["unknown".to_owned(); 45];
    for index in [
        3, 4, 12, 13, 21, 22, 30, 31, 9, 10, 11, 18, 27, 36, 15, 24, 33, 42,
    ] {
        expected[index] = "completed".to_owned();
    }
    assert!(cold.present, "{}", cold.message);
    assert_eq!(cold.cells, expected);
    assert!(cold.candidate_constraints.is_empty());
    assert_eq!(cold.shapes, vec![[4, 2], [4, 1], [3, 1]]);
    assert_eq!(cold.finish, [true, false, false]);
    assert_eq!(cold.reference_ready, [false, true, true]);
    assert_eq!(cold.completed_objects.len(), 4);
    assert!(cold.completed_objects.contains(&CompletedObject {
        item_index: None,
        x: 3,
        y: 0,
        width: 2,
        height: 4,
    }));
    for (item_index, x, y, width, height) in [(2, 0, 1, 3, 1), (2, 0, 2, 1, 3), (1, 6, 1, 1, 4)] {
        assert!(cold.completed_objects.contains(&CompletedObject {
            item_index: Some(item_index),
            x,
            y,
            width,
            height,
        }));
    }

    let mut recognizer = Recognizer::new();
    let initial = recognizer.analyze_completed_snapshot(
        &fixture("round-two-initial"),
        None,
        Some(45),
        [Some(1), Some(2), Some(5)],
    );
    assert!(initial.present);
    assert!(initial.cells.iter().all(|cell| cell == "unknown"));
    let cached =
        recognizer.analyze_completed_snapshot(&frame, None, Some(27), [Some(0), Some(1), Some(3)]);
    assert!(cached.present, "{}", cached.message);
    assert_eq!(cached.cells, expected);
    assert_eq!(cached.card_fingerprints[0], initial.card_fingerprints[0]);
    assert!(cached.completed_objects.contains(&CompletedObject {
        item_index: Some(0),
        x: 3,
        y: 0,
        width: 2,
        height: 4,
    }));
    let other_counts = recognizer.analyze_completed_snapshot(&frame, None, Some(27), [Some(99); 3]);
    assert_eq!(other_counts.cells, expected, "quantities never label cells");
}
