pub mod grid;
pub mod observed;
pub mod problem;
pub mod snapshot;
pub mod solver;
mod utils;

use anyhow::{anyhow, bail, ensure, Result};
use grid::Coord;
use problem::GameState;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

pub use observed::{solve_observed_native, ObservedInput, ObservedItem, ObservedResult};
pub use snapshot::{
    solve_snapshot_native, GridPlacement, InferredPlacement, PlacementConstraint, SnapshotInput,
    SnapshotItem, SnapshotResult,
};

/// Desktop snapshot: counts exclude completed items but include partial hits.
#[wasm_bindgen]
pub fn solve_snapshot(input: JsValue) -> JsValue {
    let result = match serde_wasm_bindgen::from_value::<SnapshotInput>(input) {
        Ok(input) => solve_snapshot_native(input),
        Err(_) => SnapshotResult::failure("input_error: invalid input shape or value type"),
    };
    result
        .serialize(&serde_wasm_bindgen::Serializer::new().serialize_missing_as_null(true))
        .unwrap()
}

/// Partial observations: item counts are the board's original totals.
#[wasm_bindgen]
pub fn solve_observed(input: JsValue) -> JsValue {
    let result = match serde_wasm_bindgen::from_value::<ObservedInput>(input) {
        Ok(input) => solve_observed_native(input),
        Err(_) => ObservedResult::failure("input_error: invalid input shape or value type"),
    };
    result
        .serialize(&serde_wasm_bindgen::Serializer::new().serialize_missing_as_null(true))
        .unwrap()
}

use crate::{
    grid::Map2d,
    problem::{Item, ItemGroup},
    solver::counter,
};

#[derive(Deserialize, Clone)]
pub struct JsInput {
    pub item_and_placement: Vec<JsItemAndPlacement>,
    pub open_map: Vec<bool>,
}

#[derive(Deserialize, Clone)]
pub struct JsItemAndPlacement {
    pub item: JsItemSet,
    pub placements: Vec<JsPlacedItem>,
}

impl TryFrom<JsInput> for GameState {
    type Error = anyhow::Error;

    fn try_from(value: JsInput) -> Result<Self, Self::Error> {
        ensure!(
            value.item_and_placement.len() == GameState::ITEM_GROUP_COUNT,
            "input_error: exactly 3 item groups required"
        );
        ensure!(
            value.open_map.len() == GameState::WIDTH * GameState::HEIGHT,
            "input_error: exactly 45 cells required"
        );
        let mut total_area = 0;
        for group in &value.item_and_placement {
            let item = &group.item.item;
            ensure!(
                item.width > 0
                    && item.height > 0
                    && GameState::item_fits(item.width as usize, item.height as usize),
                "input_error: item rectangle must fit the 9 by 5 board in either orientation"
            );
            ensure!(
                (0..=7).contains(&group.item.count),
                "input_error: item count must be from 0 to 7"
            );
            total_area += item.width * item.height * group.item.count;
        }
        ensure!(
            total_area <= (GameState::WIDTH * GameState::HEIGHT) as i32,
            "input_error: total item area exceeds 45 cells"
        );
        let open_map = Map2d::new(value.open_map, GameState::WIDTH, GameState::HEIGHT);
        let mut remaining_items = [
            value.item_and_placement[0].item.clone(),
            value.item_and_placement[1].item.clone(),
            value.item_and_placement[2].item.clone(),
        ]
        .map(|item_set| {
            ItemGroup::new(
                Item::new(
                    item_set.item.height as usize,
                    item_set.item.width as usize,
                    item_set.item.index as usize,
                ),
                item_set.count as usize,
            )
        });

        let mut placed_items = vec![];

        for placed_item in value
            .item_and_placement
            .iter()
            .flat_map(|ip| ip.placements.iter())
        {
            ensure!(
                (1..=3).contains(&placed_item.item.index),
                "input_error: invalid placed item index"
            );
            let group_index = placed_item.item.index as usize - 1;
            let shape = remaining_items[group_index].item;
            ensure!(
                placed_item.item.width > 0
                    && placed_item.item.height > 0
                    && ((placed_item.item.width as usize == shape.width()
                        && placed_item.item.height as usize == shape.height())
                        || (placed_item.item.width as usize == shape.height()
                            && placed_item.item.height as usize == shape.width())),
                "input_error: placed item dimensions must match its group"
            );
            ensure!(
                remaining_items[group_index].count > 0,
                "input_error: too many placed items"
            );
            ensure!(
                placed_item.row >= 1 && placed_item.col >= 1,
                "input_error: placed item is outside the board"
            );
            let mut item = Item::new(
                placed_item.item.height as usize,
                placed_item.item.width as usize,
                placed_item.item.index as usize - 1,
            );

            if placed_item.rotated {
                item = item.rotate().unwrap_or(item);
            }

            ensure!(
                (placed_item.row as usize - 1)
                    .checked_add(item.height())
                    .is_some_and(|end| end <= GameState::HEIGHT)
                    && (placed_item.col as usize - 1)
                        .checked_add(item.width())
                        .is_some_and(|end| end <= GameState::WIDTH),
                "input_error: placed item is outside the board"
            );
            remaining_items[item.item_index()].count -= 1;

            let item = problem::PlacedItem {
                item,
                coord: Coord::new(placed_item.row as usize - 1, placed_item.col as usize - 1),
            };

            placed_items.push(item);
        }

        // 重なっていないかチェック
        let mut overlap_map = Map2d::new_with(0u32, GameState::WIDTH, GameState::HEIGHT);
        let mut counts = [0; GameState::ITEM_GROUP_COUNT];
        let mut labels = vec![];

        for (i, placed) in placed_items.iter().enumerate() {
            counts[placed.item.item_index()] += 1;
            labels.push((
                placed.item.item_index() + 1,
                counts[placed.item.item_index()],
            ));

            let row0 = placed.coord.row;
            let col0 = placed.coord.col;
            let row1 = row0 + placed.item.height();
            let col1 = col0 + placed.item.width();

            for row in row0..row1 {
                for col in col0..col1 {
                    overlap_map[Coord::new(row, col)] |= 1 << i;
                }
            }
        }

        for row in 0..GameState::HEIGHT {
            for col in 0..GameState::WIDTH {
                let mut flag = overlap_map[Coord::new(row, col)];
                if flag.count_ones() > 1 {
                    let i = flag.trailing_zeros() as usize;
                    let (item0, index0) = labels[i];
                    flag ^= 1 << i;
                    let j = flag.trailing_zeros() as usize;
                    let (item1, index1) = labels[j];

                    return Err(anyhow!(
                        "overlap {} {} {} {} {} {}",
                        item0,
                        index0,
                        item1,
                        index1,
                        row + 1,
                        col + 1
                    ));
                }
            }
        }

        Ok(GameState::new(open_map, remaining_items, placed_items))
    }
}

#[derive(Deserialize, Clone)]
pub struct JsItemSet {
    pub item: JsItem,
    pub count: i32,
}

#[derive(Deserialize, Clone)]
pub struct JsItem {
    pub width: i32,
    pub height: i32,
    pub index: i32,
}

#[derive(Deserialize, Clone)]
pub struct JsPlacedItem {
    pub item: JsItem,
    pub rotated: bool,
    pub row: i32,
    pub col: i32,
    pub id: String,
}

#[derive(Serialize, Clone)]
pub struct ProbResult {
    pub probs: Vec<Vec<f64>>,
    pub error: String,
}

#[wasm_bindgen]
extern "C" {
    fn alert(s: &str);
}

#[wasm_bindgen]
pub fn solve(input: JsValue) -> JsValue {
    match solve_inner(input) {
        Ok(result) => serde_wasm_bindgen::to_value(&ProbResult {
            probs: result,
            error: "".to_string(),
        })
        .unwrap(),
        Err(err) => serde_wasm_bindgen::to_value(&ProbResult {
            probs: vec![],
            error: err.to_string(),
        })
        .unwrap(),
    }
}

fn solve_inner(input: JsValue) -> anyhow::Result<Vec<Vec<f64>>> {
    let input = match serde_wasm_bindgen::from_value::<JsInput>(input) {
        Ok(input) => input,
        Err(_) => {
            bail!("input_error")
        }
    };

    let game_state = GameState::try_from(input)?;

    let result = counter::calc_probabilities_all(&game_state, 100000)?;
    let result = result
        .iter()
        .map(|prob| prob.iter().copied().collect())
        .collect();

    Ok(result)
}
