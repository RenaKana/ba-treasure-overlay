//! Per-round partial-item recognition. Observed cells stay immutable; confirmed
//! footprints are a separate projection for the probability calculation.

use super::{Analysis, COLS, ROWS};
use image::RgbaImage;
use serde::{Deserialize, Serialize};

#[cfg(feature = "partial-recognition")]
#[path = "../examples/partial_recognition/competition.rs"]
mod competition;
#[cfg(feature = "partial-recognition")]
#[path = "../examples/partial_recognition/constraints.rs"]
mod constraints;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Placement {
    pub item_index: usize,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Recognition {
    pub placements: Vec<Placement>,
    /// Every actual partial observation has one supported type and rectangle.
    /// Visual angles may differ within that same physical footprint.
    pub complete: bool,
    pub message: String,
}

impl Recognition {
    fn pending(message: impl Into<String>) -> Self {
        Self {
            placements: Vec::new(),
            complete: false,
            message: message.into(),
        }
    }
}

#[derive(Default)]
pub struct Recognizer {
    #[cfg(feature = "partial-recognition")]
    session: super::experiment::Session,
    #[cfg(feature = "partial-recognition")]
    memo: Option<(InputKey, Recognition)>,
    #[cfg(feature = "partial-recognition")]
    tracking: Option<TrackingLedger>,
    #[cfg(all(feature = "partial-recognition", test))]
    cache_hits: u64,
    #[cfg(all(feature = "partial-recognition", test))]
    matching_budget: Option<std::time::Duration>,
}

#[cfg(feature = "partial-recognition")]
#[derive(PartialEq)]
struct InputKey {
    dimensions: (u32, u32),
    content: [u64; 4],
    board: [u64; 4],
    cells: Vec<String>,
    shapes: Vec<[u32; 2]>,
    counts: [Option<u32>; 3],
    finish: [bool; 3],
    card_fingerprints: [Option<String>; 3],
    reference_ready: [bool; 3],
    pixels: (RgbaImage, [RgbaImage; 3]),
}

#[cfg(feature = "partial-recognition")]
struct TrackingLedger {
    dimensions: (u32, u32),
    content: [u64; 4],
    board: [u64; 4],
    shapes: Vec<[u32; 2]>,
    card_fingerprints: [Option<String>; 3],
    finish: [bool; 3],
    counts: [u32; 3],
    cells: Vec<String>,
    placements: Vec<Placement>,
}

#[cfg(feature = "partial-recognition")]
enum TrackingUpdate {
    Valid(Vec<Placement>),
    Pending(&'static str),
    Invalid,
}

#[cfg(feature = "partial-recognition")]
impl Placement {
    fn cells(&self) -> impl Iterator<Item = usize> + '_ {
        (self.y..self.y + self.height)
            .flat_map(move |y| (self.x..self.x + self.width).map(move |x| y * COLS + x))
    }
}

#[cfg(feature = "partial-recognition")]
impl TrackingLedger {
    fn new(key: &InputKey, counts: [u32; 3], placements: Vec<Placement>) -> Self {
        Self {
            dimensions: key.dimensions,
            content: key.content,
            board: key.board,
            shapes: key.shapes.clone(),
            card_fingerprints: key.card_fingerprints.clone(),
            finish: key.finish,
            counts,
            cells: key.cells.clone(),
            placements,
        }
    }

    fn reconcile(&self, key: &InputKey, counts: [u32; 3]) -> TrackingUpdate {
        if self.dimensions != key.dimensions
            || self.content != key.content
            || self.board != key.board
        {
            return TrackingUpdate::Invalid;
        }
        // A missing read suspends tracking; only positive contradictory evidence
        // retires the confirmation. In particular, an animation is not a new card.
        if key.shapes.len() != 3 || key.shapes.iter().any(|shape| shape.contains(&0)) {
            return TrackingUpdate::Pending("物品尺寸未识别，等待画面稳定");
        }
        if self.shapes != key.shapes {
            return TrackingUpdate::Invalid;
        }
        for item in 0..3 {
            if (!key.reference_ready[item] || key.card_fingerprints[item].is_none())
                && !(key.finish[item] && counts[item] == 0)
            {
                return TrackingUpdate::Pending("物品图案参考暂时缺失，等待画面稳定");
            }
            if let Some(current) = &key.card_fingerprints[item] {
                if self.card_fingerprints[item].as_ref() != Some(current) {
                    return TrackingUpdate::Invalid;
                }
            }
            if key.finish[item] && counts[item] != 0 {
                return TrackingUpdate::Pending("完成状态与剩余件数未同步，暂不确认占格");
            }
            if self.finish[item] && !key.finish[item] {
                return TrackingUpdate::Invalid;
            }
        }
        let mut active = Vec::new();
        let mut retired = [0u32; 3];
        for placement in &self.placements {
            if placement.cells().all(|cell| key.cells[cell] == "completed") {
                retired[placement.item_index] += 1;
                continue;
            }
            if placement.cells().any(|cell| {
                key.cells[cell] == "empty"
                    || item_restriction(&key.cells[cell])
                        .is_some_and(|item| item != placement.item_index)
                    || (is_observation(&self.cells[cell]) && key.cells[cell] == "unknown")
            }) {
                return TrackingUpdate::Invalid;
            }
            if placement.cells().any(|cell| key.cells[cell] == "completed") {
                return TrackingUpdate::Pending("已确认物品的完成格尚未同步，等待画面稳定");
            }
            active.push(placement.clone());
        }
        for item in 0..3 {
            let Some(expected) = self.counts[item].checked_sub(retired[item]) else {
                return TrackingUpdate::Invalid;
            };
            if counts[item] != expected {
                if retired[item] > 0 && counts[item] > expected && counts[item] <= self.counts[item]
                {
                    return TrackingUpdate::Pending("已完成物品与剩余件数未同步，暂不确认占格");
                }
                return TrackingUpdate::Invalid;
            }
        }
        TrackingUpdate::Valid(active)
    }
}

impl Recognizer {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Explicit corrections invalidate conclusions, but retain this round's
    /// artwork references, including references currently covered by Finish.
    pub(crate) fn invalidate_confirmations(&mut self) {
        self.clear_memo();
        #[cfg(feature = "partial-recognition")]
        {
            self.tracking = None;
        }
    }

    #[cfg(all(feature = "partial-recognition", test))]
    pub(crate) fn set_test_matching_budget(&mut self, budget: std::time::Duration) {
        self.matching_budget = Some(budget);
    }

    fn clear_memo(&mut self) {
        #[cfg(feature = "partial-recognition")]
        {
            self.memo = None;
        }
    }

    #[cfg(all(feature = "partial-recognition", test))]
    pub(crate) fn cache_hits(&self) -> u64 {
        self.cache_hits
    }

    #[cfg(all(feature = "partial-recognition", test))]
    pub(crate) fn with_test_matching_budget(budget: std::time::Duration) -> Self {
        Self {
            matching_budget: Some(budget),
            ..Self::default()
        }
    }

    pub fn resolve(
        &mut self,
        frame: &RgbaImage,
        analysis: &Analysis,
        content: Option<[f64; 4]>,
        counts: [Option<u32>; 3],
    ) -> Recognition {
        let Some(content) = content.filter(|rect| super::content_rect(frame, *rect).is_some())
        else {
            self.clear_memo();
            return Recognition::pending("局部识别缺少有效的游戏画面范围");
        };
        if !valid_analysis(analysis) {
            // Failed frames do not replace useful references from this round.
            self.clear_memo();
            return Recognition::pending("当前棋盘观察无效，局部识别等待画面稳定");
        }

        #[cfg(feature = "partial-recognition")]
        {
            self.resolve_enabled(frame, analysis, content, counts)
        }
        #[cfg(not(feature = "partial-recognition"))]
        {
            let _ = (content, counts);
            if analysis.cells.iter().any(|cell| is_observation(cell)) {
                Recognition::pending("此构建未启用局部识别")
            } else {
                Recognition {
                    placements: Vec::new(),
                    complete: true,
                    message: String::new(),
                }
            }
        }
    }
}

fn is_observation(cell: &str) -> bool {
    matches!(cell, "uncertain" | "item0" | "item1" | "item2")
}

fn item_restriction(cell: &str) -> Option<usize> {
    match cell {
        "item0" => Some(0),
        "item1" => Some(1),
        "item2" => Some(2),
        _ => None,
    }
}

fn valid_analysis(analysis: &Analysis) -> bool {
    analysis.present
        && analysis.cells.len() == COLS * ROWS
        && analysis.cells.iter().all(|cell| {
            matches!(
                cell.as_str(),
                "unknown" | "empty" | "completed" | "uncertain" | "item0" | "item1" | "item2"
            )
        })
        && analysis.board.is_some_and(|board| {
            board.iter().all(|value| value.is_finite())
                && board[0] >= 0.0
                && board[1] >= 0.0
                && board[2] > 0.0
                && board[3] > 0.0
                && board[0] + board[2] <= 1.0 + f64::EPSILON
                && board[1] + board[3] <= 1.0 + f64::EPSILON
        })
}

#[cfg(feature = "partial-recognition")]
#[derive(Deserialize)]
struct Config {
    matching: super::experiment::Config,
    constraints: constraints::Config,
    scoring: Scoring,
}

#[cfg(feature = "partial-recognition")]
#[derive(Deserialize)]
struct Scoring {
    visible_foreground_max_error: f64,
    geometry_competition: competition::Config,
}

#[cfg(feature = "partial-recognition")]
fn config() -> Result<&'static Config, &'static str> {
    static CONFIG: std::sync::OnceLock<Result<Config, String>> = std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let config: Config =
                serde_json::from_str(include_str!("../examples/partial_recognition/config.json"))
                    .map_err(|error| error.to_string())?;
            config.matching.validate()?;
            config.scoring.geometry_competition.validate()?;
            if !config.scoring.visible_foreground_max_error.is_finite()
                || config.scoring.visible_foreground_max_error < 0.0
            {
                return Err("invalid final foreground threshold".into());
            }
            Ok(config)
        })
        .as_ref()
        .map_err(String::as_str)
}

// Technical execution budgets only. Exhausting either budget keeps recognition
// pending and never truncates candidate/observation counts or the search grid.
#[cfg(feature = "partial-recognition")]
const MATCHING_BUDGET_MS: u64 = 8_000;
#[cfg(feature = "partial-recognition")]
const GLOBAL_BUDGET_MS: u64 = 3_000;

#[cfg(feature = "partial-recognition")]
impl Recognizer {
    fn resolve_enabled(
        &mut self,
        frame: &RgbaImage,
        analysis: &Analysis,
        content: [f64; 4],
        counts: [Option<u32>; 3],
    ) -> Recognition {
        let config = match config() {
            Ok(config) => config,
            Err(error) => {
                self.clear_memo();
                return Recognition::pending(format!("局部识别配置无效：{error}"));
            }
        };
        let board = analysis.board.unwrap();
        let Some(pixels) = super::experiment::live_input_pixels(frame, board, content) else {
            self.clear_memo();
            return Recognition::pending("局部识别缺少有效的游戏画面范围");
        };
        let key = InputKey {
            dimensions: frame.dimensions(),
            content: content.map(f64::to_bits),
            board: board.map(f64::to_bits),
            cells: analysis.cells.clone(),
            shapes: analysis.shapes.clone(),
            counts,
            finish: analysis.finish,
            card_fingerprints: analysis.card_fingerprints.clone(),
            reference_ready: analysis.reference_ready,
            pixels,
        };
        if let Some((previous, recognition)) = &self.memo {
            if previous == &key {
                #[cfg(test)]
                {
                    self.cache_hits = self.cache_hits.saturating_add(1);
                }
                return recognition.clone();
            }
        }
        // A different frame can change the learned references even if matching
        // fails. It must invalidate the old result before touching that state.
        self.clear_memo();
        let current_counts = match counts {
            [Some(a), Some(b), Some(c)] => Some([a, b, c]),
            _ => None,
        };
        if let Some(ledger) = &self.tracking {
            let Some(current_counts) = current_counts else {
                return Recognition::pending("剩余件数未识别，局部占格暂不确认");
            };
            match ledger.reconcile(&key, current_counts) {
                TrackingUpdate::Pending(message) => return Recognition::pending(message),
                TrackingUpdate::Invalid => self.tracking = None,
                TrackingUpdate::Valid(placements) => {
                    match tracked_feasibility(analysis, current_counts, &placements) {
                        constraints::Status::Feasible => {
                            // Commit the genuine new observations, including any
                            // newly exposed cells inside a previously confirmed item.
                            self.tracking = (!placements.is_empty()).then(|| {
                                TrackingLedger::new(&key, current_counts, placements.clone())
                            });
                            let mut remaining = current_counts;
                            let mut unresolved = analysis.clone();
                            for placement in &placements {
                                remaining[placement.item_index] -= 1;
                                for cell in placement.cells() {
                                    unresolved.cells[cell] = "completed".into();
                                }
                            }
                            let (mut result, conflict) =
                                if unresolved.cells.iter().any(|cell| is_observation(cell)) {
                                    self.match_frame(
                                        frame,
                                        &unresolved,
                                        content,
                                        remaining.map(Some),
                                        config,
                                    )
                                } else {
                                    (
                                        Recognition {
                                            placements: Vec::new(),
                                            complete: true,
                                            message: String::new(),
                                        },
                                        false,
                                    )
                                };
                            if !conflict {
                                if result.complete {
                                    result.placements.extend(placements);
                                    result.placements.sort_by_key(|p| {
                                        (p.item_index, p.x, p.y, p.width, p.height)
                                    });
                                    result.message = format!(
                                        "局部图案已确认 {} 件物品占格",
                                        result.placements.len()
                                    );
                                    self.remember_success(key, current_counts, &result);
                                }
                                return result;
                            }
                            // A complete geometric contradiction is grounds to
                            // release old tracks and re-evaluate the current frame.
                            self.tracking = None;
                        }
                        constraints::Status::Infeasible => self.tracking = None,
                        _ => return Recognition::pending("已确认占格的全局检查未完成，暂不推荐"),
                    }
                }
            }
        }
        let (recognition, _) = self.match_frame(frame, analysis, content, counts, config);
        if recognition.complete {
            if let Some(counts) = current_counts {
                self.remember_success(key, counts, &recognition);
            } else {
                self.memo = Some((key, recognition.clone()));
            }
        }
        recognition
    }

    fn remember_success(&mut self, key: InputKey, counts: [u32; 3], recognition: &Recognition) {
        self.tracking = (!recognition.placements.is_empty())
            .then(|| TrackingLedger::new(&key, counts, recognition.placements.clone()));
        self.memo = Some((key, recognition.clone()));
    }

    fn match_frame(
        &mut self,
        frame: &RgbaImage,
        analysis: &Analysis,
        content: [f64; 4],
        counts: [Option<u32>; 3],
        config: &Config,
    ) -> (Recognition, bool) {
        let mut matching_input = analysis.clone();
        for cell in &mut matching_input.cells {
            if item_restriction(cell).is_some() {
                *cell = "uncertain".into();
            }
        }
        // Run even without fragments: the card references learned now must be
        // available when a later frame puts Finish over this round's artwork.
        let matching_budget = std::time::Duration::from_millis(MATCHING_BUDGET_MS);
        #[cfg(test)]
        let matching_budget = self.matching_budget.unwrap_or(matching_budget);
        let matched = match self.session.run_live(
            frame,
            &matching_input,
            content,
            "live",
            &config.matching,
            Some(matching_budget),
        ) {
            Ok(output) => output,
            Err(error) => {
                return (
                    Recognition::pending(format!("局部识别等待：{error}")),
                    false,
                )
            }
        };
        let mut conflict = false;
        let recognition = decide_matches_internal(analysis, &matched, counts, &mut conflict);
        (recognition, conflict)
    }
}

#[cfg(feature = "partial-recognition")]
fn tracked_feasibility(
    analysis: &Analysis,
    counts: [u32; 3],
    placements: &[Placement],
) -> constraints::Status {
    use wasm_solver::snapshot::{
        check_snapshot_feasibility_native, GridPlacement, PlacementConstraint, SnapshotInput,
        SnapshotItem,
    };
    let items: Option<Vec<_>> = analysis
        .shapes
        .iter()
        .zip(counts)
        .map(|(shape, count)| {
            Some(SnapshotItem {
                width: i32::try_from(shape[0]).ok()?,
                height: i32::try_from(shape[1]).ok()?,
                remaining_count: i32::try_from(count).ok()?,
            })
        })
        .collect();
    let Some(items) = items else {
        return constraints::Status::InvalidInput;
    };
    let mut cells: Vec<_> = analysis
        .cells
        .iter()
        .map(|cell| {
            if cell == "uncertain" {
                "unknown".into()
            } else {
                cell.clone()
            }
        })
        .collect();
    let candidate_constraints = placements
        .iter()
        .map(|placement| {
            let anchor = placement.y * COLS + placement.x;
            cells[anchor] = format!("item{}", placement.item_index);
            PlacementConstraint {
                anchor,
                item_index: placement.item_index,
                placements: vec![GridPlacement {
                    x: placement.x,
                    y: placement.y,
                    width: placement.width,
                    height: placement.height,
                }],
            }
        })
        .collect();
    let result = check_snapshot_feasibility_native(SnapshotInput {
        items,
        cells,
        candidate_constraints,
    });
    if result.error.is_empty()
        && result.samples > 0
        && result
            .total_patterns
            .parse::<u64>()
            .is_ok_and(|count| count > 0)
    {
        constraints::Status::Feasible
    } else if result.error
        == "no_valid_configuration: observations, remaining counts, and candidates are inconsistent"
        && result.samples == 0
        && result.total_patterns == "0"
        && result.precision.is_none()
    {
        constraints::Status::Infeasible
    } else {
        constraints::Status::InsufficientEvidence
    }
}

/// Shared decision boundary for matched evidence. Keeping it separate allows
/// the joint geometric contract to be exercised without repeating image search.
#[cfg(feature = "partial-recognition")]
pub(crate) fn decide_matches(
    analysis: &Analysis,
    matched: &super::experiment::Output,
    counts: [Option<u32>; 3],
) -> Recognition {
    decide_matches_internal(analysis, matched, counts, &mut false)
}

#[cfg(feature = "partial-recognition")]
fn decide_matches_internal(
    analysis: &Analysis,
    matched: &super::experiment::Output,
    counts: [Option<u32>; 3],
    conflict: &mut bool,
) -> Recognition {
    let config = match config() {
        Ok(config) => config,
        Err(error) => return Recognition::pending(format!("局部识别配置无效：{error}")),
    };
    let anchors: Vec<_> = analysis
        .cells
        .iter()
        .enumerate()
        .filter_map(|(index, cell)| is_observation(cell).then_some(index))
        .collect();
    if !valid_analysis(analysis)
        || !anchors.iter().copied().eq(matched
            .observations
            .iter()
            .map(|observation| observation.anchor))
    {
        return Recognition::pending("局部匹配未覆盖当前棋盘的全部图案观察");
    }
    if anchors.is_empty() {
        return Recognition {
            placements: Vec::new(),
            complete: true,
            message: String::new(),
        };
    }
    if !matched.complete {
        return Recognition::pending("局部匹配未完成，暂不确认物品占格");
    }
    let Some(counts) = counts.into_iter().collect::<Option<Vec<_>>>() else {
        return Recognition::pending("剩余件数未识别，局部占格暂不确认");
    };
    let references_ready = matched.cards.len() == 3
        && matched.cards.iter().all(|card| {
            card.fingerprint.is_some() || (card.finished && counts.get(card.item_index) == Some(&0))
        });
    if !references_ready {
        return Recognition::pending("当前轮次的物品图案参考不足，局部占格暂不确认");
    }
    if matched.observations.iter().any(|o| !o.sufficient_evidence) {
        return Recognition::pending("局部图案像素证据不足，暂不确认物品占格");
    }
    decide(analysis, matched, counts, config, conflict)
}

#[cfg(feature = "partial-recognition")]
fn geometry(pose: &super::experiment::Pose) -> constraints::Geometry {
    constraints::Geometry {
        item_index: pose.item_index,
        x: pose.rect.x as u32,
        y: pose.rect.y as u32,
        w: pose.rect.width as u32,
        h: pose.rect.height as u32,
    }
}

#[cfg(feature = "partial-recognition")]
fn foreground_score(pose: &super::experiment::Pose) -> Option<f64> {
    let base = pose.visible_base_score()?;
    let foreground = pose.visible_evidence.as_ref()?.foreground_color_error?;
    (base.is_finite() && foreground.is_finite() && base >= 0.0 && foreground >= 0.0)
        .then(|| base.max(foreground))
}

#[cfg(feature = "partial-recognition")]
fn decide(
    analysis: &Analysis,
    matched: &super::experiment::Output,
    counts: Vec<u32>,
    config: &Config,
    conflict: &mut bool,
) -> Recognition {
    use std::{collections::BTreeSet, time::Instant};

    let accepted: Vec<_> = matched
        .candidates
        .iter()
        .map(|pose| {
            let restricted = analysis
                .cells
                .get(pose.anchor)
                .and_then(|cell| item_restriction(cell));
            foreground_score(pose).filter(|score| {
                *score <= config.scoring.visible_foreground_max_error
                    && restricted.is_none_or(|item| item == pose.item_index)
            })
        })
        .collect();
    let input = constraints::ConstraintInput {
        width: COLS as u32,
        height: ROWS as u32,
        shapes: analysis.shapes.clone(),
        counts: counts.clone(),
        empty_cells: analysis
            .cells
            .iter()
            .enumerate()
            .filter_map(|(index, cell)| (cell == "empty").then_some(index))
            .collect(),
        completed_cells: analysis
            .cells
            .iter()
            .enumerate()
            .filter_map(|(index, cell)| (cell == "completed").then_some(index))
            .collect(),
        observations: matched
            .observations
            .iter()
            .map(|observation| constraints::Observation {
                id: observation.id.clone(),
                candidates: matched
                    .candidates
                    .iter()
                    .enumerate()
                    .filter(|(index, pose)| {
                        pose.anchor == observation.anchor && accepted[*index].is_some()
                    })
                    .map(|(_, pose)| geometry(pose))
                    .collect(),
            })
            .collect(),
    };
    let global_started = Instant::now();
    let mut solver_config = config.constraints.clone();
    solver_config.max_elapsed_ms = solver_config.max_elapsed_ms.min(GLOBAL_BUDGET_MS);
    let checked = constraints::check_candidates(&input, &solver_config);
    if !checked.complete || checked.status != constraints::Status::Feasible {
        *conflict = checked.complete && checked.status == constraints::Status::Infeasible;
        return Recognition::pending("局部占格未通过完整的全局约束检查");
    }
    // Only proven infeasibility removes a visual candidate. Unknown or
    // interrupted feasibility is never turned into a uniqueness assertion.
    let observations: Vec<_> = matched
        .observations
        .iter()
        .zip(&checked.observations)
        .map(|(observation, report)| competition::Observation {
            id: observation.id.clone(),
            eligible: true,
            candidates: matched
                .candidates
                .iter()
                .enumerate()
                .filter_map(|(index, pose)| {
                    let score = accepted[index]?;
                    if pose.anchor != observation.anchor
                        || report.candidates.iter().any(|candidate| {
                            candidate.geometry == geometry(pose)
                                && candidate.status == constraints::Status::Infeasible
                        })
                    {
                        return None;
                    }
                    Some(competition::ScoredCandidate {
                        index,
                        geometry: geometry(pose),
                        score,
                    })
                })
                .collect(),
        })
        .collect();
    // Baseline and Δ5's joint proposal share one global elapsed-time budget.
    solver_config.max_elapsed_ms = solver_config
        .max_elapsed_ms
        .saturating_sub(global_started.elapsed().as_millis().min(u64::MAX as u128) as u64);
    let selected = competition::select(
        &input,
        &observations,
        &config.scoring.geometry_competition,
        &solver_config,
    );
    if selected
        .constraints
        .as_ref()
        .is_some_and(|report| !report.complete)
    {
        return Recognition::pending("局部候选的联合检查未完成，暂不确认物品占格");
    }
    let mut resolved = BTreeSet::new();
    for (observation, report) in observations.iter().zip(&selected.observations) {
        let geometries: BTreeSet<_> = observation
            .candidates
            .iter()
            .filter(|candidate| report.retained_indices.contains(&candidate.index))
            .map(|candidate| candidate.geometry)
            .collect();
        if geometries.len() != 1 {
            return Recognition::pending("局部图案仍有类型或占格歧义，暂不确认");
        }
        resolved.extend(geometries);
    }
    // Several revealed cells of the same item consume one inventory entry.
    let mut masks = Vec::new();
    let mut used = [0u32; 3];
    let mut placements = Vec::new();
    for rectangle in resolved {
        let mut mask = 0u64;
        for y in rectangle.y..rectangle.y + rectangle.h {
            for x in rectangle.x..rectangle.x + rectangle.w {
                mask |= 1u64 << (y as usize * COLS + x as usize);
            }
        }
        if masks.iter().any(|other| mask & other != 0) {
            return Recognition::pending("不同物品的局部占格发生重叠，暂不确认");
        }
        used[rectangle.item_index] += 1;
        if used[rectangle.item_index] > counts[rectangle.item_index] {
            return Recognition::pending("局部物品数量超过剩余件数，暂不确认");
        }
        masks.push(mask);
        placements.push(Placement {
            item_index: rectangle.item_index,
            x: rectangle.x as usize,
            y: rectangle.y as usize,
            width: rectangle.w as usize,
            height: rectangle.h as usize,
        });
    }
    Recognition {
        message: format!("局部图案已确认 {} 件物品占格", placements.len()),
        placements,
        complete: true,
    }
}
