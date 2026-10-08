//! Per-round cover references, confirmed by stable pixels and a closed grid.
//!
//! The frame test searches for four long, connected edges in every tile. It
//! does not know a cover color, bevel, or captured cover image. A new theme
//! must still have a repeated, approximately rectangular cell frame; texture
//! alone is insufficient evidence to learn an initial board.

use super::{difference, Patch, CELL_COUNT, PATCH_SIZE};

// All geometry is in the existing 32-pixel normalized patch. Searching each
// outer third accommodates frames at different insets; the central half is
// a preliminary edge probe, not a fixed groove position.
const EDGE_SEARCH_END: usize = PATCH_SIZE / 3;
const EDGE_PROBE_BEGIN: usize = PATCH_SIZE / 4;
const EDGE_PROBE_END: usize = PATCH_SIZE - EDGE_PROBE_BEGIN;
const EDGE_POSITION_TOLERANCE: usize = 1;
const CORNER_TRIM: usize = 2;
// RGB-vector agreement rejects irregular texture even when it has large
// gradients. Real initial fixtures have >=9.7 mean contrast and >=94% probe
// agreement on their weakest side; a 2560px resample lowers one side to 7.98.
// The opened ice tile lacks such a side (its top contrast is about 4.1).
const EDGE_MIN_CONTRAST: f64 = 7.5;
const EDGE_DIRECTION_FRACTION: f64 = 0.35;
const EDGE_MIN_AGREEMENT: f64 = 0.875;
const EDGE_HALF_AGREEMENT: f64 = 0.75;
// Learning is stricter than later matching: two consecutive captures should
// be stationary, rather than merely have a similar cover outline.
const STABLE_MAX_MEAN: f64 = 1.0;
const STABLE_MAX_MEDIUM: f64 = 0.004;
const STABLE_MAX_NOVEL_PIXEL: f64 = 6.0;
// Resampling real 1920px content to 960px can produce about 4.9 mean error
// and 12% >15 differences around edges. Keep overall pixel and contour
// checks, then reject new local colors outside a reference 3x3 neighborhood.
// The local check prevents a small revealed item from hiding in a low whole-
// tile mean; it is not an affine color fit or an arbitrary cover lookup.
const MATCH_MAX_MEAN: f64 = 6.0;
const MATCH_MAX_MEDIUM: f64 = 0.15;
const MATCH_MAX_LARGE: f64 = 0.01;
const MATCH_MAX_NOVEL_PIXEL: f64 = 18.0;
// A thin, high-contrast outline can lose its peak when downsampled. A quarter
// normalized pixel of common registration plus this weak separable filter
// explains that raster change without raising any pixel-error threshold.
// This is one transform for the entire corresponding reference, not a best
// neighbor chosen independently for each observed pixel.
const MATCH_PHASE_OFFSETS: [f64; 3] = [0.0, -0.25, 0.25];
const MATCH_AA_WEIGHTS: [f64; 3] = [0.05, 0.9, 0.05];
const COMPARE_BEGIN: usize = 1;
const COMPARE_END: usize = PATCH_SIZE - 1;
// A manually selected appearance is shared across positions. The outer sample
// ring can contain a neighboring tile under the fixed board geometry, so keep
// this comparison inside the tile, including its bevel at normalized x/y=2.
const MANUAL_COMPARE_BEGIN: usize = 2;
const MANUAL_COMPARE_END: usize = PATCH_SIZE - 2;
// Cross-position raster phases vary by up to one normalized sample. A fixed
// three-tap sampling filter accounts for point-sampled UI edges and DPI
// resampling; no color/gain fit or pixel-error threshold is relaxed.
const MANUAL_PHASE_OFFSETS: [f64; 9] = [0.0, -0.25, 0.25, -0.5, 0.5, -0.75, 0.75, -1.0, 1.0];
const SAMPLING_AA_WEIGHTS: [f64; 3] = [0.2, 0.6, 0.2];

#[derive(Default)]
pub(super) struct InitialCovers {
    confirmed: Option<Vec<Patch>>,
    pending: Option<Vec<Patch>>,
}

impl InitialCovers {
    pub(super) fn observe(&mut self, pixels: &[Patch], eligible: bool) -> bool {
        // Only a round reset may replace a confirmed reference. In particular,
        // a later counter reading of 45 does not reteach a changed board.
        if self.is_ready() {
            return true;
        }
        if !eligible || !plausible_initial_grid(pixels) {
            self.clear_pending();
            return false;
        }
        let stable = self.pending.as_ref().is_some_and(|pending| {
            pending.iter().zip(pixels).all(|(a, b)| {
                let error = difference(a, b, COMPARE_BEGIN, COMPARE_END);
                error.mean <= STABLE_MAX_MEAN
                    && error.medium_fraction <= STABLE_MAX_MEDIUM
                    && error.large_fraction == 0.0
                    && novel_pixels_within(a, b, STABLE_MAX_NOVEL_PIXEL)
                    && novel_pixels_within(b, a, STABLE_MAX_NOVEL_PIXEL)
            })
        });
        if stable {
            self.confirmed = self.pending.take();
            true
        } else {
            self.pending = Some(pixels.to_vec());
            false
        }
    }

    pub(super) fn matches(&self, index: usize, patch: &Patch) -> bool {
        let Some(reference) = self.confirmed.as_ref().and_then(|p| p.get(index)) else {
            return false;
        };
        reference_matches(reference, patch, true)
    }

    pub(super) fn reset(&mut self) {
        self.confirmed = None;
        self.clear_pending();
    }

    pub(super) fn clear_pending(&mut self) {
        self.pending = None;
    }

    pub(super) fn is_ready(&self) -> bool {
        self.confirmed.is_some()
    }
}

/// Player-confirmed samples supply their own positive cover evidence, so they
/// need pixel agreement but do not require the automatic bootstrap's frame.
pub(super) fn reference_matches(reference: &Patch, patch: &Patch, require_edges: bool) -> bool {
    if !valid_patch(reference) || !valid_patch(patch) {
        return false;
    }
    // A reference learned in a smaller native window has lower peak contrast
    // than the same cover rendered fullscreen. Only across sampling sizes,
    // compare the observed peaks at the existing sampling filter's support.
    // Keep the original reference, pixel-error limits, and closed-frame gate;
    // unchanged geometry must retain the raw small-fragment sensitivity.
    if !novel_pixels_within(reference, patch, MATCH_MAX_NOVEL_PIXEL) {
        let resized = require_edges
            && matches!((reference.source_size, patch.source_size), (Some(old), Some(new)) if old != new);
        if !resized
            || !novel_pixels_within(reference, &sampling_patch(patch), MATCH_MAX_NOVEL_PIXEL)
        {
            return false;
        }
    }
    if !pixel_match(reference, patch) {
        let filtered = filtered_reference(reference);
        if !MATCH_PHASE_OFFSETS.iter().any(|&dy| {
            MATCH_PHASE_OFFSETS
                .iter()
                .any(|&dx| pixel_match(&registered_reference(&filtered, dx, dy), patch))
        }) {
            return false;
        }
    }
    if !require_edges {
        return true;
    }
    let old_edges = edge_masks(reference);
    let new_edges = edge_masks(patch);
    (0..4).all(|side| expanded(old_edges[side]) & new_edges[side] != 0)
}

/// Fixed player samples may match any position, rather than the same cell's
/// raster. Register the whole appearance with bounded sampling support. The
/// local-color guard still uses the aligned original reference, so resampling
/// cannot invent an arbitrary new item color or learn another appearance.
pub(super) fn manual_reference_matches(reference: &Patch, patch: &Patch) -> bool {
    if reference_matches(reference, patch, false) {
        return true;
    }
    if !valid_patch(reference) || !valid_patch(patch) {
        return false;
    }
    let observed = sampling_patch(patch);
    // This is a superset of the final guard's support: one sample of phase,
    // one of interpolation, and the existing one-sample local neighborhood.
    // Reject unrelated colors before trying the bounded whole-tile transforms.
    if !manual_novel_pixels_within(reference, &observed, 3) {
        return false;
    }
    let filtered = sampling_patch(reference);
    for dy in MANUAL_PHASE_OFFSETS {
        for dx in MANUAL_PHASE_OFFSETS {
            let predicted = registered_reference(&filtered, dx, dy);
            let error = difference(&predicted, &observed, MANUAL_COMPARE_BEGIN, MANUAL_COMPARE_END);
            if error.mean <= MATCH_MAX_MEAN
                && error.medium_fraction <= MATCH_MAX_MEDIUM
                && error.large_fraction <= MATCH_MAX_LARGE
                && manual_novel_pixels_within(&registered_reference(reference, dx, dy), &observed, 1)
            {
                return true;
            }
        }
    }
    false
}

fn sampling_patch(patch: &Patch) -> Patch {
    let rgb = (0..PATCH_SIZE * PATCH_SIZE)
        .map(|i| {
            let x = i % PATCH_SIZE;
            let y = i / PATCH_SIZE;
            std::array::from_fn(|c| {
                let mut value = 0.0;
                for (dy, wy) in SAMPLING_AA_WEIGHTS.iter().enumerate() {
                    for (dx, wx) in SAMPLING_AA_WEIGHTS.iter().enumerate() {
                        let xx = (x + dx).saturating_sub(1).min(PATCH_SIZE - 1);
                        let yy = (y + dy).saturating_sub(1).min(PATCH_SIZE - 1);
                        value += patch.rgb[yy * PATCH_SIZE + xx][c] as f64 * wx * wy;
                    }
                }
                value.round() as u8
            })
        })
        .collect();
    Patch::from_rgb(rgb)
}

fn manual_novel_pixels_within(reference: &Patch, observed: &Patch, radius: usize) -> bool {
    for y in MANUAL_COMPARE_BEGIN..MANUAL_COMPARE_END {
        for x in MANUAL_COMPARE_BEGIN..MANUAL_COMPARE_END {
            let mut outside = 0.0;
            for c in 0..3 {
                let mut low = u8::MAX;
                let mut high = u8::MIN;
                for yy in y.saturating_sub(radius)..=(y + radius).min(PATCH_SIZE - 1) {
                    for xx in x.saturating_sub(radius)..=(x + radius).min(PATCH_SIZE - 1) {
                        let value = reference.rgb[yy * PATCH_SIZE + xx][c];
                        low = low.min(value);
                        high = high.max(value);
                    }
                }
                let value = observed.rgb[y * PATCH_SIZE + x][c];
                outside += low.saturating_sub(value) as f64 + value.saturating_sub(high) as f64;
            }
            if outside / 3.0 > MATCH_MAX_NOVEL_PIXEL {
                return false;
            }
        }
    }
    true
}

/// Positive geometry evidence for an initial board. Every one of the 45
/// cells needs a closed frame, and those frames must agree in normalized
/// position. A single exposed cell invalidates the bootstrap candidate.
pub(super) fn plausible_initial_grid(pixels: &[Patch]) -> bool {
    if pixels.len() != CELL_COUNT || pixels.iter().any(|p| !valid_patch(p)) {
        return false;
    }
    let masks: Vec<_> = pixels.iter().map(edge_masks).collect();
    let mut common = [u32::MAX; 4];
    for edges in &masks {
        for side in 0..4 {
            if edges[side] == 0 {
                return false;
            }
            common[side] &= expanded(edges[side]);
        }
    }
    if common.contains(&0) {
        return false;
    }
    // The initial central probes could otherwise accept disconnected stripes.
    // Verify all four sides almost to their intersections, at one rectangle
    // shared by the grid. A small corner trim permits rounded/antialiased ends.
    for left in 1..=EDGE_SEARCH_END {
        for right in (PATCH_SIZE - 1 - EDGE_SEARCH_END)..(PATCH_SIZE - 1) {
            for top in 1..=EDGE_SEARCH_END {
                for bottom in (PATCH_SIZE - 1 - EDGE_SEARCH_END)..(PATCH_SIZE - 1) {
                    let positions = [left, right, top, bottom];
                    if (0..4).all(|side| common[side] & (1 << positions[side]) != 0)
                        && pixels
                            .iter()
                            .zip(&masks)
                            .all(|(patch, edges)| rectangle_supported(patch, edges, positions))
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn valid_patch(patch: &Patch) -> bool {
    patch.rgb.len() == PATCH_SIZE * PATCH_SIZE
}

fn pixel_match(reference: &Patch, patch: &Patch) -> bool {
    let error = difference(reference, patch, COMPARE_BEGIN, COMPARE_END);
    error.mean <= MATCH_MAX_MEAN
        && error.medium_fraction <= MATCH_MAX_MEDIUM
        && error.large_fraction <= MATCH_MAX_LARGE
}

fn filtered_reference(reference: &Patch) -> Patch {
    let rgb = (0..PATCH_SIZE * PATCH_SIZE)
        .map(|i| {
            let x = i % PATCH_SIZE;
            let y = i / PATCH_SIZE;
            std::array::from_fn(|c| {
                let mut value = 0.0;
                for (dy, wy) in MATCH_AA_WEIGHTS.iter().enumerate() {
                    for (dx, wx) in MATCH_AA_WEIGHTS.iter().enumerate() {
                        let xx = (x + dx).saturating_sub(1).min(PATCH_SIZE - 1);
                        let yy = (y + dy).saturating_sub(1).min(PATCH_SIZE - 1);
                        value += reference.rgb[yy * PATCH_SIZE + xx][c] as f64 * wx * wy;
                    }
                }
                value.round() as u8
            })
        })
        .collect();
    Patch::from_rgb(rgb)
}

fn registered_reference(reference: &Patch, dx: f64, dy: f64) -> Patch {
    let rgb = (0..PATCH_SIZE * PATCH_SIZE)
        .map(|i| {
            let x = (i % PATCH_SIZE) as f64 + dx;
            let y = (i / PATCH_SIZE) as f64 + dy;
            let x = x.clamp(0.0, (PATCH_SIZE - 1) as f64);
            let y = y.clamp(0.0, (PATCH_SIZE - 1) as f64);
            let (x0, y0) = (x.floor() as usize, y.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(PATCH_SIZE - 1), (y0 + 1).min(PATCH_SIZE - 1));
            let (tx, ty) = (x - x0 as f64, y - y0 as f64);
            std::array::from_fn(|c| {
                let top = reference.rgb[y0 * PATCH_SIZE + x0][c] as f64 * (1.0 - tx)
                    + reference.rgb[y0 * PATCH_SIZE + x1][c] as f64 * tx;
                let bottom = reference.rgb[y1 * PATCH_SIZE + x0][c] as f64 * (1.0 - tx)
                    + reference.rgb[y1 * PATCH_SIZE + x1][c] as f64 * tx;
                (top * (1.0 - ty) + bottom * ty).round() as u8
            })
        })
        .collect();
    Patch::from_rgb(rgb)
}

fn expanded(mask: u32) -> u32 {
    // One normalized pixel is the permitted sampling phase difference.
    mask | (mask << EDGE_POSITION_TOLERANCE) | (mask >> EDGE_POSITION_TOLERANCE)
}

fn edge_masks(patch: &Patch) -> [u32; 4] {
    std::array::from_fn(|side| {
        let near = side == 0 || side == 2;
        let begin = if near {
            1
        } else {
            PATCH_SIZE - 1 - EDGE_SEARCH_END
        };
        let end = if near {
            EDGE_SEARCH_END + 1
        } else {
            PATCH_SIZE - 1
        };
        (begin..end).fold(0, |mask, position| {
            if coherent_edge(patch, side < 2, position, EDGE_PROBE_BEGIN, EDGE_PROBE_END) {
                mask | (1 << position)
            } else {
                mask
            }
        })
    })
}

fn rectangle_supported(patch: &Patch, edges: &[u32; 4], bounds: [usize; 4]) -> bool {
    let [left, right, top, bottom] = bounds;
    (0..4).all(|side| {
        let (begin, end) = if side < 2 {
            (top + CORNER_TRIM, bottom + 1 - CORNER_TRIM)
        } else {
            (left + CORNER_TRIM, right + 1 - CORNER_TRIM)
        };
        let position = bounds[side];
        (position.saturating_sub(EDGE_POSITION_TOLERANCE)
            ..=(position + EDGE_POSITION_TOLERANCE).min(PATCH_SIZE - 2))
            .any(|p| edges[side] & (1 << p) != 0 && coherent_edge(patch, side < 2, p, begin, end))
    })
}

fn coherent_edge(patch: &Patch, vertical: bool, position: usize, begin: usize, end: usize) -> bool {
    let deltas: Vec<[f64; 3]> = (begin..end)
        .map(|along| {
            let (a, b) = if vertical {
                (
                    along * PATCH_SIZE + position - 1,
                    along * PATCH_SIZE + position + 1,
                )
            } else {
                (
                    (position - 1) * PATCH_SIZE + along,
                    (position + 1) * PATCH_SIZE + along,
                )
            };
            std::array::from_fn(|c| patch.rgb[b][c] as f64 - patch.rgb[a][c] as f64)
        })
        .collect();
    let count = deltas.len();
    let mean: [f64; 3] =
        std::array::from_fn(|c| deltas.iter().map(|d| d[c]).sum::<f64>() / count as f64);
    if mean.iter().map(|v| v.abs()).sum::<f64>() / 3.0 < EDGE_MIN_CONTRAST {
        return false;
    }
    let norm = mean.iter().map(|v| v * v).sum::<f64>();
    let agrees: Vec<_> = deltas
        .iter()
        .map(|d| {
            d.iter().zip(mean).map(|(a, b)| a * b).sum::<f64>() >= norm * EDGE_DIRECTION_FRACTION
        })
        .collect();
    let fraction =
        |a: usize, b: usize| agrees[a..b].iter().filter(|v| **v).count() as f64 / (b - a) as f64;
    fraction(0, count) >= EDGE_MIN_AGREEMENT
        && fraction(0, count / 2) >= EDGE_HALF_AGREEMENT
        && fraction(count / 2, count) >= EDGE_HALF_AGREEMENT
        && fraction(0, 3) >= 2.0 / 3.0
        && fraction(count - 3, count) >= 2.0 / 3.0
}

fn novel_pixels_within(reference: &Patch, observed: &Patch, maximum: f64) -> bool {
    for y in COMPARE_BEGIN..COMPARE_END {
        for x in COMPARE_BEGIN..COMPARE_END {
            let mut outside = 0.0;
            for c in 0..3 {
                let mut low = u8::MAX;
                let mut high = u8::MIN;
                for yy in y - 1..=y + 1 {
                    for xx in x - 1..=x + 1 {
                        let value = reference.rgb[yy * PATCH_SIZE + xx][c];
                        low = low.min(value);
                        high = high.max(value);
                    }
                }
                let value = observed.rgb[y * PATCH_SIZE + x][c];
                outside += low.saturating_sub(value) as f64 + value.saturating_sub(high) as f64;
            }
            if outside / 3.0 > maximum {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::Rect;
    use super::*;
    use image::{imageops, RgbaImage};

    fn generated_grid() -> Vec<Patch> {
        (0..CELL_COUNT)
            .map(|index| {
                let fill = [
                    50 + (index % 4) as u8 * 40,
                    25,
                    105 + (index % 3) as u8 * 50,
                ];
                let mut rgb = vec![[15, 32, 48]; PATCH_SIZE * PATCH_SIZE];
                for y in 3..29 {
                    for x in 3..29 {
                        rgb[y * PATCH_SIZE + x] = if x == 3 || x == 28 || y == 3 || y == 28 {
                            [235, 200, 250]
                        } else {
                            // A flat outlined tile with new colors and a small
                            // interior texture, without the captured bevel/groove.
                            [fill[0] + ((x + y) % 3) as u8, fill[1], fill[2]]
                        };
                    }
                }
                Patch::from_rgb(rgb)
            })
            .collect()
    }

    fn initial_frame() -> RgbaImage {
        image::load_from_memory(include_bytes!("../tests/fixtures/vision-initial.png"))
            .unwrap()
            .to_rgba8()
    }

    fn read_grid(frame: &RgbaImage, board: Rect) -> Vec<Patch> {
        (0..CELL_COUNT)
            .map(|i| Patch::read(frame, board.cell(i)))
            .collect()
    }

    fn original_grid(frame: &RgbaImage) -> Vec<Patch> {
        read_grid(
            frame,
            Rect {
                x: 910.0,
                y: 346.0,
                w: 936.0,
                h: 520.0,
            },
        )
    }

    #[test]
    fn stable_two_frames_learn_new_frame_and_colors_by_cell() {
        let grid = generated_grid();
        assert!(plausible_initial_grid(&grid));
        let mut covers = InitialCovers::default();
        assert!(!covers.observe(&grid, true));
        assert!(!covers.is_ready());
        assert!(covers.observe(&grid, true));
        for (index, patch) in grid.iter().enumerate() {
            assert!(covers.matches(index, patch), "cell {index}");
        }
        assert!(!covers.matches(0, &grid[1]));
        assert!(!covers.matches(CELL_COUNT, &grid[0]));
    }

    #[test]
    fn ineligible_frames_and_occlusion_break_consecutive_learning() {
        let grid = generated_grid();
        let flat = vec![Patch::from_rgb(vec![[80, 100, 120]; PATCH_SIZE * PATCH_SIZE]); CELL_COUNT];
        let mut covers = InitialCovers::default();
        assert!(!covers.observe(&grid, false));
        assert!(!covers.observe(&grid, false));
        assert!(!covers.observe(&grid, true));
        assert!(!covers.observe(&grid, false));
        assert!(!covers.observe(&grid, true));
        assert!(!covers.observe(&flat, true));
        assert!(!covers.observe(&grid, true));
        assert!(covers.observe(&grid, true));
        covers.clear_pending();
        assert!(covers.is_ready());
        covers.reset();
        assert!(!covers.is_ready());
        assert!(!covers.matches(0, &grid[0]));
    }

    #[test]
    fn changing_pixels_break_learning_and_never_overwrite_confirmed_references() {
        let grid = generated_grid();
        let mut changed = grid.clone();
        for y in 14..17 {
            for x in 14..17 {
                changed[7].rgb[y * PATCH_SIZE + x] = [250, 250, 250];
            }
        }
        let mut covers = InitialCovers::default();
        assert!(!covers.observe(&grid, true));
        assert!(!covers.observe(&changed, true));
        assert!(!covers.observe(&grid, true));
        assert!(covers.observe(&grid, true));
        assert!(
            !covers.matches(7, &changed[7]),
            "a small exposed region is not a cover"
        );
        assert!(covers.observe(&changed, true));
        assert!(covers.observe(&changed, true));
        assert!(covers.matches(7, &grid[7]));
        assert!(!covers.matches(7, &changed[7]));
        assert!(covers.observe(&grid, false));
    }

    #[test]
    fn same_sampling_size_keeps_raw_single_pixel_novelty_guard() {
        let mut reference = generated_grid()[0].clone();
        reference.source_size = Some([104.0, 104.0]);
        let mut changed = reference.clone();
        for channel in &mut changed.rgb[16 * PATCH_SIZE + 16] {
            *channel += 30;
        }
        assert!(!novel_pixels_within(&reference, &changed, MATCH_MAX_NOVEL_PIXEL));
        assert!(novel_pixels_within(&reference, &sampling_patch(&changed), MATCH_MAX_NOVEL_PIXEL));
        assert!(!reference_matches(&reference, &changed, true));
    }

    #[test]
    fn flat_texture_stripes_and_one_unframed_cell_are_not_initial_grids() {
        let flat = Patch::from_rgb(vec![[80, 100, 120]; PATCH_SIZE * PATCH_SIZE]);
        assert!(!plausible_initial_grid(&vec![flat.clone(); CELL_COUNT]));
        let texture = Patch::from_rgb(
            (0..PATCH_SIZE * PATCH_SIZE)
                .map(|i| {
                    let x = i % PATCH_SIZE;
                    let y = i / PATCH_SIZE;
                    let value = ((x * 37 + y * 53 + x * y * 11) % 170) as u8;
                    [value, value.saturating_add(40), 210 - value]
                })
                .collect(),
        );
        assert!(!plausible_initial_grid(&vec![texture; CELL_COUNT]));
        let stripes = Patch::from_rgb(
            (0..PATCH_SIZE * PATCH_SIZE)
                .map(|i| {
                    if i % PATCH_SIZE % 6 < 3 {
                        [20, 50, 100]
                    } else {
                        [150, 180, 210]
                    }
                })
                .collect(),
        );
        assert!(!plausible_initial_grid(&vec![stripes; CELL_COUNT]));
        let mut grid = generated_grid();
        grid[21] = flat;
        assert!(!plausible_initial_grid(&grid));
        assert!(!plausible_initial_grid(&grid[..CELL_COUNT - 1]));
    }

    #[test]
    fn real_initial_frames_pass_and_any_real_opened_cell_fails_bootstrap() {
        assert!(plausible_initial_grid(&original_grid(&initial_frame())));
        let opened = image::load_from_memory(include_bytes!("../tests/fixtures/vision-opened.png"))
            .unwrap()
            .to_rgba8();
        assert!(!plausible_initial_grid(&original_grid(&opened)));
        for (bytes, y) in [
            (
                &include_bytes!("../tests/fixtures/pc-global-zh-hant-initial.png")[..],
                235.6666666667,
            ),
            (
                &include_bytes!("../tests/fixtures/pc-global-zh-hant-four-three-initial.png")[..],
                355.6666666667,
            ),
        ] {
            let frame = image::load_from_memory(bytes).unwrap().to_rgba8();
            let grid = read_grid(
                &frame,
                Rect {
                    x: 607.3333333333,
                    y,
                    w: 624.0,
                    h: 346.6666666667,
                },
            );
            assert!(plausible_initial_grid(&grid), "native board y={y}");
        }
    }

    #[test]
    fn confirmed_real_cells_match_after_uniform_resizing() {
        let frame = initial_frame();
        let grid = original_grid(&frame);
        let mut covers = InitialCovers::default();
        assert!(!covers.observe(&grid, true));
        assert!(covers.observe(&grid, true));
        let game = imageops::crop_imm(&frame, 2, 60, 1920, 1080).to_image();
        for width in [960, 1440, 1920, 2560, 3840] {
            let scaled =
                imageops::resize(&game, width, width * 9 / 16, imageops::FilterType::Triangle);
            let scale = width as f64 / 1920.0;
            let resized = read_grid(
                &scaled,
                Rect {
                    x: 908.0 * scale,
                    y: 286.0 * scale,
                    w: 936.0 * scale,
                    h: 520.0 * scale,
                },
            );
            assert!(plausible_initial_grid(&resized), "width={width}");
            for (index, patch) in resized.iter().enumerate() {
                assert!(covers.matches(index, patch), "width={width}, cell={index}");
            }
        }
    }
}
