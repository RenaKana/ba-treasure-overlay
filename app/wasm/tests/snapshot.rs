use wasm_solver::{
    solve_observed_native, solve_snapshot_native, GridPlacement, InferredPlacement, ObservedInput,
    ObservedItem, PlacementConstraint, SnapshotInput, SnapshotItem, SnapshotResult,
};
use wasm_solver::snapshot::check_snapshot_feasibility_native;

fn input(shapes: [(i32, i32, i32); 3], active_width: usize, active_height: usize) -> SnapshotInput {
    SnapshotInput {
        items: shapes
            .into_iter()
            .map(|(width, height, remaining_count)| SnapshotItem {
                width,
                height,
                remaining_count,
            })
            .collect(),
        cells: (0..45)
            .map(|i| {
                if i / 9 < active_height && i % 9 < active_width {
                    "unknown"
                } else {
                    "empty"
                }
                .into()
            })
            .collect(),
        candidate_constraints: vec![],
    }
}

fn rect(x: usize, y: usize, width: usize, height: usize) -> GridPlacement {
    GridPlacement {
        x,
        y,
        width,
        height,
    }
}

fn constraint(
    anchor: usize,
    item_index: usize,
    placements: Vec<GridPlacement>,
) -> PlacementConstraint {
    PlacementConstraint {
        anchor,
        item_index,
        placements,
    }
}

fn exact(result: &SnapshotResult, count: u64) {
    assert_eq!(result.error, "");
    assert_eq!(result.precision, Some("exact"));
    assert_eq!(result.total_patterns, count.to_string());
    assert_eq!(result.samples, count);
    assert_eq!(result.probs.len(), 8);
    assert!(result.probs.iter().all(|row| row.len() == 45));
}

fn failure(result: &SnapshotResult, prefix: &str) {
    assert!(result.error.starts_with(prefix), "{result:?}");
    assert_eq!(result.precision, None);
    assert_eq!(result.total_patterns, "0");
    assert_eq!(result.samples, 0);
    assert!(result.probs.is_empty());
    assert!(result.inferred_placements.is_empty());
}

#[test]
fn partial_hit_keeps_the_remaining_item_and_does_not_guess_its_placement() {
    let mut board = input([(2, 1, 1), (1, 1, 0), (1, 1, 0)], 3, 3);
    board.cells[10] = "item0".into();
    let result = solve_snapshot_native(board);
    exact(&result, 4);
    assert_eq!(result.probs[1][10], 1.0);
    assert_eq!(result.probs[1].iter().sum::<f64>(), 2.0);
    for cell in [1, 9, 11, 19] {
        assert_eq!(result.probs[1][cell], 0.25);
    }
    assert!(result.inferred_placements.is_empty());
}

#[test]
fn one_rectangle_covers_multiple_hits_and_deduplicates_inferred_placements() {
    let mut board = input([(2, 2, 1), (1, 1, 0), (1, 1, 0)], 3, 3);
    board.cells[0] = "item0".into();
    board.cells[10] = "item0".into();
    let placement = rect(0, 0, 2, 2);
    board.candidate_constraints = vec![
        constraint(0, 0, vec![placement, placement]),
        constraint(10, 0, vec![placement]),
    ];
    let result = solve_snapshot_native(board);
    exact(&result, 1);
    assert_eq!(result.probs[1].iter().sum::<f64>(), 4.0);
    assert_eq!(
        result.inferred_placements,
        vec![InferredPlacement {
            item_index: 0,
            x: 0,
            y: 0,
            width: 2,
            height: 2
        }]
    );
}

#[test]
fn completing_an_item_uses_the_reduced_inventory_and_blocks_its_cells() {
    let mut board = input([(2, 1, 2), (1, 1, 0), (1, 1, 0)], 4, 1);
    board.cells[0] = "item0".into();
    board.cells[3] = "item0".into();
    let partial = solve_snapshot_native(board.clone());
    exact(&partial, 1);
    assert_eq!(partial.probs[1].iter().sum::<f64>(), 4.0);

    board.items[0].remaining_count -= 1;
    board.cells[0] = "completed".into();
    board.cells[1] = "completed".into();
    let completed = solve_snapshot_native(board);
    exact(&completed, 1);
    assert_eq!(completed.probs[1].iter().sum::<f64>(), 2.0);
    for flag in 0..8 {
        assert_eq!(completed.probs[flag][0], 0.0);
        assert_eq!(completed.probs[flag][1], 0.0);
    }
    assert_eq!(completed.probs[1][2], 1.0);
    assert_eq!(completed.probs[1][3], 1.0);
}

#[test]
fn multiple_completed_regions_need_no_type_and_never_consume_remaining_inventory() {
    let mut board = input([(2, 1, 1), (1, 1, 0), (1, 1, 0)], 3, 3);
    for cell in [0, 1, 9, 10] {
        board.cells[cell] = "completed".into();
    }
    let result = solve_snapshot_native(board);
    exact(&result, 4);
    assert_eq!(result.probs[1].iter().sum::<f64>(), 2.0);
    for flag in 0..8 {
        for cell in [0, 1, 9, 10] {
            assert_eq!(result.probs[flag][cell], 0.0);
        }
    }
}

#[test]
fn zero_remaining_items_allow_completed_cells_but_reject_unfinished_hits() {
    let mut board = input([(1, 1, 0); 3], 9, 5);
    for cell in [0, 1, 9, 10, 43, 44] {
        board.cells[cell] = "completed".into();
    }
    let result = solve_snapshot_native(board.clone());
    exact(&result, 1);
    assert!(result.probs.iter().flatten().all(|&p| p == 0.0));
    assert!(result.inferred_placements.is_empty());
    board.cells[20] = "item1".into();
    failure(&solve_snapshot_native(board), "no_valid_configuration");
}

#[test]
fn rotated_candidate_is_used_as_a_complete_rectangle() {
    let mut board = input([(3, 1, 1), (1, 1, 0), (1, 1, 0)], 3, 3);
    board.cells[10] = "item0".into();
    board.candidate_constraints = vec![constraint(10, 0, vec![rect(1, 0, 1, 3)])];
    let result = solve_snapshot_native(board);
    exact(&result, 1);
    for cell in [1, 10, 19] {
        assert_eq!(result.probs[1][cell], 1.0);
    }
    assert_eq!(
        result.inferred_placements,
        vec![InferredPlacement {
            item_index: 0,
            x: 1,
            y: 0,
            width: 1,
            height: 3
        }]
    );
}

#[test]
fn ambiguous_candidate_directions_are_not_inferred() {
    let mut board = input([(2, 1, 1), (1, 1, 0), (1, 1, 0)], 3, 3);
    board.cells[10] = "item0".into();
    board.candidate_constraints = vec![constraint(10, 0, vec![rect(0, 1, 2, 1), rect(1, 0, 1, 2)])];
    let result = solve_snapshot_native(board);
    exact(&result, 2);
    assert_eq!(result.probs[1][10], 1.0);
    assert_eq!(result.probs[1][1], 0.5);
    assert_eq!(result.probs[1][9], 0.5);
    assert!(result.inferred_placements.is_empty());
}

#[test]
fn every_constraint_on_a_covered_hit_must_be_satisfied() {
    let mut board = input([(2, 2, 1), (1, 1, 0), (1, 1, 0)], 3, 3);
    board.cells[10] = "item0".into();
    board.cells[11] = "item0".into();
    board.candidate_constraints = vec![
        constraint(10, 0, vec![rect(1, 0, 2, 2)]),
        constraint(11, 0, vec![rect(1, 1, 2, 2)]),
    ];
    failure(&solve_snapshot_native(board), "no_valid_configuration");
}

#[test]
fn contradictory_candidates_return_no_inferred_placements() {
    let mut base = input([(2, 1, 1), (1, 1, 0), (1, 1, 0)], 3, 3);
    base.cells[0] = "item0".into();
    base.candidate_constraints = vec![constraint(0, 0, vec![rect(0, 0, 2, 1)])];
    let mut variants = vec![];
    let mut board = base.clone();
    board.cells[1] = "empty".into();
    variants.push(board);
    let mut board = base.clone();
    board.cells[1] = "completed".into();
    variants.push(board);
    let mut board = base.clone();
    board.cells[1] = "item1".into();
    variants.push(board);
    let mut board = base.clone();
    board.items[0].remaining_count = 0;
    variants.push(board);
    let mut board = base;
    board.cells[18] = "item0".into();
    board
        .candidate_constraints
        .push(constraint(18, 0, vec![rect(0, 2, 2, 1)]));
    variants.push(board);
    for board in variants {
        failure(&solve_snapshot_native(board), "no_valid_configuration");
    }
}

#[test]
fn duplicate_constraint_anchors_are_invalid_instead_of_counted_twice() {
    let mut board = input([(2, 1, 1), (1, 1, 0), (1, 1, 0)], 3, 3);
    board.cells[0] = "item0".into();
    board.candidate_constraints = vec![
        constraint(0, 0, vec![rect(0, 0, 2, 1)]),
        constraint(0, 0, vec![rect(0, 0, 1, 2)]),
    ];
    failure(
        &solve_snapshot_native(board),
        "input_error: duplicate constraint anchor",
    );
}

#[test]
fn invalid_lengths_shapes_remaining_counts_and_cells_are_rejected() {
    let base = input([(1, 1, 0); 3], 9, 5);
    let mut variants = vec![];
    let mut board = base.clone();
    board.items.pop();
    variants.push(board);
    let mut board = base.clone();
    board.cells.pop();
    variants.push(board);
    for (width, height, remaining_count) in [
        (0, 1, 0),
        (10, 1, 0),
        (1, -1, 0),
        (1, 1, -1),
        (1, 1, 8),
        (4, 4, 3),
        (i32::MAX, i32::MAX, i32::MAX),
    ] {
        let mut board = base.clone();
        board.items[0] = SnapshotItem {
            width,
            height,
            remaining_count,
        };
        variants.push(board);
    }
    for cell in ["uncertain", "item3", "completed0", ""] {
        let mut board = base.clone();
        board.cells[0] = cell.into();
        variants.push(board);
    }
    for board in variants {
        failure(&solve_snapshot_native(board), "input_error");
    }
}

#[test]
fn invalid_candidate_anchors_geometry_and_overflow_are_rejected() {
    let mut base = input([(2, 1, 1), (1, 1, 0), (1, 1, 0)], 3, 3);
    base.cells[0] = "item0".into();
    for candidate in [
        constraint(45, 0, vec![rect(0, 0, 2, 1)]),
        constraint(usize::MAX, 0, vec![rect(0, 0, 2, 1)]),
        constraint(0, 3, vec![rect(0, 0, 2, 1)]),
        constraint(0, usize::MAX, vec![rect(0, 0, 2, 1)]),
        constraint(1, 0, vec![rect(0, 0, 2, 1)]),
        constraint(0, 1, vec![rect(0, 0, 1, 1)]),
        constraint(0, 0, vec![]),
        constraint(0, 0, vec![rect(0, 0, 0, 1)]),
        constraint(0, 0, vec![rect(0, 0, 3, 1)]),
        constraint(0, 0, vec![rect(0, 0, usize::MAX, 1)]),
        constraint(0, 0, vec![rect(usize::MAX, 0, 2, 1)]),
        constraint(0, 0, vec![rect(0, usize::MAX, 2, 1)]),
        constraint(0, 0, vec![rect(8, 0, 2, 1)]),
        constraint(0, 0, vec![rect(0, 4, 1, 2)]),
        constraint(0, 0, vec![rect(1, 0, 2, 1)]),
    ] {
        let mut board = base.clone();
        board.candidate_constraints = vec![candidate];
        failure(&solve_snapshot_native(board), "input_error");
    }
    for kind in ["empty", "completed"] {
        let mut board = base.clone();
        board.cells[0] = kind.into();
        board.candidate_constraints = vec![constraint(0, 0, vec![rect(0, 0, 2, 1)])];
        failure(&solve_snapshot_native(board), "input_error");
    }
}

#[test]
fn unconstrained_snapshot_matches_the_previous_observed_input() {
    for clue in [None, Some((0, "item0")), Some((10, "item1"))] {
        let mut board = input([(2, 1, 1), (1, 1, 1), (1, 1, 0)], 3, 3);
        board.cells[11] = "empty".into();
        if let Some((cell, kind)) = clue {
            board.cells[cell] = kind.into();
        }
        let old = solve_observed_native(ObservedInput {
            items: board
                .items
                .iter()
                .map(|item| ObservedItem {
                    width: item.width,
                    height: item.height,
                    count: item.remaining_count,
                })
                .collect(),
            cells: board.cells.clone(),
        });
        let new = solve_snapshot_native(board);
        assert_eq!(new.error, old.error);
        assert_eq!(new.precision, old.precision);
        assert_eq!(new.total_patterns, old.total_patterns);
        assert_eq!(new.samples, old.samples);
        assert_eq!(new.probs, old.probs);
        assert!(new.inferred_placements.is_empty());
    }
}

fn covers(placement: GridPlacement, cell: usize) -> bool {
    (placement.x..placement.x + placement.width).contains(&(cell % 9))
        && (placement.y..placement.y + placement.height).contains(&(cell / 9))
}

fn geometry(item: &SnapshotItem, anchor: Option<usize>) -> Vec<GridPlacement> {
    let mut shapes = vec![(item.width as usize, item.height as usize)];
    if item.width != item.height {
        shapes.push((item.height as usize, item.width as usize));
    }
    let mut placements = vec![];
    for (width, height) in shapes {
        for y in 0..=5 - height {
            for x in 0..=9 - width {
                let placement = rect(x, y, width, height);
                if anchor.is_none_or(|cell| covers(placement, cell)) {
                    placements.push(placement);
                }
            }
        }
    }
    placements
}

// Independent oracle: choose complete non-overlapping rectangles by item group,
// then validate typed hits and anchor constraints only at complete configurations.
fn brute_force(board: &SnapshotInput) -> (u64, Vec<Vec<f64>>) {
    let candidates: Vec<Vec<(GridPlacement, u64)>> = board
        .items
        .iter()
        .map(|item| {
            geometry(item, None)
                .into_iter()
                .filter_map(|placement| {
                    let mut mask = 0;
                    for cell in 0..45 {
                        if covers(placement, cell) {
                            if matches!(board.cells[cell].as_str(), "empty" | "completed") {
                                return None;
                            }
                            mask |= 1u64 << cell;
                        }
                    }
                    Some((placement, mask))
                })
                .collect()
        })
        .collect();
    fn visit(
        board: &SnapshotInput,
        candidates: &[Vec<(GridPlacement, u64)>],
        group: usize,
        remaining: i32,
        start: usize,
        masks: &mut [u64; 3],
        chosen: &mut Vec<(usize, GridPlacement)>,
        total: &mut u64,
        counts: &mut [[u64; 45]; 3],
    ) {
        if group == 3 {
            if (0..45).any(|cell| match board.cells[cell].as_str() {
                "item0" => masks[0] & (1 << cell) == 0,
                "item1" => masks[1] & (1 << cell) == 0,
                "item2" => masks[2] & (1 << cell) == 0,
                _ => false,
            }) {
                return;
            }
            if board.candidate_constraints.iter().any(|constraint| {
                !chosen.iter().any(|(item_index, placement)| {
                    *item_index == constraint.item_index
                        && covers(*placement, constraint.anchor)
                        && constraint.placements.contains(placement)
                })
            }) {
                return;
            }
            *total += 1;
            for i in 0..3 {
                for cell in 0..45 {
                    counts[i][cell] += u64::from(masks[i] & (1 << cell) != 0);
                }
            }
        } else if remaining == 0 {
            visit(
                board,
                candidates,
                group + 1,
                board
                    .items
                    .get(group + 1)
                    .map_or(0, |item| item.remaining_count),
                0,
                masks,
                chosen,
                total,
                counts,
            );
        } else {
            let occupied = masks.iter().fold(0, |a, b| a | b);
            for index in start..candidates[group].len() {
                let (placement, mask) = candidates[group][index];
                if occupied & mask != 0 {
                    continue;
                }
                masks[group] |= mask;
                chosen.push((group, placement));
                visit(
                    board,
                    candidates,
                    group,
                    remaining - 1,
                    index + 1,
                    masks,
                    chosen,
                    total,
                    counts,
                );
                chosen.pop();
                masks[group] ^= mask;
            }
        }
    }
    let mut total = 0;
    let mut counts = [[0; 45]; 3];
    visit(
        board,
        &candidates,
        0,
        board.items[0].remaining_count,
        0,
        &mut [0; 3],
        &mut vec![],
        &mut total,
        &mut counts,
    );
    let probs = (0..8)
        .map(|flag| {
            (0..45)
                .map(|cell| {
                    (0..3)
                        .filter(|i| flag & (1 << i) != 0)
                        .map(|i| counts[i][cell])
                        .sum::<u64>() as f64
                        / total as f64
                })
                .collect()
        })
        .collect();
    (total, probs)
}

#[test]
fn small_boards_match_independent_complete_rectangle_enumeration() {
    for shapes in [
        [(2, 1, 2), (1, 1, 1), (1, 1, 0)],
        [(2, 2, 1), (2, 1, 1), (1, 1, 1)],
        [(1, 1, 1); 3],
    ] {
        for clue in [None, Some((0, 0)), Some((10, 1)), Some((20, 2))] {
            for candidate_count in [0, 1, 2, usize::MAX] {
                let mut board = input(shapes, 3, 3);
                board.cells[1] = "completed".into();
                if let Some((cell, group)) = clue {
                    board.cells[cell] = format!("item{group}");
                    if candidate_count > 0 {
                        let candidates = geometry(&board.items[group], Some(cell));
                        board.candidate_constraints = vec![constraint(
                            cell,
                            group,
                            candidates.into_iter().take(candidate_count).collect(),
                        )];
                    }
                }
                let (count, probs) = brute_force(&board);
                let result = solve_snapshot_native(board);
                if count == 0 {
                    failure(&result, "no_valid_configuration");
                } else {
                    exact(&result, count);
                    assert_eq!(result.probs, probs);
                }
            }
        }
    }
    let mut board = input([(2, 2, 1), (1, 1, 1), (1, 1, 0)], 3, 3);
    board.cells[10] = "item0".into();
    board.cells[11] = "item0".into();
    board.candidate_constraints = vec![
        constraint(10, 0, geometry(&board.items[0], Some(10))),
        constraint(11, 0, vec![rect(1, 0, 2, 2)]),
    ];
    let (count, probs) = brute_force(&board);
    let result = solve_snapshot_native(board);
    exact(&result, count);
    assert_eq!(result.probs, probs);
}

#[test]
fn sampled_probability_one_only_infers_singleton_input_constraints() {
    let mut board = input([(2, 1, 1), (1, 1, 3), (1, 1, 2)], 9, 5);
    board.cells[0] = "item0".into();
    board.candidate_constraints = vec![constraint(0, 0, vec![rect(0, 0, 2, 1), rect(0, 0, 1, 2)])];
    let ambiguous = solve_snapshot_native(board.clone());
    assert_eq!(ambiguous.error, "");
    assert_eq!(ambiguous.precision, Some("sampled"));
    assert_eq!(ambiguous.samples, 100_000);
    assert_eq!(ambiguous.probs[1][0], 1.0);
    assert!(ambiguous.inferred_placements.is_empty());

    board.candidate_constraints[0].placements.pop();
    let singleton = solve_snapshot_native(board);
    assert_eq!(singleton.error, "");
    assert_eq!(singleton.precision, Some("sampled"));
    assert_eq!(singleton.samples, 100_000);
    assert_eq!(
        singleton.inferred_placements,
        vec![InferredPlacement {
            item_index: 0,
            x: 0,
            y: 0,
            width: 2,
            height: 1
        }]
    );
    assert_eq!(
        ambiguous.total_patterns.parse::<u64>().unwrap(),
        singleton.total_patterns.parse::<u64>().unwrap() * 2
    );
}

#[test]
fn wide_rectangles_respect_completed_cells_hits_and_candidate_constraints() {
    let mut board = input([(5, 1, 1), (1, 1, 0), (1, 1, 0)], 9, 1);
    board.cells[0] = "completed".into();
    board.cells[1] = "completed".into();
    board.cells[4] = "item0".into();
    let result = solve_snapshot_native(board.clone());
    exact(&result, 3);
    assert_eq!(result.probs[1][0], 0.0);
    assert_eq!(result.probs[1][1], 0.0);
    assert_eq!(result.probs[1][4], 1.0);
    board.candidate_constraints = vec![constraint(4, 0, vec![rect(2, 0, 5, 1)])];
    let result = solve_snapshot_native(board.clone());
    exact(&result, 1);
    assert_eq!(result.inferred_placements.len(), 1);
    assert_eq!(result.inferred_placements[0].width, 5);
    for cell in 0..45 { assert_eq!(result.probs[1][cell], if (2..7).contains(&cell) { 1.0 } else { 0.0 }); }
    board.cells[6] = "completed".into();
    failure(&solve_snapshot_native(board), "no_valid_configuration");
}

#[test]
fn full_board_rotation_and_large_invalid_dimensions_are_validated() {
    for (width, height) in [(9, 5), (5, 9)] {
        let mut board = input([(width, height, 1), (1, 1, 0), (1, 1, 0)], 9, 5);
        board.cells[44] = "item0".into();
        board.candidate_constraints = vec![constraint(44, 0, vec![rect(0, 0, 9, 5)])];
        let result = solve_snapshot_native(board);
        exact(&result, 1);
        assert!(result.probs[1].iter().all(|&p| p == 1.0));
    }
    for (width, height) in [(6, 6), (10, 1), (1, 10)] {
        failure(&solve_snapshot_native(input([(width, height, 0), (1, 1, 0), (1, 1, 0)], 9, 5)), "input_error");
    }
}

#[test]
fn feasibility_retains_exact_count_and_constraints_with_only_one_witness() {
    let mut board = input([(3, 3, 1), (2, 2, 4), (2, 1, 3)], 9, 5);
    board.cells[10] = "item1".into();
    board.candidate_constraints = vec![constraint(10, 1, vec![rect(1, 1, 2, 2)])];

    let probabilities = solve_snapshot_native(board.clone());
    let feasibility = check_snapshot_feasibility_native(board);
    assert_eq!(probabilities.error, "");
    assert_eq!(probabilities.samples, 100_000);
    assert_eq!(feasibility.error, "");
    assert_eq!(feasibility.precision, Some("sampled"));
    assert_eq!(feasibility.samples, 1);
    assert_eq!(feasibility.total_patterns, probabilities.total_patterns);
    assert_eq!(feasibility.inferred_placements, probabilities.inferred_placements);
    assert!(feasibility.probs.is_empty());
}

#[test]
fn feasibility_preserves_unique_zero_invalid_and_overflow_results() {
    let mut unique = input([(2, 1, 1), (1, 1, 0), (1, 1, 0)], 2, 1);
    unique.cells[0] = "item0".into();
    unique.candidate_constraints = vec![constraint(0, 0, vec![rect(0, 0, 2, 1)])];
    let result = check_snapshot_feasibility_native(unique.clone());
    assert_eq!(result.error, "");
    assert_eq!(result.precision, Some("exact"));
    assert_eq!(result.total_patterns, "1");
    assert_eq!(result.samples, 1);
    assert!(result.probs.is_empty());

    let mut no_layout = unique.clone();
    no_layout.cells[1] = "completed".into();
    let mut invalid = unique;
    invalid.candidate_constraints[0].anchor = 45;
    for board in [no_layout, invalid, input([(1, 1, 7); 3], 9, 5)] {
        let probabilities = solve_snapshot_native(board.clone());
        let feasibility = check_snapshot_feasibility_native(board);
        assert!(!probabilities.error.is_empty());
        assert_eq!(feasibility.error, probabilities.error);
        assert_eq!(feasibility.precision, None);
        assert_eq!(feasibility.total_patterns, "0");
        assert_eq!(feasibility.samples, 0);
        assert!(feasibility.probs.is_empty());
        assert!(feasibility.inferred_placements.is_empty());
    }
}
