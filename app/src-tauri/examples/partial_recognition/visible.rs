//! Observed foreground is the denominator, including pixels outside a pose's
//! predicted mask/core. This adds evidence without changing retained scores.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WeightedCoreEvidence {
    pub color_error: Option<f64>,
    pub bidirectional_score: Option<f64>,
    pub weight: f64,
    pub pixels: usize,
    pub low_confidence_pixels: usize,
}

/// Keep the original RGB search gate and scores intact. Only the experimental
/// final/foreground-refinement score uses the already configured confidence.
/// Do not cap core RGB errors: contradictory interior art retains its full loss.
pub(super) fn measure_core(
    image: &RgbaImage,
    c: &Candidate,
    cells: &[String],
    cfg: &Config,
    penalty: f64,
    mut expected: impl FnMut(usize, usize) -> (bool, [u8; 3]),
) -> WeightedCoreEvidence {
    let p = &c.placement;
    let mut result = WeightedCoreEvidence {
        color_error: None, bidirectional_score: None, weight: 0.0,
        pixels: 0, low_confidence_pixels: 0,
    };
    let mut loss = 0.0;
    for y in 0..p.height * MATCH_CELL {
        for x in 0..p.width * MATCH_CELL {
            let cell = (p.y + y / MATCH_CELL) * COLS + p.x + x / MATCH_CELL;
            if cells[cell] == "unknown" { continue; }
            let (core, predicted) = expected(x, y);
            if !core { continue; }
            let observed = rgb(image, p.x * MATCH_CELL + x, p.y * MATCH_CELL + y);
            let low_confidence = x % MATCH_CELL < cfg.reliable_border
                || x % MATCH_CELL >= MATCH_CELL - cfg.reliable_border
                || y % MATCH_CELL < cfg.reliable_border
                || y % MATCH_CELL >= MATCH_CELL - cfg.reliable_border
                || super::super::super::selection_green(observed);
            let weight = if low_confidence { cfg.low_confidence_weight } else { 1.0 };
            loss += distance(observed, predicted) * weight;
            result.weight += weight;
            result.pixels += 1;
            result.low_confidence_pixels += usize::from(low_confidence);
        }
    }
    if result.weight > 0.0 {
        let error = loss / result.weight;
        result.color_error = Some(error);
        result.bidirectional_score = Some(error + penalty);
    }
    result
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisibleEvidence {
    pub foreground_color_error: Option<f64>,
    #[serde(default)]
    pub uncapped_foreground_color_error: Option<f64>,
    pub missing_foreground_fraction: Option<f64>,
    pub contradicted_foreground_fraction: Option<f64>,
    pub reliable_weight: f64,
    pub reliable_pixels: usize,
    #[serde(default)]
    pub low_confidence_pixels: usize,
    #[serde(default)]
    pub capped_pixels: usize,
    pub tiles: Vec<VisibleTileEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisibleTileEvidence {
    pub board_cell: usize,
    pub column: usize,
    pub row: usize,
    pub foreground_color_error: Option<f64>,
    #[serde(default)]
    pub uncapped_foreground_color_error: Option<f64>,
    pub missing_foreground_fraction: Option<f64>,
    pub contradicted_foreground_fraction: Option<f64>,
    pub reliable_weight: f64,
    pub reliable_pixels: usize,
    #[serde(default)]
    pub low_confidence_pixels: usize,
    #[serde(default)]
    pub capped_pixels: usize,
}

#[derive(Clone, Default)]
struct Accumulator {
    color_error: f64,
    uncapped_color_error: f64,
    missing: f64,
    contradicted: f64,
    weight: f64,
    pixels: usize,
    low_confidence_pixels: usize,
    capped_pixels: usize,
}
impl Accumulator {
    fn add(&mut self, error: f64, mask: bool, border: bool, weight: f64, cfg: &Config) {
        let capped_error = if border {
            cfg.low_confidence_error_cap
                .map_or(error, |cap| error.min(cap))
        } else {
            error
        };
        self.color_error += capped_error * weight;
        self.uncapped_color_error += error * weight;
        self.missing += f64::from(!mask) * weight;
        self.contradicted += f64::from(!mask || error > cfg.mismatch_error) * weight;
        self.weight += weight;
        self.pixels += 1;
        self.low_confidence_pixels += usize::from(border);
        self.capped_pixels += usize::from(capped_error < error);
    }
    fn mean(&self, value: f64) -> Option<f64> {
        (self.weight > 0.0).then(|| value / self.weight)
    }
}

/// Both backends use this same row-major traversal and accumulation. The
/// closure supplies the exact existing transform at the local pixel center.
/// Unknown cells never invoke the transform closure or contribute evidence.
pub(super) fn measure(
    image: &RgbaImage,
    c: &Candidate,
    cells: &[String],
    cfg: &Config,
    mut expected: impl FnMut(usize, usize) -> (bool, [u8; 3]),
) -> VisibleEvidence {
    let p = &c.placement;
    let (w, h) = (p.width * MATCH_CELL, p.height * MATCH_CELL);
    let mut total = Accumulator::default();
    let mut tiles = vec![Accumulator::default(); p.width * p.height];
    // Include 0 and w/h-1: unlike the existing edge scorer, foreground
    // evidence requires no neighbor pixel and must cover rectangle boundaries.
    for y in 0..h {
        for x in 0..w {
            let (column, row) = (x / MATCH_CELL, y / MATCH_CELL);
            let board_cell = (p.y + row) * COLS + p.x + column;
            if cells[board_cell] == "unknown" {
                continue;
            }
            let observed = rgb(image, p.x * MATCH_CELL + x, p.y * MATCH_CELL + y);
            if !reliable(observed, cfg) {
                continue;
            }
            let border = x % MATCH_CELL < cfg.reliable_border
                || x % MATCH_CELL >= MATCH_CELL - cfg.reliable_border
                || y % MATCH_CELL < cfg.reliable_border
                || y % MATCH_CELL >= MATCH_CELL - cfg.reliable_border;
            let weight = if border || super::super::super::selection_green(observed) {
                cfg.low_confidence_weight
            } else {
                1.0
            };
            let (mask, predicted) = expected(x, y);
            let error = if mask {
                distance(observed, predicted)
            } else {
                255.0
            };
            total.add(error, mask, border, weight, cfg);
            tiles[row * p.width + column].add(error, mask, border, weight, cfg);
        }
    }
    let tiles = tiles
        .into_iter()
        .enumerate()
        .filter_map(|(local, tile)| {
            let (column, row) = (local % p.width, local / p.width);
            let board_cell = (p.y + row) * COLS + p.x + column;
            (cells[board_cell] != "unknown").then(|| VisibleTileEvidence {
                board_cell,
                column,
                row,
                foreground_color_error: tile.mean(tile.color_error),
                uncapped_foreground_color_error: tile.mean(tile.uncapped_color_error),
                missing_foreground_fraction: tile.mean(tile.missing),
                contradicted_foreground_fraction: tile.mean(tile.contradicted),
                reliable_weight: tile.weight,
                reliable_pixels: tile.pixels,
                low_confidence_pixels: tile.low_confidence_pixels,
                capped_pixels: tile.capped_pixels,
            })
        })
        .collect();
    VisibleEvidence {
        foreground_color_error: total.mean(total.color_error),
        uncapped_foreground_color_error: total.mean(total.uncapped_color_error),
        missing_foreground_fraction: total.mean(total.missing),
        contradicted_foreground_fraction: total.mean(total.contradicted),
        reliable_weight: total.weight,
        reliable_pixels: total.pixels,
        low_confidence_pixels: total.low_confidence_pixels,
        capped_pixels: total.capped_pixels,
        tiles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOREGROUND: [u8; 4] = [180, 30, 70, 255];
    const BACKGROUND: [u8; 4] = [240, 240, 240, 255];

    fn config() -> Config {
        Config {
            coarse_angle_step: 90,
            fills: vec![0.92],
            aspect_log_tolerance: 0.4,
            max_color_error: 255.0,
            min_anchor_pixels: 1,
            edge_weight: 0.2,
            reverse_weight: 17.0,
            foreground_chroma: 25,
            foreground_dark: 180,
            reliable_border: 3,
            low_confidence_weight: 0.1,
            low_confidence_error_cap: None,
            mismatch_error: 35.0,
            min_texture_std: 0.0,
            refine: vec![],
            coarse_offset_rescue: false,
            foreground_refinement: false,
            confidence_weighted_scoring: false,
            threshold_free_search: false,
            max_evaluations: 0,
            tile_cache_bytes: default_tile_cache_bytes(),
            extraction: ExtractionConfig::default(),
        }
    }
    fn candidate(width: usize, height: usize) -> Candidate {
        Candidate {
            placement: GridPlacement {
                x: 1,
                y: 1,
                width,
                height,
            },
            angle: 0.0,
            fill: 0.92,
            dx: 0.0,
            dy: 0.0,
            score: 0.0,
            counts: [0; 45],
        }
    }
    fn blank() -> RgbaImage {
        RgbaImage::from_pixel(
            (COLS * MATCH_CELL) as u32,
            (ROWS * MATCH_CELL) as u32,
            Rgba(BACKGROUND),
        )
    }
    fn pixel(image: &mut RgbaImage, c: &Candidate, x: usize, y: usize, color: [u8; 4]) {
        image.put_pixel(
            (c.placement.x * MATCH_CELL + x) as u32,
            (c.placement.y * MATCH_CELL + y) as u32,
            Rgba(color),
        );
    }
    fn known(c: &Candidate) -> Vec<String> {
        let mut cells = vec!["unknown".into(); COLS * ROWS];
        for row in 0..c.placement.height {
            for col in 0..c.placement.width {
                cells[(c.placement.y + row) * COLS + c.placement.x + col] = "uncertain".into();
            }
        }
        cells
    }
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-8, "{a} != {b}");
    }

    #[test]
    fn core_confidence_preserves_interior_errors_and_ignores_covered_cells() {
        let c = candidate(2, 1);
        let mut cfg = config();
        cfg.low_confidence_weight = 0.1;
        let mut image = blank();
        pixel(&mut image, &c, 0, 0, [0, 0, 0, 255]);
        pixel(&mut image, &c, 8, 8, [30, 30, 30, 255]);
        let mut cells = known(&c);
        cells[c.placement.y * COLS + c.placement.x + 1] = "unknown".into();
        let evidence = measure_core(&image, &c, &cells, &cfg, 5.0, |x, y| {
            assert!(x < MATCH_CELL);
            ([(0, 0), (8, 8)].contains(&(x, y)), [100; 3])
        });
        close(evidence.color_error.unwrap(), (100.0 * 0.1 + 70.0) / 1.1);
        close(evidence.bidirectional_score.unwrap(), evidence.color_error.unwrap() + 5.0);
        assert_eq!(evidence.pixels, 2);
        assert_eq!(evidence.low_confidence_pixels, 1);
        cfg.low_confidence_weight = 0.0;
        let border_only = measure_core(&image, &c, &cells, &cfg, 5.0, |x, y| {
            (x == 0 && y == 0, [100; 3])
        });
        assert_eq!(border_only.color_error, None);
        assert_eq!(border_only.bidirectional_score, None);
    }

    #[test]
    fn revealed_foreground_outside_predicted_end_has_worst_pixel_error() {
        let c = candidate(2, 1);
        let cfg = config();
        let mut image = blank();
        for y in 0..MATCH_CELL {
            for x in 0..2 * MATCH_CELL {
                pixel(&mut image, &c, x, y, FOREGROUND);
            }
        }
        let evidence = measure(&image, &c, &known(&c), &cfg, |x, _| {
            (x < MATCH_CELL, [180, 30, 70])
        });
        assert_eq!(evidence.reliable_pixels, 2 * MATCH_CELL * MATCH_CELL);
        close(evidence.foreground_color_error.unwrap(), 127.5);
        close(evidence.missing_foreground_fraction.unwrap(), 0.5);
        close(evidence.contradicted_foreground_fraction.unwrap(), 0.5);
        assert_eq!(evidence.tiles.len(), 2);
        assert_eq!(evidence.tiles[0].foreground_color_error, Some(0.0));
        close(evidence.tiles[1].foreground_color_error.unwrap(), 255.0);
        assert_eq!(evidence.tiles[1].missing_foreground_fraction, Some(1.0));
    }

    #[test]
    fn covered_cell_foreground_never_invokes_transform_or_changes_evidence() {
        let c = candidate(2, 1);
        let cfg = config();
        let mut image = blank();
        for y in 0..MATCH_CELL {
            for x in 0..2 * MATCH_CELL {
                pixel(&mut image, &c, x, y, FOREGROUND);
            }
        }
        let mut cells = known(&c);
        cells[c.placement.y * COLS + c.placement.x + 1] = "unknown".into();
        let mut calls = 0;
        let first = measure(&image, &c, &cells, &cfg, |x, _| {
            assert!(x < MATCH_CELL);
            calls += 1;
            (true, [180, 30, 70])
        });
        assert_eq!(calls, MATCH_CELL * MATCH_CELL);
        for y in 0..MATCH_CELL {
            for x in MATCH_CELL..2 * MATCH_CELL {
                pixel(&mut image, &c, x, y, [0, 0, 0, 255]);
            }
        }
        let second = measure(&image, &c, &cells, &cfg, |x, _| {
            assert!(x < MATCH_CELL);
            (true, [180, 30, 70])
        });
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&second).unwrap()
        );
        assert_eq!(first.tiles.len(), 1);
        assert_eq!(
            first.tiles[0].board_cell,
            c.placement.y * COLS + c.placement.x
        );
    }

    #[test]
    fn zero_observed_weight_returns_no_success_error_or_fraction() {
        let c = candidate(1, 1);
        let mut cfg = config();
        let mut image = blank();
        let evidence = measure(&image, &c, &known(&c), &cfg, |_, _| {
            panic!("background cannot invoke transform")
        });
        assert_eq!(evidence.reliable_pixels, 0);
        assert_eq!(evidence.reliable_weight, 0.0);
        assert_eq!(evidence.foreground_color_error, None);
        assert_eq!(evidence.missing_foreground_fraction, None);
        assert_eq!(evidence.contradicted_foreground_fraction, None);
        assert_eq!(evidence.tiles[0].foreground_color_error, None);
        cfg.low_confidence_weight = 0.0;
        pixel(&mut image, &c, 0, 0, FOREGROUND);
        let zero_weight = measure(&image, &c, &known(&c), &cfg, |_, _| (false, [0; 3]));
        assert_eq!(zero_weight.reliable_pixels, 1);
        assert_eq!(zero_weight.reliable_weight, 0.0);
        assert_eq!(zero_weight.foreground_color_error, None);
        assert_eq!(zero_weight.missing_foreground_fraction, None);
        assert_eq!(zero_weight.contradicted_foreground_fraction, None);
        assert_eq!(zero_weight.tiles[0].reliable_pixels, 1);
        assert_eq!(zero_weight.tiles[0].foreground_color_error, None);
    }

    #[test]
    fn configured_border_is_downweighted_and_selection_green_is_excluded() {
        let c = candidate(1, 1);
        let cfg = config();
        let mut image = blank();
        pixel(&mut image, &c, 0, 0, FOREGROUND);
        pixel(&mut image, &c, 10, 10, FOREGROUND);
        pixel(&mut image, &c, 11, 11, [100, 220, 50, 255]);
        let evidence = measure(&image, &c, &known(&c), &cfg, |x, y| {
            (x == 10 && y == 10, [180, 30, 70])
        });
        assert_eq!(evidence.reliable_pixels, 2);
        close(evidence.reliable_weight, 1.1);
        close(evidence.foreground_color_error.unwrap(), 25.5 / 1.1);
        close(evidence.missing_foreground_fraction.unwrap(), 0.1 / 1.1);
        close(
            evidence.contradicted_foreground_fraction.unwrap(),
            0.1 / 1.1,
        );
        assert_eq!(evidence.low_confidence_pixels, 1);
        assert_eq!(evidence.capped_pixels, 0);
        assert_eq!(
            evidence.uncapped_foreground_color_error,
            evidence.foreground_color_error
        );
    }

    #[test]
    fn error_cap_only_changes_border_color_and_preserves_raw_fractions() {
        let c = candidate(2, 1);
        let mut cfg = config();
        let mut image = blank();
        for (x, y) in [(0, 0), (1, 0), (2, 0), (3, 3), (10, 10), (MATCH_CELL, 0)] {
            pixel(&mut image, &c, x, y, FOREGROUND);
        }
        pixel(&mut image, &c, 11, 11, [100, 220, 50, 255]);
        let mut cells = known(&c);
        cells[c.placement.y * COLS + c.placement.x + 1] = "unknown".into();
        let predicted = |x, y| {
            assert!(x < MATCH_CELL, "unknown pixels cannot invoke the transform");
            assert_ne!((x, y), (11, 11), "selection green remains excluded");
            match (x, y) {
                (0, 0) | (3, 3) => (false, [0; 3]),
                (1, 0) | (10, 10) => (true, [20, 20, 200]),
                (2, 0) => (true, [180, 30, 70]),
                _ => panic!("background cannot invoke the transform"),
            }
        };
        let legacy = measure(&image, &c, &cells, &cfg, predicted);
        cfg.low_confidence_error_cap = Some(35.0);
        let capped = measure(&image, &c, &cells, &cfg, predicted);
        assert_eq!(capped.reliable_pixels, 5);
        assert_eq!(capped.low_confidence_pixels, 3);
        assert_eq!(capped.capped_pixels, 2);
        close(capped.reliable_weight, 2.3);
        // Both the 255 missing-mask error and the 100 RGB error are capped
        // on the border; the same two errors in the interior remain intact.
        close(capped.foreground_color_error.unwrap(), 362.0 / 2.3);
        close(capped.uncapped_foreground_color_error.unwrap(), 390.5 / 2.3);
        assert_eq!(
            capped.uncapped_foreground_color_error.unwrap().to_bits(),
            legacy.foreground_color_error.unwrap().to_bits()
        );
        assert_eq!(capped.reliable_pixels, legacy.reliable_pixels);
        assert_eq!(
            capped.reliable_weight.to_bits(),
            legacy.reliable_weight.to_bits()
        );
        assert_eq!(
            capped.missing_foreground_fraction,
            legacy.missing_foreground_fraction
        );
        assert_eq!(
            capped.contradicted_foreground_fraction,
            legacy.contradicted_foreground_fraction
        );
        close(capped.missing_foreground_fraction.unwrap(), 1.1 / 2.3);
        close(capped.contradicted_foreground_fraction.unwrap(), 2.2 / 2.3);
        assert_eq!(capped.tiles.len(), 1);
        let tile = &capped.tiles[0];
        assert_eq!(tile.low_confidence_pixels, capped.low_confidence_pixels);
        assert_eq!(tile.capped_pixels, capped.capped_pixels);
        assert_eq!(tile.foreground_color_error, capped.foreground_color_error);
        assert_eq!(
            tile.uncapped_foreground_color_error,
            capped.uncapped_foreground_color_error
        );
        assert_eq!(
            tile.missing_foreground_fraction,
            capped.missing_foreground_fraction
        );
        assert_eq!(
            tile.contradicted_foreground_fraction,
            capped.contradicted_foreground_fraction
        );
        cfg.low_confidence_error_cap = Some(255.0);
        let unchanged = measure(&image, &c, &cells, &cfg, predicted);
        assert_eq!(
            unchanged.foreground_color_error,
            legacy.foreground_color_error
        );
        assert_eq!(unchanged.capped_pixels, 0);
    }

    #[test]
    fn all_rectangle_and_internal_cell_boundaries_supply_foreground_evidence() {
        let c = candidate(2, 1);
        let cfg = config();
        let mut image = blank();
        let (w, h) = (2 * MATCH_CELL, MATCH_CELL);
        for y in 0..h {
            for x in 0..w {
                if x == 0
                    || x == w - 1
                    || y == 0
                    || y == h - 1
                    || x == MATCH_CELL - 1
                    || x == MATCH_CELL
                {
                    pixel(&mut image, &c, x, y, FOREGROUND);
                }
            }
        }
        let evidence = measure(&image, &c, &known(&c), &cfg, |_, _| (false, [0; 3]));
        let per_cell_perimeter = 4 * MATCH_CELL - 4;
        assert_eq!(evidence.reliable_pixels, 2 * per_cell_perimeter);
        close(
            evidence.reliable_weight,
            2.0 * per_cell_perimeter as f64 * cfg.low_confidence_weight,
        );
        close(evidence.foreground_color_error.unwrap(), 255.0);
        for tile in &evidence.tiles {
            assert_eq!(tile.reliable_pixels, per_cell_perimeter);
            close(
                tile.reliable_weight,
                per_cell_perimeter as f64 * cfg.low_confidence_weight,
            );
        }
    }

    #[test]
    fn multi_tile_metrics_keep_real_coordinates_and_separate_missing_from_contradiction() {
        let c = candidate(2, 2);
        let cfg = config();
        let mut image = blank();
        for y in 0..2 * MATCH_CELL {
            for x in 0..2 * MATCH_CELL {
                pixel(&mut image, &c, x, y, FOREGROUND);
            }
        }
        let mut cells = known(&c);
        cells[(c.placement.y + 1) * COLS + c.placement.x + 1] = "unknown".into();
        let evidence = measure(&image, &c, &cells, &cfg, |x, y| {
            if x >= MATCH_CELL {
                (false, [0; 3])
            } else if y >= MATCH_CELL {
                (true, [20, 20, 200])
            } else {
                (true, [180, 30, 70])
            }
        });
        assert_eq!(evidence.reliable_pixels, 3 * MATCH_CELL * MATCH_CELL);
        close(evidence.foreground_color_error.unwrap(), 355.0 / 3.0);
        close(evidence.missing_foreground_fraction.unwrap(), 1.0 / 3.0);
        close(
            evidence.contradicted_foreground_fraction.unwrap(),
            2.0 / 3.0,
        );
        assert_eq!(
            evidence
                .tiles
                .iter()
                .map(|t| (t.column, t.row, t.board_cell))
                .collect::<Vec<_>>(),
            vec![(0, 0, 10), (1, 0, 11), (0, 1, 19)]
        );
        close(evidence.tiles[0].foreground_color_error.unwrap(), 0.0);
        close(evidence.tiles[1].foreground_color_error.unwrap(), 255.0);
        close(evidence.tiles[2].foreground_color_error.unwrap(), 100.0);
        assert_eq!(evidence.tiles[2].missing_foreground_fraction, Some(0.0));
        assert_eq!(
            evidence.tiles[2].contradicted_foreground_fraction,
            Some(1.0)
        );
    }
}
