//! Feature-gated child of vision_dynamic: reuse the actual extractor and transforms.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

#[path = "extraction.rs"]
mod extraction;
#[path = "tiled.rs"]
mod tiled;
#[path = "visible.rs"]
mod visible;
pub use extraction::{Config as ExtractionConfig, Diagnostic as ExtractionDiagnostic};
#[allow(unused_imports)] // Public experiment schema, consumed by external diagnostics.
pub use tiled::{TileCacheStats, TileDiagnostic, TileEvidence};
#[allow(unused_imports)] // Public experiment schema, consumed by external diagnostics.
pub use visible::{VisibleEvidence, VisibleTileEvidence};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Direct,
    Tiled,
}

fn default_tile_cache_bytes() -> usize {
    64 * 1024 * 1024
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub coarse_angle_step: usize,
    pub fills: Vec<f64>,
    pub aspect_log_tolerance: f64,
    pub max_color_error: f64,
    pub min_anchor_pixels: usize,
    pub edge_weight: f64,
    pub reverse_weight: f64,
    pub foreground_chroma: u8,
    pub foreground_dark: u8,
    pub reliable_border: usize,
    pub low_confidence_weight: f64,
    /// Optional visible-foreground color cap for low-confidence cell borders.
    #[serde(default)]
    pub low_confidence_error_cap: Option<f64>,
    pub mismatch_error: f64,
    pub min_texture_std: f64,
    pub refine: Vec<Refine>,
    /// Retry coarse basins without a seed at the first refinement's offsets.
    #[serde(default)]
    pub coarse_offset_rescue: bool,
    /// Keep the RGB search and refine an independent visible-foreground seed.
    #[serde(default)]
    pub foreground_refinement: bool,
    /// Use the same cell-border confidence for template RGB and reverse evidence.
    #[serde(default)]
    pub confidence_weighted_scoring: bool,
    /// Score every evidence-valid pose for offline nearest-match comparisons.
    #[serde(default)]
    pub threshold_free_search: bool,
    /// Technical execution budget, not a candidate-count cutoff. Zero disables it.
    pub max_evaluations: u64,
    /// Memory budget for transformed template storage, never a search cutoff.
    /// Zero disables storage; every requested transform is still computed.
    #[serde(default = "default_tile_cache_bytes")]
    pub tile_cache_bytes: usize,
    #[serde(default)]
    pub extraction: ExtractionConfig,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refine {
    pub angles: Vec<f64>,
    pub fills: Vec<f64>,
    pub offsets: Vec<f64>,
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        self.extraction.validate()?;
        if self.coarse_angle_step == 0
            || self.coarse_angle_step > 360
            || 360 % self.coarse_angle_step != 0
        {
            return Err("coarse_angle_step must divide 360".into());
        }
        if self.fills.is_empty()
            || self
                .fills
                .iter()
                .any(|f| !f.is_finite() || *f <= 0.0 || *f > 1.0)
            || self.refine.iter().any(|r| {
                r.angles.is_empty()
                    || r.fills.is_empty()
                    || r.offsets.is_empty()
                    || r.angles
                        .iter()
                        .chain(&r.fills)
                        .chain(&r.offsets)
                        .any(|n| !n.is_finite())
            })
            || [
                self.aspect_log_tolerance,
                self.max_color_error,
                self.edge_weight,
                self.reverse_weight,
                self.min_texture_std,
            ]
            .iter()
            .any(|x| !x.is_finite() || *x < 0.0)
            || self.reliable_border >= MATCH_CELL / 2
            || !self.low_confidence_weight.is_finite()
            || !(0.0..=1.0).contains(&self.low_confidence_weight)
            || self
                .low_confidence_error_cap
                .is_some_and(|cap| !cap.is_finite() || cap <= 0.0 || cap > 255.0)
            || !self.mismatch_error.is_finite()
            || self.mismatch_error < 0.0
            || self.min_anchor_pixels == 0
        {
            return Err("invalid search, evidence, or scoring configuration".into());
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Pose {
    pub observation_id: String,
    pub anchor: usize,
    pub reference_id: String,
    pub item_index: usize,
    pub rect: GridPlacement,
    pub angle: f64,
    pub fill: f64,
    pub scale: f64,
    pub offset: [f64; 2],
    pub evidence_pixels: usize,
    pub effective_evidence: f64,
    pub color_error: f64,
    pub edge_error: f64,
    pub predicted_mismatch: f64,
    pub unexplained_foreground: f64,
    pub scores: [f64; 3],
    pub accepted: [bool; 3],
    #[serde(default)]
    pub tile_template: Option<TileDiagnostic>,
    #[serde(default)]
    pub visible_evidence: Option<VisibleEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_weighted: Option<visible::WeightedCoreEvidence>,
}
impl Pose {
    pub fn visible_base_score(&self) -> Option<f64> {
        match &self.confidence_weighted {
            Some(evidence) => evidence.bidirectional_score,
            None => Some(self.scores[2]),
        }
    }
}
#[derive(Serialize)]
pub struct Observation {
    pub id: String,
    pub anchor: usize,
    /// Interior-only texture: border/grid contrast must not rescue a flat fragment.
    pub texture_std: f64,
    /// Preserve the original interior count for diagnostics.
    pub reliable_foreground_pixels: usize,
    pub border_reliable_foreground_pixels: usize,
    /// The same observed-foreground border weight used by V.
    pub effective_foreground_pixels: f64,
    pub sufficient_evidence: bool,
}
#[derive(Serialize)]
pub struct Card {
    pub item_index: usize,
    pub fingerprint: Option<String>,
    pub reference_source: Option<String>,
    pub finished: bool,
    pub mask_pixels: usize,
    pub core_pixels: usize,
    pub touches_crop: bool,
    pub extraction: Option<ExtractionDiagnostic>,
}
#[derive(Serialize)]
pub struct Output {
    pub backend: Backend,
    pub cards: Vec<Card>,
    pub observations: Vec<Observation>,
    pub candidates: Vec<Pose>,
    pub complete: bool,
    pub interruption: Option<String>,
    pub evaluations: u64,
    pub rescue_evaluations: u64,
    pub recovered_basins: u64,
    pub foreground_evaluations: u64,
    pub foreground_basins: u64,
    pub extraction_ms: f64,
    pub matching_ms: f64,
    pub tile_cache: Option<TileCacheStats>,
    /// Transformed tile diagnostic PNG generation, outside matching_ms.
    pub asset_export_ms: f64,
    pub legacy_evaluator: &'static str,
}
#[derive(Default)]
pub struct Session {
    templates: [Option<CardTemplate>; 3],
    sources: [Option<String>; 3],
    source_revisions: [u64; 3],
    tile_cache: tiled::Cache,
    extraction_diagnostics: [Option<ExtractionDiagnostic>; 3],
    live_context: Option<LiveContext>,
    live_rectangles: BTreeMap<RectangleKey, CachedRectangle>,
    #[cfg(test)]
    test_live_workers: Option<usize>,
}

#[derive(PartialEq)]
struct LiveContext {
    dimensions: (u32, u32),
    content: [u64; 4],
    board: [u64; 4],
    shapes: Vec<[u32; 2]>,
    finish: [bool; 3],
    config: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct RectangleKey {
    item_index: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

#[derive(PartialEq)]
struct RectangleInput {
    template_revision: u64,
    cells: Vec<String>,
    pixels: Vec<u8>,
    anchors: Vec<usize>,
}

struct CachedRectangle {
    input: RectangleInput,
    poses: Vec<Pose>,
}

impl RectangleKey {
    fn cells(self) -> impl Iterator<Item = usize> {
        (self.y..self.y + self.height)
            .flat_map(move |y| (self.x..self.x + self.width).map(move |x| y * COLS + x))
    }

    fn input(
        self,
        image: &RgbaImage,
        cells: &[String],
        anchors: &[&Observation],
        template_revision: u64,
    ) -> RectangleInput {
        let mut pixels = Vec::new();
        let raw = image.as_raw();
        let stride = image.width() as usize * 4;
        // Unknown pixels never enter evaluate(), edge/reverse or visible evidence.
        // Keep every observed tile, including borders and non-anchor observations.
        for cell in self.cells().filter(|&cell| cells[cell] != "unknown") {
            for y in cell / COLS * MATCH_CELL..(cell / COLS + 1) * MATCH_CELL {
                let start = y * stride + cell % COLS * MATCH_CELL * 4;
                pixels.extend_from_slice(&raw[start..start + MATCH_CELL * 4]);
            }
        }
        RectangleInput {
            template_revision,
            cells: self.cells().map(|cell| cells[cell].clone()).collect(),
            pixels,
            anchors: anchors.iter().map(|o| o.anchor).collect(),
        }
    }
}

/// Exact pixels consumed by the live matcher, using its existing samplers.
/// The caller has already validated the normalized board and current analysis.
pub(crate) fn live_input_pixels(
    frame: &RgbaImage,
    normalized_board: [f64; 4],
    content: [f64; 4],
) -> Option<(RgbaImage, [RgbaImage; 3])> {
    let viewport = super::super::content_rect(frame, content)?;
    let board = Rect {
        x: normalized_board[0] * frame.width() as f64,
        y: normalized_board[1] * frame.height() as f64,
        w: normalized_board[2] * frame.width() as f64,
        h: normalized_board[3] * frame.height() as f64,
    };
    Some((
        read_board(frame, board, MATCH_CELL),
        std::array::from_fn(|item| read_card(frame, viewport, item)),
    ))
}

impl Session {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn run(
        &mut self,
        frame: &RgbaImage,
        analysis: &super::super::Analysis,
        id: &str,
        config: &Config,
        out: &Path,
    ) -> Result<Output, String> {
        self.run_backend(frame, analysis, id, config, out, Backend::Direct)
    }
    pub fn run_backend(
        &mut self,
        frame: &RgbaImage,
        analysis: &super::super::Analysis,
        id: &str,
        config: &Config,
        out: &Path,
        backend: Backend,
    ) -> Result<Output, String> {
        let content = super::super::locate_content(frame).ok_or("content not located")?;
        self.run_internal(
            frame,
            analysis,
            content,
            id,
            config,
            Some(out),
            backend,
            None,
        )
    }

    /// Use the host's already located content region without diagnostic images.
    /// Reuse full rectangle searches and omit geometry crossing observed empty
    /// or completed cells, which cannot satisfy the live global constraints.
    /// Every remaining rectangle retains the exact search grid and scores.
    /// A deadline interrupts the search; partial output cannot establish a hit.
    pub fn run_live(
        &mut self,
        frame: &RgbaImage,
        analysis: &super::super::Analysis,
        content: [f64; 4],
        id: &str,
        config: &Config,
        max_elapsed: Option<Duration>,
    ) -> Result<Output, String> {
        self.run_internal(
            frame,
            analysis,
            content,
            id,
            config,
            None,
            Backend::Direct,
            max_elapsed,
        )
    }

    fn run_internal(
        &mut self,
        frame: &RgbaImage,
        analysis: &super::super::Analysis,
        content: [f64; 4],
        id: &str,
        config: &Config,
        out: Option<&Path>,
        backend: Backend,
        max_elapsed: Option<Duration>,
    ) -> Result<Output, String> {
        config.validate()?;
        // Offline diagnostics retain their original exhaustive visual candidates.
        // Live caches contain full rectangle searches, never selected placements.
        let live = out.is_none() && backend == Backend::Direct && !config.threshold_free_search;
        self.tile_cache.begin_run(config.tile_cache_bytes);
        let start = Instant::now();
        let deadline = max_elapsed.and_then(|duration| start.checked_add(duration));
        let viewport = super::super::content_rect(frame, content).ok_or("invalid content")?;
        let normalized = analysis.board.ok_or("board not located")?;
        if analysis.cells.len() != COLS * ROWS
            || normalized.iter().any(|value| !value.is_finite())
            || normalized[0] < 0.0
            || normalized[1] < 0.0
            || normalized[2] <= 0.0
            || normalized[3] <= 0.0
            || normalized[0] + normalized[2] > 1.0 + f64::EPSILON
            || normalized[1] + normalized[3] > 1.0 + f64::EPSILON
        {
            return Err("invalid board observations or geometry".into());
        }
        let board = Rect {
            x: normalized[0] * frame.width() as f64,
            y: normalized[1] * frame.height() as f64,
            w: normalized[2] * frame.width() as f64,
            h: normalized[3] * frame.height() as f64,
        };
        let previous = self.templates.clone();
        let finish = update_cards(frame, viewport, &mut self.templates);
        let mut cards = Vec::new();
        for item in 0..3 {
            let raw = read_card(frame, viewport, item);
            if !finish[item] {
                let (correction, diagnostic) = extraction::repair(&raw, &config.extraction);
                if let Some(corrected) = correction {
                    self.templates[item] = corrected;
                }
                self.extraction_diagnostics[item] = Some(diagnostic);
            }
            if !finish[item] {
                self.sources[item] = self.templates[item].as_ref().map(|_| id.to_owned());
            }
            let t = self.templates[item].as_ref();
            if let Some(out) = out {
                export_card(&raw, t, item, out)?;
            }
            cards.push(Card {
                item_index: item,
                fingerprint: t.map(|t| t.fingerprint.clone()),
                reference_source: self.sources[item].clone(),
                finished: finish[item],
                mask_pixels: t.map_or(0, |t| t.mask.iter().filter(|&&v| v).count()),
                core_pixels: t.map_or(0, |t| t.core.iter().filter(|&&v| v).count()),
                touches_crop: t.is_some_and(|t| {
                    t.bounds.x == 0
                        || t.bounds.y == 0
                        || t.bounds.x + t.bounds.w == CARD_W
                        || t.bounds.y + t.bounds.h == CARD_H
                }),
                extraction: self.extraction_diagnostics[item].clone().map(|mut d| {
                    d.reused_on_finish = finish[item];
                    d
                }),
            });
        }
        self.refresh_template_revisions(&previous);
        let image = read_board(frame, board, MATCH_CELL);
        if live {
            let context = LiveContext {
                dimensions: frame.dimensions(),
                content: content.map(f64::to_bits),
                board: normalized.map(f64::to_bits),
                shapes: analysis.shapes.clone(),
                finish,
                config: serde_json::to_vec(config).map_err(|e| e.to_string())?,
            };
            if self.live_context.as_ref() != Some(&context) {
                self.live_rectangles.clear();
                self.live_context = Some(context);
            }
        }
        if let Some(out) = out {
            image
                .save(out.join("board.png"))
                .map_err(|e| e.to_string())?;
            let mut visibility = image.clone();
            for y in 0..ROWS * MATCH_CELL {
                for x in 0..COLS * MATCH_CELL {
                    let v = if analysis.cells[y / MATCH_CELL * COLS + x / MATCH_CELL] == "unknown" {
                        0
                    } else {
                        255
                    };
                    visibility.put_pixel(x as u32, y as u32, Rgba([v, v, v, 255]));
                }
            }
            visibility
                .save(out.join("visibility.png"))
                .map_err(|e| e.to_string())?;
        }
        let extraction_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let observations: Vec<_> = analysis
            .cells
            .iter()
            .enumerate()
            .filter(|(_, s)| s.as_str() == "uncertain")
            .map(|(anchor, _)| observe_cell(&image, id, anchor, config))
            .collect();
        let mut state = SearchState::default();
        let mut complete = true;
        let scheduled_live = live && config.max_evaluations == 0;
        let mut live_jobs = Vec::new();
        let mut live_output = Vec::new();
        let rotations: [BTreeMap<_, _>; 3] = std::array::from_fn(|item| {
            self.templates[item].as_ref().map_or_else(BTreeMap::new, |t| {
                (0..360).step_by(config.coarse_angle_step)
                    .map(|angle| (angle, Rotation::new(t, angle as f64))).collect()
            })
        });
        // Each legal rectangle and coarse angular basin retains its own nuisance optimum.
        // The gated path preserves all coarse candidates. Threshold-free output
        // retains both requested metric minima per observation, item, and rectangle.
        'search: for (item, t) in self
            .templates
            .iter()
            .enumerate()
            .filter_map(|(i, t)| t.as_ref().map(|t| (i, t)))
        {
            let rotations = &rotations[item];
            for (cw, ch) in orientations(analysis.shapes.get(item).copied().unwrap_or([0, 0])) {
                let positions = match backend {
                    Backend::Direct => (0..=ROWS - ch)
                        .flat_map(|y| (0..=COLS - cw).map(move |x| (x, y)))
                        .collect(),
                    Backend::Tiled => tiled::positions(
                        &observations.iter().map(|o| o.anchor).collect::<Vec<_>>(),
                        cw,
                        ch,
                    ),
                };
                for (x, y) in positions {
                    let anchors: Vec<_> = observations
                        .iter()
                        .filter(|o| {
                            o.anchor % COLS >= x
                                && o.anchor % COLS < x + cw
                                && o.anchor / COLS >= y
                                && o.anchor / COLS < y + ch
                        })
                        .collect();
                    let rectangle = RectangleKey {
                        item_index: item,
                        x,
                        y,
                        width: cw,
                        height: ch,
                    };
                    if anchors.is_empty() {
                        if live {
                            self.live_rectangles.remove(&rectangle);
                        }
                        continue;
                    }
                    // These live candidates are independently impossible in the
                    // global solver: remaining objects cannot cross blocked cells.
                    if live && rectangle.cells().any(|cell| {
                        matches!(analysis.cells[cell].as_str(), "empty" | "completed")
                    }) {
                        self.live_rectangles.remove(&rectangle);
                        continue;
                    }
                    let cache_input = live.then(|| {
                        rectangle.input(
                            &image,
                            &analysis.cells,
                            &anchors,
                            self.source_revisions[item],
                        )
                    });
                    if let Some(input) = &cache_input {
                        if let Some(cached) = self.live_rectangles.get(&rectangle) {
                            if &cached.input == input {
                                let mut reused = Vec::with_capacity(cached.poses.len());
                                for stored in &cached.poses {
                                    let mut pose = stored.clone();
                                    let current = anchors
                                        .iter()
                                        .find(|o| o.anchor == pose.anchor)
                                        .expect("cached anchors have the same dependency key");
                                    if pose.observation_id != current.id {
                                        pose.observation_id.clone_from(&current.id);
                                    }
                                    reused.push(pose);
                                }
                                if scheduled_live {
                                    live_output.push(reused);
                                } else {
                                    state.candidates.extend(reused);
                                }
                                continue;
                            }
                        }
                        // Retire old evidence before a replacement can be interrupted.
                        self.live_rectangles.remove(&rectangle);
                    }
                    let first_rectangle_pose = state.candidates.len();
                    if scheduled_live {
                        live_jobs.push(LiveSearchJob {
                            rectangle,
                            input: cache_input.expect("live rectangle dependency"),
                            anchors,
                            template: t,
                            rotations,
                            output_index: live_output.len(),
                        });
                        live_output.push(Vec::new());
                        continue;
                    }
                    if !search_rectangle(
                        &mut self.tile_cache, backend, rectangle,
                        self.source_revisions[item], &image, t, rotations,
                        &anchors, &analysis.cells, config, deadline, &mut state,
                    ) {
                        complete = false;
                        break 'search;
                    }
                    if let Some(input) = cache_input {
                        // Any budget interruption breaks 'search above and cannot
                        // reach this commit. Empty complete searches are valid too.
                        self.live_rectangles.insert(
                            rectangle,
                            CachedRectangle {
                                input,
                                poses: state.candidates[first_rectangle_pose..].to_vec(),
                            },
                        );
                    }
                }
            }
        }
        if scheduled_live {
            let workers = live_search_workers();
            #[cfg(test)]
            let workers = self.test_live_workers.unwrap_or(workers);
            let results = search_live_jobs(
                &live_jobs, &image, &analysis.cells, config, deadline, workers,
            )?;
            for (job, (result, finished)) in live_jobs.into_iter().zip(results) {
                complete &= finished;
                state.evaluations += result.evaluations;
                state.rescue_evaluations += result.rescue_evaluations;
                state.recovered_basins += result.recovered_basins;
                state.foreground_evaluations += result.foreground_evaluations;
                state.foreground_basins += result.foreground_basins;
                cache_complete_rectangle(&mut self.live_rectangles, job.rectangle, job.input, &result, finished);
                live_output[job.output_index] = result.candidates;
            }
            state.candidates = live_output.into_iter().flatten().collect();
        }
        let SearchState { mut candidates, threshold_free_minima, evaluations,
            rescue_evaluations, recovered_basins, foreground_evaluations,
            foreground_basins } = state;
        if config.threshold_free_search {
            candidates = threshold_free_minima
                .into_values()
                .flatten()
                .flatten()
                .collect();
        }
        candidates.sort_by(|a, b| {
            a.anchor
                .cmp(&b.anchor)
                .then(a.scores[0].total_cmp(&b.scores[0]))
        });
        candidates.dedup_by(|a, b| {
            a.anchor == b.anchor
                && a.item_index == b.item_index
                && a.rect == b.rect
                && a.angle == b.angle
                && a.fill == b.fill
                && (!config.foreground_refinement || a.scale == b.scale)
                && a.offset == b.offset
                && (!config.threshold_free_search || nearest_scores(a) == nearest_scores(b))
        });
        if config.foreground_refinement && !config.threshold_free_search {
            // Several supported poses can have exactly the same RGB score.
            // Remove repeated transforms across both routes even when such ties
            // separate duplicates in the unchanged RGB-sorted output.
            let mut seen = std::collections::HashSet::new();
            let bits = |value: f64| if value == 0.0 { 0 } else { value.to_bits() };
            candidates.retain(|pose| {
                seen.insert((
                    pose.anchor,
                    pose.item_index,
                    pose.rect.x,
                    pose.rect.y,
                    pose.rect.width,
                    pose.rect.height,
                    bits(pose.angle),
                    bits(pose.fill),
                    bits(pose.scale),
                    bits(pose.offset[0]),
                    bits(pose.offset[1]),
                ))
            });
        }
        let matching_ms = start.elapsed().as_secs_f64() * 1000.0;
        let tile_cache = (backend == Backend::Tiled).then(|| self.tile_cache.stats());
        let asset_export_ms = if let Some(out) =
            out.filter(|_| backend == Backend::Tiled && !config.threshold_free_search)
        {
            let start = Instant::now();
            tiled::export(&mut candidates, &self.templates, out)?;
            start.elapsed().as_secs_f64() * 1000.0
        } else {
            0.0
        };
        Ok(Output {
            backend,
            cards,
            observations,
            candidates,
            complete,
            interruption: (!complete).then(|| {
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    "configured elapsed budget exhausted; no unique conclusion allowed".into()
                } else {
                    "configured evaluation budget exhausted; no unique conclusion allowed".into()
                }
            }),
            evaluations,
            rescue_evaluations,
            recovered_basins,
            foreground_evaluations,
            foreground_basins,
            extraction_ms,
            matching_ms,
            tile_cache,
            asset_export_ms,
            legacy_evaluator: if config.threshold_free_search {
                "offline threshold-free evaluate(): foreground core, at least 55 pixels; 32 px/cell; pixel bad-ratio and max_color_error rejection disabled; min_anchor_pixels and legal geometry retained; every explored evidence-valid pose contributes to stable RGB/composite minima per observation, item, and rectangle"
            } else {
                "production evaluate(): foreground core, at least 55 pixels, <=40% pixels with RGB MAE>35; 32 px/cell; original RGB refinement route preserved; foreground_refinement=true adds an independent route minimizing max(selected bidirectional score, observed foreground error); confidence_weighted_scoring selects border-weighted core RGB while preserving legacy scores"
            },
        })
    }
    fn refresh_template_revisions(&mut self, previous: &[Option<CardTemplate>; 3]) {
        for (item, old) in previous.iter().enumerate() {
            if !tiled::same_template(old.as_ref(), self.templates[item].as_ref()) {
                self.source_revisions[item] = self.source_revisions[item].wrapping_add(1);
                self.tile_cache.invalidate(item);
                self.live_rectangles
                    .retain(|rectangle, _| rectangle.item_index != item);
            }
        }
    }
}

#[derive(Default)]
struct SearchState {
    candidates: Vec<Pose>,
    threshold_free_minima: ThresholdFreeMinima,
    evaluations: u64,
    rescue_evaluations: u64,
    recovered_basins: u64,
    foreground_evaluations: u64,
    foreground_basins: u64,
}

// A technical CPU ceiling: reserve at least half the available logical CPUs for
// capture, the foreground game, and the desktop; never limit search coverage.
const MAX_LIVE_SEARCH_WORKERS: usize = 8;

fn live_search_workers() -> usize {
    (std::thread::available_parallelism().map_or(1, usize::from) / 2)
        .clamp(1, MAX_LIVE_SEARCH_WORKERS)
}

struct LiveSearchJob<'a> {
    rectangle: RectangleKey,
    input: RectangleInput,
    anchors: Vec<&'a Observation>,
    template: &'a CardTemplate,
    rotations: &'a BTreeMap<usize, Rotation>,
    output_index: usize,
}

fn search_live_jobs(
    jobs: &[LiveSearchJob<'_>],
    image: &RgbaImage,
    cells: &[String],
    config: &Config,
    deadline: Option<Instant>,
    workers: usize,
) -> Result<Vec<(SearchState, bool)>, String> {
    let search = |job: &LiveSearchJob<'_>,
                  cache: &mut tiled::Cache,
                  angles: std::ops::RangeInclusive<usize>| {
        let mut state = SearchState::default();
        // Even a rectangle with no compatible rotation is a cache miss. An
        // expired deadline cannot certify that missing search as complete.
        let complete = !budget_exhausted(config, 0, deadline)
            && search_rectangle(
                cache,
                Backend::Direct,
                job.rectangle,
                job.input.template_revision,
                image,
                job.template,
                job.rotations.range(angles),
                &job.anchors,
                cells,
                config,
                deadline,
                &mut state,
            );
        (state, complete)
    };
    if workers <= 1 {
        let mut cache = tiled::Cache::default();
        return Ok(jobs
            .iter()
            .map(|job| search(job, &mut cache, 0..=359))
            .collect());
    }
    // Each coarse-angle basin owns its complete rescue and refinement chains.
    // Splitting at this boundary balances a heavy rectangle without changing
    // any seed decision or floating-point accumulation within that basin.
    let tasks: Vec<_> = jobs
        .iter()
        .enumerate()
        .flat_map(|(rectangle, job)| job.rotations.keys().map(move |&angle| (rectangle, angle)))
        .collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        let mut failure = None;
        for _ in 0..workers.min(tasks.len()) {
            match std::thread::Builder::new().spawn_scoped(scope, || {
                let mut cache = tiled::Cache::default();
                let mut results = Vec::new();
                loop {
                    let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&(rectangle, angle)) = tasks.get(index) else {
                        break;
                    };
                    results.push((index, search(&jobs[rectangle], &mut cache, angle..=angle)));
                }
                results
            }) {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    failure = Some(format!(
                        "could not start live rectangle search worker: {error}"
                    ));
                    break;
                }
            }
        }
        let mut ordered: Vec<_> = (0..tasks.len()).map(|_| None).collect();
        for handle in handles {
            match handle.join() {
                Ok(results) => {
                    for (index, result) in results {
                        ordered[index] = Some(result);
                    }
                }
                Err(_) => failure = Some("live rectangle search worker panicked".into()),
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        let parts: Result<Vec<_>, String> = tasks
            .iter()
            .copied()
            .zip(ordered)
            .map(|((rectangle, _), result)| {
                result
                    .map(|(state, complete)| (rectangle, state, complete))
                    .ok_or_else(|| "live rectangle search task did not finish".into())
            })
            .collect();
        Ok(merge_live_search_parts(jobs.len(), parts?))
    })
}

fn cache_complete_rectangle(
    cache: &mut BTreeMap<RectangleKey, CachedRectangle>,
    rectangle: RectangleKey,
    input: RectangleInput,
    result: &SearchState,
    complete: bool,
) {
    if complete {
        cache.insert(
            rectangle,
            CachedRectangle {
                input,
                poses: result.candidates.clone(),
            },
        );
    }
}

fn merge_live_search_parts(
    rectangles: usize,
    parts: Vec<(usize, SearchState, bool)>,
) -> Vec<(SearchState, bool)> {
    let mut results: Vec<_> = (0..rectangles)
        .map(|_| (SearchState::default(), true))
        .collect();
    // Task results arrive here in original rectangle/angle order, independent
    // of worker completion order. A single unfinished basin invalidates the
    // whole rectangle for caching, while retaining diagnostic partial poses.
    for (rectangle, part, complete) in parts {
        let (result, finished) = &mut results[rectangle];
        *finished &= complete;
        result.evaluations += part.evaluations;
        result.rescue_evaluations += part.rescue_evaluations;
        result.recovered_basins += part.recovered_basins;
        result.foreground_evaluations += part.foreground_evaluations;
        result.foreground_basins += part.foreground_basins;
        result.candidates.extend(part.candidates);
    }
    results
}

fn search_rectangle<'rotation>(
    tile_cache: &mut tiled::Cache,
    backend: Backend,
    rectangle: RectangleKey,
    revision: u64,
    image: &RgbaImage,
    t: &CardTemplate,
    rotations: impl IntoIterator<Item = (&'rotation usize, &'rotation Rotation)>,
    anchors: &[&Observation],
    cells: &[String],
    config: &Config,
    deadline: Option<Instant>,
    state: &mut SearchState,
) -> bool {
    let RectangleKey {
        item_index: item,
        x,
        y,
        width: cw,
        height: ch,
    } = rectangle;
    let SearchState {
        candidates,
        threshold_free_minima,
        evaluations,
        rescue_evaluations,
        recovered_basins,
        foreground_evaluations,
        foreground_basins,
    } = state;
    for (&angle, &r) in rotations {
        if ((r.w / r.h) / (cw as f64 / ch as f64)).ln().abs() > config.aspect_log_tolerance {
            continue;
        }
        let mut seed: Option<Candidate> = None;
        let mut seed_tiles = None;
        let mut foreground_seed = None;
        for &fill in &config.fills {
            if budget_exhausted(config, *evaluations, deadline) {
                return false;
            }
            *evaluations += 1;
            let c = Candidate {
                placement: GridPlacement {
                    x,
                    y,
                    width: cw,
                    height: ch,
                },
                angle: angle as f64,
                fill,
                dx: 0.0,
                dy: 0.0,
                score: 255.0,
                counts: [0; 45],
            };
            let (evaluated, tiles) = evaluate_backend(
                tile_cache,
                backend,
                item,
                revision,
                image,
                t,
                r,
                &c,
                cells,
                config.threshold_free_search,
            );
            if let Some(c) = evaluated {
                if (config.threshold_free_search || c.score <= config.max_color_error)
                    && anchors
                        .iter()
                        .any(|a| c.counts[a.anchor] >= config.min_anchor_pixels)
                {
                    let first_pose = candidates.len();
                    push_backend_poses(
                        candidates,
                        image,
                        t,
                        &c,
                        anchors,
                        item,
                        cells,
                        config,
                        tiles.as_deref(),
                        revision,
                    );
                    if config.foreground_refinement {
                        consider_foreground_seed(
                            &mut foreground_seed,
                            &c,
                            &candidates[first_pose..],
                        );
                    }
                    if config.threshold_free_search {
                        retain_threshold_free_poses(
                            threshold_free_minima,
                            candidates.drain(first_pose..),
                        );
                    }
                    if seed.as_ref().map_or(true, |s| c.score < s.score) {
                        seed = Some(c);
                        seed_tiles = tiles;
                    }
                }
            }
        }
        if seed.is_none() && config.coarse_offset_rescue {
            if let Some(refine) = config.refine.first() {
                for &fill in &config.fills {
                    for &dy in &refine.offsets {
                        for &dx in &refine.offsets {
                            if dx == 0.0 && dy == 0.0 {
                                continue;
                            }
                            if budget_exhausted(config, *evaluations, deadline) {
                                return false;
                            }
                            *evaluations += 1;
                            *rescue_evaluations += 1;
                            let c = Candidate {
                                placement: GridPlacement {
                                    x,
                                    y,
                                    width: cw,
                                    height: ch,
                                },
                                angle: angle as f64,
                                fill,
                                dx,
                                dy,
                                score: 255.0,
                                counts: [0; 45],
                            };
                            let (evaluated, tiles) = evaluate_backend(
                                tile_cache,
                                backend,
                                item,
                                revision,
                                image,
                                t,
                                r,
                                &c,
                                cells,
                                config.threshold_free_search,
                            );
                            if let Some(c) = evaluated {
                                if (config.threshold_free_search
                                    || c.score <= config.max_color_error)
                                    && anchors
                                        .iter()
                                        .any(|a| c.counts[a.anchor] >= config.min_anchor_pixels)
                                {
                                    let first_pose = candidates.len();
                                    push_backend_poses(
                                        candidates,
                                        image,
                                        t,
                                        &c,
                                        anchors,
                                        item,
                                        cells,
                                        config,
                                        tiles.as_deref(),
                                        revision,
                                    );
                                    if config.foreground_refinement {
                                        consider_foreground_seed(
                                            &mut foreground_seed,
                                            &c,
                                            &candidates[first_pose..],
                                        );
                                    }
                                    if config.threshold_free_search {
                                        retain_threshold_free_poses(
                                            threshold_free_minima,
                                            candidates.drain(first_pose..),
                                        );
                                    }
                                    if seed.is_none() {
                                        *recovered_basins += 1;
                                    }
                                    if seed.as_ref().map_or(true, |s| c.score < s.score) {
                                        seed = Some(c);
                                        seed_tiles = tiles;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        let Some(mut seed) = seed else {
            continue;
        };
        for refine in &config.refine {
            let origin = seed.clone();
            for &da in &refine.angles {
                let r = Rotation::new(t, origin.angle + da);
                for &df in &refine.fills {
                    for &dy in &refine.offsets {
                        for &dx in &refine.offsets {
                            if budget_exhausted(config, *evaluations, deadline) {
                                return false;
                            }
                            let c = Candidate {
                                angle: origin.angle + da,
                                fill: origin.fill + df,
                                dx: origin.dx + dx,
                                dy: origin.dy + dy,
                                ..origin.clone()
                            };
                            if c.fill <= 0.0 || c.fill > 1.0 {
                                continue;
                            }
                            *evaluations += 1;
                            let (evaluated, tiles) = evaluate_backend(
                                tile_cache,
                                backend,
                                item,
                                revision,
                                image,
                                t,
                                r,
                                &c,
                                cells,
                                config.threshold_free_search,
                            );
                            if let Some(c) = evaluated {
                                if config.threshold_free_search {
                                    let first_pose = candidates.len();
                                    push_backend_poses(
                                        candidates,
                                        image,
                                        t,
                                        &c,
                                        anchors,
                                        item,
                                        cells,
                                        config,
                                        tiles.as_deref(),
                                        revision,
                                    );
                                    retain_threshold_free_poses(
                                        threshold_free_minima,
                                        candidates.drain(first_pose..),
                                    );
                                }
                                if c.score < seed.score
                                    && anchors
                                        .iter()
                                        .any(|a| c.counts[a.anchor] >= config.min_anchor_pixels)
                                {
                                    seed = c;
                                    seed_tiles = tiles;
                                }
                            }
                        }
                    }
                }
            }
        }
        let first_pose = candidates.len();
        push_backend_poses(
            candidates,
            image,
            t,
            &seed,
            anchors,
            item,
            cells,
            config,
            seed_tiles.as_deref(),
            revision,
        );
        if config.threshold_free_search {
            retain_threshold_free_poses(threshold_free_minima, candidates.drain(first_pose..));
        }
        if let Some(foreground_seed) = foreground_seed {
            *foreground_basins += 1;
            if !refine_foreground(
                tile_cache,
                backend,
                item,
                revision,
                image,
                t,
                anchors,
                cells,
                config,
                foreground_seed,
                candidates,
                threshold_free_minima,
                evaluations,
                foreground_evaluations,
                deadline,
            ) {
                return false;
            }
        }
    }
    true
}

fn budget_exhausted(config: &Config, evaluations: u64, deadline: Option<Instant>) -> bool {
    (config.max_evaluations > 0 && evaluations >= config.max_evaluations)
        || deadline.is_some_and(|deadline| Instant::now() >= deadline)
}

fn export_card(
    raw: &RgbaImage,
    template: Option<&CardTemplate>,
    item: usize,
    out: &Path,
) -> Result<(), String> {
    raw.save(out.join(format!("card-{item}.png")))
        .map_err(|e| e.to_string())?;
    let mut cut = RgbaImage::new(CARD_W as u32, CARD_H as u32);
    let mut mask = cut.clone();
    let mut core = cut.clone();
    if let Some(template) = template {
        for y in 0..CARD_H {
            for x in 0..CARD_W {
                let index = y * CARD_W + x;
                if template.mask[index] {
                    cut.put_pixel(
                        x as u32,
                        y as u32,
                        *template.image.get_pixel(x as u32, y as u32),
                    );
                }
                let masked = if template.mask[index] { 255 } else { 0 };
                let core_value = if template.core[index] { 255 } else { 0 };
                mask.put_pixel(x as u32, y as u32, Rgba([masked, masked, masked, 255]));
                core.put_pixel(
                    x as u32,
                    y as u32,
                    Rgba([core_value, core_value, core_value, 255]),
                );
            }
        }
    }
    for (name, image) in [("cut", cut), ("mask", mask), ("core", core)] {
        image
            .save(out.join(format!("{name}-{item}.png")))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn evaluate_backend(
    cache: &mut tiled::Cache,
    backend: Backend,
    item: usize,
    revision: u64,
    image: &RgbaImage,
    template: &CardTemplate,
    rotation: Rotation,
    candidate: &Candidate,
    cells: &[String],
    threshold_free_search: bool,
) -> (
    Option<Candidate>,
    Option<std::sync::Arc<tiled::Transformed>>,
) {
    match backend {
        Backend::Direct => (
            evaluate_direct(
                image,
                template,
                rotation,
                candidate,
                cells,
                threshold_free_search,
            ),
            None,
        ),
        Backend::Tiled => {
            let tiles = cache.prepare(item, revision, template, rotation, candidate);
            let (result, visible) =
                tiled::evaluate_with_policy(image, &tiles, candidate, cells, threshold_free_search);
            cache.scored_visible_blocks(visible);
            (result, Some(tiles))
        }
    }
}

/// The disabled path calls production unchanged. The experiment's ungated
/// path uses identical transforms and row-major RGB arithmetic, retaining the
/// production evidence minimum without its pixel-error rejection.
fn evaluate_direct(
    image: &RgbaImage,
    t: &CardTemplate,
    r: Rotation,
    c: &Candidate,
    cells: &[String],
    threshold_free_search: bool,
) -> Option<Candidate> {
    if !threshold_free_search {
        return evaluate(image, t, r, c, cells);
    }
    let p = &c.placement;
    let (w, h) = (p.width * MATCH_CELL, p.height * MATCH_CELL);
    let scale = (w as f64 / r.w).min(h as f64 / r.h) * c.fill;
    let (ox, oy) = (
        (w as f64 - r.w * scale) / 2.0 + c.dx,
        (h as f64 - r.h * scale) / 2.0 + c.dy,
    );
    let mut sum = 0.0;
    let mut count = 0;
    let mut counts = [0; 45];
    for y in 0..h {
        for x in 0..w {
            let idx = (p.y + y / MATCH_CELL) * COLS + p.x + x / MATCH_CELL;
            if cells[idx] == "unknown" {
                continue;
            }
            let (sx, sy) = r.source((x as f64 + 0.5 - ox) / scale, (y as f64 + 0.5 - oy) / scale);
            if !mask_at(&t.core, sx, sy) {
                continue;
            }
            let observed = rgb(image, p.x * MATCH_CELL + x, p.y * MATCH_CELL + y);
            let expected = sample(&t.image, sx, sy);
            sum += distance(observed, expected);
            count += 1;
            counts[idx] += 1;
        }
    }
    if count < 55 {
        return None;
    }
    let mut result = c.clone();
    result.score = sum / count as f64;
    result.counts = counts;
    Some(result)
}

fn push_backend_pose(
    out: &mut Vec<Pose>,
    image: &RgbaImage,
    t: &CardTemplate,
    c: &Candidate,
    o: &Observation,
    item: usize,
    cells: &[String],
    cfg: &Config,
    tiles: Option<&tiled::Transformed>,
    revision: u64,
) {
    if let Some(tiles) = tiles {
        tiled::push_pose(out, image, t, c, o, item, cells, cfg, tiles, revision);
    } else {
        push_pose(out, image, t, c, o, item, cells, cfg);
    }
}

fn push_backend_poses(
    out: &mut Vec<Pose>,
    image: &RgbaImage,
    t: &CardTemplate,
    c: &Candidate,
    anchors: &[&Observation],
    item: usize,
    cells: &[String],
    cfg: &Config,
    tiles: Option<&tiled::Transformed>,
    revision: u64,
) {
    // Leave the tiled diagnostics and their anchor-specific metadata unchanged.
    if tiles.is_some() {
        for anchor in anchors {
            push_backend_pose(out, image, t, c, anchor, item, cells, cfg, tiles, revision);
        }
        return;
    }
    if !cfg.threshold_free_search && c.score > cfg.max_color_error {
        return;
    }
    let mut eligible = anchors
        .iter()
        .copied()
        .filter(|anchor| c.counts[anchor.anchor] >= cfg.min_anchor_pixels);
    let Some(first) = eligible.next() else {
        return;
    };
    let first_pose = out.len();
    push_backend_pose(out, image, t, c, first, item, cells, cfg, None, revision);
    let Some(second) = eligible.next() else {
        return;
    };
    // The original scorer keeps its exact traversal and floating-point sums.
    // Every remaining field measures this whole rectangle, shared by its anchors.
    let scored = out[first_pose].clone();
    for anchor in std::iter::once(second).chain(eligible) {
        let mut pose = scored.clone();
        pose.observation_id.clone_from(&anchor.id);
        pose.anchor = anchor.anchor;
        pose.evidence_pixels = c.counts[anchor.anchor];
        out.push(pose);
    }
}

struct ForegroundSeed {
    candidate: Candidate,
    score: f64,
}

fn foreground_objective(pose: &Pose) -> Option<f64> {
    let base = pose.visible_base_score()?;
    pose.visible_evidence
        .as_ref()?
        .foreground_color_error
        .map(|error| base.max(error))
}

// One minimum per requested metric and legal geometry bounds output storage
// without pruning evaluated poses or changing either observation's argmin.
type ThresholdFreeMinima = BTreeMap<(usize, usize, usize, usize, usize, usize), [Option<Pose>; 2]>;

fn nearest_scores(pose: &Pose) -> [Option<f64>; 2] {
    [Some(pose.scores[0]), foreground_objective(pose)]
}

fn retain_threshold_free_poses(
    minima: &mut ThresholdFreeMinima,
    poses: impl IntoIterator<Item = Pose>,
) {
    for pose in poses {
        let best = minima
            .entry((
                pose.anchor,
                pose.item_index,
                pose.rect.x,
                pose.rect.y,
                pose.rect.width,
                pose.rect.height,
            ))
            .or_insert_with(|| [None, None]);
        for (metric, score) in nearest_scores(&pose).into_iter().enumerate() {
            let Some(score) = score else { continue };
            if best[metric].as_ref().map_or(true, |previous| {
                score < nearest_scores(previous)[metric].unwrap_or(f64::INFINITY)
            }) {
                best[metric] = Some(pose.clone());
            }
        }
    }
}

fn consider_foreground_seed(
    seed: &mut Option<ForegroundSeed>,
    candidate: &Candidate,
    poses: &[Pose],
) {
    // Scores use the whole exposed rectangle, so all anchors share this target.
    let Some(score) = poses.first().and_then(foreground_objective) else {
        return;
    };
    if seed.as_ref().map_or(true, |best| score < best.score) {
        *seed = Some(ForegroundSeed {
            candidate: candidate.clone(),
            score,
        });
    }
}

fn refine_foreground(
    cache: &mut tiled::Cache,
    backend: Backend,
    item: usize,
    revision: u64,
    image: &RgbaImage,
    template: &CardTemplate,
    anchors: &[&Observation],
    cells: &[String],
    cfg: &Config,
    mut seed: ForegroundSeed,
    candidates: &mut Vec<Pose>,
    minima: &mut ThresholdFreeMinima,
    evaluations: &mut u64,
    foreground_evaluations: &mut u64,
    deadline: Option<Instant>,
) -> bool {
    for refine in &cfg.refine {
        let origin = seed.candidate.clone();
        for &da in &refine.angles {
            let rotation = Rotation::new(template, origin.angle + da);
            for &df in &refine.fills {
                for &dy in &refine.offsets {
                    for &dx in &refine.offsets {
                        if budget_exhausted(cfg, *evaluations, deadline) {
                            return false;
                        }
                        let candidate = Candidate {
                            angle: origin.angle + da,
                            fill: origin.fill + df,
                            dx: origin.dx + dx,
                            dy: origin.dy + dy,
                            ..origin.clone()
                        };
                        if candidate.fill <= 0.0 || candidate.fill > 1.0 {
                            continue;
                        }
                        *evaluations += 1;
                        *foreground_evaluations += 1;
                        let (evaluated, tiles) = evaluate_backend(
                            cache,
                            backend,
                            item,
                            revision,
                            image,
                            template,
                            rotation,
                            &candidate,
                            cells,
                            cfg.threshold_free_search,
                        );
                        let Some(candidate) = evaluated else {
                            continue;
                        };
                        // The experiment disables color gates, not evidence requirements.
                        if (!cfg.threshold_free_search && candidate.score > cfg.max_color_error)
                            || !anchors
                                .iter()
                                .any(|o| candidate.counts[o.anchor] >= cfg.min_anchor_pixels)
                        {
                            continue;
                        }
                        let mut poses = Vec::new();
                        push_backend_poses(
                            &mut poses,
                            image,
                            template,
                            &candidate,
                            anchors,
                            item,
                            cells,
                            cfg,
                            tiles.as_deref(),
                            revision,
                        );
                        let score = poses.first().and_then(foreground_objective);
                        if cfg.threshold_free_search {
                            retain_threshold_free_poses(minima, poses.drain(..));
                        }
                        let Some(score) = score else {
                            continue;
                        };
                        let improves = score < seed.score;
                        // Keep the improving path for diagnosis, plus ALL poses that meet
                        // the retention bound. Selecting a nuisance optimum must not turn
                        // several supported angles into a false unique direction.
                        if !cfg.threshold_free_search && (improves || score <= cfg.max_color_error)
                        {
                            candidates.extend(poses);
                        }
                        if improves {
                            seed = ForegroundSeed { candidate, score };
                        }
                    }
                }
            }
        }
    }
    true
}
fn reliable(p: [u8; 3], cfg: &Config) -> bool {
    let max = *p.iter().max().unwrap();
    let min = *p.iter().min().unwrap();
    !super::super::selection_green(p)
        && (max - min > cfg.foreground_chroma || max < cfg.foreground_dark)
}

fn observe_cell(image: &RgbaImage, id: &str, anchor: usize, cfg: &Config) -> Observation {
    let mut values = Vec::new();
    let mut border_pixels = 0;
    for y in 0..MATCH_CELL {
        for x in 0..MATCH_CELL {
            let p = rgb(
                image,
                anchor % COLS * MATCH_CELL + x,
                anchor / COLS * MATCH_CELL + y,
            );
            if !reliable(p, cfg) {
                continue;
            }
            let border = x < cfg.reliable_border
                || x >= MATCH_CELL - cfg.reliable_border
                || y < cfg.reliable_border
                || y >= MATCH_CELL - cfg.reliable_border;
            if border {
                border_pixels += 1;
            } else {
                values.push(p.iter().map(|&p| p as f64).sum::<f64>() / 3.0);
            }
        }
    }
    let mean = values.iter().sum::<f64>() / values.len().max(1) as f64;
    let std = (values.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / values.len().max(1) as f64)
        .sqrt();
    // Count observed border foreground as low confidence, as V already does.
    // Retain interior-only texture and the existing minimum; this does not
    // change candidate generation, its template-core count, or any score.
    let effective = values.len() as f64 + border_pixels as f64 * cfg.low_confidence_weight;
    Observation {
        id: format!("{id}:cell-{anchor}"),
        anchor,
        texture_std: std,
        reliable_foreground_pixels: values.len(),
        border_reliable_foreground_pixels: border_pixels,
        effective_foreground_pixels: effective,
        sufficient_evidence: effective >= cfg.min_anchor_pixels as f64
            && std >= cfg.min_texture_std,
    }
}

/// Explicit replay derivatives, kept in the source screenshot's split/group.
/// These pixels never become production references or new real samples.
pub fn derive(original: &RgbaImage, kind: &str) -> Result<RgbaImage, String> {
    let mut frame = original.clone();
    let content = super::super::locate_content(&frame).ok_or("cannot derive without content")?;
    let vp = super::super::content_rect(&frame, content).ok_or("invalid derivation content")?;
    if let Some(width) = kind.strip_prefix("resize-") {
        let width: u32 = width.parse().map_err(|_| "invalid resize width")?;
        if width == 0 {
            return Err("resize width is zero".into());
        }
        let crop = image::imageops::crop_imm(
            &frame,
            vp.x.round() as u32,
            vp.y.round() as u32,
            vp.w.round() as u32,
            vp.h.round() as u32,
        )
        .to_image();
        return Ok(image::imageops::resize(
            &crop,
            width,
            (width as f64 * vp.h / vp.w).round() as u32,
            image::imageops::FilterType::Triangle,
        ));
    }
    if kind == "rotate-cards-17" {
        // Same whole-sprite transform as the established dynamic regression.
        for item in 0..3 {
            let t = extract(read_card(&frame, vp, item)).ok_or("rotation reference unavailable")?;
            let r = Rotation::new(&t, 17.0);
            let scale = ((CARD_W as f64 - 10.0) / r.w).min((CARD_H as f64 - 10.0) / r.h) * 0.91;
            let (ox, oy) = (
                (CARD_W as f64 - r.w * scale) / 2.0,
                (CARD_H as f64 - r.h * scale) / 2.0,
            );
            let bottom = anchored_viewport(vp, LayoutAnchor::Bottom);
            for y in 0..CARD_H {
                for x in 0..CARD_W {
                    let (sx, sy) = r.source((x as f64 - ox) / scale, (y as f64 - oy) / scale);
                    let p = if mask_at(&t.mask, sx, sy) {
                        sample(&t.image, sx, sy)
                    } else {
                        [55, 105, 165]
                    };
                    let xx = (bottom.x
                        + ([182.0, 389.0, 595.0][item] + x as f64) * bottom.w / 1920.0)
                        .round() as u32;
                    let yy = (bottom.y + (880.0 + y as f64) * bottom.h / 1080.0).round() as u32;
                    if xx < frame.width() && yy < frame.height() {
                        frame.put_pixel(xx, yy, Rgba([p[0], p[1], p[2], 255]));
                    }
                }
            }
        }
    } else if ["solid-fragment", "symmetric-fragment"].contains(&kind) {
        let tile = super::super::expected_board(vp).cell(33);
        for y in tile.y as u32..(tile.y + tile.h) as u32 {
            for x in tile.x as u32..(tile.x + tile.w) as u32 {
                let radius = (((x as f64 - tile.x) / tile.w - 0.5).powi(2)
                    + ((y as f64 - tile.y) / tile.h - 0.5).powi(2))
                .sqrt();
                let p = if kind == "solid-fragment" || radius < 0.30 {
                    [65, 110, 175, 255]
                } else {
                    [235, 240, 246, 255]
                };
                frame.put_pixel(x, y, Rgba(p));
            }
        }
    } else if kind == "wrong-reference" {
        // Existing negative replaces only the active gun card, no truth inference.
        for y in 948..1060 {
            for x in 194..334 {
                frame.put_pixel(x, y, Rgba([70, 70, 70, 255]));
            }
        }
    } else if kind == "background-pollution" {
        for y in 940..1064 {
            for x in 184..338 {
                if (x < 191 || x > 330) && (x + y) % 5 < 2 {
                    frame.put_pixel(x, y, Rgba([210, 65, 180, 255]));
                }
            }
        }
    } else {
        return Err(format!("unknown derivative: {kind}"));
    }
    Ok(frame)
}
fn push_pose(
    out: &mut Vec<Pose>,
    image: &RgbaImage,
    t: &CardTemplate,
    c: &Candidate,
    o: &Observation,
    item: usize,
    cells: &[String],
    cfg: &Config,
) {
    if (!cfg.threshold_free_search && c.score > cfg.max_color_error)
        || c.counts[o.anchor] < cfg.min_anchor_pixels
    {
        return;
    }
    let r = Rotation::new(t, c.angle);
    let p = &c.placement;
    let (w, h) = (p.width * MATCH_CELL, p.height * MATCH_CELL);
    let scale = (w as f64 / r.w).min(h as f64 / r.h) * c.fill;
    let (ox, oy) = (
        (w as f64 - r.w * scale) / 2.0 + c.dx,
        (h as f64 - r.h * scale) / 2.0 + c.dy,
    );
    let (
        mut edge,
        mut edge_n,
        mut bad,
        mut expected_n,
        mut unexplained,
        mut actual_n,
        mut effective,
    ) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let (bx, by) = (p.x * MATCH_CELL + x, p.y * MATCH_CELL + y);
            if cells[by / MATCH_CELL * COLS + bx / MATCH_CELL] == "unknown" {
                continue;
            }
            let observed = rgb(image, bx, by);
            let border = x % MATCH_CELL < cfg.reliable_border
                || x % MATCH_CELL >= MATCH_CELL - cfg.reliable_border
                || y % MATCH_CELL < cfg.reliable_border
                || y % MATCH_CELL >= MATCH_CELL - cfg.reliable_border;
            let weight = if border || super::super::selection_green(observed) {
                cfg.low_confidence_weight
            } else {
                1.0
            };
            let (sx, sy) = r.source((x as f64 + 0.5 - ox) / scale, (y as f64 + 0.5 - oy) / scale);
            let predicted = mask_at(&t.mask, sx, sy);
            let core = mask_at(&t.core, sx, sy);
            let error = if predicted {
                distance(observed, sample(&t.image, sx, sy))
            } else {
                255.0
            };
            if core {
                expected_n += weight;
                bad += weight * f64::from(error > cfg.mismatch_error);
                effective += weight;
            }
            if reliable(observed, cfg) {
                actual_n += weight;
                if !predicted || error > cfg.mismatch_error {
                    unexplained += weight;
                }
            }
            if core {
                let (tx, ty) =
                    r.source((x as f64 + 1.5 - ox) / scale, (y as f64 + 0.5 - oy) / scale);
                if mask_at(&t.core, tx, ty) && (bx + 1) / MATCH_CELL == bx / MATCH_CELL {
                    edge += (distance(rgb(image, bx + 1, by), observed)
                        - distance(sample(&t.image, tx, ty), sample(&t.image, sx, sy)))
                    .abs()
                        * weight;
                    edge_n += weight;
                }
                let (tx, ty) =
                    r.source((x as f64 + 0.5 - ox) / scale, (y as f64 + 1.5 - oy) / scale);
                if mask_at(&t.core, tx, ty) && (by + 1) / MATCH_CELL == by / MATCH_CELL {
                    edge += (distance(rgb(image, bx, by + 1), observed)
                        - distance(sample(&t.image, tx, ty), sample(&t.image, sx, sy)))
                    .abs()
                        * weight;
                    edge_n += weight;
                }
            }
        }
    }
    let edge_error = edge / edge_n.max(1.0);
    let predicted_mismatch = bad / expected_n.max(1.0);
    let unexplained_foreground = unexplained / actual_n.max(1.0);
    let s1 = c.score + cfg.edge_weight * edge_error;
    let scores = [
        c.score,
        s1,
        s1 + cfg.reverse_weight * (predicted_mismatch + unexplained_foreground),
    ];
    out.push(Pose {
        observation_id: o.id.clone(),
        anchor: o.anchor,
        reference_id: format!("card-{item}:{}", t.fingerprint),
        item_index: item,
        rect: p.clone(),
        angle: c.angle.rem_euclid(360.0),
        fill: c.fill,
        scale,
        offset: [c.dx, c.dy],
        evidence_pixels: c.counts[o.anchor],
        effective_evidence: effective,
        color_error: c.score,
        edge_error,
        predicted_mismatch,
        unexplained_foreground,
        scores,
        accepted: scores.map(|s| s <= cfg.max_color_error),
        tile_template: None,
        visible_evidence: Some(visible::measure(image, c, cells, cfg, |x, y| {
            let (sx, sy) = r.source((x as f64 + 0.5 - ox) / scale, (y as f64 + 0.5 - oy) / scale);
            let mask = mask_at(&t.mask, sx, sy);
            (
                mask,
                if mask {
                    sample(&t.image, sx, sy)
                } else {
                    [0; 3]
                },
            )
        })),
        confidence_weighted: cfg.confidence_weighted_scoring.then(|| {
            visible::measure_core(
                image,
                c,
                cells,
                cfg,
                cfg.edge_weight * edge_error
                    + cfg.reverse_weight * (predicted_mismatch + unexplained_foreground),
                |x, y| {
                    let (sx, sy) =
                        r.source((x as f64 + 0.5 - ox) / scale, (y as f64 + 0.5 - oy) / scale);
                    (mask_at(&t.core, sx, sy), sample(&t.image, sx, sy))
                },
            )
        }),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        // Original search parameters, independent of the report's experiment
        // switches so the disabled control remains a stable regression.
        Config {
            coarse_angle_step: 10,
            fills: vec![0.84, 0.92, 0.99],
            aspect_log_tolerance: 0.4,
            max_color_error: 38.0,
            min_anchor_pixels: 45,
            edge_weight: 0.12,
            reverse_weight: 10.0,
            foreground_chroma: 30,
            foreground_dark: 190,
            reliable_border: 3,
            low_confidence_weight: 0.1,
            low_confidence_error_cap: None,
            mismatch_error: 35.0,
            min_texture_std: 6.0,
            refine: vec![
                Refine {
                    angles: vec![-6.0, -4.0, -2.0, 0.0, 2.0, 4.0, 6.0],
                    fills: vec![-0.04, -0.02, 0.0, 0.02, 0.04],
                    offsets: vec![-1.5, 0.0, 1.5],
                },
                Refine {
                    angles: vec![-1.0, 0.0, 1.0],
                    fills: vec![-0.01, 0.0, 0.01],
                    offsets: vec![-0.75, 0.0, 0.75],
                },
            ],
            coarse_offset_rescue: false,
            foreground_refinement: false,
            confidence_weighted_scoring: false,
            threshold_free_search: false,
            max_evaluations: 0,
            tile_cache_bytes: default_tile_cache_bytes(),
            extraction: ExtractionConfig {
                enabled: true,
                ..ExtractionConfig::default()
            },
        }
    }

    fn old_config() -> Config {
        let mut value = serde_json::to_value(config()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("coarse_offset_rescue");
        value
            .as_object_mut()
            .unwrap()
            .remove("low_confidence_error_cap");
        value
            .as_object_mut()
            .unwrap()
            .remove("foreground_refinement");
        value
            .as_object_mut()
            .unwrap()
            .remove("confidence_weighted_scoring");
        value
            .as_object_mut()
            .unwrap()
            .remove("threshold_free_search");
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn observation_border_count_uses_existing_confidence_without_lowering_minimum() {
        let cfg = config();
        for anchor in [0, COLS * ROWS - 1] {
            let mut image = RgbaImage::from_pixel(
                (COLS * MATCH_CELL) as u32,
                (ROWS * MATCH_CELL) as u32,
                Rgba([240, 240, 240, 255]),
            );
            let (ox, oy) = (anchor % COLS * MATCH_CELL, anchor / COLS * MATCH_CELL);
            for n in 0..43 {
                image.put_pixel(
                    (ox + 3 + n % 26) as u32,
                    (oy + 3 + n / 26) as u32,
                    Rgba([if n % 2 == 0 { 120 } else { 180 }, 30, 50, 255]),
                );
            }
            let mut added = 0;
            for y in 0..MATCH_CELL {
                for x in 0..MATCH_CELL {
                    if (x < 3 || x >= 29 || y < 3 || y >= 29) && added < 109 {
                        image.put_pixel(
                            (ox + x) as u32,
                            (oy + y) as u32,
                            Rgba([230, 180, 80, 255]),
                        );
                        added += 1;
                    }
                }
            }
            let o = observe_cell(&image, "weighted-border", anchor, &cfg);
            assert_eq!(o.reliable_foreground_pixels, 43);
            assert_eq!(o.border_reliable_foreground_pixels, 109);
            assert!((o.effective_foreground_pixels - 53.9).abs() < 1e-10);
            assert!(o.texture_std >= 6.0 && o.sufficient_evidence);
            let mut no_border_weight = cfg.clone();
            no_border_weight.low_confidence_weight = 0.0;
            let control = observe_cell(&image, "zero-border-weight", anchor, &no_border_weight);
            assert_eq!(control.effective_foreground_pixels, 43.0);
            assert_eq!(control.texture_std, o.texture_std);
            assert!(!control.sufficient_evidence);
        }
    }

    #[test]
    fn observation_grid_border_and_flat_interior_cannot_supply_texture() {
        let cfg = config();
        let mut image = RgbaImage::from_pixel(
            (COLS * MATCH_CELL) as u32,
            (ROWS * MATCH_CELL) as u32,
            Rgba([240, 240, 240, 255]),
        );
        for y in 0..MATCH_CELL {
            for x in 0..MATCH_CELL {
                if x < 3 || x >= 29 || y < 3 || y >= 29 {
                    image.put_pixel(
                        x as u32,
                        y as u32,
                        if (x + y) % 2 == 0 {
                            Rgba([100, 100, 100, 255])
                        } else {
                            Rgba([230, 180, 80, 255])
                        },
                    );
                }
            }
        }
        let border_only = observe_cell(&image, "border-only", 0, &cfg);
        assert_eq!(border_only.border_reliable_foreground_pixels, 348);
        assert!((border_only.effective_foreground_pixels - 34.8).abs() < 1e-10);
        assert_eq!(border_only.texture_std, 0.0);
        assert!(!border_only.sufficient_evidence);
        for n in 0..43 {
            image.put_pixel(
                (3 + n % 26) as u32,
                (3 + n / 26) as u32,
                Rgba([80, 80, 80, 255]),
            );
        }
        let flat = observe_cell(&image, "flat-interior", 0, &cfg);
        assert!(flat.effective_foreground_pixels > cfg.min_anchor_pixels as f64);
        assert_eq!(
            flat.texture_std, 0.0,
            "grid contrast must not count as fragment texture"
        );
        assert!(!flat.sufficient_evidence);
    }

    #[test]
    fn observation_selection_green_is_excluded_interior_and_border() {
        let image = RgbaImage::from_pixel(
            (COLS * MATCH_CELL) as u32,
            (ROWS * MATCH_CELL) as u32,
            Rgba([100, 220, 50, 255]),
        );
        let o = observe_cell(&image, "selection", 0, &config());
        assert_eq!(o.reliable_foreground_pixels, 0);
        assert_eq!(o.border_reliable_foreground_pixels, 0);
        assert_eq!(o.effective_foreground_pixels, 0.0);
        assert!(!o.sufficient_evidence);
    }

    fn comparable_poses(output: &Output) -> serde_json::Value {
        let poses: Vec<_> = output
            .candidates
            .iter()
            .cloned()
            .map(|mut pose| {
                pose.tile_template = None;
                pose
            })
            .collect();
        serde_json::to_value(poses).unwrap()
    }

    #[test]
    fn optional_experiments_default_off_and_color_cap_requires_positive_finite_error() {
        let old = old_config();
        assert!(!old.coarse_offset_rescue);
        assert!(!old.foreground_refinement);
        assert!(!old.confidence_weighted_scoring);
        assert!(!old.threshold_free_search);
        assert_eq!(old.low_confidence_error_cap, None);
        assert_eq!(
            serde_json::to_value(&old).unwrap(),
            serde_json::to_value(config()).unwrap()
        );
        old.validate().unwrap();
        let mut cfg = config();
        for cap in [
            0.0,
            -1.0,
            255.01,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ] {
            cfg.low_confidence_error_cap = Some(cap);
            assert!(cfg.validate().is_err(), "invalid cap {cap}");
        }
        for cap in [0.1, 35.0, 255.0] {
            cfg.low_confidence_error_cap = Some(cap);
            cfg.validate().unwrap();
        }
    }

    #[test]
    fn real_phone_coarse_rescue_is_exact_across_backends_and_obeys_budget() {
        let frame = image::load_from_memory(include_bytes!(
            "../../tests/fixtures/vision-user-20261006-phone-strap.png"
        ))
        .unwrap()
        .to_rgba8();
        let analysis = super::super::super::Recognizer::new().analyze_completed_snapshot(
            &frame,
            None,
            Some(30),
            [Some(0), Some(5), Some(2)],
        );
        assert_eq!(analysis.cells[5], "uncertain");
        assert_eq!(analysis.shapes[1], [3, 1]);
        let truth = GridPlacement {
            x: 3,
            y: 0,
            width: 3,
            height: 1,
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let out = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.artifacts/partial-recognition/strap-engineering")
            .join(format!("module-rescue-{stamp}"));
        let run = |cfg: &Config, backend: Backend, name: &str| {
            let folder = out.join(name);
            std::fs::create_dir_all(&folder).unwrap();
            Session::default()
                .run_backend(&frame, &analysis, "real-phone", cfg, &folder, backend)
                .unwrap()
        };
        let mut cfg = old_config();
        let control = run(&cfg, Backend::Direct, "control");
        assert!(control.complete);
        assert_eq!(control.rescue_evaluations, 0);
        assert_eq!(control.recovered_basins, 0);
        assert!(!control
            .candidates
            .iter()
            .any(|p| p.item_index == 1 && p.rect == truth));
        cfg.coarse_offset_rescue = true;
        cfg.low_confidence_error_cap = Some(35.0);
        let direct = run(&cfg, Backend::Direct, "direct");
        let tiled = run(&cfg, Backend::Tiled, "tiled");
        assert!(direct.complete && tiled.complete);
        assert!(direct.rescue_evaluations > 0);
        assert!(direct.recovered_basins > 0);
        assert_eq!(direct.evaluations, tiled.evaluations);
        assert_eq!(direct.rescue_evaluations, tiled.rescue_evaluations);
        assert_eq!(direct.recovered_basins, tiled.recovered_basins);
        assert_eq!(comparable_poses(&direct), comparable_poses(&tiled));
        assert!(direct.candidates.iter().any(|p| {
            let visible = p.visible_evidence.as_ref().unwrap();
            p.item_index == 1
                && p.rect == truth
                && p.color_error <= 38.0
                && p.evidence_pixels >= 45
                && visible.capped_pixels > 0
                && visible.foreground_color_error < visible.uncapped_foreground_color_error
        }));
        // Retained coarse rescue poses include actual offsets, before the
        // unchanged refinement selects a basin's best original RGB score.
        assert!(
            direct
                .candidates
                .iter()
                .filter(|p| {
                    p.item_index == 1
                        && p.rect == truth
                        && p.angle % 10.0 == 0.0
                        && p.offset != [0.0, 0.0]
                        && cfg.fills.contains(&p.fill)
                        && p.offset.iter().all(|v| cfg.refine[0].offsets.contains(v))
                })
                .count()
                > 1
        );
        // This first phone basin has three rejected original fills. The next
        // evaluation is the first offset retry, so this cuts inside rescue.
        cfg.max_evaluations = 4;
        let limited_direct = run(&cfg, Backend::Direct, "limited-direct");
        let limited_tiled = run(&cfg, Backend::Tiled, "limited-tiled");
        for limited in [&limited_direct, &limited_tiled] {
            assert!(!limited.complete);
            assert_eq!(limited.evaluations, 4);
            assert_eq!(limited.rescue_evaluations, 1);
            assert!(limited
                .interruption
                .as_ref()
                .unwrap()
                .contains("no unique conclusion allowed"));
        }
        assert_eq!(
            comparable_poses(&limited_direct),
            comparable_poses(&limited_tiled)
        );
        assert_eq!(
            limited_direct.recovered_basins,
            limited_tiled.recovered_basins
        );
    }

    #[test]
    fn real_ring_foreground_route_preserves_rgb_control_and_backend_parity() {
        let frame = image::load_from_memory(include_bytes!(
            "../../tests/fixtures/vision-user-20261007-swim-ring.png"
        ))
        .unwrap()
        .to_rgba8();
        let analysis = super::super::super::Recognizer::new().analyze_completed_snapshot(
            &frame,
            None,
            Some(38),
            [Some(1), Some(3), Some(2)],
        );
        assert_eq!(analysis.cells[20], "uncertain");
        assert_eq!(analysis.shapes[0], [3, 3]);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let out = std::env::temp_dir().join(format!("ba-foreground-refinement-{stamp}"));
        let run = |cfg: &Config, backend: Backend, name: &str| {
            let folder = out.join(name);
            std::fs::create_dir_all(&folder).unwrap();
            Session::default()
                .run_backend(&frame, &analysis, "real-ring", cfg, &folder, backend)
                .unwrap()
        };
        let mut cfg = config();
        cfg.coarse_offset_rescue = true;
        cfg.low_confidence_error_cap = Some(35.0);
        let control = run(&cfg, Backend::Direct, "disabled");
        assert!(control.complete);
        assert_eq!(control.foreground_evaluations, 0);
        assert_eq!(control.foreground_basins, 0);
        let truth = GridPlacement {
            x: 0,
            y: 0,
            width: 3,
            height: 3,
        };
        assert!(control
            .candidates
            .iter()
            .any(|p| p.item_index == 0 && p.rect == truth));
        cfg.foreground_refinement = true;
        let direct = run(&cfg, Backend::Direct, "direct");
        let tiled = run(&cfg, Backend::Tiled, "tiled");
        assert!(direct.complete && tiled.complete);
        assert!(direct.foreground_evaluations > 0 && direct.foreground_basins > 0);
        assert_eq!(direct.evaluations, tiled.evaluations);
        assert_eq!(direct.foreground_evaluations, tiled.foreground_evaluations);
        assert_eq!(direct.foreground_basins, tiled.foreground_basins);
        assert_eq!(comparable_poses(&direct), comparable_poses(&tiled));
        let control_poses = comparable_poses(&control);
        let enabled_poses = comparable_poses(&direct);
        for pose in control_poses.as_array().unwrap() {
            assert!(
                enabled_poses.as_array().unwrap().contains(pose),
                "the independent foreground route removed an original RGB pose"
            );
        }
        let best_combined = |output: &Output| {
            output
                .candidates
                .iter()
                .filter(|p| p.item_index == 0 && p.rect == truth)
                .filter_map(foreground_objective)
                .min_by(f64::total_cmp)
                .expect("the correct ring rectangle has measured foreground evidence")
        };
        let control_best = best_combined(&control);
        let enabled_best = best_combined(&direct);
        assert!(
            enabled_best < control_best,
            "foreground refinement did not improve the correct ring rectangle: {enabled_best} >= {control_best}"
        );
        assert!(direct.candidates.iter().all(|p| {
            p.color_error <= cfg.max_color_error && p.evidence_pixels >= cfg.min_anchor_pixels
        }));
        cfg.confidence_weighted_scoring = true;
        let weighted = run(&cfg, Backend::Direct, "weighted-direct");
        let weighted_tiled = run(&cfg, Backend::Tiled, "weighted-tiled");
        assert!(weighted.complete && weighted_tiled.complete);
        assert_eq!(
            comparable_poses(&weighted),
            comparable_poses(&weighted_tiled)
        );
        assert!(best_combined(&weighted) <= cfg.max_color_error);
        assert!(
            weighted.candidates.iter().any(|p| {
                p.item_index == 0
                    && p.rect == truth
                    && p.scores[2] > cfg.max_color_error
                    && foreground_objective(p).is_some_and(|s| s <= cfg.max_color_error)
            }),
            "the ring must pass through confidence consistency, not a relaxed threshold"
        );
        assert!(weighted.candidates.iter().all(|p| {
            p.color_error <= cfg.max_color_error && p.evidence_pixels >= cfg.min_anchor_pixels
        }));
    }

    #[test]
    fn real_bow_keeps_original_periodic_angle_transform_evidence() {
        let frame = image::load_from_memory(include_bytes!(
            "../../tests/fixtures/vision-user-20261007-bow.png"
        ))
        .unwrap()
        .to_rgba8();
        let analysis = super::super::super::Recognizer::new().analyze_completed_snapshot(
            &frame,
            None,
            Some(44),
            [Some(1), Some(4), Some(3)],
        );
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let out = std::env::temp_dir().join(format!("ba-foreground-bow-{stamp}"));
        let run = |cfg: &Config, name: &str| {
            let folder = out.join(name);
            std::fs::create_dir_all(&folder).unwrap();
            Session::default()
                .run_backend(&frame, &analysis, "real-bow", cfg, &folder, Backend::Direct)
                .unwrap()
        };
        let mut cfg = config();
        cfg.coarse_offset_rescue = true;
        cfg.low_confidence_error_cap = Some(35.0);
        let control = run(&cfg, "disabled");
        cfg.foreground_refinement = true;
        let enabled = run(&cfg, "enabled");
        assert!(control.complete && enabled.complete);
        let before = comparable_poses(&control);
        let after = comparable_poses(&enabled);
        // -3 and 357 degrees can report the same canonical angle while their
        // computed scales differ slightly. Neither transform may replace the other.
        for pose in before.as_array().unwrap() {
            assert!(after.as_array().unwrap().contains(pose));
        }
    }

    fn live_cache_fixture() -> (RgbaImage, super::super::super::Analysis, [f64; 4], Config) {
        let frame = image::load_from_memory(include_bytes!(
            "../../tests/fixtures/vision-pc-user-20261008-remaining44-partial.png"
        ))
        .unwrap()
        .to_rgba8();
        let analysis = super::super::super::Recognizer::new().analyze_completed_snapshot(
            &frame, None, Some(44), [Some(1), Some(4), Some(3)],
        );
        assert!(analysis.present);
        assert_eq!(analysis.cells[10], "uncertain");
        assert!(analysis.cells.iter().enumerate().all(|(i, s)| i == 10 || s == "unknown"));
        let content = super::super::super::locate_content(&frame).unwrap();
        let value: serde_json::Value = serde_json::from_str(include_str!("config.json")).unwrap();
        let cfg = serde_json::from_value(value["matching"].clone()).unwrap();
        (frame, analysis, content, cfg)
    }

    fn paint_live_cell(frame: &mut RgbaImage, board: [f64; 4], cell: usize, color: Rgba<u8>) {
        let x0 = board[0] * frame.width() as f64;
        let y0 = board[1] * frame.height() as f64;
        let w = board[2] * frame.width() as f64 / COLS as f64;
        let h = board[3] * frame.height() as f64 / ROWS as f64;
        for y in (y0 + (cell / COLS) as f64 * h + 2.0).ceil() as u32
            ..(y0 + (cell / COLS + 1) as f64 * h - 2.0).floor() as u32
        {
            for x in (x0 + (cell % COLS) as f64 * w + 2.0).ceil() as u32
                ..(x0 + (cell % COLS + 1) as f64 * w - 2.0).floor() as u32
            {
                frame.put_pixel(x, y, color);
            }
        }
    }

    #[test]
    fn direct_batch_exact_parity_includes_anchor_gates_and_foreground_refinement() {
        let (frame, _, content, mut cfg) = live_cache_fixture();
        let viewport = super::super::super::content_rect(&frame, content).unwrap();
        let t = extract(read_card(&frame, viewport, 0)).unwrap();
        let mut c = Candidate {
            placement: GridPlacement { x: 1, y: 1, width: 3, height: 3 },
            angle: 10.0, fill: 0.92, dx: 0.0, dy: 0.0, score: 255.0, counts: [0; 45],
        };
        let r = Rotation::new(&t, c.angle);
        let (w, h) = (c.placement.width * MATCH_CELL, c.placement.height * MATCH_CELL);
        let scale = (w as f64 / r.w).min(h as f64 / r.h) * c.fill;
        let (ox, oy) = ((w as f64 - r.w * scale) / 2.0, (h as f64 - r.h * scale) / 2.0);
        let mut image = RgbaImage::from_pixel((COLS * MATCH_CELL) as u32,
            (ROWS * MATCH_CELL) as u32, Rgba([255, 255, 255, 255]));
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) = r.source((x as f64 + 0.5 - ox) / scale,
                    (y as f64 + 0.5 - oy) / scale);
                if mask_at(&t.mask, sx, sy) {
                    let pixel = sample(&t.image, sx, sy);
                    image.put_pixel((c.placement.x * MATCH_CELL + x) as u32,
                        (c.placement.y * MATCH_CELL + y) as u32,
                        Rgba([pixel[0], pixel[1], pixel[2], 255]));
                }
            }
        }
        let mut cells = vec!["unknown".to_owned(); COLS * ROWS];
        for y in c.placement.y..c.placement.y + c.placement.height {
            for x in c.placement.x..c.placement.x + c.placement.width {
                cells[y * COLS + x] = "uncertain".into();
            }
        }
        let observations: Vec<_> = cells.iter().enumerate()
            .filter(|(_, cell)| cell.as_str() == "uncertain")
            .map(|(anchor, _)| observe_cell(&image, "batch", anchor, &cfg)).collect();
        let anchors: Vec<_> = observations.iter().collect();
        c = evaluate(&image, &t, r, &c, &cells).unwrap();
        assert!(anchors.iter().filter(|o| c.counts[o.anchor] >= cfg.min_anchor_pixels).count() > 1);

        for threshold_free in [false, true] {
            cfg.threshold_free_search = threshold_free;
            for score in [c.score, cfg.max_color_error + 1.0] {
                for supported in [0, 1, anchors.len() - 1] {
                    let mut candidate = c.clone();
                    candidate.score = score;
                    candidate.counts.fill(0);
                    candidate.counts[anchors[0].anchor] = cfg.min_anchor_pixels - 1;
                    for (index, anchor) in anchors.iter().skip(1).take(supported).enumerate() {
                        candidate.counts[anchor.anchor] = cfg.min_anchor_pixels + index;
                    }
                    let mut reference = Vec::new();
                    for anchor in &anchors {
                        push_backend_pose(&mut reference, &image, &t, &candidate, anchor,
                            0, &cells, &cfg, None, 7);
                    }
                    let mut batch = Vec::new();
                    push_backend_poses(&mut batch, &image, &t, &candidate, &anchors,
                        0, &cells, &cfg, None, 7);
                    assert_eq!(serde_json::to_value(&batch).unwrap(),
                        serde_json::to_value(&reference).unwrap(),
                        "threshold_free={threshold_free}, score={score}, supported={supported}");
                    assert_eq!(batch.len(), if threshold_free || score <= cfg.max_color_error {
                        supported
                    } else { 0 });
                    let mut batch_seed = None;
                    let mut reference_seed = None;
                    consider_foreground_seed(&mut batch_seed, &candidate, &batch);
                    consider_foreground_seed(&mut reference_seed, &candidate, &reference);
                    assert_eq!(batch_seed.map(|seed| seed.score.to_bits()),
                        reference_seed.map(|seed| seed.score.to_bits()));
                }
            }
        }

        // Tiled still scores each anchor separately, providing the unchanged
        // reference for foreground seed updates, retained poses and metric minima.
        cfg.refine = vec![Refine {
            angles: vec![0.0, 1.0], fills: vec![0.0, -0.01], offsets: vec![0.0, 0.75],
        }];
        for threshold_free in [false, true] {
            cfg.threshold_free_search = threshold_free;
            let mut original = Vec::new();
            for anchor in &anchors {
                push_backend_pose(&mut original, &image, &t, &c, anchor,
                    0, &cells, &cfg, None, 7);
            }
            let seed_score = foreground_objective(&original[0]).unwrap();
            let mut outcomes = Vec::new();
            for backend in [Backend::Direct, Backend::Tiled] {
                let mut cache = tiled::Cache::default();
                cache.begin_run(cfg.tile_cache_bytes);
                let mut poses = Vec::new();
                let mut minima = ThresholdFreeMinima::new();
                let (mut evaluations, mut foreground_evaluations) = (0, 0);
                assert!(refine_foreground(&mut cache, backend, 0, 7, &image, &t, &anchors,
                    &cells, &cfg, ForegroundSeed { candidate: c.clone(), score: seed_score },
                    &mut poses, &mut minima, &mut evaluations, &mut foreground_evaluations, None));
                assert!(foreground_evaluations > 0);
                if threshold_free {
                    assert!(minima.len() > 1);
                    assert!(poses.is_empty());
                } else {
                    assert!(poses.len() > 1);
                }
                for pose in &mut poses { pose.tile_template = None; }
                let metric_poses: Vec<_> = minima.into_values().flatten().flatten()
                    .map(|mut pose| { pose.tile_template = None; pose }).collect();
                outcomes.push((serde_json::to_value(poses).unwrap(),
                    serde_json::to_value(metric_poses).unwrap(), evaluations, foreground_evaluations));
            }
            assert_eq!(outcomes[0], outcomes[1], "foreground threshold_free={threshold_free}");
        }
    }

    fn assert_exact_live_search(a: &Output, b: &Output) {
        assert_eq!(a.complete, b.complete);
        assert_eq!(a.interruption, b.interruption);
        // Serialized floats retain their exact round-trippable value, including
        // signed zero. Compare the entire ordered pose schema, not just winners.
        assert_eq!(
            serde_json::to_vec(&a.candidates).unwrap(),
            serde_json::to_vec(&b.candidates).unwrap()
        );
        assert_eq!(
            [
                a.evaluations,
                a.rescue_evaluations,
                a.recovered_basins,
                a.foreground_evaluations,
                a.foreground_basins
            ],
            [
                b.evaluations,
                b.rescue_evaluations,
                b.recovered_basins,
                b.foreground_evaluations,
                b.foreground_basins
            ],
        );
    }

    #[test]
    fn parallel_live_exact_cold_hot_changed_and_budgeted_serial_parity() {
        let (mut frame, mut analysis, content, cfg) = live_cache_fixture();
        let mut serial = Session {
            test_live_workers: Some(1),
            ..Session::default()
        };
        let mut parallel = Session {
            test_live_workers: Some(4),
            ..Session::default()
        };
        for step in 0..6 {
            match step {
                2 => analysis.cells[44] = "uncertain".into(),
                3 => analysis.cells[19] = "uncertain".into(),
                4 => paint_live_cell(
                    &mut frame,
                    analysis.board.unwrap(),
                    10,
                    Rgba([125, 80, 170, 255]),
                ),
                5 => analysis.cells[19] = "unknown".into(),
                _ => {}
            }
            let id = format!("parallel-{step}");
            let a = serial
                .run_live(&frame, &analysis, content, &id, &cfg, None)
                .unwrap();
            let b = parallel
                .run_live(&frame, &analysis, content, &id, &cfg, None)
                .unwrap();
            assert!(a.complete && b.complete);
            assert_exact_live_search(&a, &b);
            if step == 0 {
                assert!(!a.candidates.is_empty());
                assert!(a.rescue_evaluations > 0 && a.foreground_evaluations > 0);
                // A nonzero global evaluation budget uses the retained serial
                // path, independently of the live worker override.
                let mut budgeted = cfg.clone();
                budgeted.max_evaluations = u64::MAX;
                let original = Session::default()
                    .run_live(&frame, &analysis, content, &id, &budgeted, None)
                    .unwrap();
                assert_exact_live_search(&a, &original);
            } else if step == 1 {
                assert_eq!(a.evaluations, 0);
            } else {
                assert!(a.evaluations > 0);
            }
        }
    }

    #[test]
    fn parallel_live_deadline_keeps_only_complete_cache_entries() {
        let (mut frame, mut analysis, content, cfg) = live_cache_fixture();
        let mut session = Session {
            test_live_workers: Some(4),
            ..Session::default()
        };
        let cold = session
            .run_live(
                &frame,
                &analysis,
                content,
                "deadline",
                &cfg,
                Some(Duration::ZERO),
            )
            .unwrap();
        assert!(!cold.complete && cold.evaluations == 0);
        assert!(session.live_rectangles.is_empty());
        let first = session
            .run_live(&frame, &analysis, content, "deadline", &cfg, None)
            .unwrap();
        assert!(first.complete);
        let cached = session.live_rectangles.len();
        let hot = session
            .run_live(
                &frame,
                &analysis,
                content,
                "deadline",
                &cfg,
                Some(Duration::ZERO),
            )
            .unwrap();
        assert!(hot.complete && hot.evaluations == 0);
        assert_eq!(
            serde_json::to_vec(&first.candidates).unwrap(),
            serde_json::to_vec(&hot.candidates).unwrap()
        );
        analysis.cells[44] = "uncertain".into();
        let mixed = session
            .run_live(
                &frame,
                &analysis,
                content,
                "deadline",
                &cfg,
                Some(Duration::ZERO),
            )
            .unwrap();
        assert!(!mixed.complete && mixed.evaluations == 0);
        assert_eq!(session.live_rectangles.len(), cached);
        assert_eq!(
            serde_json::to_vec(&first.candidates).unwrap(),
            serde_json::to_vec(&mixed.candidates).unwrap()
        );
        assert!(
            session
                .run_live(&frame, &analysis, content, "deadline", &cfg, None)
                .unwrap()
                .complete
        );
        assert!(session.live_rectangles.len() > cached);
        paint_live_cell(
            &mut frame,
            analysis.board.unwrap(),
            10,
            Rgba([125, 80, 170, 255]),
        );
        let changed = session
            .run_live(
                &frame,
                &analysis,
                content,
                "deadline",
                &cfg,
                Some(Duration::ZERO),
            )
            .unwrap();
        assert!(!changed.complete && changed.evaluations == 0);
        assert!(
            !session.live_rectangles.is_empty(),
            "unaffected complete rectangles survive"
        );
        assert!(session
            .live_rectangles
            .keys()
            .all(|key| !key.cells().any(|cell| cell == 10)));
        let resumed = session
            .run_live(&frame, &analysis, content, "deadline", &cfg, None)
            .unwrap();
        let fresh = Session {
            test_live_workers: Some(1),
            ..Session::default()
        }
        .run_live(&frame, &analysis, content, "deadline", &cfg, None)
        .unwrap();
        assert!(resumed.complete && fresh.complete);
        assert_eq!(
            serde_json::to_vec(&resumed.candidates).unwrap(),
            serde_json::to_vec(&fresh.candidates).unwrap()
        );
        assert!(resumed.evaluations < fresh.evaluations);
    }

    #[test]
    fn parallel_live_preserves_global_evaluation_budget_between_rectangles() {
        let (frame, analysis, content, mut cfg) = live_cache_fixture();
        cfg.coarse_angle_step = 360;
        cfg.fills = vec![0.92];
        cfg.refine.clear();
        cfg.coarse_offset_rescue = false;
        cfg.foreground_refinement = false;
        cfg.max_evaluations = 2;
        let mut session = Session {
            test_live_workers: Some(4),
            ..Session::default()
        };
        let output = session
            .run_live(&frame, &analysis, content, "budget", &cfg, None)
            .unwrap();
        assert!(!output.complete);
        assert_eq!(output.evaluations, 2);
        assert_eq!(
            session.live_rectangles.len(),
            2,
            "the two completed rectangles stay reusable"
        );
    }

    #[test]
    fn parallel_live_unfinished_angle_part_cannot_enter_rectangle_cache() {
        let results = merge_live_search_parts(
            2,
            vec![
                (
                    0,
                    SearchState {
                        evaluations: 3,
                        ..SearchState::default()
                    },
                    true,
                ),
                (
                    0,
                    SearchState {
                        evaluations: 2,
                        ..SearchState::default()
                    },
                    false,
                ),
                (
                    0,
                    SearchState {
                        evaluations: 7,
                        ..SearchState::default()
                    },
                    true,
                ),
                (
                    1,
                    SearchState {
                        evaluations: 4,
                        ..SearchState::default()
                    },
                    true,
                ),
                (
                    1,
                    SearchState {
                        evaluations: 5,
                        ..SearchState::default()
                    },
                    true,
                ),
            ],
        );
        assert_eq!(results[0].0.evaluations, 12);
        assert!(
            !results[0].1,
            "later completed angles cannot erase an interruption"
        );
        assert_eq!(results[1].0.evaluations, 9);
        assert!(results[1].1);
        let mut session = Session::default();
        for (x, (result, complete)) in results.into_iter().enumerate() {
            cache_complete_rectangle(
                &mut session.live_rectangles,
                RectangleKey {
                    item_index: 0,
                    x,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                RectangleInput {
                    template_revision: 1,
                    cells: vec!["uncertain".into()],
                    pixels: vec![],
                    anchors: vec![x],
                },
                &result,
                complete,
            );
        }
        assert_eq!(session.live_rectangles.len(), 1);
        assert_eq!(session.live_rectangles.keys().next().unwrap().x, 1);
    }

    #[test]
    fn parallel_live_worker_panic_returns_error() {
        let (frame, analysis, content, cfg) = live_cache_fixture();
        let viewport = super::super::super::content_rect(&frame, content).unwrap();
        let template = extract(read_card(&frame, viewport, 0)).unwrap();
        let rotations = (0..360)
            .step_by(cfg.coarse_angle_step)
            .map(|angle| (angle, Rotation::new(&template, angle as f64)))
            .collect();
        let observation = observe_cell(&frame, "panic", 10, &cfg);
        let jobs: Vec<_> = (0..2)
            .map(|output_index| LiveSearchJob {
                rectangle: RectangleKey {
                    item_index: 0,
                    x: 0,
                    y: 0,
                    width: 3,
                    height: 3,
                },
                input: RectangleInput {
                    template_revision: 1,
                    cells: Vec::new(),
                    pixels: Vec::new(),
                    anchors: vec![10],
                },
                anchors: vec![&observation],
                template: &template,
                rotations: &rotations,
                output_index,
            })
            .collect();
        // Malformed private worker input deliberately trips the evaluator's cell
        // indexing; public run_internal validates this input before scheduling.
        let error = search_live_jobs(&jobs, &frame, &[], &cfg, None, 2)
            .err()
            .unwrap();
        assert_eq!(error, "live rectangle search worker panicked");
        assert_eq!(analysis.cells.len(), COLS * ROWS);
    }

    #[test]
    fn live_cache_keeps_all_poses_rebinds_ids_and_ignores_unknown_pixels() {
        let (mut frame, analysis, content, cfg) = live_cache_fixture();
        let mut session = Session::default();
        let first = session.run_live(&frame, &analysis, content, "first", &cfg, None).unwrap();
        assert!(first.complete && first.candidates.len() > 1);
        assert!(session.live_rectangles.values().any(|entry| entry.poses.is_empty()));
        let fresh = Session::default().run_live(&frame, &analysis, content, "next", &cfg, None).unwrap();
        let reused = session.run_live(&frame, &analysis, content, "next", &cfg, Some(Duration::ZERO)).unwrap();
        assert!(reused.complete);
        assert_eq!(reused.evaluations, 0);
        assert_eq!(comparable_poses(&reused), comparable_poses(&fresh));
        assert_eq!(serde_json::to_value(&reused.observations).unwrap(), serde_json::to_value(&fresh.observations).unwrap());
        assert_eq!(serde_json::to_value(&reused.cards).unwrap(), serde_json::to_value(&fresh.cards).unwrap());

        paint_live_cell(&mut frame, analysis.board.unwrap(), 0, Rgba([255, 0, 255, 255]));
        let covered = session.run_live(&frame, &analysis, content, "next", &cfg, Some(Duration::ZERO)).unwrap();
        assert!(covered.complete);
        assert_eq!(covered.evaluations, 0);
        assert_eq!(comparable_poses(&covered), comparable_poses(&fresh));
    }

    #[test]
    fn live_cache_incremental_far_and_near_observations_match_full_search() {
        let (frame, mut analysis, content, cfg) = live_cache_fixture();
        let mut session = Session::default();
        assert!(session.run_live(&frame, &analysis, content, "sequence", &cfg, None).unwrap().complete);
        for (cell, state) in [(44, "uncertain"), (19, "uncertain"), (19, "unknown")] {
            analysis.cells[cell] = state.into();
            let incremental = session.run_live(&frame, &analysis, content, "sequence", &cfg, None).unwrap();
            let fresh = Session::default().run_live(&frame, &analysis, content, "sequence", &cfg, None).unwrap();
            assert!(incremental.complete && fresh.complete);
            assert!(incremental.evaluations > 0);
            assert!(incremental.evaluations < fresh.evaluations);
            assert_eq!(comparable_poses(&incremental), comparable_poses(&fresh));
        }
    }

    #[test]
    fn live_cache_budget_cannot_hide_new_search_or_restore_retired_evidence() {
        let (frame, analysis, content, cfg) = live_cache_fixture();
        let mut session = Session::default();
        assert!(session.run_live(&frame, &analysis, content, "budget", &cfg, None).unwrap().complete);
        let mut additional = analysis.clone();
        additional.cells[44] = "uncertain".into();
        let pending = session.run_live(&frame, &additional, content, "budget", &cfg, Some(Duration::ZERO)).unwrap();
        assert!(!pending.complete);
        assert_eq!(pending.evaluations, 0);
        assert!(pending.candidates.iter().all(|pose| pose.anchor == 10));
        let first_rectangle = RectangleKey { item_index: 0, x: 0, y: 0, width: 3, height: 3 };
        assert!(session.live_rectangles.contains_key(&first_rectangle));
        let mut changed = frame.clone();
        paint_live_cell(&mut changed, analysis.board.unwrap(), 10, Rgba([125, 80, 170, 255]));
        let pending = session.run_live(&changed, &analysis, content, "budget", &cfg, Some(Duration::ZERO)).unwrap();
        assert!(!pending.complete);
        assert!(!session.live_rectangles.contains_key(&first_rectangle));
        let restored = session.run_live(&frame, &analysis, content, "budget", &cfg, Some(Duration::ZERO)).unwrap();
        assert!(!restored.complete, "interrupted rectangle cannot recover a historical cache entry");
        session.reset();
        assert!(session.live_rectangles.is_empty());
        assert!(session.live_context.is_none());
    }

    #[test]
    fn live_cache_context_and_exact_template_changes_invalidate_evidence() {
        let (frame, analysis, content, cfg) = live_cache_fixture();
        for change in ["config", "shapes", "board", "content", "dimensions"] {
            let mut session = Session::default();
            assert!(session.run_live(&frame, &analysis, content, "context", &cfg, None).unwrap().complete);
            let mut changed_frame = frame.clone();
            let mut changed_analysis = analysis.clone();
            let mut changed_content = content;
            let mut changed_cfg = cfg.clone();
            match change {
                "config" => changed_cfg.reverse_weight += 0.1,
                "shapes" => changed_analysis.shapes[1] = [1, 1],
                "board" => changed_analysis.board.as_mut().unwrap()[0] += 1e-12,
                "content" => changed_content[0] += 1e-12,
                "dimensions" => {
                    changed_frame = RgbaImage::new(frame.width() + 1, frame.height());
                    image::imageops::replace(&mut changed_frame, &frame, 0, 0);
                }
                _ => unreachable!(),
            }
            let pending = session.run_live(&changed_frame, &changed_analysis, changed_content, "context", &changed_cfg, Some(Duration::ZERO)).unwrap();
            assert!(!pending.complete, "{change}");
            assert!(session.live_rectangles.is_empty(), "{change}");
        }

        let mut session = Session::default();
        session.run_live(&frame, &analysis, content, "template", &cfg, None).unwrap();
        let previous = session.templates.clone();
        let revision = session.source_revisions[1];
        let template = session.templates[1].as_mut().unwrap();
        let fingerprint = template.fingerprint.clone();
        let pixel = template.image.get_pixel_mut(50, 50);
        pixel[0] ^= 1;
        assert_eq!(template.fingerprint, fingerprint);
        session.refresh_template_revisions(&previous);
        assert_ne!(session.source_revisions[1], revision);
        assert!(session.live_rectangles.keys().all(|rectangle| rectangle.item_index != 1));

        // Finish changes current card evidence while retaining the learned artwork.
        let mut session = Session::default();
        session.run_live(&frame, &analysis, content, "finish", &cfg, None).unwrap();
        let revision = session.source_revisions[1];
        let fingerprint = session.templates[1].as_ref().unwrap().fingerprint.clone();
        let mut finished = frame.clone();
        let viewport = anchored_viewport(
            super::super::super::content_rect(&frame, content).unwrap(), LayoutAnchor::Bottom,
        );
        let x0 = viewport.x + 389.0 * viewport.w / 1920.0;
        let y0 = viewport.y + 920.0 * viewport.h / 1080.0;
        let x1 = viewport.x + (389 + CARD_W) as f64 * viewport.w / 1920.0;
        let y1 = viewport.y + 965.0 * viewport.h / 1080.0;
        for y in y0.floor() as u32..=y1.ceil() as u32 {
            for x in x0.floor() as u32..=x1.ceil() as u32 {
                let local_x = (x as f64 - x0) * 1920.0 / viewport.w;
                let local_y = (y as f64 - y0) * 1080.0 / viewport.h;
                let color = if (40.0..90.0).contains(&local_x) && (10.0..15.0).contains(&local_y) {
                    Rgba([240, 215, 40, 255])
                } else {
                    Rgba([20, 25, 30, 255])
                };
                finished.put_pixel(x, y, color);
            }
        }
        let pending = session.run_live(&finished, &analysis, content, "finish", &cfg, Some(Duration::ZERO)).unwrap();
        assert!(pending.cards[1].finished);
        assert_eq!(session.templates[1].as_ref().unwrap().fingerprint, fingerprint);
        assert_eq!(session.source_revisions[1], revision);
        assert!(!pending.complete);
        assert!(session.live_rectangles.is_empty());
    }

    #[test]
    fn covered_pixels_do_not_contribute_even_when_they_match() {
        let frame = image::load_from_memory(include_bytes!(
            "../../tests/fixtures/vision-partial-watergun.png"
        ))
        .unwrap()
        .to_rgba8();
        let viewport = super::super::super::content_rect(
            &frame,
            super::super::super::locate_content(&frame).unwrap(),
        )
        .unwrap();
        let t = extract(read_card(&frame, viewport, 0)).unwrap();
        let image = RgbaImage::new(288, 160);
        let cells = vec!["unknown".to_string(); 45];
        let c = Candidate {
            placement: GridPlacement {
                x: 0,
                y: 0,
                width: 3,
                height: 2,
            },
            angle: 0.0,
            fill: 0.92,
            dx: 0.0,
            dy: 0.0,
            score: 0.0,
            counts: [0; 45],
        };
        assert!(evaluate(&image, &t, Rotation::new(&t, 0.0), &c, &cells).is_none());
    }
}
