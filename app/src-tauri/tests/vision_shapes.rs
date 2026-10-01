#[path = "../src/vision.rs"]
mod vision;

use image::{imageops, Rgba, RgbaImage};
use vision::{Analysis, Recognizer};

const ICON_X: [u32; 3] = [168, 375, 581];
const ICON_Y: u32 = 1068;
const LIGHT: [u8; 4] = [180, 217, 242, 255];
const SQUARE: [u8; 4] = [40, 75, 140, 255];

fn previous_round() -> RgbaImage {
    image::load_from_memory(include_bytes!("fixtures/vision-initial.png"))
        .unwrap()
        .to_rgba8()
}

fn second_round() -> RgbaImage {
    image::load_from_memory(include_bytes!("fixtures/vision-round-two-initial.png"))
        .unwrap()
        .to_rgba8()
}

fn analyze(frame: &RgbaImage) -> Analysis {
    Recognizer::new().analyze(frame, None, Some(45))
}

fn assert_covered_board(result: &Analysis, frame: &RgbaImage) {
    assert!(result.present, "{}", result.message);
    assert_eq!(result.cells, vec!["unknown"; 45]);
    assert_eq!(result.reference_ready, [true; 3]);
    let [x, y, w, h] = result.board.unwrap();
    let chrome = frame.height() as f64 > frame.width() as f64 * 9.0 / 16.0 + 8.0;
    let viewport_w = frame.width() as f64 - if chrome { 4.0 } else { 0.0 };
    let viewport_h = viewport_w * 9.0 / 16.0;
    let viewport_x = (frame.width() as f64 - viewport_w) / 2.0;
    let viewport_y = frame.height() as f64 - viewport_h - if chrome { 2.0 } else { 0.0 };
    assert!((x * frame.width() as f64 - viewport_x - viewport_w * 908.0 / 1920.0).abs() < 2.0);
    assert!((y * frame.height() as f64 - viewport_y - viewport_h * 286.0 / 1080.0).abs() < 2.0);
    assert!((w * frame.width() as f64 - viewport_w * 936.0 / 1920.0).abs() < 2.0);
    assert!((h * frame.height() as f64 - viewport_h * 520.0 / 1080.0).abs() < 2.0);
}

fn paint(frame: &mut RgbaImage, card: usize, rect: [u32; 4], color: [u8; 4]) {
    let [x, y, w, h] = rect;
    for yy in y..y + h {
        for xx in x..x + w {
            assert!(xx < 42 && yy < 32);
            frame.put_pixel(ICON_X[card] + xx, ICON_Y + yy, Rgba(color));
        }
    }
}

fn clear_icon(frame: &mut RgbaImage, card: usize) {
    paint(frame, card, [0, 0, 42, 32], LIGHT);
}

fn grid(
    frame: &mut RgbaImage,
    card: usize,
    [columns, rows]: [u32; 2],
    side: u32,
    [dx, dy]: [i32; 2],
) -> Vec<[u32; 4]> {
    clear_icon(frame, card);
    let gap = if side == 3 { 1 } else { 2 };
    let width = columns * side + (columns - 1) * gap;
    let height = rows * side + (rows - 1) * gap;
    let x = ((42 - width) / 2) as i32 + dx;
    let y = ((32 - height) / 2) as i32 + dy;
    let mut squares = Vec::new();
    for row in 0..rows {
        for column in 0..columns {
            let rect = [
                (x + (column * (side + gap)) as i32) as u32,
                (y + (row * (side + gap)) as i32) as u32,
                side,
                side,
            ];
            paint(frame, card, rect, SQUARE);
            squares.push(rect);
        }
    }
    squares
}

fn assert_first_unknown(frame: &RgbaImage) {
    let result = analyze(frame);
    assert_covered_board(&result, frame);
    assert_eq!(result.shapes, vec![[0, 0], [3, 1], [2, 1]]);
}

#[test]
fn real_previous_round_keeps_shapes_references_and_board() {
    let frame = previous_round();
    let result = analyze(&frame);
    assert_covered_board(&result, &frame);
    assert_eq!(result.shapes, vec![[3, 2], [3, 1], [2, 1]]);
}

#[test]
fn real_second_round_reads_four_column_icons_without_changing_board() {
    let frame = second_round();
    let result = analyze(&frame);
    assert_covered_board(&result, &frame);
    assert_eq!(result.shapes, vec![[4, 2], [4, 1], [3, 1]]);
    let previous = analyze(&previous_round());
    assert_ne!(result.card_fingerprints, previous.card_fingerprints);
    assert_eq!(result.board, previous.board);
}

#[test]
fn real_second_round_retains_shapes_at_existing_viewport_sizes() {
    let viewport = imageops::crop_imm(&second_round(), 2, 60, 1920, 1080).to_image();
    for (width, height) in [(1920, 1080), (1440, 810), (960, 540)] {
        let frame = imageops::resize(&viewport, width, height, imageops::FilterType::Triangle);
        let result = analyze(&frame);
        assert_covered_board(&result, &frame);
        assert_eq!(
            result.shapes,
            vec![[4, 2], [4, 1], [3, 1]],
            "viewport {width}x{height}"
        );
    }
}

#[test]
fn all_one_to_four_row_and_column_combinations_use_only_icon_pixels() {
    let original = previous_round();
    let original_result = analyze(&original);
    for columns in 1..=4 {
        for rows in 1..=4 {
            let mut frame = original.clone();
            for card in 0..3 {
                // 4x4 occupies y=3..28: both rows formerly clipped by the
                // narrower search area must remain complete and observed.
                grid(&mut frame, card, [columns, rows], 5, [0, 0]);
            }
            let result = analyze(&frame);
            assert_covered_board(&result, &frame);
            assert_eq!(
                result.shapes,
                vec![[columns, rows]; 3],
                "synthetic {columns}x{rows} icon"
            );
            assert_eq!(result.card_fingerprints, original_result.card_fingerprints);
        }
    }
}

#[test]
fn square_pixels_beyond_each_search_edge_are_not_trimmed_into_complete_shapes() {
    for shift in [[0, -2], [0, 2], [-5, 0], [4, 0]] {
        let mut frame = previous_round();
        grid(&mut frame, 0, [4, 4], 5, shift);
        assert_first_unknown(&frame);
    }
}

#[test]
fn missing_square_in_observed_grid_stays_unknown() {
    let mut frame = previous_round();
    let squares = grid(&mut frame, 0, [4, 4], 5, [0, 0]);
    paint(&mut frame, 0, squares[0], LIGHT);
    assert_first_unknown(&frame);
}

#[test]
fn extra_non_square_component_does_not_leave_a_guessed_legal_shape() {
    let mut frame = previous_round();
    grid(&mut frame, 0, [2, 2], 5, [0, 0]);
    paint(&mut frame, 0, [34, 10, 1, 8], SQUARE);
    assert_first_unknown(&frame);
}

#[test]
fn larger_observed_icons_are_read_when_either_orientation_fits_the_board() {
    for dimensions in [[5, 1], [1, 5], [5, 5], [8, 5], [5, 6]] {
        let mut frame = previous_round();
        grid(&mut frame, 0, dimensions, 3, [0, 0]);
        let result = analyze(&frame);
        assert_covered_board(&result, &frame);
        assert_eq!(result.shapes, vec![dimensions, [3, 1], [2, 1]]);
    }
}

#[test]
fn complete_observed_icon_that_cannot_fit_the_board_stays_unknown() {
    let mut frame = previous_round();
    // All 36 squares fit completely inside the badge's search area, but
    // neither 6x6 orientation fits the 9x5 board.
    grid(&mut frame, 0, [6, 6], 3, [0, 0]);
    assert_first_unknown(&frame);
}

#[test]
fn repeated_row_column_intersections_are_not_a_complete_rectangle() {
    let mut frame = previous_round();
    clear_icon(&mut frame, 0);
    // Four separate corner-touching squares have two x/y clusters but
    // occupy only two of their four intersections.
    for rect in [
        [10, 8, 3, 3],
        [13, 11, 3, 3],
        [24, 19, 3, 3],
        [27, 22, 3, 3],
    ] {
        paint(&mut frame, 0, rect, SQUARE);
    }
    assert_first_unknown(&frame);
}

#[test]
fn blank_and_noise_only_icons_do_not_supply_dimensions() {
    let mut frame = previous_round();
    clear_icon(&mut frame, 0);
    assert_first_unknown(&frame);
    for rect in [[8, 8, 2, 2], [18, 16, 1, 2], [30, 24, 2, 1]] {
        paint(&mut frame, 0, rect, SQUARE);
    }
    assert_first_unknown(&frame);
}
