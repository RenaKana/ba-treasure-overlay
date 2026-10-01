//! Observation-only adapter for the original rectangle-placement DP.
use crate::{
    grid::Map2d,
    problem::{GameState, Item, ItemGroup},
    solver::counter,
};
use anyhow::{bail, ensure, Result};
use rand::SeedableRng;
use rand_pcg::Pcg64Mcg;
use serde::{Deserialize, Serialize};

const CELL_COUNT: usize = GameState::WIDTH * GameState::HEIGHT;
const SAMPLE_LIMIT: u64 = 100_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedInput {
    pub items: Vec<ObservedItem>,
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedItem {
    pub width: i32,
    pub height: i32,
    pub count: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservedResult {
    /// Bitmask rows 0..8, each containing 45 row-major cell probabilities.
    pub probs: Vec<Vec<f64>>,
    pub error: String,
    pub precision: Option<&'static str>,
    pub total_patterns: String,
    pub samples: u64,
}

impl ObservedResult {
    pub(crate) fn failure(error: impl Into<String>) -> Self {
        Self {
            probs: vec![],
            error: error.into(),
            precision: None,
            total_patterns: "0".into(),
            samples: 0,
        }
    }
}

/// Native counterpart of `solve_observed`, useful for tests and timing fixtures.
pub fn solve_observed_native(input: ObservedInput) -> ObservedResult {
    match solve_inner(input) {
        Ok(result) => result,
        Err(error) => ObservedResult::failure(error.to_string()),
    }
}

fn solve_inner(input: ObservedInput) -> Result<ObservedResult> {
    ensure!(input.items.len() == 3, "input_error: exactly 3 item groups required");
    ensure!(input.cells.len() == CELL_COUNT, "input_error: exactly 45 cells required");

    let mut total_area = 0;
    for item in &input.items {
        ensure!(item.width > 0 && item.height > 0
                && GameState::item_fits(item.width as usize, item.height as usize),
            "input_error: item rectangle must fit the 9 by 5 board in either orientation");
        ensure!((0..=7).contains(&item.count), "input_error: item count must be from 0 to 7");
        total_area += item.width * item.height * item.count;
    }
    ensure!(total_area <= CELL_COUNT as i32, "input_error: total item area exceeds 45 cells");

    let mut hits = Vec::with_capacity(CELL_COUNT);
    let mut empty = Vec::with_capacity(CELL_COUNT);
    for cell in &input.cells {
        let hit = match cell.as_str() {
            "unknown" | "empty" => None,
            "item0" => Some(0),
            "item1" => Some(1),
            "item2" => Some(2),
            _ => bail!("input_error: invalid cell type"),
        };
        hits.push(hit);
        empty.push(cell == "empty");
    }

    let groups = std::array::from_fn(|index| {
        let item = &input.items[index];
        ItemGroup::new(Item::new(item.height as usize, item.width as usize, index), item.count as usize)
    });
    let state = GameState::new(Map2d::new(empty, GameState::WIDTH, GameState::HEIGHT), groups, vec![]);
    let mut rng = Pcg64Mcg::from_entropy();
    let sampled = counter::sample_observed_placements(&state, Some(&hits), SAMPLE_LIMIT, &mut rng)?;
    ensure!(sampled.all_count > 0, "no_valid_configuration: observations and item totals are inconsistent");

    let probs = (0..8).map(|flag| {
        counter::calc_probabilities(&state, flag, sampled.sampled_count, &sampled.sampled_item_counts)
            .iter().copied().collect()
    }).collect();
    Ok(ObservedResult {
        probs,
        error: String::new(),
        precision: Some(if sampled.all_count <= SAMPLE_LIMIT { "exact" } else { "sampled" }),
        total_patterns: sampled.all_count.to_string(),
        samples: sampled.sampled_count,
    })
}
