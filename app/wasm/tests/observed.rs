use wasm_solver::{
    grid::Map2d,
    problem::{GameState, Item, ItemGroup},
    solve_observed_native, ObservedInput, ObservedItem, ObservedResult,
    solver::counter,
};

fn input(shapes: [(i32, i32, i32); 3], active_width: usize, active_height: usize) -> ObservedInput {
    ObservedInput {
        items: shapes.into_iter().map(|(width, height, count)| ObservedItem { width, height, count }).collect(),
        cells: (0..45).map(|i| if i / 9 < active_height && i % 9 < active_width { "unknown" } else { "empty" }.into()).collect(),
    }
}

fn exact(result: &ObservedResult, count: u64) {
    assert_eq!(result.error, "");
    assert_eq!(result.precision, Some("exact"));
    assert_eq!(result.total_patterns, count.to_string());
    assert_eq!(result.samples, count);
    assert_eq!(result.probs.len(), 8);
    assert!(result.probs.iter().all(|row| row.len() == 45));
}

#[test]
fn partial_hit_does_not_guess_anchor_or_rotation() {
    let mut board = input([(2, 1, 1), (1, 1, 0), (1, 1, 0)], 3, 3);
    board.cells[10] = "item0".into();
    let result = solve_observed_native(board);
    exact(&result, 4);
    assert_eq!(result.probs[1][10], 1.0);
    for cell in [1, 9, 11, 19] {
        assert_eq!(result.probs[1][cell], 0.25);
    }
}

#[test]
fn multiple_hits_in_one_item_consume_one_count() {
    let mut board = input([(2, 2, 1), (1, 1, 0), (1, 1, 0)], 3, 3);
    board.cells[0] = "item0".into();
    board.cells[10] = "item0".into();
    let result = solve_observed_native(board);
    exact(&result, 1);
    assert_eq!(result.probs[1].iter().sum::<f64>(), 4.0);
}

#[test]
fn rotation_can_be_forced_by_empty_cells() {
    let mut board = input([(3, 1, 1), (1, 1, 0), (1, 1, 0)], 1, 3);
    board.cells[9] = "item0".into();
    let result = solve_observed_native(board);
    exact(&result, 1);
    assert_eq!(result.probs[1][18], 1.0);
}

#[test]
fn exhausted_count_and_contradictory_types_have_no_solution() {
    let mut no_items = input([(1, 1, 0); 3], 3, 3);
    no_items.cells[0] = "item1".into();
    let mut too_many_hits = input([(1, 1, 1), (1, 1, 0), (1, 1, 0)], 3, 3);
    too_many_hits.cells[0] = "item0".into();
    too_many_hits.cells[1] = "item0".into();
    let mut wrong_type = input([(2, 1, 1), (1, 1, 1), (1, 1, 0)], 3, 1);
    wrong_type.cells[0] = "item0".into();
    wrong_type.cells[1] = "item1".into();
    for board in [no_items, too_many_hits, wrong_type] {
        let result = solve_observed_native(board);
        assert!(result.error.starts_with("no_valid_configuration"));
        assert_eq!(result.precision, None);
        assert_eq!(result.samples, 0);
        assert!(result.probs.is_empty());
    }
}

#[test]
fn zero_items_is_one_empty_configuration() {
    let result = solve_observed_native(input([(1, 1, 0); 3], 9, 5));
    exact(&result, 1);
    assert!(result.probs.iter().flatten().all(|&p| p == 0.0));
}

#[test]
fn invalid_lengths_shapes_counts_types_and_area_are_rejected() {
    let base = input([(1, 1, 0); 3], 9, 5);
    let mut variants = vec![];
    let mut board = base.clone(); board.items.pop(); variants.push(board);
    let mut board = base.clone(); board.cells.pop(); variants.push(board);
    for (width, height, count) in [(0, 1, 0), (10, 1, 0), (1, -1, 0), (1, 1, -1), (1, 1, 8), (4, 4, 3)] {
        let mut board = base.clone();
        board.items[0] = ObservedItem { width, height, count };
        variants.push(board);
    }
    let mut board = base; board.cells[0] = "item3".into(); variants.push(board);
    for board in variants {
        let result = solve_observed_native(board);
        assert!(result.error.starts_with("input_error"), "{result:?}");
        assert_eq!(result.precision, None);
    }
}

// Independent oracle: enumerate complete rectangles by group, choosing an
// increasing candidate index for indistinguishable copies, then filter clues.
fn brute_force(board: &ObservedInput) -> (u64, Vec<Vec<f64>>) {
    let candidates: Vec<Vec<u64>> = board.items.iter().map(|item| {
        let mut shapes = vec![(item.width as usize, item.height as usize)];
        if item.width != item.height { shapes.push((item.height as usize, item.width as usize)); }
        let mut masks = vec![];
        for (width, height) in shapes {
            if width > 9 || height > 5 { continue; }
            for row in 0..=5-height {
                for col in 0..=9-width {
                    let mut mask = 0;
                    for r in row..row+height { for c in col..col+width { mask |= 1u64 << (r*9+c); } }
                    if (0..45).all(|cell| mask & (1 << cell) == 0 || board.cells[cell] != "empty") { masks.push(mask); }
                }
            }
        }
        masks
    }).collect();
    fn visit(board: &ObservedInput, candidates: &[Vec<u64>], group: usize, remaining: i32, start: usize,
        masks: &mut [u64; 3], total: &mut u64, counts: &mut [[u64; 45]; 3]) {
        if group == 3 {
            if (0..45).any(|cell| match board.cells[cell].as_str() {
                "item0" => masks[0] & (1 << cell) == 0,
                "item1" => masks[1] & (1 << cell) == 0,
                "item2" => masks[2] & (1 << cell) == 0,
                _ => false,
            }) { return; }
            *total += 1;
            for i in 0..3 { for cell in 0..45 { counts[i][cell] += u64::from(masks[i] & (1 << cell) != 0); } }
        } else if remaining == 0 {
            visit(board, candidates, group+1, board.items.get(group+1).map_or(0, |item| item.count), 0, masks, total, counts);
        } else {
            let occupied = masks.iter().fold(0, |a, b| a | b);
            for index in start..candidates[group].len() {
                let mask = candidates[group][index];
                if occupied & mask != 0 { continue; }
                masks[group] |= mask;
                visit(board, candidates, group, remaining-1, index+1, masks, total, counts);
                masks[group] ^= mask;
            }
        }
    }
    let mut total = 0;
    let mut counts = [[0; 45]; 3];
    visit(board, &candidates, 0, board.items[0].count, 0, &mut [0; 3], &mut total, &mut counts);
    let probs = (0..8).map(|flag| (0..45).map(|cell| {
        (0..3).filter(|i| flag & (1 << i) != 0).map(|i| counts[i][cell]).sum::<u64>() as f64 / total as f64
    }).collect()).collect();
    (total, probs)
}

#[test]
fn small_boards_match_independent_complete_rectangle_enumeration() {
    for shapes in [ [(2, 1, 2), (1, 1, 1), (1, 1, 0)], [(2, 2, 1), (2, 1, 1), (1, 1, 1)], [(1, 1, 1); 3] ] {
        for clue in [None, Some((0, "item0")), Some((10, "item1")), Some((20, "item2"))] {
            let mut board = input(shapes, 3, 3);
            if let Some((cell, kind)) = clue { board.cells[cell] = kind.into(); }
            let (count, probs) = brute_force(&board);
            let result = solve_observed_native(board);
            if count == 0 { assert!(result.error.starts_with("no_valid_configuration")); }
            else { exact(&result, count); assert_eq!(result.probs, probs); }
        }
    }
}

#[test]
fn no_hit_observations_match_legacy_probabilities() {
    let mut board = input([(2, 1, 1), (1, 1, 1), (1, 1, 0)], 3, 3);
    board.cells[10] = "empty".into();
    let state = GameState::new(
        Map2d::new(board.cells.iter().map(|cell| cell == "empty").collect(), 9, 5),
        std::array::from_fn(|i| ItemGroup::new(Item::new(board.items[i].height as usize, board.items[i].width as usize, i), board.items[i].count as usize)), vec![]);
    let legacy = counter::calc_probabilities_all(&state, 100_000).unwrap();
    let result = solve_observed_native(board);
    assert_eq!(result.error, "");
    for (old, new) in legacy.iter().zip(&result.probs) { assert_eq!(old.iter().copied().collect::<Vec<_>>(), *new); }
}

#[test]
fn large_configuration_space_reports_actual_sampling_metadata() {
    let board = input([(1, 1, 3), (1, 1, 2), (1, 1, 2)], 9, 5);
    let result = solve_observed_native(board);
    assert_eq!(result.error, "");
    assert_eq!(result.precision, Some("sampled"));
    // Choose 3 cells, then 2, then 2 for the three indistinguishable groups.
    assert_eq!(result.total_patterns, (14190u64 * 861 * 780).to_string());
    assert_eq!(result.samples, 100_000);
    assert!(result.probs.iter().flatten().all(|p| p.is_finite() && (0.0..=1.0).contains(p)));
    assert!((result.probs[7].iter().sum::<f64>() - 7.0).abs() < 1e-12);
}

#[test]
fn current_event_nine_item_inventory_is_supported() {
    let board = input([(3, 2, 2), (3, 1, 5), (2, 1, 2)], 9, 5);
    let started = std::time::Instant::now();
    let result = solve_observed_native(board);
    eprintln!("nine-item unknown board: {:?}, {} configurations", started.elapsed(), result.total_patterns);
    assert_eq!(result.error, "");
    assert_eq!(result.precision, Some("sampled"));
    assert!(result.total_patterns.parse::<u64>().unwrap() > 100_000);
    assert_eq!(result.samples, 100_000);
    for (flag, area) in [(1, 12.0), (2, 15.0), (4, 4.0), (7, 31.0)] {
        assert!((result.probs[flag].iter().sum::<f64>() - area).abs() < 1e-12);
    }
}

#[test]
fn current_event_inventory_fits_a_complete_and_partial_synthetic_layout() {
    // Inventory comes from the live event; positions are an explicit synthetic
    // fixture, not a claim about an actual game's hidden item placements.
    let mut board = input([(3, 2, 2), (3, 1, 5), (2, 1, 2)], 0, 0);
    let placements = [
        (0, 0, 0, 3, 2), (0, 0, 3, 3, 2),
        (1, 2, 0, 3, 1), (1, 2, 3, 3, 1), (1, 2, 6, 3, 1),
        (1, 3, 0, 3, 1), (1, 3, 3, 3, 1),
        (2, 4, 0, 2, 1), (2, 4, 2, 2, 1),
    ];
    for (group, row, col, width, height) in placements {
        for r in row..row + height {
            for c in col..col + width { board.cells[r * 9 + c] = format!("item{group}"); }
        }
    }
    let complete = solve_observed_native(board.clone());
    exact(&complete, 1);
    for (cell, kind) in board.cells.iter().enumerate() {
        assert_eq!(complete.probs[7][cell], if kind == "empty" { 0.0 } else { 1.0 });
    }
    board.cells[0] = "unknown".into();
    board.cells[10] = "unknown".into();
    let started = std::time::Instant::now();
    let partial = solve_observed_native(board);
    eprintln!("nine-item partial synthetic board: {:?}", started.elapsed());
    exact(&partial, 1);
    assert_eq!(partial.probs, complete.probs);
}

#[test]
fn board_sized_rectangles_and_rotations_have_known_counts() {
    for (width, height, count) in [(5, 1, 34), (1, 5, 34), (9, 1, 5), (5, 5, 5), (9, 5, 1), (5, 9, 1)] {
        let result = solve_observed_native(input([(width, height, 1), (1, 1, 0), (1, 1, 0)], 9, 5));
        exact(&result, count);
        assert!((result.probs[1].iter().sum::<f64>() - (width * height) as f64).abs() < 1e-12);
    }
    for (width, height) in [(6, 6), (10, 1), (1, 10)] {
        assert!(solve_observed_native(input([(width, height, 0), (1, 1, 0), (1, 1, 0)], 9, 5)).error.starts_with("input_error"));
    }
}

#[test]
fn wide_rectangles_match_independent_enumeration_with_hits_and_empty_cells() {
    for shapes in [ [(5, 1, 2), (1, 1, 1), (1, 1, 0)], [(1, 5, 1), (2, 1, 1), (1, 1, 1)], [(9, 1, 1), (1, 1, 1), (1, 1, 0)] ] {
        for clue in [None, Some((0, "item0")), Some((10, "item1")), Some((20, "empty"))] {
            let mut board = input(shapes, 9, 3);
            if let Some((cell, kind)) = clue { board.cells[cell] = kind.into(); }
            let (count, probs) = brute_force(&board);
            let result = solve_observed_native(board);
            if count == 0 { assert!(result.error.starts_with("no_valid_configuration")); }
            else { exact(&result, count); assert_eq!(result.probs, probs); }
        }
    }
}

#[test]
fn wide_sparse_frontier_preserves_sampling_limit_and_total_count() {
    let result = solve_observed_native(input([(9, 1, 1), (1, 1, 3), (1, 1, 0)], 9, 5));
    assert_eq!(result.error, "");
    // Five full-row placements, then choose three cells from the other 36.
    assert_eq!(result.total_patterns, (5u64 * 36 * 35 * 34 / 6).to_string());
    assert_eq!(result.precision, Some("exact"));
    let result = solve_observed_native(input([(9, 1, 1), (1, 1, 4), (1, 1, 0)], 9, 5));
    assert_eq!(result.error, "");
    assert_eq!(result.total_patterns, (5u64 * 36 * 35 * 34 * 33 / 24).to_string());
    assert_eq!(result.precision, Some("sampled"));
    assert_eq!(result.samples, 100_000);
    assert!(result.probs.iter().flatten().all(|p| p.is_finite() && (0.0..=1.0).contains(p)));
    assert!((result.probs[7].iter().sum::<f64>() - 13.0).abs() < 1e-12);
}

#[test]
fn round_two_covered_inventory_keeps_dense_path_and_sampling() {
    let board = input([(4, 2, 1), (4, 1, 2), (3, 1, 5)], 9, 5);
    let started = std::time::Instant::now();
    let result = solve_observed_native(board);
    eprintln!("round-two covered board: {:?}, {} configurations", started.elapsed(), result.total_patterns);
    assert_eq!(result.error, "");
    assert_eq!(result.precision, Some("sampled"));
    assert_eq!(result.samples, 100_000);
    for (flag, area) in [(1, 8.0), (2, 8.0), (4, 15.0), (7, 31.0)] {
        assert!((result.probs[flag].iter().sum::<f64>() - area).abs() < 1e-12);
    }
}
