//! Offline existential checks for geometric candidates. No probability or visual
//! angle is used to infer uniqueness, and the observed input is never modified.
//!
//! The native snapshot solver supports three groups, counts of 0..=7, and a 9x5
//! board. Smaller effective boards are padded with blocked cells. An observation
//! is an OR of its candidates; observations are combined with AND. An empty
//! candidate list means missing recognition evidence, rather than a proof of
//! inconsistency. Counts are remaining inventory, including partial items.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
use wasm_solver::problem::GameState;
use wasm_solver::snapshot::{
    check_snapshot_feasibility_native, GridPlacement, PlacementConstraint, SnapshotInput,
    SnapshotItem, SnapshotResult,
};

const NATIVE_WIDTH: usize = GameState::WIDTH;
const NATIVE_HEIGHT: usize = GameState::HEIGHT;
const GROUP_COUNT: usize = GameState::ITEM_GROUP_COUNT;
const NATIVE_MAX_COUNT: u32 = 7; // SnapshotInput's existing technical limit.
const NATIVE_ZERO_ERROR: &str =
    "no_valid_configuration: observations, remaining counts, and candidates are inconsistent";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Geometry {
    pub item_index: usize,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub candidates: Vec<Geometry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintInput {
    pub width: u32,
    pub height: u32,
    pub shapes: Vec<[u32; 2]>,
    pub counts: Vec<u32>,
    /// Indices are row-major in the effective board, not the padded native board.
    pub empty_cells: Vec<usize>,
    pub completed_cells: Vec<usize>,
    pub observations: Vec<Observation>,
}

/// These are execution budgets, not limits on candidate or observation counts.
/// Zero stops before the corresponding work. The native API cannot be cancelled:
/// elapsed time is checked between calls, and one call may exceed this budget.
/// A completed call's evidence remains valid even if it used the remaining time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub max_search_nodes: u64,
    pub max_solver_calls: u64,
    pub max_elapsed_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_search_nodes: 50_000,
            max_solver_calls: 256,
            max_elapsed_ms: 3_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Feasible,
    Infeasible,
    InvalidInput,
    Interrupted,
    InsufficientEvidence,
}

impl Status {
    fn definitive(self) -> bool {
        matches!(self, Self::Feasible | Self::Infeasible)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateReport {
    pub geometry: Geometry,
    pub status: Status,
    /// Definitive existence/non-existence, never a uniqueness assertion.
    pub complete: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationReport {
    pub observation_id: String,
    /// Repeated type+rectangle candidates are reported once; visual angles belong
    /// to the recognizer and share the same geometric feasibility result.
    pub candidates: Vec<CandidateReport>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConstraintStats {
    pub elapsed_ms: f64,
    pub solver_elapsed_ms: f64,
    pub search_nodes: u64,
    pub solver_calls: u64,
    pub solver_cache_hits: u64,
    pub geometry_cache_hits: u64,
    pub exact_witnesses: u64,
    pub sampled_witnesses: u64,
    pub exact_no_solutions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintReport {
    /// Feasible means at least one full-layout witness was found. It can coexist
    /// with complete=false when other candidates remain unchecked.
    pub status: Status,
    /// True only when every supplied geometric candidate was resolved.
    pub complete: bool,
    pub reason: Option<String>,
    pub observations: Vec<ObservationReport>,
    pub stats: ConstraintStats,
}

#[derive(Debug, Clone)]
struct Decision {
    status: Status,
    reason: String,
}

impl Decision {
    fn new(status: Status, reason: impl Into<String>) -> Self {
        Self {
            status,
            reason: reason.into(),
        }
    }
}

pub fn check_candidates(input: &ConstraintInput, config: &Config) -> ConstraintReport {
    check_with_solver(input, config, check_snapshot_feasibility_native)
}

fn check_with_solver(
    input: &ConstraintInput,
    config: &Config,
    solver: impl FnMut(SnapshotInput) -> SnapshotResult,
) -> ConstraintReport {
    let started = Instant::now();
    let invalid = validate(input).err();
    let missing = input.observations.is_empty()
        || input
            .observations
            .iter()
            .any(|observation| observation.candidates.is_empty());
    let normalized: Vec<Vec<Geometry>> = input
        .observations
        .iter()
        .map(|observation| {
            let mut candidates = observation.candidates.clone();
            candidates.sort_unstable();
            candidates.dedup();
            candidates
        })
        .collect();
    let mut engine = Engine {
        input,
        config,
        started,
        solver,
        observations: &normalized,
        blocked: blocked_mask(input),
        stats: ConstraintStats::default(),
        solver_cache: HashMap::new(),
        geometry_cache: HashMap::new(),
    };
    let mut observations = Vec::with_capacity(input.observations.len());
    for (observation, candidates) in input.observations.iter().zip(&normalized) {
        let mut results = Vec::with_capacity(candidates.len());
        for &geometry in candidates {
            let decision = if let Some(reason) = &invalid {
                Decision::new(Status::InvalidInput, reason)
            } else if missing {
                Decision::new(
                    Status::InsufficientEvidence,
                    "an observation has no candidates",
                )
            } else if let Some(decision) = engine.geometry_cache.get(&geometry) {
                engine.stats.geometry_cache_hits += 1;
                decision.clone()
            } else {
                let decision = if engine.compatible(geometry, &[]) {
                    engine.search(&mut vec![geometry])
                } else {
                    Decision::new(
                        Status::Infeasible,
                        "candidate conflicts with blocked cells or remaining count",
                    )
                };
                engine.geometry_cache.insert(geometry, decision.clone());
                decision
            };
            results.push(CandidateReport {
                geometry,
                status: decision.status,
                complete: decision.status.definitive(),
                reason: decision.reason,
            });
        }
        observations.push(ObservationReport {
            observation_id: observation.id.clone(),
            candidates: results,
        });
    }
    let candidates: Vec<_> = observations.iter().flat_map(|o| &o.candidates).collect();
    let complete = !missing && invalid.is_none() && candidates.iter().all(|c| c.complete);
    let status = if invalid.is_some() {
        Status::InvalidInput
    } else if candidates.iter().any(|c| c.status == Status::Feasible) {
        Status::Feasible
    } else if missing
        || candidates
            .iter()
            .any(|c| c.status == Status::InsufficientEvidence)
    {
        if candidates.iter().any(|c| c.status == Status::Interrupted) {
            Status::Interrupted
        } else {
            Status::InsufficientEvidence
        }
    } else if candidates.iter().any(|c| c.status == Status::Interrupted) {
        Status::Interrupted
    } else {
        Status::Infeasible
    };
    engine.stats.elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    ConstraintReport {
        status,
        complete,
        reason: invalid
            .or_else(|| missing.then(|| "no complete set of observation candidates".into())),
        observations,
        stats: engine.stats,
    }
}

fn validate(input: &ConstraintInput) -> Result<(), String> {
    if input.width == 0
        || input.height == 0
        || input.width as usize > NATIVE_WIDTH
        || input.height as usize > NATIVE_HEIGHT
    {
        return Err(format!(
            "effective board must fit native {NATIVE_WIDTH}x{NATIVE_HEIGHT} board"
        ));
    }
    if input.shapes.len() != GROUP_COUNT || input.counts.len() != GROUP_COUNT {
        return Err(format!(
            "native solver requires exactly {GROUP_COUNT} shapes and counts"
        ));
    }
    let mut area = 0u64;
    for (shape, &count) in input.shapes.iter().zip(&input.counts) {
        if !GameState::item_fits(shape[0] as usize, shape[1] as usize) {
            return Err("item shape must fit native board in either orientation".into());
        }
        if count > NATIVE_MAX_COUNT {
            return Err(format!(
                "native solver remaining count must be at most {NATIVE_MAX_COUNT}"
            ));
        }
        area += u64::from(shape[0]) * u64::from(shape[1]) * u64::from(count);
    }
    if area > (NATIVE_WIDTH * NATIVE_HEIGHT) as u64 {
        return Err("native solver total remaining area exceeds board area".into());
    }
    let cells = input.width as usize * input.height as usize;
    if input
        .empty_cells
        .iter()
        .chain(&input.completed_cells)
        .any(|&cell| cell >= cells)
    {
        return Err("observed cell is outside the effective board".into());
    }
    if input
        .empty_cells
        .iter()
        .any(|cell| input.completed_cells.contains(cell))
    {
        return Err("a cell cannot be both empty and completed".into());
    }
    let mut ids = HashSet::new();
    for observation in &input.observations {
        if !ids.insert(&observation.id) {
            return Err("observation ids must be unique".into());
        }
        for geometry in &observation.candidates {
            let Some(shape) = input.shapes.get(geometry.item_index) else {
                return Err("candidate item_index is outside the shape groups".into());
            };
            if [geometry.w, geometry.h] != *shape && [geometry.h, geometry.w] != *shape {
                return Err("candidate dimensions must match its item or rotation".into());
            }
            if !geometry
                .x
                .checked_add(geometry.w)
                .is_some_and(|x| x <= input.width)
                || !geometry
                    .y
                    .checked_add(geometry.h)
                    .is_some_and(|y| y <= input.height)
            {
                return Err("candidate is outside the effective board".into());
            }
        }
    }
    Ok(())
}

fn blocked_mask(input: &ConstraintInput) -> u64 {
    // Invalid inputs are never searched; avoid arithmetic/allocation using them.
    if input.width == 0
        || input.width as usize > NATIVE_WIDTH
        || input.height as usize > NATIVE_HEIGHT
    {
        return 0;
    }
    input
        .empty_cells
        .iter()
        .chain(&input.completed_cells)
        .filter(|&&cell| cell < input.width as usize * input.height as usize)
        .fold(0, |mask, &cell| {
            mask | (1u64
                << ((cell / input.width as usize) * NATIVE_WIDTH + cell % input.width as usize))
        })
}

fn geometry_mask(geometry: Geometry) -> u64 {
    let mut mask = 0;
    for y in geometry.y..geometry.y + geometry.h {
        for x in geometry.x..geometry.x + geometry.w {
            mask |= 1u64 << (y as usize * NATIVE_WIDTH + x as usize);
        }
    }
    mask
}

struct Engine<'a, F> {
    input: &'a ConstraintInput,
    config: &'a Config,
    observations: &'a [Vec<Geometry>],
    started: Instant,
    solver: F,
    blocked: u64,
    stats: ConstraintStats,
    solver_cache: HashMap<Vec<Geometry>, Decision>,
    geometry_cache: HashMap<Geometry, Decision>,
}

impl<F: FnMut(SnapshotInput) -> SnapshotResult> Engine<'_, F> {
    fn time_exhausted(&self) -> bool {
        self.started.elapsed() >= Duration::from_millis(self.config.max_elapsed_ms)
    }

    fn compatible(&self, geometry: Geometry, selected: &[Geometry]) -> bool {
        if selected.contains(&geometry) {
            return true;
        }
        let mask = geometry_mask(geometry);
        self.blocked & mask == 0
            && selected
                .iter()
                .all(|&other| geometry_mask(other) & mask == 0)
            && selected
                .iter()
                .filter(|other| other.item_index == geometry.item_index)
                .count()
                < self.input.counts[geometry.item_index] as usize
    }

    fn search(&mut self, selected: &mut Vec<Geometry>) -> Decision {
        if self.time_exhausted() || self.stats.search_nodes >= self.config.max_search_nodes {
            return Decision::new(
                Status::Interrupted,
                "search node or elapsed-time budget exhausted",
            );
        }
        self.stats.search_nodes += 1;
        // Already selected identical rectangles satisfy every attached fragment.
        // Choose one alternative from each still-unsatisfied observation.
        let next = self
            .observations
            .iter()
            .filter(|candidates| {
                !candidates
                    .iter()
                    .any(|candidate| selected.contains(candidate))
            })
            .map(|candidates| {
                candidates
                    .iter()
                    .copied()
                    .filter(|&g| self.compatible(g, selected))
                    .collect::<Vec<_>>()
            })
            .min_by_key(Vec::len);
        let Some(alternatives) = next else {
            return self.solve_selected(selected);
        };
        let mut unresolved = None;
        for geometry in alternatives {
            selected.push(geometry);
            let decision = self.search(selected);
            selected.pop();
            match decision.status {
                Status::Feasible => return decision,
                Status::Interrupted => return decision,
                Status::InsufficientEvidence | Status::InvalidInput => unresolved = Some(decision),
                Status::Infeasible => {}
            }
        }
        unresolved.unwrap_or_else(|| {
            Decision::new(
                Status::Infeasible,
                "all observation alternatives have exact conflicts or no full layout",
            )
        })
    }

    fn solve_selected(&mut self, selected: &[Geometry]) -> Decision {
        let mut key = selected.to_vec();
        key.sort_unstable();
        key.dedup();
        if let Some(decision) = self.solver_cache.get(&key) {
            self.stats.solver_cache_hits += 1;
            return decision.clone();
        }
        if self.time_exhausted() || self.stats.solver_calls >= self.config.max_solver_calls {
            return Decision::new(
                Status::Interrupted,
                "solver call or elapsed-time budget exhausted",
            );
        }
        let mut cells = vec!["empty".to_owned(); NATIVE_WIDTH * NATIVE_HEIGHT];
        for y in 0..self.input.height as usize {
            for x in 0..self.input.width as usize {
                cells[y * NATIVE_WIDTH + x] = "unknown".into();
            }
        }
        for &cell in &self.input.empty_cells {
            cells[(cell / self.input.width as usize) * NATIVE_WIDTH
                + cell % self.input.width as usize] = "empty".into();
        }
        for &cell in &self.input.completed_cells {
            cells[(cell / self.input.width as usize) * NATIVE_WIDTH
                + cell % self.input.width as usize] = "completed".into();
        }
        let candidate_constraints = key
            .iter()
            .map(|g| {
                // Synthetic typed anchors encode fixed predictions only inside this
                // solver request. They never become observed cells in the report.
                let anchor = g.y as usize * NATIVE_WIDTH + g.x as usize;
                cells[anchor] = format!("item{}", g.item_index);
                PlacementConstraint {
                    anchor,
                    item_index: g.item_index,
                    placements: vec![GridPlacement {
                        x: g.x as usize,
                        y: g.y as usize,
                        width: g.w as usize,
                        height: g.h as usize,
                    }],
                }
            })
            .collect();
        let items = self
            .input
            .shapes
            .iter()
            .zip(&self.input.counts)
            .map(|(shape, &count)| SnapshotItem {
                width: shape[0] as i32,
                height: shape[1] as i32,
                remaining_count: count as i32,
            })
            .collect();
        self.stats.solver_calls += 1;
        let started = Instant::now();
        let result = (self.solver)(SnapshotInput {
            items,
            cells,
            candidate_constraints,
        });
        self.stats.solver_elapsed_ms += started.elapsed().as_secs_f64() * 1_000.0;
        let decision = classify_native(&result);
        match decision.status {
            Status::Feasible if result.precision == Some("sampled") => {
                self.stats.sampled_witnesses += 1
            }
            Status::Feasible => self.stats.exact_witnesses += 1,
            Status::Infeasible => self.stats.exact_no_solutions += 1,
            _ => {}
        }
        self.solver_cache.insert(key, decision.clone());
        decision
    }
}

fn classify_native(result: &SnapshotResult) -> Decision {
    // Audited native contract: counter::sample_top_left_counts computes the full
    // checked-u64 DP count BEFORE reconstruction/sampling. snapshot::solve_inner
    // emits this exact error only for all_count == 0. Its failure wrapper drops
    // precision metadata, so recognize only this specific zero-count contract.
    if result.error == NATIVE_ZERO_ERROR
        && result.samples == 0
        && result.total_patterns == "0"
        && result.precision.is_none()
    {
        return Decision::new(
            Status::Infeasible,
            "native exhaustive DP found no full layout",
        );
    }
    if !result.error.is_empty() {
        return Decision::new(
            Status::InsufficientEvidence,
            format!(
                "native solver did not establish feasibility: {}",
                result.error
            ),
        );
    }
    if matches!(result.precision, Some("exact" | "sampled"))
        && result.samples > 0
        && result
            .total_patterns
            .parse::<u64>()
            .is_ok_and(|count| count > 0)
    {
        return Decision::new(
            Status::Feasible,
            if result.precision == Some("sampled") {
                "sampled full-layout witness; no uniqueness claim"
            } else {
                "exact full-layout witness; no uniqueness claim"
            },
        );
    }
    Decision::new(
        Status::InsufficientEvidence,
        "native result has no full-layout witness or exact zero-count proof",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(item_index: usize, x: u32, y: u32, w: u32, h: u32) -> Geometry {
        Geometry {
            item_index,
            x,
            y,
            w,
            h,
        }
    }

    fn board(
        width: u32,
        height: u32,
        shapes: [[u32; 2]; 3],
        counts: [u32; 3],
        observations: Vec<Vec<Geometry>>,
    ) -> ConstraintInput {
        ConstraintInput {
            width,
            height,
            shapes: shapes.to_vec(),
            counts: counts.to_vec(),
            empty_cells: vec![],
            completed_cells: vec![],
            observations: observations
                .into_iter()
                .enumerate()
                .map(|(index, candidates)| Observation {
                    id: format!("o{index}"),
                    candidates,
                })
                .collect(),
        }
    }

    fn config() -> Config {
        Config {
            max_search_nodes: 100_000,
            max_solver_calls: 1_000,
            max_elapsed_ms: 60_000,
        }
    }

    fn statuses(report: &ConstraintReport) -> Vec<Status> {
        report
            .observations
            .iter()
            .flat_map(|o| o.candidates.iter().map(|c| c.status))
            .collect()
    }

    // Independent complete-layout oracle. It enumerates whole item inventories,
    // not observation selections or synthetic anchors; blocked/overlap cells are
    // checked directly on the effective board rather than the adapter's masks.
    fn oracle(input: &ConstraintInput) -> HashSet<Geometry> {
        let placements: Vec<Vec<Geometry>> = input
            .shapes
            .iter()
            .enumerate()
            .map(|(item, &[w, h])| {
                let mut result = Vec::new();
                for [w, h] in if w == h {
                    vec![[w, h]]
                } else {
                    vec![[w, h], [h, w]]
                } {
                    if w > input.width || h > input.height {
                        continue;
                    }
                    for y in 0..=input.height - h {
                        for x in 0..=input.width - w {
                            result.push(g(item, x, y, w, h));
                        }
                    }
                }
                result
            })
            .collect();
        fn visit(
            input: &ConstraintInput,
            placements: &[Vec<Geometry>],
            group: usize,
            remaining: u32,
            start: usize,
            occupied: &mut [bool],
            chosen: &mut Vec<Geometry>,
            feasible: &mut HashSet<Geometry>,
        ) {
            if group == input.counts.len() {
                if input.observations.iter().all(|o| {
                    o.candidates
                        .iter()
                        .any(|candidate| chosen.contains(candidate))
                }) {
                    feasible.extend(
                        input
                            .observations
                            .iter()
                            .flat_map(|o| &o.candidates)
                            .filter(|candidate| chosen.contains(candidate))
                            .copied(),
                    );
                }
                return;
            }
            if remaining == 0 {
                visit(
                    input,
                    placements,
                    group + 1,
                    input.counts.get(group + 1).copied().unwrap_or(0),
                    0,
                    occupied,
                    chosen,
                    feasible,
                );
                return;
            }
            for index in start..placements[group].len() {
                let rectangle = placements[group][index];
                let cells: Vec<_> = (rectangle.y..rectangle.y + rectangle.h)
                    .flat_map(|y| {
                        (rectangle.x..rectangle.x + rectangle.w)
                            .map(move |x| (y * input.width + x) as usize)
                    })
                    .collect();
                if cells.iter().any(|&cell| {
                    occupied[cell]
                        || input.empty_cells.contains(&cell)
                        || input.completed_cells.contains(&cell)
                }) {
                    continue;
                }
                for &cell in &cells {
                    occupied[cell] = true;
                }
                chosen.push(rectangle);
                visit(
                    input,
                    placements,
                    group,
                    remaining - 1,
                    index + 1,
                    occupied,
                    chosen,
                    feasible,
                );
                chosen.pop();
                for &cell in &cells {
                    occupied[cell] = false;
                }
            }
        }
        let mut feasible = HashSet::new();
        visit(
            input,
            &placements,
            0,
            input.counts[0],
            0,
            &mut vec![false; (input.width * input.height) as usize],
            &mut vec![],
            &mut feasible,
        );
        feasible
    }

    fn assert_matches_oracle(input: &ConstraintInput) -> ConstraintReport {
        let feasible = oracle(input);
        let report = check_candidates(input, &config());
        assert!(report.complete, "{report:?}");
        for observation in &report.observations {
            for candidate in &observation.candidates {
                assert_eq!(
                    candidate.status,
                    if feasible.contains(&candidate.geometry) {
                        Status::Feasible
                    } else {
                        Status::Infeasible
                    },
                    "{candidate:?}"
                );
            }
        }
        assert_eq!(
            report.status,
            if feasible.is_empty() {
                Status::Infeasible
            } else {
                Status::Feasible
            }
        );
        report
    }

    #[test]
    fn alternatives_are_or_and_observations_are_and() {
        let a = g(0, 0, 0, 1, 1);
        let b = g(1, 0, 0, 1, 1);
        let c = g(0, 1, 0, 1, 1);
        let or = board(2, 1, [[1, 1]; 3], [1, 0, 0], vec![vec![a, b]]);
        assert_eq!(
            statuses(&assert_matches_oracle(&or)),
            vec![Status::Feasible, Status::Infeasible]
        );
        let and = board(2, 1, [[1, 1]; 3], [1, 0, 0], vec![vec![a, b], vec![c]]);
        assert!(statuses(&assert_matches_oracle(&and))
            .iter()
            .all(|&status| status == Status::Infeasible));
    }

    #[test]
    fn duplicate_fragments_and_visual_angles_consume_one_item() {
        let a = g(0, 0, 0, 2, 1);
        let input = board(
            3,
            2,
            [[2, 1], [1, 1], [1, 1]],
            [1, 0, 0],
            vec![vec![a, a], vec![a]],
        );
        let report = assert_matches_oracle(&input);
        assert_eq!(report.observations[0].candidates.len(), 1);
        assert_eq!(report.stats.solver_calls, 1);
        assert_eq!(report.stats.geometry_cache_hits, 1);
    }

    #[test]
    fn distinct_overlapping_rectangles_cannot_share_cells() {
        for input in [
            board(
                3,
                1,
                [[2, 1], [1, 1], [1, 1]],
                [2, 0, 0],
                vec![vec![g(0, 0, 0, 2, 1)], vec![g(0, 1, 0, 2, 1)]],
            ),
            board(
                2,
                1,
                [[1, 1]; 3],
                [1, 1, 0],
                vec![vec![g(0, 0, 0, 1, 1)], vec![g(1, 0, 0, 1, 1)]],
            ),
        ] {
            assert_matches_oracle(&input);
        }
    }

    #[test]
    fn full_remaining_layout_is_checked_after_selected_rectangles_fit() {
        let input = board(
            3,
            1,
            [[2, 1], [2, 1], [1, 1]],
            [1, 1, 0],
            vec![vec![g(0, 0, 0, 2, 1)]],
        );
        let report = assert_matches_oracle(&input);
        assert_eq!(report.stats.exact_no_solutions, 1);
    }

    #[test]
    fn blocked_empty_and_completed_cells_do_not_change_observations_or_counts() {
        let mut input = board(
            3,
            2,
            [[2, 1], [1, 1], [1, 1]],
            [1, 1, 0],
            vec![vec![g(0, 0, 0, 2, 1), g(0, 0, 1, 2, 1), g(0, 2, 0, 1, 2)]],
        );
        input.empty_cells = vec![1];
        input.completed_cells = vec![3];
        let before = serde_json::to_value(&input).unwrap();
        assert_matches_oracle(&input);
        assert_eq!(serde_json::to_value(&input).unwrap(), before);
    }

    #[test]
    fn mixed_type_rotation_and_fragment_sets_match_complete_layout_oracle() {
        for counts in [[1, 1, 0], [2, 1, 0], [1, 1, 1]] {
            for blocked in [None, Some(1), Some(4)] {
                let mut input = board(
                    3,
                    2,
                    [[2, 1], [1, 1], [1, 1]],
                    counts,
                    vec![
                        vec![g(0, 0, 0, 2, 1), g(0, 0, 0, 1, 2), g(1, 0, 0, 1, 1)],
                        vec![g(0, 0, 0, 2, 1), g(0, 1, 1, 2, 1), g(2, 2, 1, 1, 1)],
                    ],
                );
                input.completed_cells = blocked.into_iter().collect();
                assert_matches_oracle(&input);
            }
        }
    }

    #[test]
    fn bounds_shapes_counts_ids_and_conflicting_cell_states_are_invalid() {
        let base = board(2, 2, [[1, 1]; 3], [1, 0, 0], vec![vec![g(0, 0, 0, 1, 1)]]);
        let mut variants = Vec::new();
        for rectangle in [
            g(3, 0, 0, 1, 1),
            g(0, 0, 0, 2, 1),
            g(0, 2, 0, 1, 1),
            g(0, 0, 2, 1, 1),
            g(0, u32::MAX, 0, 1, 1),
        ] {
            let mut input = base.clone();
            input.observations[0].candidates[0] = rectangle;
            variants.push(input);
        }
        let mut input = base.clone();
        input.width = 10;
        variants.push(input);
        let mut input = base.clone();
        input.height = 0;
        variants.push(input);
        let mut input = base.clone();
        input.shapes[0] = [0, 1];
        variants.push(input);
        let mut input = base.clone();
        input.counts[0] = 8;
        variants.push(input);
        let mut input = base.clone();
        input.counts.pop();
        variants.push(input);
        let mut input = base.clone();
        input.empty_cells.push(4);
        variants.push(input);
        let mut input = base.clone();
        input.empty_cells.push(0);
        input.completed_cells.push(0);
        variants.push(input);
        let mut input = base.clone();
        input.observations.push(input.observations[0].clone());
        variants.push(input);
        for input in variants {
            let report = check_candidates(&input, &config());
            assert_eq!(report.status, Status::InvalidInput, "{report:?}");
            assert!(!report.complete);
            assert_eq!(report.stats.solver_calls, 0);
        }
        let full = board(
            9,
            5,
            [[5, 9], [1, 1], [1, 1]],
            [1, 0, 0],
            vec![vec![g(0, 0, 0, 9, 5)]],
        );
        assert_eq!(check_candidates(&full, &config()).status, Status::Feasible);
    }

    #[test]
    fn budgets_interrupt_without_removing_unchecked_candidates() {
        let input = board(
            2,
            1,
            [[1, 1]; 3],
            [1, 0, 0],
            vec![vec![g(0, 0, 0, 1, 1), g(0, 1, 0, 1, 1)]],
        );
        for budget in [
            Config {
                max_solver_calls: 0,
                ..config()
            },
            Config {
                max_search_nodes: 0,
                ..config()
            },
            Config {
                max_elapsed_ms: 0,
                ..config()
            },
        ] {
            let report = check_candidates(&input, &budget);
            assert_eq!(report.status, Status::Interrupted);
            assert!(!report.complete);
            assert_eq!(statuses(&report), vec![Status::Interrupted; 2]);
            assert_eq!(report.stats.solver_calls, 0);
        }
        let report = check_candidates(
            &input,
            &Config {
                max_solver_calls: 1,
                ..config()
            },
        );
        assert_eq!(report.status, Status::Feasible);
        assert!(!report.complete);
        assert_eq!(
            statuses(&report),
            vec![Status::Feasible, Status::Interrupted]
        );
    }

    #[test]
    fn empty_candidate_lists_are_missing_evidence() {
        let input = board(
            2,
            1,
            [[1, 1]; 3],
            [1, 0, 0],
            vec![vec![g(0, 0, 0, 1, 1)], vec![]],
        );
        let report = check_candidates(&input, &config());
        assert_eq!(report.status, Status::InsufficientEvidence);
        assert!(!report.complete);
        assert_eq!(statuses(&report), vec![Status::InsufficientEvidence]);
        assert_eq!(report.stats.solver_calls, 0);
    }

    fn result(
        precision: Option<&'static str>,
        total: &str,
        samples: u64,
        error: &str,
    ) -> SnapshotResult {
        SnapshotResult {
            precision,
            total_patterns: total.into(),
            samples,
            error: error.into(),
            probs: vec![vec![1.0; 45]; 8],
            inferred_placements: vec![],
        }
    }

    #[test]
    fn sampled_witness_is_feasible_but_sample_absence_and_overflow_are_unknown() {
        let input = board(
            2,
            1,
            [[1, 1]; 3],
            [1, 0, 0],
            vec![vec![g(0, 0, 0, 1, 1), g(0, 1, 0, 1, 1)]],
        );
        let report = check_with_solver(&input, &config(), |_| {
            result(Some("sampled"), "100001", 100000, "")
        });
        assert!(report.complete);
        assert_eq!(statuses(&report), vec![Status::Feasible; 2]);
        assert_eq!(report.stats.sampled_witnesses, 2);
        // Even fake 100% probabilities never establish geometric uniqueness.
        assert_eq!(report.observations[0].candidates.len(), 2);
        for unknown in [
            result(Some("sampled"), "0", 0, ""),
            result(None, "0", 0, "configuration_count_overflow"),
            result(Some("sampled"), "0", 0, NATIVE_ZERO_ERROR),
            result(None, "0", 0, "no_valid_configuration: sampled absence"),
        ] {
            let report = check_with_solver(&input, &config(), |_| unknown.clone());
            assert_eq!(report.status, Status::InsufficientEvidence);
            assert!(!report.complete);
            assert_eq!(statuses(&report), vec![Status::InsufficientEvidence; 2]);
        }
    }

    #[test]
    fn an_unknown_type_branch_does_not_make_a_candidate_infeasible() {
        let input = board(
            3,
            1,
            [[1, 1]; 3],
            [1, 1, 0],
            vec![
                vec![g(0, 0, 0, 1, 1)],
                vec![g(1, 1, 0, 1, 1), g(1, 2, 0, 1, 1)],
            ],
        );
        let report = check_with_solver(&input, &config(), |snapshot| {
            if snapshot
                .candidate_constraints
                .iter()
                .any(|constraint| constraint.anchor == 1)
            {
                result(None, "0", 0, "configuration_count_overflow")
            } else {
                result(None, "0", 0, NATIVE_ZERO_ERROR)
            }
        });
        assert_eq!(
            report.observations[0].candidates[0].status,
            Status::InsufficientEvidence
        );
        assert!(!report.complete);
        assert!(report.stats.solver_cache_hits > 0);
    }

    #[test]
    fn serde_config_defaults_and_report_status_are_explicit() {
        let config: Config = serde_json::from_str("{\"max_solver_calls\":0}").unwrap();
        assert_eq!(config.max_solver_calls, 0);
        assert_eq!(config.max_search_nodes, Config::default().max_search_nodes);
        let input = board(2, 1, [[1, 1]; 3], [1, 0, 0], vec![vec![g(0, 0, 0, 1, 1)]]);
        let json = serde_json::to_value(check_candidates(&input, &config)).unwrap();
        assert_eq!(json["status"], "interrupted");
        assert_eq!(json["complete"], false);
        assert_eq!(json["observations"][0]["observation_id"], "o0");
        assert_eq!(
            json["observations"][0]["candidates"][0]["geometry"]["item_index"],
            0
        );
    }
}
