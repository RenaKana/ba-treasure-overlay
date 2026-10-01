use wasm_solver::{
    problem::GameState, solver::counter, JsInput, JsItem, JsItemAndPlacement, JsItemSet,
    JsPlacedItem,
};

fn legacy(width: i32, height: i32) -> JsInput {
    JsInput {
        item_and_placement: (0..3)
            .map(|index| JsItemAndPlacement {
                item: JsItemSet {
                    item: JsItem {
                        width: if index == 0 { width } else { 1 },
                        height: if index == 0 { height } else { 1 },
                        index: index + 1,
                    },
                    count: if index == 0 { 1 } else { 0 },
                },
                placements: vec![],
            })
            .collect(),
        open_map: vec![false; 45],
    }
}

#[test]
fn legacy_solver_accepts_board_fitting_dimensions_and_rejects_invalid_shapes() {
    for (width, height) in [(5, 1), (1, 5), (9, 1), (5, 5), (9, 5), (5, 9)] {
        let state = GameState::try_from(legacy(width, height)).unwrap();
        let probs = counter::calc_probabilities_all(&state, 100_000).unwrap();
        assert!((probs[1].iter().sum::<f64>() - (width * height) as f64).abs() < 1e-12);
    }
    for (width, height) in [(0, 1), (6, 6), (10, 1), (1, 10), (-1, 1)] {
        assert!(GameState::try_from(legacy(width, height))
            .unwrap_err()
            .to_string()
            .starts_with("input_error"));
    }
}

#[test]
fn legacy_completed_rotated_rectangle_stays_inside_board() {
    let mut input = legacy(5, 9);
    input.item_and_placement[0].placements.push(JsPlacedItem {
        item: JsItem {
            width: 5,
            height: 9,
            index: 1,
        },
        rotated: true,
        row: 1,
        col: 1,
        id: "full".into(),
    });
    let state = GameState::try_from(input.clone()).unwrap();
    assert_eq!(state.remaining_items[0].count, 0);
    let probs = counter::calc_probabilities_all(&state, 100_000).unwrap();
    assert!(probs[1].iter().all(|&p| p == 1.0));
    input.item_and_placement[0].placements[0].rotated = false;
    assert!(GameState::try_from(input)
        .unwrap_err()
        .to_string()
        .contains("outside the board"));
}
