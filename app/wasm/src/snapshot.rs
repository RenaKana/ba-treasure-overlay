//! Desktop observations use the UI's remaining inventory and block completed cells.
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

// serde-wasm-bindgen's struct path only visits declared fields. Visiting the
// actual map first makes deny_unknown_fields effective for every nested input.
macro_rules! strict_input_struct {
    ($(#[$attr:meta])* pub struct $name:ident {
        $($(#[$field_attr:meta])* pub $field:ident: $ty:ty,)*
    }) => {
        $(#[$attr])*
        pub struct $name { $(pub $field: $ty,)* }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Fields { $($(#[$field_attr])* $field: $ty,)* }
                struct FieldsVisitor;
                impl<'de> serde::de::Visitor<'de> for FieldsVisitor {
                    type Value = Fields;
                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        formatter.write_str(concat!("a ", stringify!($name), " object"))
                    }
                    fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> std::result::Result<Self::Value, A::Error> {
                        Fields::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                    }
                }
                let fields = deserializer.deserialize_map(FieldsVisitor)?;
                Ok(Self { $($field: fields.$field,)* })
            }
        }
    };
}

strict_input_struct! {
    #[derive(Debug, Clone)]
    pub struct SnapshotInput {
        pub items: Vec<SnapshotItem>,
        pub cells: Vec<String>,
        #[serde(default)]
        pub candidate_constraints: Vec<PlacementConstraint>,
    }
}

strict_input_struct! {
    #[derive(Debug, Clone)]
    pub struct SnapshotItem {
        pub width: i32,
        pub height: i32,
        pub remaining_count: i32,
    }
}

strict_input_struct! {
    #[derive(Debug, Clone)]
    pub struct PlacementConstraint {
        pub anchor: usize,
        pub item_index: usize,
        pub placements: Vec<GridPlacement>,
    }
}

strict_input_struct! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub struct GridPlacement {
        pub x: usize,
        pub y: usize,
        pub width: usize,
        pub height: usize,
    }
}

impl GridPlacement {
    pub(crate) fn contains(&self, cell: usize) -> bool {
        let x = cell % GameState::WIDTH;
        let y = cell / GameState::WIDTH;
        x >= self.x && x - self.x < self.width && y >= self.y && y - self.y < self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct InferredPlacement {
    pub item_index: usize,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotResult {
    pub probs: Vec<Vec<f64>>,
    pub error: String,
    pub precision: Option<&'static str>,
    pub total_patterns: String,
    pub samples: u64,
    /// Only singleton input constraints are certain, even when sampling is used.
    pub inferred_placements: Vec<InferredPlacement>,
}

impl SnapshotResult {
    pub(crate) fn failure(error: impl Into<String>) -> Self {
        Self {
            probs: vec![],
            error: error.into(),
            precision: None,
            total_patterns: "0".into(),
            samples: 0,
            inferred_placements: vec![],
        }
    }
}

/// Native counterpart of `solve_snapshot` for deterministic validation fixtures.
pub fn solve_snapshot_native(input: SnapshotInput) -> SnapshotResult {
    match solve_inner(input, SAMPLE_LIMIT, true) {
        Ok(result) => result,
        Err(error) => SnapshotResult::failure(error.to_string()),
    }
}

/// Check the same complete layout constraints with one witness and no probabilities.
/// The checked DP count is still exhaustive; overflow and input errors remain errors.
pub fn check_snapshot_feasibility_native(input: SnapshotInput) -> SnapshotResult {
    match solve_inner(input, 1, false) {
        Ok(result) => result,
        Err(error) => SnapshotResult::failure(error.to_string()),
    }
}

fn solve_inner(
    mut input: SnapshotInput,
    sample_limit: u64,
    include_probabilities: bool,
) -> Result<SnapshotResult> {
    ensure!(
        input.items.len() == 3,
        "input_error: exactly 3 item groups required"
    );
    ensure!(
        input.cells.len() == CELL_COUNT,
        "input_error: exactly 45 cells required"
    );

    let mut total_area = 0;
    for item in &input.items {
        ensure!(
            item.width > 0 && item.height > 0
                && GameState::item_fits(item.width as usize, item.height as usize),
            "input_error: item rectangle must fit the 9 by 5 board in either orientation"
        );
        ensure!(
            (0..=7).contains(&item.remaining_count),
            "input_error: item remaining_count must be from 0 to 7"
        );
        total_area += item.width * item.height * item.remaining_count;
    }
    ensure!(
        total_area <= CELL_COUNT as i32,
        "input_error: total remaining item area exceeds 45 cells"
    );

    let mut hits = Vec::with_capacity(CELL_COUNT);
    let mut blocked = Vec::with_capacity(CELL_COUNT);
    for cell in &input.cells {
        let hit = match cell.as_str() {
            "unknown" | "empty" | "completed" => None,
            "item0" => Some(0),
            "item1" => Some(1),
            "item2" => Some(2),
            _ => bail!("input_error: invalid cell type"),
        };
        hits.push(hit);
        blocked.push(cell == "empty" || cell == "completed");
    }

    let mut anchors = [false; CELL_COUNT];
    for constraint in &mut input.candidate_constraints {
        ensure!(
            constraint.anchor < CELL_COUNT,
            "input_error: constraint anchor is outside the board"
        );
        ensure!(
            constraint.item_index < GameState::ITEM_GROUP_COUNT,
            "input_error: constraint item_index must be from 0 to 2"
        );
        ensure!(
            hits[constraint.anchor] == Some(constraint.item_index),
            "input_error: constraint anchor must be a matching typed hit"
        );
        ensure!(
            !anchors[constraint.anchor],
            "input_error: duplicate constraint anchor"
        );
        anchors[constraint.anchor] = true;
        ensure!(
            !constraint.placements.is_empty(),
            "input_error: constraint placements must not be empty"
        );
        let item = &input.items[constraint.item_index];
        let (width, height) = (item.width as usize, item.height as usize);
        for placement in &constraint.placements {
            ensure!(
                (placement.width == width && placement.height == height)
                    || (placement.width == height && placement.height == width),
                "input_error: constraint placement dimensions must match the item or its rotation"
            );
            let right = placement.x.checked_add(placement.width);
            let bottom = placement.y.checked_add(placement.height);
            ensure!(
                right.is_some_and(|right| right <= GameState::WIDTH)
                    && bottom.is_some_and(|bottom| bottom <= GameState::HEIGHT),
                "input_error: constraint placement is outside the board"
            );
            ensure!(
                placement.contains(constraint.anchor),
                "input_error: constraint placement must contain its anchor"
            );
        }
        constraint.placements.sort_unstable();
        constraint.placements.dedup();
    }

    let groups = std::array::from_fn(|index| {
        let item = &input.items[index];
        ItemGroup::new(
            Item::new(item.height as usize, item.width as usize, index),
            item.remaining_count as usize,
        )
    });
    let state = GameState::new(
        Map2d::new(blocked, GameState::WIDTH, GameState::HEIGHT),
        groups,
        vec![],
    );
    let constraints =
        (!input.candidate_constraints.is_empty()).then_some(input.candidate_constraints.as_slice());
    let mut rng = Pcg64Mcg::from_entropy();
    let sampled = counter::sample_constrained_placements(
        &state,
        Some(&hits),
        constraints,
        sample_limit,
        &mut rng,
    )?;
    ensure!(
        sampled.all_count > 0,
        "no_valid_configuration: observations, remaining counts, and candidates are inconsistent"
    );

    let probs = if include_probabilities {
        (0..8)
            .map(|flag| {
                counter::calc_probabilities(
                    &state,
                    flag,
                    sampled.sampled_count,
                    &sampled.sampled_item_counts,
                )
                .iter()
                .copied()
                .collect()
            })
            .collect()
    } else {
        vec![]
    };
    let mut inferred_placements: Vec<_> = input
        .candidate_constraints
        .iter()
        .filter(|constraint| constraint.placements.len() == 1)
        .map(|constraint| {
            let placement = constraint.placements[0];
            InferredPlacement {
                item_index: constraint.item_index,
                x: placement.x,
                y: placement.y,
                width: placement.width,
                height: placement.height,
            }
        })
        .collect();
    inferred_placements.sort_unstable();
    inferred_placements.dedup();
    Ok(SnapshotResult {
        probs,
        error: String::new(),
        precision: Some(if sampled.all_count <= sample_limit {
            "exact"
        } else {
            "sampled"
        }),
        total_patterns: sampled.all_count.to_string(),
        samples: sampled.sampled_count,
        inferred_placements,
    })
}
