#[path = "../src/vision.rs"]
mod vision;

use image::{imageops, Rgba, RgbaImage};
use vision::Recognizer;

fn initial() -> RgbaImage {
    image::load_from_memory(include_bytes!("fixtures/vision-initial.png"))
        .unwrap()
        .to_rgba8()
}

fn opened() -> RgbaImage {
    image::load_from_memory(include_bytes!("fixtures/vision-opened.png"))
        .unwrap()
        .to_rgba8()
}

fn partial_watergun() -> RgbaImage {
    image::load_from_memory(include_bytes!("fixtures/vision-partial-watergun.png"))
        .unwrap()
        .to_rgba8()
}

fn full_watergun() -> RgbaImage {
    image::load_from_memory(include_bytes!("fixtures/vision-full-watergun.png"))
        .unwrap()
        .to_rgba8()
}

fn other_items() -> RgbaImage {
    image::load_from_memory(include_bytes!("fixtures/vision-other-items.png"))
        .unwrap()
        .to_rgba8()
}

fn realtime_flip() -> RgbaImage {
    image::load_from_memory(include_bytes!("fixtures/vision-after-realtime-flip.png"))
        .unwrap()
        .to_rgba8()
}

fn realtime_unselected() -> RgbaImage {
    image::load_from_memory(include_bytes!("fixtures/vision-realtime-unselected.png"))
        .unwrap()
        .to_rgba8()
}

fn blue_selected() -> RgbaImage {
    image::load_from_memory(include_bytes!("fixtures/vision-blue-selected.png"))
        .unwrap()
        .to_rgba8()
}

fn waterguns_finished() -> RgbaImage {
    image::load_from_memory(include_bytes!("fixtures/vision-waterguns-finished.png"))
        .unwrap()
        .to_rgba8()
}

#[test]
fn real_initial_has_45_covered_cells_and_observed_shapes() {
    let mut recognizer = Recognizer::new();
    let result = recognizer.analyze(&initial(), None, Some(45));
    assert!(result.present, "{}", result.message);
    assert_eq!(result.cells.len(), 45);
    assert!(result.cells.iter().all(|cell| cell == "unknown"));
    assert_eq!(result.shapes, vec![[3, 2], [3, 1], [2, 1]]);
    let [x, y, w, h] = result.board.unwrap();
    assert!((x * 1924.0 - 911.0).abs() < 5.0);
    assert!((y * 1142.0 - 346.0).abs() < 5.0);
    assert!((w * 1924.0 - 934.0).abs() < 5.0);
    assert!((h * 1142.0 - 519.0).abs() < 5.0);
    eprintln!(
        "initial: board={:?}, covered=45/45, shapes={:?}",
        result.board, result.shapes
    );
}

#[test]
fn real_opened_has_one_texture_confirmed_empty_without_count_inference() {
    let frame = opened();
    let mut recognizer = Recognizer::new();
    let result = recognizer.analyze(&frame, None, Some(44));
    assert!(result.present, "{}", result.message);
    assert_eq!(result.cells[15], "empty");
    assert_eq!(
        result.cells.iter().filter(|cell| *cell == "empty").count(),
        1
    );
    assert_eq!(
        result
            .cells
            .iter()
            .filter(|cell| *cell == "unknown")
            .count(),
        44
    );
    let without_count = recognizer.analyze(&frame, None, None);
    assert!(without_count.present, "{}", without_count.message);
    assert_eq!(without_count.cells, result.cells);
    assert!(without_count.message.contains("未识别"));
    eprintln!("opened: covered=44/45, empty index=15; absent OCR preserves observations");
}

#[test]
fn wrong_counter_never_creates_a_revealed_cell() {
    let mut recognizer = Recognizer::new();
    let result = recognizer.analyze(&initial(), None, Some(44));
    assert!(!result.present);
    assert!(result.cells.iter().all(|cell| cell == "unknown"));
    assert!(result.message.contains("不一致"));
    let opened_result = recognizer.analyze(&opened(), None, Some(43));
    assert!(!opened_result.present);
    assert_eq!(opened_result.cells[15], "empty");
    assert_eq!(
        opened_result
            .cells
            .iter()
            .filter(|cell| *cell == "unknown")
            .count(),
        44
    );
}

#[test]
fn manual_range_cannot_turn_nonboard_or_occlusion_into_a_board() {
    let mut recognizer = Recognizer::new();
    let located = recognizer
        .analyze(&initial(), None, Some(45))
        .board
        .unwrap();
    let blank = RgbaImage::from_pixel(1924, 1142, Rgba([240, 242, 245, 255]));
    let nonboard = recognizer.analyze(&blank, Some(located), Some(45));
    assert!(!nonboard.present);
    assert!(nonboard.board.is_none());
    let mut occluded = initial();
    for y in 210..930 {
        for x in 950..1760 {
            occluded.put_pixel(x, y, Rgba([252, 252, 252, 255]));
        }
    }
    let popup = recognizer.analyze(&occluded, Some(located), Some(45));
    assert!(!popup.present);
    assert!(popup.board.is_none());
    assert!(recognizer
        .analyze(&initial(), Some([0.05, 0.1, 0.3, 0.3]), Some(45))
        .board
        .is_none());
}

#[test]
fn window_fullscreen_and_scaled_frames_keep_the_same_cell_observations() {
    let window = opened();
    let mut recognizer = Recognizer::new();
    let window_result = recognizer.analyze(&window, None, Some(44));
    assert!(window_result.present, "{}", window_result.message);
    let game = imageops::crop_imm(&window, 2, 60, 1920, 1080).to_image();
    for (w, h) in [(1920, 1080), (1440, 810), (960, 540)] {
        let frame = imageops::resize(&game, w, h, imageops::FilterType::Triangle);
        let result = recognizer.analyze(&frame, None, Some(44));
        assert!(result.present, "{w}x{h}: {}", result.message);
        assert_eq!(result.cells, window_result.cells, "{w}x{h}");
        assert_eq!(result.shapes, window_result.shapes, "{w}x{h}");
        let [x, y, bw, bh] = result.board.unwrap();
        let [wx, wy, ww, wh] = window_result.board.unwrap();
        assert!((x * 1920.0 - (wx * 1924.0 - 2.0)).abs() < 3.0);
        assert!((y * 1080.0 - (wy * 1142.0 - 60.0)).abs() < 3.0);
        assert!((bw * 1920.0 - ww * 1924.0).abs() < 3.0);
        assert!((bh * 1080.0 - wh * 1142.0).abs() < 3.0);
        eprintln!(
            "derived {w}x{h}: board={:?}, covered=44, empty=1",
            result.board
        );
    }
}

fn white_fragment() -> RgbaImage {
    let mut frame = opened();
    // Synthetic regression guard, not a real typed-item acceptance sample:
    // make a white local object region over the actual empty ice texture.
    for y in 478..516 {
        for x in 1563..1601 {
            frame.put_pixel(x, y, Rgba([255, 255, 255, 255]));
        }
    }
    frame
}

#[test]
fn white_object_fragments_are_uncertain_and_corrections_bind_to_pixels() {
    let frame = white_fragment();
    let mut recognizer = Recognizer::new();
    assert!(recognizer.correct(15, "item0").is_err());
    let observed = recognizer.analyze(&frame, None, Some(44));
    assert!(observed.present, "{}", observed.message);
    assert_eq!(observed.cells[15], "uncertain");
    assert_eq!(
        observed
            .cells
            .iter()
            .filter(|cell| *cell == "empty")
            .count(),
        0
    );
    assert!(recognizer.correct(0, "empty").is_err());
    assert!(recognizer.correct(45, "empty").is_err());
    assert!(recognizer.correct(15, "invalid").is_err());
    recognizer.correct(15, "item0").unwrap();
    assert_eq!(
        recognizer.analyze(&frame, None, Some(44)).cells[15],
        "item0"
    );
    let changed = recognizer.analyze(&opened(), None, Some(44));
    assert_eq!(changed.cells[15], "empty");
    recognizer.analyze(&frame, None, Some(44));
    recognizer.correct(15, "item0").unwrap();
    recognizer.reset();
    assert_eq!(
        recognizer.analyze(&frame, None, Some(44)).cells[15],
        "uncertain"
    );
    recognizer.correct(15, "item1").unwrap();
    assert!(recognizer.analyze(&initial(), None, Some(45)).present);
    assert_eq!(
        recognizer.analyze(&frame, None, Some(44)).cells[15],
        "uncertain"
    );
}

#[test]
fn real_watergun_partial_and_three_distinct_empty_textures_are_observed() {
    let frame = partial_watergun();
    let mut recognizer = Recognizer::new();
    let result = recognizer.analyze(&frame, None, Some(41));
    assert!(result.present, "{}", result.message);
    assert_eq!(
        result
            .cells
            .iter()
            .filter(|cell| *cell == "unknown")
            .count(),
        41
    );
    for index in [11, 15, 29] {
        assert_eq!(result.cells[index], "empty", "index {index}");
    }
    assert_eq!(result.cells[33], "item0");
    assert_eq!(
        result.cells.iter().filter(|cell| *cell == "item0").count(),
        1
    );
    assert_eq!(
        result.cells.iter().filter(|cell| *cell == "empty").count(),
        3
    );
    assert!(result
        .cells
        .iter()
        .all(|cell| !matches!(cell.as_str(), "uncertain" | "item1" | "item2")));
    eprintln!("real partial: covered=41, empty=[11,15,29], observed item0=[33]");

    let fullscreen = imageops::crop_imm(&frame, 2, 60, 1920, 1080).to_image();
    for (w, h) in [(1920, 1080), (1440, 810), (960, 540)] {
        let scaled = imageops::resize(&fullscreen, w, h, imageops::FilterType::Triangle);
        let observed = recognizer.analyze(&scaled, None, Some(41));
        assert!(observed.present, "derived {w}x{h}: {}", observed.message);
        assert_eq!(observed.cells, result.cells, "derived {w}x{h}");
    }
}

#[test]
fn typed_watergun_identity_requires_its_visible_card_and_never_guesses_neighbors() {
    let frame = partial_watergun();
    let mut recognizer = Recognizer::new();
    let result = recognizer.analyze(&frame, None, Some(41));
    assert_eq!(result.cells[33], "item0");
    for neighbor in [24, 32, 34, 42] {
        assert_eq!(result.cells[neighbor], "unknown", "neighbor {neighbor}");
    }
    let mut different_card = frame.clone();
    for y in 948..1060 {
        for x in 194..334 {
            different_card.put_pixel(x, y, Rgba([70, 70, 70, 255]));
        }
    }
    let changed = recognizer.analyze(&different_card, None, Some(41));
    assert!(changed.present, "{}", changed.message);
    assert_eq!(changed.cells[33], "uncertain");
    assert_eq!(
        changed.cells.iter().filter(|cell| *cell == "empty").count(),
        3
    );
    assert!(!changed.cells.iter().any(|cell| cell == "item0"));
    // A different type copied from a reference card is a synthetic negative
    // case, not acceptance evidence for real item1/item2 board fragments.
    for (x, y, w, h) in [(400, 946, 136, 114), (610, 945, 126, 117)] {
        let sprite = imageops::crop_imm(&frame, x, y, w, h).to_image();
        let sprite = imageops::resize(&sprite, 104, 104, imageops::FilterType::Triangle);
        let mut synthetic = frame.clone();
        imageops::overlay(&mut synthetic, &sprite, 1534, 658);
        let observed = recognizer.analyze(&synthetic, None, Some(41));
        assert_ne!(observed.cells[33], "empty");
        assert_ne!(observed.cells[33], "item0");
        assert!(!matches!(observed.cells[33].as_str(), "item1" | "item2"));
    }
}

#[test]
fn real_completed_gray_watergun_is_six_observed_cells_without_neighbor_expansion() {
    let frame = full_watergun();
    let mut recognizer = Recognizer::new();
    let result = recognizer.analyze(&frame, None, Some(36));
    assert!(result.present, "{}", result.message);
    assert_eq!(
        result
            .cells
            .iter()
            .filter(|cell| *cell == "unknown")
            .count(),
        36
    );
    assert_eq!(
        result.cells.iter().filter(|cell| *cell == "empty").count(),
        3
    );
    assert_eq!(
        result
            .cells
            .iter()
            .filter(|cell| *cell == "completed")
            .count(),
        6
    );
    for index in [24, 25, 33, 34, 42, 43] {
        assert_eq!(
            result.cells[index], "completed",
            "observed completed index {index}"
        );
    }
    for index in [11, 15, 29] {
        assert_eq!(result.cells[index], "empty");
    }
    for neighbor in [23, 26, 32, 35, 41, 44] {
        assert_eq!(
            result.cells[neighbor], "unknown",
            "covered neighbor {neighbor}"
        );
    }
    eprintln!("real complete: covered=36, empty=[11,15,29], gray item0=[24,25,33,34,42,43]");
    let fullscreen = imageops::crop_imm(&frame, 2, 60, 1920, 1080).to_image();
    for (w, h) in [(1920, 1080), (1440, 810), (960, 540)] {
        let scaled = imageops::resize(&fullscreen, w, h, imageops::FilterType::Triangle);
        let observed = recognizer.analyze(&scaled, None, Some(36));
        assert!(
            observed.present,
            "derived complete {w}x{h}: {}",
            observed.message
        );
        assert_eq!(observed.cells, result.cells, "derived complete {w}x{h}");
    }
}

fn expected_other_items() -> Vec<String> {
    let mut expected = vec!["unknown".to_owned(); 45];
    for (cell, indices) in [
        ("item0", vec![13]),
        ("completed", vec![1, 10, 19, 6, 7, 24, 25, 33, 34, 42, 43]),
        ("item1", vec![26, 39]),
        ("empty", vec![11, 15, 17, 29, 31, 37]),
    ] {
        for index in indices {
            expected[index] = cell.to_owned();
        }
    }
    expected
}

#[test]
fn real_three_item_sample_matches_every_cell_without_filling_covered_tiles() {
    let frame = other_items();
    let expected = expected_other_items();
    let mut recognizer = Recognizer::new();
    let result = recognizer.analyze(&frame, None, Some(25));
    assert!(result.present, "{}", result.message);
    assert_eq!(result.cells, expected, "real 45-cell ground truth");
    assert_eq!(
        result
            .cells
            .iter()
            .filter(|cell| *cell == "unknown")
            .count(),
        25
    );
    assert_eq!(result.shapes, vec![[3, 2], [3, 1], [2, 1]]);
    eprintln!(
        "real three types: 45/45 labels matched, unknown=25, empty=6, item0=7, item1=5, item2=2"
    );
    let fullscreen = imageops::crop_imm(&frame, 2, 60, 1920, 1080).to_image();
    for (w, h) in [(1920, 1080), (1440, 810), (960, 540)] {
        let scaled = imageops::resize(&fullscreen, w, h, imageops::FilterType::Triangle);
        let observed = recognizer.analyze(&scaled, None, Some(25));
        assert!(
            observed.present,
            "derived three types {w}x{h}: {}",
            observed.message
        );
        assert_eq!(observed.cells, expected, "derived three types {w}x{h}");
    }
}

#[test]
fn a_rotated_single_gray_tile_is_not_an_entire_completed_object() {
    let original = other_items();
    let tile = imageops::crop_imm(&original, 1534, 346, 104, 104).to_image();
    let mut derived = original.clone();
    imageops::overlay(&mut derived, &imageops::rotate90(&tile), 1534, 346);
    let result = Recognizer::new().analyze(&derived, None, Some(25));
    assert_ne!(result.cells[6], "empty");
    assert_eq!(result.cells[5], "unknown");
    assert_eq!(result.cells[14], "unknown");
}

#[test]
fn real_selected_cover_stays_covered_while_only_the_new_phone_cell_changes() {
    let before = other_items();
    let after = realtime_flip();
    let mut expected = expected_other_items();
    expected[21] = "item1".to_owned();
    let mut recognizer = Recognizer::new();
    let previous = recognizer.analyze(&before, None, Some(25));
    assert_eq!(previous.cells, expected_other_items());
    let observed = recognizer.analyze(&after, None, Some(24));
    assert!(observed.present, "{}", observed.message);
    assert_eq!(
        observed.cells, expected,
        "real 45-cell selected-cover regression"
    );
    let changed: Vec<usize> = previous
        .cells
        .iter()
        .zip(&observed.cells)
        .enumerate()
        .filter_map(|(index, (old, new))| (old != new).then_some(index))
        .collect();
    assert_eq!(changed, vec![21]);
    assert_eq!(observed.cells[23], "unknown");
    assert!(!observed.cells.iter().any(|cell| cell == "uncertain"));
    assert_eq!(
        observed
            .cells
            .iter()
            .filter(|cell| *cell == "unknown")
            .count(),
        24
    );
}

#[test]
fn real_selected_and_cancelled_cover_frames_have_equal_45_cell_observations() {
    let selected = realtime_flip();
    let unselected = realtime_unselected();
    let mut expected = expected_other_items();
    expected[21] = "item1".to_owned();
    let mut recognizer = Recognizer::new();
    let marked = recognizer.analyze(&selected, None, Some(24));
    let cancelled = recognizer.analyze(&unselected, None, Some(24));
    assert!(marked.present, "{}", marked.message);
    assert!(cancelled.present, "{}", cancelled.message);
    assert_eq!(marked.cells, expected);
    assert_eq!(cancelled.cells, expected);
    assert_eq!(
        marked.cells, cancelled.cells,
        "real selection/cancellation must not alter any observation"
    );
    assert_eq!(marked.cells[23], "unknown");
    assert_eq!(cancelled.cells[23], "unknown");
    eprintln!(
        "real selected/unselected: 45/45 identical labels; index21=item1 and index23=unknown"
    );
    let game = imageops::crop_imm(&selected, 2, 60, 1920, 1080).to_image();
    for (w, h) in [(1920, 1080), (960, 540)] {
        let resized = imageops::resize(&game, w, h, imageops::FilterType::Triangle);
        let observed = recognizer.analyze(&resized, None, Some(24));
        assert_eq!(observed.cells, expected, "derived selected {w}x{h}");
    }
}

#[test]
fn green_color_or_incomplete_marker_is_not_a_selected_cover() {
    let frame = realtime_flip();
    let selected_tile = imageops::crop_imm(&frame, 1430, 554, 104, 104).to_image();
    let old = other_items();
    let old_tile = imageops::crop_imm(&old, 1430, 554, 104, 104).to_image();
    let mut recognizer = Recognizer::new();
    let flat_green = RgbaImage::from_pixel(104, 104, Rgba([160, 240, 70, 255]));
    let mut border_missing = selected_tile.clone();
    let mut tick_missing = selected_tile.clone();
    let mut contour_missing = selected_tile.clone();
    for y in 0..104 {
        for x in 0..104 {
            if x < 8 || x >= 96 || y < 8 || y >= 96 {
                border_missing.put_pixel(x, y, *old_tile.get_pixel(x, y));
            }
            if (26..80).contains(&x) && (24..82).contains(&y) {
                tick_missing.put_pixel(x, y, Rgba([147, 84, 114, 255]));
            }
            let [r, g, b, _] = selected_tile.get_pixel(x, y).0;
            let lime = g > 175
                && r > 85
                && b < 155
                && g > r.saturating_add(20)
                && g > b.saturating_add(55);
            if !lime && (8..96).contains(&x) && (8..96).contains(&y) {
                contour_missing.put_pixel(x, y, Rgba([147, 84, 114, 255]));
            }
        }
    }
    // Synthetic negative cases validate all three independent requirements;
    // none is a claim about a captured physical selection transition.
    for (name, tile) in [
        ("arbitrary lime item", flat_green),
        ("no border", border_missing),
        ("no check", tick_missing),
        ("no cover contour", contour_missing),
    ] {
        let mut synthetic = frame.clone();
        imageops::overlay(&mut synthetic, &tile, 1430, 554);
        let result = recognizer.analyze(&synthetic, None, Some(24));
        assert_eq!(result.cells[23], "uncertain", "{name}");
    }
}

#[test]
fn real_blue_selection_matches_cancelled_frame_with_fresh_and_cached_location() {
    let mut expected = expected_other_items();
    expected[21] = "item1".to_owned();
    let blue = blue_selected();
    let cancelled = realtime_unselected();
    let mut recognizer = Recognizer::new();
    let fresh = recognizer.analyze(&blue, None, Some(24));
    let uncertain: Vec<_> = fresh
        .cells
        .iter()
        .enumerate()
        .filter_map(|(index, cell)| (cell == "uncertain").then_some(index))
        .collect();
    eprintln!(
        "blue fresh: board={:?}, uncertain={uncertain:?}",
        fresh.board
    );
    assert!(fresh.present, "{}", fresh.message);
    assert_eq!(fresh.cells, expected, "real blue fresh 45-cell regression");
    recognizer.reset();
    let prior = recognizer.analyze(&cancelled, None, Some(24));
    let cached = recognizer.analyze(&blue, None, Some(24));
    assert_eq!(prior.cells, expected);
    assert_eq!(cached.cells, expected);
    assert_eq!(cached.cells, prior.cells);
}

#[test]
fn real_finished_card_keeps_both_waterguns_pixel_confirmed_without_count_inference() {
    let frame = waterguns_finished();
    let mut expected = expected_other_items();
    expected[21] = "item1".to_owned();
    for index in [4, 5, 13, 14, 22, 23] {
        expected[index] = "completed".to_owned();
    }
    let mut recognizer = Recognizer::new();
    let fresh = recognizer.analyze(&frame, None, Some(19));
    assert!(fresh.present, "{}", fresh.message);
    assert_eq!(
        fresh.cells, expected,
        "real finished-card 45-cell ground truth"
    );
    assert_eq!(
        fresh.cells.iter().filter(|cell| *cell == "unknown").count(),
        19
    );
    assert_eq!(
        fresh
            .cells
            .iter()
            .filter(|cell| *cell == "completed")
            .count(),
        17
    );
    assert!(!fresh.reference_ready[0]);
    assert!(fresh.finish[0]);
    recognizer.reset();
    let previous = recognizer.analyze(&blue_selected(), None, Some(24));
    assert_eq!(previous.cells[22], "unknown");
    let cached = recognizer.analyze(&frame, None, Some(19));
    assert_eq!(
        cached.cells, expected,
        "real active-to-finished card transition"
    );
    let no_counter = recognizer.analyze(&frame, None, None);
    assert_eq!(
        no_counter.cells, expected,
        "card quantity never supplies a board count"
    );
    eprintln!(
        "real finished card: 45/45 labels matched; unknown=19, empty=6, item0=12, item1=6, item2=2"
    );
}

#[test]
fn finish_without_cache_confirms_gray_occupancy_without_inventing_item_identity() {
    let mut frame = waterguns_finished();
    // Preserve the actual Finish banner and x0 quantity, but remove the
    // type-specific sprite above/below the banner. This is a synthetic guard.
    for y in 948..1060 {
        for x in 194..334 {
            if !(982..1028).contains(&y) {
                frame.put_pixel(x, y, Rgba([37, 68, 90, 255]));
            }
        }
    }
    let mut recognizer = Recognizer::new();
    let result = recognizer.analyze(&frame, None, Some(19));
    assert!(result.present, "{}", result.message);
    assert!(!result.reference_ready[0]);
    assert!(!result.cells.iter().any(|cell| cell == "item0"));
    assert!(result
        .completed_objects
        .iter()
        .filter(|o| o.width * o.height == 6)
        .all(|o| o.item_index.is_none()));
    for index in [4, 5, 13, 14, 22, 23, 24, 25, 33, 34, 42, 43] {
        assert_eq!(
            result.cells[index], "completed",
            "untyped occupied index {index}"
        );
    }
    assert_eq!(
        result
            .cells
            .iter()
            .filter(|cell| *cell == "unknown")
            .count(),
        19
    );
    assert_eq!(
        result.cells.iter().filter(|cell| *cell == "empty").count(),
        6
    );
}
