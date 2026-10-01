//! Conservative, local-only recognition of the current 9 by 5 treasure board.
//!
//! Embedded references are pixels from the captured game, not solver
//! predictions. Only the observed ice texture can produce an empty tile.
//! Unrecognized item fragments stay uncertain.

use image::RgbaImage;
use serde::{Deserialize, Serialize};
#[path = "vision_dynamic.rs"]
mod dynamic;
use dynamic::CardTemplate;
#[path = "cover_reference.rs"]
pub mod cover_reference;
#[path = "vision_initial.rs"]
mod initial;
use cover_reference::CoverSample;

pub(crate) const COLS: usize = 9;
pub(crate) const ROWS: usize = 5;
const CELL_COUNT: usize = COLS * ROWS;
const PATCH_SIZE: usize = cover_reference::SAMPLE_SIZE;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GridPlacement {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CompletedObject {
    pub item_index: Option<usize>,
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlacementConstraint {
    pub anchor: usize,
    pub item_index: usize,
    pub placements: Vec<GridPlacement>,
}

pub struct Analysis {
    /// Current-frame full-cover evidence for round reset only. It must not
    /// turn a mismatch against a learned cell into a covered-cell label.
    pub fresh_initial_grid: bool,
    /// Hint only: positive initial-board geometry confirms unopened cells that
    /// the fixed player samples cannot explain. It never changes references.
    pub manual_cover_mismatch: bool,
    pub completed_objects: Vec<CompletedObject>,
    pub candidate_constraints: Vec<PlacementConstraint>,
    pub reference_ready: [bool; 3],
    pub card_fingerprints: [Option<String>; 3],
    pub finish: [bool; 3],
    /// Normalized x, y, width and height in the complete captured frame.
    pub board: Option<[f64; 4]>,
    pub cells: Vec<String>,
    pub present: bool,
    pub message: String,
    /// Observed width/height of each card's small shape mask. Zero means unknown.
    pub shapes: Vec<[u32; 2]>,
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn cell(self, index: usize) -> Self {
        let w = self.w / COLS as f64;
        let h = self.h / ROWS as f64;
        Self {
            x: self.x + (index % COLS) as f64 * w,
            y: self.y + (index / COLS) as f64 * h,
            w,
            h,
        }
    }

    fn normalized(self, frame: &RgbaImage) -> [f64; 4] {
        [
            self.x / frame.width() as f64,
            self.y / frame.height() as f64,
            self.w / frame.width() as f64,
            self.h / frame.height() as f64,
        ]
    }
}

#[derive(Clone)]
struct Patch {
    rgb: Vec<[u8; 3]>,
    average: [f64; 3],
}

impl Patch {
    fn read(frame: &RgbaImage, rect: Rect) -> Self {
        let mut rgb = Vec::with_capacity(PATCH_SIZE * PATCH_SIZE);
        for y in 0..PATCH_SIZE {
            for x in 0..PATCH_SIZE {
                rgb.push(sample(
                    frame,
                    rect.x + (x as f64 + 0.5) * rect.w / PATCH_SIZE as f64,
                    rect.y + (y as f64 + 0.5) * rect.h / PATCH_SIZE as f64,
                ));
            }
        }
        Self::from_rgb(rgb)
    }

    fn from_rgb(rgb: Vec<[u8; 3]>) -> Self {
        let mut average = [0.0; 3];
        for y in 3..29 {
            for x in 3..29 {
                for channel in 0..3 {
                    average[channel] += rgb[y * PATCH_SIZE + x][channel] as f64;
                }
            }
        }
        for value in &mut average {
            *value /= 26.0 * 26.0;
        }
        Self { rgb, average }
    }
}

struct References {
    hidden: Vec<Patch>,
    selected_cover: Patch,
    ice: Vec<Patch>,
}

impl References {
    fn load() -> Option<Self> {
        let hidden =
            image::load_from_memory(include_bytes!("../tests/fixtures/vision-hidden-tiles.png"))
                .ok()?
                .to_rgba8();
        let selected_cover = image::load_from_memory(include_bytes!(
            "../tests/fixtures/vision-selected-cover.png"
        ))
        .ok()?
        .to_rgba8();
        let tile_w = hidden.width() as f64 / 5.0;
        Some(Self {
            selected_cover: Patch::read(
                &selected_cover,
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: selected_cover.width() as f64,
                    h: selected_cover.height() as f64,
                },
            ),
            hidden: (0..5)
                .map(|i| {
                    Patch::read(
                        &hidden,
                        Rect {
                            x: i as f64 * tile_w,
                            y: 0.0,
                            w: tile_w,
                            h: hidden.height() as f64,
                        },
                    )
                })
                .collect(),
            ice: Vec::new(),
        })
    }

    fn hidden_error(&self, patch: &Patch) -> Difference {
        let exact = self
            .hidden
            .iter()
            .map(|r| difference(patch, r, 4, 28))
            .min_by(|a, b| a.mean.total_cmp(&b.mean))
            .unwrap_or_default();
        if exact.mean < 15.0 && exact.large_fraction < 0.09 {
            return exact;
        }
        self.hidden
            .iter()
            .filter_map(|r| ordinary_cover_contour_error(patch, r))
            .min_by(f64::total_cmp)
            .map_or(exact, |mean| Difference {
                mean,
                medium_fraction: 0.0,
                large_fraction: 0.0,
            })
    }
}

#[derive(Clone)]
struct Correction {
    index: usize,
    cell: String,
    pixels: Patch,
}

pub struct Recognizer {
    references: Option<References>,
    initial_covers: initial::InitialCovers,
    manual_covers: Vec<Patch>,
    last_board: Option<Rect>,
    last_size: (u32, u32),
    last_cells: Vec<String>,
    last_pixels: Vec<Patch>,
    corrections: Vec<Correction>,
    last_remaining: Option<u32>,
    last_hidden: usize,
    templates: [Option<CardTemplate>; 3],
}

impl Default for Recognizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Recognizer {
    pub fn new() -> Self {
        Self {
            references: References::load(),
            initial_covers: initial::InitialCovers::default(),
            manual_covers: Vec::new(),
            last_board: None,
            last_size: (0, 0),
            last_cells: Vec::new(),
            last_pixels: Vec::new(),
            corrections: Vec::new(),
            last_remaining: None,
            last_hidden: 0,
            templates: std::array::from_fn(|_| None),
        }
    }

    pub fn with_cover_samples(samples: &[CoverSample]) -> Self {
        let mut recognizer = Self::new();
        recognizer.manual_covers = samples
            .iter()
            .filter(|s| s.valid())
            .map(|s| Patch::from_rgb(s.rgb.clone()))
            .collect();
        recognizer
    }

    pub fn analyze(
        &mut self,
        frame: &RgbaImage,
        board_override: Option<[f64; 4]>,
        remaining: Option<u32>,
    ) -> Analysis {
        self.analyze_snapshot(frame, board_override, remaining, [None; 3])
    }

    pub fn analyze_snapshot(
        &mut self,
        frame: &RgbaImage,
        board_override: Option<[f64; 4]>,
        remaining: Option<u32>,
        remaining_counts: [Option<u32>; 3],
    ) -> Analysis {
        let content = self.locate_content(frame);
        self.analyze_internal(
            frame,
            content,
            board_override,
            remaining,
            remaining_counts,
            false,
        )
    }

    /// Production recognition stops at covers, observed empty cells, and fully
    /// revealed neutral objects. Partial artwork remains uncertain until the
    /// player finishes revealing the object.
    pub fn analyze_completed_snapshot(
        &mut self,
        frame: &RgbaImage,
        board_override: Option<[f64; 4]>,
        remaining: Option<u32>,
        remaining_counts: [Option<u32>; 3],
    ) -> Analysis {
        let content = self.locate_content(frame);
        self.analyze_internal(
            frame,
            content,
            board_override,
            remaining,
            remaining_counts,
            true,
        )
    }

    /// Analyze using the same absolute content rectangle as the OCR pass.
    pub fn analyze_completed_in_content(
        &mut self,
        frame: &RgbaImage,
        content: [f64; 4],
        board_override: Option<[f64; 4]>,
        remaining: Option<u32>,
        counts: [Option<u32>; 3],
    ) -> Analysis {
        self.analyze_internal(
            frame,
            Some(content),
            board_override,
            remaining,
            counts,
            true,
        )
    }

    fn analyze_internal(
        &mut self,
        frame: &RgbaImage,
        content: Option<[f64; 4]>,
        board_override: Option<[f64; 4]>,
        remaining: Option<u32>,
        _remaining_counts: [Option<u32>; 3],
        completed_only: bool,
    ) -> Analysis {
        let Some(viewport) = content.and_then(|rect| content_rect(frame, rect)) else {
            self.initial_covers.clear_pending();
            return self.absent("画面尺寸不足或不是横向游戏画面", vec![[0, 0]; 3]);
        };
        let (shapes, card_anchors) = read_shapes(frame, viewport);
        if card_anchors < 2 || !round_header_present(frame, viewport) {
            self.initial_covers.clear_pending();
            return self.absent("当前画面不是可识别的寻宝棋盘，或棋盘被弹窗遮挡", shapes);
        }
        let expected = expected_board(viewport);
        let seed = if let Some(normalized) = board_override {
            let Some(manual) = manual_board(frame, normalized, expected) else {
                self.initial_covers.clear_pending();
                return self.absent("手动棋盘范围不符合当前 9×5 方格区域", shapes);
            };
            manual
        } else {
            expected
        };
        // The game fixes the 9x5 board relative to its viewport. Searching
        // background pixels moved the grid when a cover became selected.
        // Validate the fixed geometry against actual visible cover structure.
        let board = seed;
        if self.last_size != frame.dimensions()
            || self.last_board.is_some_and(|old| {
                [
                    old.x - board.x,
                    old.y - board.y,
                    old.w - board.w,
                    old.h - board.h,
                ]
                .iter()
                .any(|delta| delta.abs() > 0.01)
            })
        {
            // Pixel corrections belong to a raster geometry. Round artwork
            // remains useful across a resize, including cards under Finish.
            self.corrections.clear();
            self.last_pixels.clear();
            if let Some(references) = self.references.as_mut() {
                references.ice.clear();
            }
        }
        let Some(references) = self.references.as_ref() else {
            self.initial_covers.clear_pending();
            return self.absent("本地棋盘图像参考无法解码", shapes);
        };
        if board.x < 0.0
            || board.y < 0.0
            || board.x + board.w > frame.width() as f64
            || board.y + board.h > frame.height() as f64
        {
            self.initial_covers.clear_pending();
            return self.absent("棋盘范围超出当前游戏画面", shapes);
        }

        let pixels: Vec<_> = (0..CELL_COUNT)
            .map(|index| Patch::read(frame, board.cell(index)))
            .collect();
        // The count only permits learning; every cell must independently look
        // closed and stay stable across observations. Never learn from Finish.
        let initial_eligible = remaining == Some(CELL_COUNT as u32)
            && card_anchors == 3
            && shapes.iter().all(|&[w, h]| {
                w > 0
                    && h > 0
                    && ((w <= COLS as u32 && h <= ROWS as u32)
                        || (h <= COLS as u32 && w <= ROWS as u32))
            })
            && dynamic::finished_cards(frame, viewport).iter().all(|f| !f)
            && !pixels.iter().enumerate().any(|(index, patch)| {
                selected_cover_present(
                    frame,
                    board.cell(index),
                    patch,
                    references,
                    viewport.w / 1920.0,
                )
            });
        // A different cover arrangement must still let the host notice the
        // next round when its number is unreadable. This is reset evidence,
        // never permission to overwrite references or relabel changed tiles.
        let fresh_initial_grid = self.initial_covers.is_ready()
            && initial_eligible
            && initial::plausible_initial_grid(&pixels);
        if self.manual_covers.is_empty() {
            self.initial_covers.observe(&pixels, initial_eligible);
        }
        let mut cells = Vec::with_capacity(CELL_COUNT);
        for (index, patch) in pixels.iter().enumerate() {
            let tile = board.cell(index);
            let hidden = references.hidden_error(patch);
            let cell = if self
                .manual_covers
                .iter()
                .any(|r| initial::manual_reference_matches(r, patch))
                || (self.manual_covers.is_empty() && self.initial_covers.matches(index, patch))
                || (self.manual_covers.is_empty()
                    && !self.initial_covers.is_ready()
                    && ((hidden.mean < 15.0 && hidden.large_fraction < 0.09)
                        || ordinary_cover_registered(
                            frame,
                            tile,
                            patch,
                            references,
                            viewport.w / 1920.0,
                        )))
                || selected_cover_present(frame, tile, patch, references, viewport.w / 1920.0)
            {
                "unknown"
            } else if ice_present(frame, tile, references, viewport.w / 1920.0) {
                "empty"
            } else {
                "uncertain"
            };
            cells.push(cell.to_owned());
        }
        // Covers independently confirmed by the fixed bevel/check structure
        // teach this frame's actual raster phases and hues. This second pass
        // still requires the same contour bounds; counts never create covers.
        let visible_covers: Vec<_> = pixels
            .iter()
            .zip(&cells)
            .filter_map(|(p, s)| (s == "unknown").then_some(p))
            .collect();
        for (i, p) in pixels.iter().enumerate() {
            if cells[i] == "uncertain"
                && self.manual_covers.is_empty()
                && !self.initial_covers.is_ready()
                && visible_covers
                    .iter()
                    .any(|r| ordinary_cover_contour_error(p, r).is_some())
            {
                cells[i] = "unknown".to_owned();
            }
        }
        // Any of the 45 cells can anchor the grid, including a selected cover.
        // A fixed nine-cell sample loses its anchors as those cells are opened.
        // With only one/two covers left, require that many observed contours;
        // the counter lowers the required evidence count but never labels a cell.
        // Zero covers still needs header/cards, positive opened-cell evidence,
        // and the full counter/uncertainty checks below and in the capture host.
        let required_covers = remaining.unwrap_or(3).min(3) as usize;
        if cells.iter().filter(|cell| *cell == "unknown").count() < required_covers {
            let mut result = self.absent("方格纹理与 9×5 棋盘位置无法相互验证", shapes);
            result.fresh_initial_grid = fresh_initial_grid;
            return result;
        }
        let finish = dynamic::update_cards(frame, viewport, &mut self.templates);
        let reference_ready = std::array::from_fn(|i| self.templates[i].is_some());
        let card_fingerprints =
            std::array::from_fn(|i| self.templates[i].as_ref().map(|t| t.fingerprint.clone()));
        let mut completed_objects =
            dynamic::completed_objects(frame, board, &self.templates, &shapes, &mut cells);
        // Validate full neutral silhouettes before the interior-only automatic
        // background test: a thin tail at a cell edge is still observed art.
        // Covers and trusted ice already classified above remain gated out.
        dynamic::background_cells(&pixels, &mut cells);
        let mut candidate_constraints = if completed_only {
            Vec::new()
        } else {
            dynamic::colored_observations(frame, board, &self.templates, &shapes, &mut cells)
        };
        let hidden = cells.iter().filter(|cell| *cell == "unknown").count();
        let empty = cells.iter().filter(|cell| *cell == "empty").count();
        // A rectangle at expected coordinates is not enough. At least nine
        // cells must independently exhibit a known, observed tile texture.
        let completed = cells.iter().filter(|cell| *cell == "completed").count();
        if hidden + empty + completed < 9 {
            let mut result = self.absent("棋盘方格被大面积遮挡或当前主题尚未验证", shapes);
            result.fresh_initial_grid = fresh_initial_grid;
            return result;
        }

        // Count rises and tiles returning to their covered state indicate a
        // fresh round. Never retain a correction merely because its index fits.
        if (self.last_board.is_some() && hidden > self.last_hidden)
            || matches!((self.last_remaining, remaining), (Some(old), Some(new)) if new > old)
        {
            self.corrections.clear();
        }
        self.corrections.retain(|correction| {
            let error = difference(&pixels[correction.index], &correction.pixels, 2, 30);
            error.mean < 2.0 && error.large_fraction < 0.01
        });
        for correction in &self.corrections {
            cells[correction.index] = correction.cell.clone();
        }

        completed_objects.retain(|object| {
            (object.y..object.y + object.height).all(|y| {
                (object.x..object.x + object.width).all(|x| cells[y * COLS + x] == "completed")
            })
        });
        candidate_constraints.retain(|constraint| {
            cells[constraint.anchor] == format!("item{}", constraint.item_index)
        });
        // OCR is a cross-check, not a source of invented revealed cells.
        // An unreadable count remains an explicit limitation.
        let observed_hidden = cells.iter().filter(|cell| *cell == "unknown").count();
        let unresolved = cells.iter().filter(|cell| *cell == "uncertain").count();
        let counter_consistent = remaining.map_or(true, |count| {
            count <= CELL_COUNT as u32
                && count as usize >= observed_hidden
                && count as usize <= observed_hidden + unresolved
        });
        self.last_board = Some(board);
        self.last_size = frame.dimensions();
        self.last_remaining = remaining;
        self.last_hidden = hidden;
        self.last_cells = cells.clone();
        let manual_cover_mismatch = !self.manual_covers.is_empty()
            && unresolved > 0
            && initial_eligible
            && initial::plausible_initial_grid(&pixels);
        self.last_pixels = pixels;
        let message = if !counter_consistent {
            "剩余格数与已观察的格子状态不一致，请等待画面稳定或检查计数".to_owned()
        } else if manual_cover_mismatch {
            "未翻开样本不匹配，请检查画面或更新样本".to_owned()
        } else if unresolved > 0 {
            format!("有 {unresolved} 格无法从真实像素确认类型，请手动校正")
        } else if remaining.is_none() {
            "棋盘已定位，但剩余格数未识别，未推测计数".to_owned()
        } else {
            format!(
                "已观察 {observed_hidden} 格未翻开、{} 格为空",
                cells.iter().filter(|cell| *cell == "empty").count()
            )
        };
        Analysis {
            fresh_initial_grid,
            manual_cover_mismatch,
            board: Some(board.normalized(frame)),
            cells,
            present: counter_consistent,
            message,
            shapes,
            completed_objects,
            candidate_constraints,
            reference_ready,
            card_fingerprints,
            finish,
        }
    }

    pub fn correct(&mut self, index: usize, cell: &str) -> Result<(), String> {
        if index >= CELL_COUNT {
            return Err("格子索引必须在 0..45 内".to_owned());
        }
        if !matches!(
            cell,
            "unknown" | "empty" | "item0" | "item1" | "item2" | "completed" | "uncertain"
        ) {
            return Err("无效的格子状态".to_owned());
        }
        if self.last_pixels.len() != CELL_COUNT || self.last_board.is_none() {
            return Err("请先识别当前棋盘".to_owned());
        }
        if self.last_cells[index] == "unknown" && cell != "unknown" {
            return Err("该格仍呈现未翻方块，确认开启后才能校正为已翻状态".to_owned());
        }
        self.corrections
            .retain(|correction| correction.index != index);
        self.corrections.push(Correction {
            index,
            cell: cell.to_owned(),
            pixels: self.last_pixels[index].clone(),
        });
        self.last_cells[index] = cell.to_owned();
        if cell == "empty" {
            if let Some(ref mut references) = self.references {
                references.ice.push(self.last_pixels[index].clone());
            }
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        self.initial_covers.reset();
        self.last_board = None;
        self.last_size = (0, 0);
        self.last_cells.clear();
        self.last_pixels.clear();
        self.corrections.clear();
        self.last_remaining = None;
        self.last_hidden = 0;
        self.templates = std::array::from_fn(|_| None);
        if let Some(ref mut references) = self.references {
            references.ice.clear();
        }
    }

    /// Use this round's observed covers when locating a partially opened board.
    pub fn locate_content(&mut self, frame: &RgbaImage) -> Option<[f64; 4]> {
        let content = locate_content_with_covers(
            frame,
            Some(&self.initial_covers),
            &self.manual_covers,
            false,
        );
        if content.is_none() {
            self.initial_covers.clear_pending();
        }
        content
    }

    fn absent(&mut self, message: &str, shapes: Vec<[u32; 2]>) -> Analysis {
        // A modal or failed frame must not erase the round's card templates.
        self.last_board = None;
        self.last_pixels.clear();
        self.last_cells.clear();
        self.corrections.clear();
        Analysis {
            fresh_initial_grid: false,
            manual_cover_mismatch: false,
            board: None,
            cells: vec!["uncertain".to_owned(); CELL_COUNT],
            present: false,
            message: if self.manual_covers.is_empty() {
                message.to_owned()
            } else {
                "未翻开样本不匹配，请检查画面或更新样本".to_owned()
            },
            shapes,
            completed_objects: Vec::new(),
            candidate_constraints: Vec::new(),
            reference_ready: std::array::from_fn(|i| self.templates[i].is_some()),
            card_fingerprints: std::array::from_fn(|i| {
                self.templates[i].as_ref().map(|t| t.fingerprint.clone())
            }),
            finish: [false; 3],
        }
    }
}

/// Locate the supported 16:9 or 4:3 game content inside the captured window.
/// Frame borders are proposals only: independent header, card masks and board
/// textures must agree before OCR or recognition receives this rectangle.
pub fn locate_content(frame: &RgbaImage) -> Option<[f64; 4]> {
    locate_content_with_covers(frame, None, &[], false)
}

/// Capture proposals are for the player's sample picker only. Passing header
/// and card geometry here never authorizes OCR, cell labels, or probabilities.
pub fn sample_cover_candidates(
    frame: &RgbaImage,
    content: Option<[f64; 4]>,
    profile_board: Option<[f64; 4]>,
) -> Option<Vec<CoverSample>> {
    let content = content.or_else(|| locate_content_with_covers(frame, None, &[], true))?;
    let viewport = content_rect(frame, content)?;
    let expected = expected_board(viewport);
    let board = if let Some([x, y, w, h]) = profile_board {
        let region = anchored_viewport(viewport, LayoutAnchor::Center);
        manual_board(
            frame,
            [
                (region.x + x * region.w) / frame.width() as f64,
                (region.y + y * region.h) / frame.height() as f64,
                w * region.w / frame.width() as f64,
                h * region.h / frame.height() as f64,
            ],
            expected,
        )?
    } else {
        expected
    };
    Some(
        (0..CELL_COUNT)
            .map(|index| CoverSample {
                rgb: Patch::read(frame, board.cell(index)).rgb,
            })
            .collect(),
    )
}

fn locate_content_with_covers(
    frame: &RgbaImage,
    initial_covers: Option<&initial::InitialCovers>,
    manual_covers: &[Patch],
    sampling_only: bool,
) -> Option<[f64; 4]> {
    if frame.width() == 0 || frame.height() == 0 {
        return None;
    }
    static REFERENCES: std::sync::OnceLock<Option<References>> = std::sync::OnceLock::new();
    let references = REFERENCES.get_or_init(References::load).as_ref()?;
    let valid = |rect: Rect| {
        if content_rect(frame, [rect.x, rect.y, rect.w, rect.h]).is_none()
            || !round_header_present(frame, rect)
        {
            return false;
        }
        let (shapes, anchors) = read_shapes(frame, rect);
        if anchors < 2 {
            return false;
        }
        if sampling_only {
            return anchors == 3 && shapes.iter().all(|s| s[0] > 0 && s[1] > 0);
        }
        let board = expected_board(rect);
        let mut pixels = Vec::with_capacity(CELL_COUNT);
        let mut cells = Vec::with_capacity(CELL_COUNT);
        for index in 0..CELL_COUNT {
            let tile = board.cell(index);
            let patch = Patch::read(frame, tile);
            let hidden = if patch.rgb.iter().all(|p| *p == patch.rgb[0]) {
                Difference::default()
            } else {
                references.hidden_error(&patch)
            };
            let cell = if manual_covers
                .iter()
                .any(|r| initial::manual_reference_matches(r, &patch))
                || (manual_covers.is_empty() && hidden.mean < 15.0 && hidden.large_fraction < 0.09)
                || initial_covers.is_some_and(|covers| covers.matches(index, &patch))
            {
                "unknown"
            } else {
                "uncertain"
            };
            pixels.push(patch);
            cells.push(cell.to_owned());
        }
        if cells.iter().filter(|c| c.as_str() == "unknown").count() >= 9 {
            return true;
        }
        // This only admits a candidate to same-frame OCR. Committing all 45
        // references additionally requires count=45, valid cards, no Finish,
        // and consecutive stable observations in analyze_internal.
        if anchors == 3 && initial::plausible_initial_grid(&pixels) {
            return true;
        }
        // A flat popup has no texture to register. Reject before the costly
        // subpixel registration; at least nine observed cells are mandatory.
        let has_texture = |patch: &Patch| {
            (4..28).any(|y| {
                (4..28).any(|x| patch.rgb[y * PATCH_SIZE + x] != patch.rgb[4 * PATCH_SIZE + 4])
            })
        };
        if pixels.iter().filter(|p| has_texture(p)).count() < 9 {
            return false;
        }
        for (index, cell) in cells.iter_mut().enumerate() {
            if cell == "uncertain"
                && has_texture(&pixels[index])
                && ice_present(frame, board.cell(index), references, rect.w / 1920.0)
            {
                *cell = "empty".to_owned();
            }
        }
        dynamic::background_cells(&pixels, &mut cells);
        if cells.iter().filter(|c| c.as_str() != "uncertain").count() >= 9 {
            return true;
        }
        // Fully opened boards still need positive texture/object evidence.
        dynamic::completed_objects(
            frame,
            board,
            &std::array::from_fn(|_| None),
            &shapes,
            &mut cells,
        );
        cells.iter().filter(|c| c.as_str() != "uncertain").count() >= 9
    };
    let output = |r: Rect| [r.x, r.y, r.w, r.h];
    // A borderless 16:9 frame has exact geometry. For window captures, measure
    // the actual borders first: resizing a capture also scales its chrome,
    // so assuming a persistent two-pixel border shifts small card masks.
    let legacy = game_viewport(frame);
    if let Some(rect) = legacy {
        if rect.x == 0.0 && rect.y == 0.0 && valid(rect) {
            return Some(output(rect));
        }
    }
    let four_three = game_viewport_at_aspect(frame, 4.0 / 3.0);
    if let Some(rect) = four_three {
        if rect.x == 0.0 && rect.y == 0.0 && valid(rect) {
            return Some(output(rect));
        }
    }
    let xs = content_edges(frame, true);
    let ys = content_edges(frame, false);
    // The native 4:3 window has measurable side/bottom edges; the pale game
    // background can blend into the title bar. Validate those measured edges
    // before searching 16:9 combinations, still requiring all board anchors.
    if let Some(rect) = four_three {
        if xs.contains(&rect.x)
            && xs.contains(&(rect.x + rect.w))
            && (ys.contains(&rect.y) || ys.contains(&(rect.y + rect.h)))
            && valid(rect)
        {
            return Some(output(rect));
        }
    }
    for aspect in [16.0 / 9.0, 4.0 / 3.0] {
        for (i, &a) in xs.iter().enumerate() {
            for &b in &xs[i + 1..] {
                let (left, right) = (a.min(b), a.max(b));
                let w = right - left;
                let h = w / aspect;
                for &edge in &ys {
                    for y in [edge, edge - h] {
                        let rect = Rect { x: left, y, w, h };
                        if valid(rect) {
                            return Some(output(rect));
                        }
                    }
                }
            }
        }
        for (i, &a) in ys.iter().enumerate() {
            for &b in &ys[i + 1..] {
                let (top, bottom) = (a.min(b), a.max(b));
                let h = bottom - top;
                let w = h * aspect;
                for &edge in &xs {
                    for x in [edge, edge - w] {
                        let rect = Rect { x, y: top, w, h };
                        if valid(rect) {
                            return Some(output(rect));
                        }
                    }
                }
            }
        }
    }
    // The historical geometry remains a candidate for frames whose border
    // contrast is too low to appear in the measured edge proposals.
    legacy
        .filter(|rect| valid(*rect))
        .or_else(|| four_three.filter(|rect| valid(*rect)))
        .map(output)
}

fn content_rect(frame: &RgbaImage, [x, y, w, h]: [f64; 4]) -> Option<Rect> {
    if [x, y, w, h].iter().any(|v| !v.is_finite())
        || x < 0.0
        || y < 0.0
        || w <= 0.0
        || h <= 0.0
        || x + w > frame.width() as f64 + 0.01
        || y + h > frame.height() as f64 + 0.01
        || ![16.0 / 9.0, 4.0 / 3.0]
            .iter()
            .any(|aspect| (w / h - aspect).abs() <= 0.001)
    {
        return None;
    }
    Some(Rect { x, y, w, h })
}

fn content_edges(frame: &RgbaImage, vertical: bool) -> Vec<f64> {
    let (length, across) = if vertical {
        frame.dimensions()
    } else {
        (frame.height(), frame.width())
    };
    // Sampling budget controls search cost, not supported capture dimensions.
    const LINE_SAMPLES: u32 = 128;
    const EDGE_CANDIDATES: usize = 24;
    let stride = (across / LINE_SAMPLES).max(1);
    let mut energies = Vec::new();
    let mut transitions = Vec::new();
    let mut previous_flat = None;
    for coordinate in 0..length {
        let mut low = [255u8; 3];
        let mut high = [0u8; 3];
        let mut energy = 0u64;
        for other in (0..across).step_by(stride as usize) {
            let (x, y) = if vertical {
                (coordinate, other)
            } else {
                (other, coordinate)
            };
            let pixel = frame.get_pixel(x, y);
            for c in 0..3 {
                low[c] = low[c].min(pixel[c]);
                high[c] = high[c].max(pixel[c]);
            }
            if coordinate > 0 {
                let prior = if vertical {
                    frame.get_pixel(x - 1, y)
                } else {
                    frame.get_pixel(x, y - 1)
                };
                energy += (0..3)
                    .map(|c| pixel[c].abs_diff(prior[c]) as u64)
                    .sum::<u64>();
            }
        }
        let flat = (0..3).all(|c| high[c] - low[c] < 12);
        if previous_flat.is_some_and(|prior| prior != flat) {
            transitions.push((energy, coordinate));
        }
        previous_flat = Some(flat);
        if coordinate > 0 {
            energies.push((energy, coordinate));
        }
    }
    energies.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let mut edges = vec![(0, 0), (0, length)];
    edges.extend(transitions);
    edges.extend(
        energies
            .into_iter()
            .take(EDGE_CANDIDATES)
            .filter(|(energy, _)| *energy > 0),
    );
    // Border discontinuities outrank a geometry formula that only happens to
    // leave nine recognizable covers. This also keeps fractional bottom
    // alignment instead of rounding the 16:9 content height independently.
    edges.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut coordinates = Vec::new();
    for (_, coordinate) in edges {
        if !coordinates.contains(&(coordinate as f64)) {
            coordinates.push(coordinate as f64);
        }
    }
    coordinates
}

fn game_viewport(frame: &RgbaImage) -> Option<Rect> {
    game_viewport_at_aspect(frame, 16.0 / 9.0)
}

fn game_viewport_at_aspect(frame: &RgbaImage, aspect: f64) -> Option<Rect> {
    let (width, height) = frame.dimensions();
    if width < 640 || height < 360 || width <= height {
        return None;
    }
    let chrome = height as f64 > width as f64 / aspect + 8.0;
    let w = width as f64 - if chrome { 4.0 } else { 0.0 };
    let h = w / aspect;
    let y = height as f64 - h - if chrome { 2.0 } else { 0.0 };
    if y < -2.0 || y > height as f64 * 0.25 {
        return None;
    }
    Some(Rect {
        x: (width as f64 - w) / 2.0,
        y: y.max(0.0),
        w,
        h,
    })
}

fn expected_board(viewport: Rect) -> Rect {
    let viewport = anchored_viewport(viewport, LayoutAnchor::Center);
    // A search seed, never an unconditional positive detection.
    Rect {
        x: viewport.x + viewport.w * 908.0 / 1920.0,
        y: viewport.y + viewport.h * 286.0 / 1080.0,
        w: viewport.w * 936.0 / 1920.0,
        h: viewport.h * 520.0 / 1080.0,
    }
}

fn manual_board(frame: &RgbaImage, normalized: [f64; 4], expected: Rect) -> Option<Rect> {
    if normalized.iter().any(|n| !n.is_finite()) {
        return None;
    }
    let [x, y, w, h] = normalized;
    if x < 0.0 || y < 0.0 || w <= 0.0 || h <= 0.0 || x + w > 1.0 || y + h > 1.0 {
        return None;
    }
    let rect = Rect {
        x: x * frame.width() as f64,
        y: y * frame.height() as f64,
        w: w * frame.width() as f64,
        h: h * frame.height() as f64,
    };
    let square_ratio = rect.w / 9.0 / (rect.h / 5.0);
    if !(0.95..=1.05).contains(&square_ratio)
        || (rect.x - expected.x).abs() > expected.w * 0.12
        || (rect.y - expected.y).abs() > expected.h * 0.12
        || (rect.w / expected.w - 1.0).abs() > 0.07
        || (rect.h / expected.h - 1.0).abs() > 0.07
    {
        return None;
    }
    Some(rect)
}

fn round_header_present(frame: &RgbaImage, viewport: Rect) -> bool {
    let viewport = anchored_viewport(viewport, LayoutAnchor::Center);
    let mut navy = 0;
    let mut white = 0;
    let mut count = 0;
    for y in (200..242).step_by(3) {
        for x in (1260..1496).step_by(3) {
            let [r, g, b] = viewport_sample(frame, viewport, x as f64, y as f64);
            navy += usize::from(
                r < 95
                    && (40..135).contains(&g)
                    && (70..180).contains(&b)
                    && b > g.saturating_add(10)
                    && g > r.saturating_add(8),
            );
            white += usize::from(r > 215 && g > 215 && b > 215);
            count += 1;
        }
    }
    navy as f64 / count as f64 > 0.35 && white as f64 / count as f64 > 0.08
}

fn read_shapes(frame: &RgbaImage, viewport: Rect) -> (Vec<[u32; 2]>, usize) {
    let viewport = anchored_viewport(viewport, LayoutAnchor::Bottom);
    let mut shapes = Vec::with_capacity(3);
    let mut anchors = 0;
    let light_pixel = |[r, g, b]: [u8; 3]| (80..230).contains(&r) && g > 95 && b > 120;
    for card_x in [166.0, 373.0, 579.0] {
        let mut pixels = vec![[0; 3]; 42 * 32];
        let mut light = 0;
        for y in 0..32 {
            for x in 0..42 {
                let rgb = viewport_sample(frame, viewport, card_x + x as f64, 1008.0 + y as f64);
                light += usize::from(light_pixel(rgb));
                pixels[y * 42 + x] = rgb;
            }
        }
        let mut greens: Vec<_> = pixels.iter().map(|p| p[1]).collect();
        greens.sort_unstable();
        let square_limit = greens[greens.len() / 2] as f64 * 0.68;
        let square = |p: [u8; 3]| mask_square_pixel(p) && (p[1] as f64) < square_limit;
        let mut seen = vec![false; 42 * 32];
        let mut centers = Vec::new();
        let mut incomplete = false;
        // Exclude the rope/border. Count only separate squares in the light
        // icon; quantities are not inferred from a shape or preset. Trace
        // beyond the search area so an edge-clipped
        // square cannot be accepted as a smaller complete component.
        for y in 3..29 {
            // Raster rounding can move the light icon panel down one sample.
            // Entirely blue artwork rows above it are not mask observations.
            // Once seeded inside the panel, still trace beyond every search
            // edge so clipped actual squares remain incomplete.
            if !pixels[y * 42..(y + 1) * 42]
                .iter()
                .copied()
                .any(light_pixel)
            {
                continue;
            }
            for x in 5..37 {
                let index = y * 42 + x;
                if seen[index] || !square(pixels[index]) {
                    continue;
                }
                let mut stack = vec![(x, y)];
                seen[index] = true;
                let (mut min_x, mut max_x, mut min_y, mut max_y, mut area) = (x, x, y, y, 0);
                while let Some((xx, yy)) = stack.pop() {
                    min_x = min_x.min(xx);
                    max_x = max_x.max(xx);
                    min_y = min_y.min(yy);
                    max_y = max_y.max(yy);
                    area += 1;
                    for (nx, ny) in [(xx - 1, yy), (xx + 1, yy), (xx, yy - 1), (xx, yy + 1)] {
                        if !(1..41).contains(&nx) || !(1..31).contains(&ny) {
                            continue;
                        }
                        let next = ny * 42 + nx;
                        if !seen[next] && square(pixels[next]) {
                            seen[next] = true;
                            stack.push((nx, ny));
                        }
                    }
                }
                let w = max_x - min_x + 1;
                let h = max_y - min_y + 1;
                let ratio = w as f64 / h as f64;
                if area >= 6
                    && min_x >= 5
                    && max_x < 37
                    && min_y >= 3
                    && max_y < 29
                    && (3..=10).contains(&w)
                    && (3..=10).contains(&h)
                    && (0.6..1.6).contains(&ratio)
                {
                    centers.push(((min_x + max_x) as f64 / 2.0, (min_y + max_y) as f64 / 2.0));
                } else if area >= 3 {
                    incomplete = true;
                }
            }
        }
        let anchored = light > 600 && !centers.is_empty();
        anchors += usize::from(anchored);
        let columns = clustered_count(centers.iter().map(|center| center.0).collect());
        let rows = clustered_count(centers.iter().map(|center| center.1).collect());
        let fits_board = (columns <= COLS && rows <= ROWS) || (columns <= ROWS && rows <= COLS);
        let complete = anchored
            && !incomplete
            && columns > 0
            && rows > 0
            && fits_board
            && columns * rows == centers.len()
            && centers.iter().enumerate().all(|(index, &(x, y))| {
                // There must be exactly one square at every row/column
                // intersection, rather than merely the same total count.
                centers[..index]
                    .iter()
                    .all(|&(xx, yy)| (x - xx).abs() > 3.0 || (y - yy).abs() > 3.0)
                    && centers
                        .iter()
                        .filter(|&&(xx, _)| (x - xx).abs() <= 3.0)
                        .count()
                        == rows
                    && centers
                        .iter()
                        .filter(|&&(_, yy)| (y - yy).abs() <= 3.0)
                        .count()
                        == columns
            });
        shapes.push(if complete {
            [columns as u32, rows as u32]
        } else {
            [0, 0]
        });
    }
    (shapes, anchors)
}

fn mask_square_pixel([r, g, b]: [u8; 3]) -> bool {
    r < 90 && (35..150).contains(&g) && b > g.saturating_add(25) && b > r.saturating_add(40)
}

fn clustered_count(mut values: Vec<f64>) -> usize {
    values.sort_by(f64::total_cmp);
    let mut groups = 0;
    let mut previous = -100.0;
    for value in values {
        if value - previous > 3.0 {
            groups += 1;
        }
        previous = value;
    }
    groups
}

fn ice_present(frame: &RgbaImage, tile: Rect, references: &References, scale: f64) -> bool {
    // Grid fitting and fractional cell widths can differ by a few source
    // pixels at the far right. Register locally instead of loosening texture
    // thresholds, and try the smallest shifts first.
    const OFFSETS: [f64; 13] = [
        0.0, -0.5, 0.5, -1.0, 1.0, -1.5, 1.5, -2.0, 2.0, -2.5, 2.5, -3.0, 3.0,
    ];
    for dx in OFFSETS {
        for dy in OFFSETS {
            let shifted = Rect {
                x: tile.x + dx * scale,
                y: tile.y + dy * scale,
                ..tile
            };
            let patch = Patch::read(frame, shifted);
            for reference in &references.ice {
                if mean_rgb_difference(&patch, reference) >= 4.5 {
                    continue;
                }
                let error = difference(&patch, reference, 3, 29);
                // Keep the strict bounds for each real ice pattern. Do not
                // broaden whiteness thresholds to accept a different texture.
                if error.mean < 4.5 && error.medium_fraction < 0.025 && error.large_fraction < 0.003
                {
                    return true;
                }
            }
        }
    }
    false
}

fn selection_green([r, g, b]: [u8; 3]) -> bool {
    // The selection marker is yellow/lime green. Ordinary green covered
    // blocks and cyan item edges do not by themselves meet this signature.
    g > 175 && r > 85 && b < 155 && g > r.saturating_add(20) && g > b.saturating_add(55)
}

fn green_mask_iou(a: &Patch, b: &Patch, region: [usize; 4]) -> f64 {
    let [x0, y0, x1, y1] = region;
    let (mut intersection, mut union) = (0, 0);
    for y in y0..y1 {
        for x in x0..x1 {
            let index = y * PATCH_SIZE + x;
            let aa = selection_green(a.rgb[index]);
            let bb = selection_green(b.rgb[index]);
            intersection += usize::from(aa && bb);
            union += usize::from(aa || bb);
        }
    }
    intersection as f64 / union.max(1) as f64
}

fn cover_contour_agrees(
    observed: &Patch,
    reference: &Patch,
    reference_channel: Option<usize>,
) -> bool {
    // Ignore the tick and its shadow while testing the inset square/bevel.
    // An affine grayscale fit tolerates the cover color's selected tint, but
    // still requires the actual contour and contrast instead of a flat fill.
    let points = (4..28)
        .flat_map(|y| (4..28).map(move |x| (x, y)))
        .filter(|(x, y)| !((8..25).contains(x) && (8..25).contains(y)));
    let mut values = Vec::new();
    for (x, y) in points {
        let index = y * PATCH_SIZE + x;
        let gray = |patch: &Patch| {
            patch.rgb[index]
                .iter()
                .map(|channel| *channel as f64)
                .sum::<f64>()
                / 3.0
        };
        let reference_value = reference_channel.map_or_else(
            || gray(reference),
            |channel| reference.rgb[index][channel] as f64,
        );
        values.push((reference_value, gray(observed)));
    }
    let count = values.len() as f64;
    let reference_mean = values.iter().map(|(value, _)| value).sum::<f64>() / count;
    let observed_mean = values.iter().map(|(_, value)| value).sum::<f64>() / count;
    let mut reference_variance = 0.0;
    let mut observed_variance = 0.0;
    let mut covariance = 0.0;
    for (reference, observed) in &values {
        let a = reference - reference_mean;
        let b = observed - observed_mean;
        reference_variance += a * a;
        observed_variance += b * b;
        covariance += a * b;
    }
    if reference_variance <= 0.0 || (observed_variance / count).sqrt() < 3.0 {
        return false;
    }
    let gain = covariance / reference_variance;
    let correlation = covariance / (reference_variance * observed_variance).sqrt();
    if !(0.4..2.5).contains(&gain) || correlation < 0.86 {
        return false;
    }
    let residual = values
        .iter()
        .map(|(reference, observed)| {
            let predicted = observed_mean + gain * (reference - reference_mean);
            (observed - predicted).powi(2)
        })
        .sum::<f64>();
    (residual / count).sqrt() < 4.5
}

fn ordinary_cover_contour_error(observed: &Patch, reference: &Patch) -> Option<f64> {
    if observed.rgb.iter().filter(|p| selection_green(**p)).count() > 20 {
        return None;
    }
    let mut best = f64::INFINITY;
    for channel in 0..4 {
        let points: Vec<_> = (4..28)
            .flat_map(|y| (4..28).map(move |x| y * PATCH_SIZE + x))
            .map(|i| {
                (
                    if channel == 3 {
                        reference.rgb[i].iter().map(|v| *v as f64).sum::<f64>() / 3.0
                    } else {
                        reference.rgb[i][channel] as f64
                    },
                    observed.rgb[i].iter().map(|v| *v as f64).sum::<f64>() / 3.0,
                )
            })
            .collect();
        let n = points.len() as f64;
        let a = points.iter().map(|p| p.0).sum::<f64>() / n;
        let b = points.iter().map(|p| p.1).sum::<f64>() / n;
        let va = points.iter().map(|p| (p.0 - a).powi(2)).sum::<f64>();
        let vb = points.iter().map(|p| (p.1 - b).powi(2)).sum::<f64>();
        let cov = points.iter().map(|p| (p.0 - a) * (p.1 - b)).sum::<f64>();
        if va < 1.0 || (vb / n).sqrt() < 3.0 || cov / (va * vb).sqrt() < 0.94 {
            continue;
        }
        let gain = cov / va;
        if !(0.25..4.0).contains(&gain) {
            continue;
        }
        let rms = (points
            .iter()
            .map(|p| (p.1 - b - gain * (p.0 - a)).powi(2))
            .sum::<f64>()
            / n)
            .sqrt();
        if rms < 4.5 {
            best = best.min(rms);
        }
    }
    best.is_finite().then_some(best)
}

fn ordinary_cover_registered(
    frame: &RgbaImage,
    tile: Rect,
    patch: &Patch,
    references: &References,
    scale: f64,
) -> bool {
    let max = patch
        .average
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let min = patch.average.iter().copied().fold(f64::INFINITY, f64::min);
    if max - min < 20.0 || patch.rgb.iter().filter(|p| selection_green(**p)).count() > 20 {
        return false;
    }
    // Fixed viewport geometry can place the far cell edge a few source
    // pixels from the rasterized bevel. Register that UI contour locally.
    for dy in [
        0.0, -0.5, 0.5, -1.0, 1.0, -1.5, 1.5, -2.0, 2.0, -2.5, 2.5, -3.0, 3.0,
    ] {
        for dx in [
            0.0, -0.5, 0.5, -1.0, 1.0, -1.5, 1.5, -2.0, 2.0, -2.5, 2.5, -3.0, 3.0,
        ] {
            let p = Patch::read(
                frame,
                Rect {
                    x: tile.x + dx * scale,
                    y: tile.y + dy * scale,
                    ..tile
                },
            );
            if references
                .hidden
                .iter()
                .any(|r| ordinary_cover_contour_error(&p, r).is_some())
            {
                return true;
            }
        }
    }
    false
}

fn selected_cover_present(
    frame: &RgbaImage,
    tile: Rect,
    initial: &Patch,
    references: &References,
    scale: f64,
) -> bool {
    let lime_pixels = initial
        .rgb
        .iter()
        .filter(|pixel| selection_green(**pixel))
        .count();
    let center_lime = (8..25)
        .flat_map(|y| (8..25).map(move |x| y * PATCH_SIZE + x))
        .filter(|index| selection_green(initial.rgb[*index]))
        .count();
    if lime_pixels < 100 || center_lime < 20 {
        return false;
    }
    const OFFSETS: [f64; 13] = [
        0.0, -0.5, 0.5, -1.0, 1.0, -1.5, 1.5, -2.0, 2.0, -2.5, 2.5, -3.0, 3.0,
    ];
    for dx in OFFSETS {
        for dy in OFFSETS {
            let shifted = Rect {
                x: tile.x + dx * scale,
                y: tile.y + dy * scale,
                ..tile
            };
            let patch = Patch::read(frame, shifted);
            let reference = &references.selected_cover;
            // All four frame sides and the exact tick-shaped mask must agree.
            // Green sprites, a border alone, or a tick alone are insufficient.
            if green_mask_iou(&patch, reference, [8, 8, 25, 25]) < 0.78 {
                continue;
            }
            if [
                [0, 0, 32, 3],
                [0, 29, 32, 32],
                [0, 0, 3, 32],
                [29, 0, 32, 32],
            ]
            .iter()
            .any(|region| green_mask_iou(&patch, reference, *region) < 0.65)
            {
                continue;
            }
            // Cover colors have different bevel contrast in each channel.
            // Reuse their grayscale/RGB contours with identical affine-fit
            // bounds; the observed grayscale must still exhibit that shape.
            if cover_contour_agrees(&patch, reference, None)
                || references.hidden.iter().any(|cover| {
                    [None, Some(0), Some(1), Some(2)]
                        .iter()
                        .any(|channel| cover_contour_agrees(&patch, cover, *channel))
                })
            {
                return true;
            }
        }
    }
    false
}

#[derive(Clone, Copy)]
struct Difference {
    mean: f64,
    medium_fraction: f64,
    large_fraction: f64,
}

fn mean_rgb_difference(a: &Patch, b: &Patch) -> f64 {
    a.average
        .iter()
        .zip(b.average.iter())
        .map(|(a, b)| (a - b).abs())
        .sum::<f64>()
        / 3.0
}

impl Default for Difference {
    fn default() -> Self {
        Self {
            mean: 255.0,
            medium_fraction: 1.0,
            large_fraction: 1.0,
        }
    }
}

fn difference(a: &Patch, b: &Patch, begin: usize, end: usize) -> Difference {
    let (mut total, mut medium, mut large, mut count) = (0.0, 0, 0, 0);
    for y in begin..end {
        for x in begin..end {
            let i = y * PATCH_SIZE + x;
            let mut delta = 0;
            for channel in 0..3 {
                delta += (a.rgb[i][channel] as i32 - b.rgb[i][channel] as i32).unsigned_abs();
            }
            let delta = delta as f64 / 3.0;
            total += delta;
            medium += usize::from(delta > 15.0);
            large += usize::from(delta > 40.0);
            count += 1;
        }
    }
    Difference {
        mean: total / count as f64,
        medium_fraction: medium as f64 / count as f64,
        large_fraction: large as f64 / count as f64,
    }
}

#[derive(Clone, Copy)]
pub(crate) enum LayoutAnchor {
    Center,
    Bottom,
}

/// The native 4:3 client keeps the 16:9 artwork scale: the board/HUD are
/// vertically centered and the three cards are bottom anchored. Keep the
/// actual content rectangle separate from these shared vision/OCR regions.
/// See fixtures/pc-global-four-three-provenance.md for measured native pairs.
pub(crate) fn layout_region(content: [f64; 4], anchor: LayoutAnchor) -> [f64; 4] {
    let [x, y, w, h] = content;
    if (w / h - 4.0 / 3.0).abs() > 0.001 {
        return content;
    }
    let layout_height = w * 9.0 / 16.0;
    let extra = h - layout_height;
    let offset = match anchor {
        LayoutAnchor::Center => extra / 2.0,
        LayoutAnchor::Bottom => extra,
    };
    [x, y + offset, w, layout_height]
}

fn anchored_viewport(viewport: Rect, anchor: LayoutAnchor) -> Rect {
    let [x, y, w, h] = layout_region([viewport.x, viewport.y, viewport.w, viewport.h], anchor);
    Rect { x, y, w, h }
}

fn viewport_sample(frame: &RgbaImage, viewport: Rect, x: f64, y: f64) -> [u8; 3] {
    sample(
        frame,
        viewport.x + x * viewport.w / 1920.0,
        viewport.y + y * viewport.h / 1080.0,
    )
}

fn sample(frame: &RgbaImage, x: f64, y: f64) -> [u8; 3] {
    let x = x.clamp(0.0, frame.width().saturating_sub(1) as f64);
    let y = y.clamp(0.0, frame.height().saturating_sub(1) as f64);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(frame.width() - 1);
    let y1 = (y0 + 1).min(frame.height() - 1);
    let tx = x - x0 as f64;
    let ty = y - y0 as f64;
    let mut rgb = [0; 3];
    for channel in 0..3 {
        let top = frame.get_pixel(x0, y0)[channel] as f64 * (1.0 - tx)
            + frame.get_pixel(x1, y0)[channel] as f64 * tx;
        let bottom = frame.get_pixel(x0, y1)[channel] as f64 * (1.0 - tx)
            + frame.get_pixel(x1, y1)[channel] as f64 * tx;
        rgb[channel] = (top * (1.0 - ty) + bottom * ty).round() as u8;
    }
    rgb
}
