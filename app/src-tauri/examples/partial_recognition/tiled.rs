//! Pre-split transforms for the default-off offline experiment.
//! Board position and ground truth never enter a transform key.
use super::*;
use std::{collections::BTreeSet, mem::size_of, sync::Arc};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TileCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    /// Conservatively charged storage, including tile and cache metadata.
    pub peak_bytes: usize,
    pub resident_bytes: usize,
    pub preparation_ms: f64,
    /// Visible blocks visited by base comparisons, including rejected poses.
    pub scored_visible_blocks: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TileEvidence {
    pub column: usize,
    pub row: usize,
    pub board_cell: usize,
    pub core_pixels: usize,
    pub color_error: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TileDiagnostic {
    pub id: String,
    pub image: String,
    pub mask: String,
    pub core: String,
    pub columns: usize,
    pub rows: usize,
    pub cell_pixels: usize,
    pub anchor_tile: [usize; 2],
    pub visible_tiles: Vec<TileEvidence>,
    // Keep the evaluated angle bits even when Pose.angle is normalized to 0..360.
    // Asset rebuilding must use the actual evaluated transform.
    #[serde(skip)]
    key: Option<Key>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    item: usize,
    revision: u64,
    columns: usize,
    rows: usize,
    angle: u64,
    fill: u64,
    dx: u64,
    dy: u64,
}
impl Key {
    fn new(item: usize, revision: u64, c: &Candidate) -> Self {
        Self {
            item,
            revision,
            columns: c.placement.width,
            rows: c.placement.height,
            angle: c.angle.to_bits(),
            fill: c.fill.to_bits(),
            dx: c.dx.to_bits(),
            dy: c.dy.to_bits(),
        }
    }
    fn candidate(self) -> Candidate {
        Candidate {
            placement: GridPlacement {
                x: 0,
                y: 0,
                width: self.columns,
                height: self.rows,
            },
            angle: f64::from_bits(self.angle),
            fill: f64::from_bits(self.fill),
            dx: f64::from_bits(self.dx),
            dy: f64::from_bits(self.dy),
            score: 255.0,
            counts: [0; 45],
        }
    }
}

#[derive(Clone, Copy)]
struct Pixel {
    expected: [u8; 3],
    mask: bool,
    core: bool,
}
struct Tile {
    column: usize,
    row: usize,
    pixels: Box<[Pixel]>,
}
pub(super) struct Transformed {
    columns: usize,
    rows: usize,
    scale: f64,
    tiles: Box<[Tile]>,
}
impl Transformed {
    fn new(t: &CardTemplate, r: Rotation, c: &Candidate) -> Self {
        let (columns, rows) = (c.placement.width, c.placement.height);
        let (w, h) = (columns * MATCH_CELL, rows * MATCH_CELL);
        let scale = (w as f64 / r.w).min(h as f64 / r.h) * c.fill;
        let (ox, oy) = (
            (w as f64 - r.w * scale) / 2.0 + c.dx,
            (h as f64 - r.h * scale) / 2.0 + c.dy,
        );
        let tiles = (0..rows)
            .flat_map(|row| {
                (0..columns).map(move |column| {
                    let pixels = (0..MATCH_CELL)
                        .flat_map(|local_y| {
                            (0..MATCH_CELL).map(move |local_x| {
                                let (x, y) =
                                    (column * MATCH_CELL + local_x, row * MATCH_CELL + local_y);
                                let (sx, sy) = r.source(
                                    (x as f64 + 0.5 - ox) / scale,
                                    (y as f64 + 0.5 - oy) / scale,
                                );
                                let mask = mask_at(&t.mask, sx, sy);
                                let core = mask_at(&t.core, sx, sy);
                                Pixel {
                                    expected: if mask || core {
                                        sample(&t.image, sx, sy)
                                    } else {
                                        [0; 3]
                                    },
                                    mask,
                                    core,
                                }
                            })
                        })
                        .collect::<Vec<_>>()
                        .into_boxed_slice();
                    Tile {
                        column,
                        row,
                        pixels,
                    }
                })
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            columns,
            rows,
            scale,
            tiles,
        }
    }
    fn at(&self, x: usize, y: usize) -> Pixel {
        self.tiles[y / MATCH_CELL * self.columns + x / MATCH_CELL].pixels
            [y % MATCH_CELL * MATCH_CELL + x % MATCH_CELL]
    }
    fn bytes(&self) -> usize {
        size_of::<Self>()
            + self.tiles.len() * size_of::<Tile>()
            + self.tiles.len() * MATCH_CELL * MATCH_CELL * size_of::<Pixel>()
            + 2 * size_of::<usize>() // Arc counters
    }
}

struct Entry {
    transform: Arc<Transformed>,
    bytes: usize,
    used: u64,
}
#[derive(Default)]
pub(super) struct Cache {
    entries: BTreeMap<Key, Entry>,
    budget: usize,
    clock: u64,
    stats: TileCacheStats,
}
impl Cache {
    pub(super) fn begin_run(&mut self, budget: usize) {
        self.budget = budget;
        self.stats = TileCacheStats {
            resident_bytes: self.stats.resident_bytes,
            peak_bytes: 0,
            ..TileCacheStats::default()
        };
        while self.stats.resident_bytes > budget {
            self.evict_oldest();
        }
        self.stats.peak_bytes = self.stats.resident_bytes;
    }
    pub(super) fn invalidate(&mut self, item: usize) {
        let keys: Vec<_> = self
            .entries
            .keys()
            .filter(|k| k.item == item)
            .copied()
            .collect();
        for key in keys {
            self.remove(key);
        }
    }
    fn remove(&mut self, key: Key) {
        if let Some(entry) = self.entries.remove(&key) {
            self.stats.resident_bytes -= entry.bytes;
            self.stats.evictions += 1;
        }
    }
    fn evict_oldest(&mut self) {
        if let Some(key) = self
            .entries
            .iter()
            .min_by_key(|(_, e)| e.used)
            .map(|(k, _)| *k)
        {
            self.remove(key);
        }
    }
    pub(super) fn prepare(
        &mut self,
        item: usize,
        revision: u64,
        t: &CardTemplate,
        r: Rotation,
        c: &Candidate,
    ) -> Arc<Transformed> {
        let key = Key::new(item, revision, c);
        self.clock = self.clock.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.used = self.clock;
            self.stats.hits += 1;
            return Arc::clone(&entry.transform);
        }
        self.stats.misses += 1;
        let start = Instant::now();
        let transform = Arc::new(Transformed::new(t, r, c));
        self.stats.preparation_ms += start.elapsed().as_secs_f64() * 1000.0;
        // A BTree leaf has eleven entry slots and an internal node also has
        // twelve child pointers. Charging a whole node per stored key is a
        // conservative bound, rather than claiming allocator-exact heap usage.
        let bytes = transform.bytes() + 11 * size_of::<(Key, Entry)>() + 16 * size_of::<usize>();
        if bytes <= self.budget {
            while self.stats.resident_bytes > self.budget - bytes {
                self.evict_oldest();
            }
            self.entries.insert(
                key,
                Entry {
                    transform: Arc::clone(&transform),
                    bytes,
                    used: self.clock,
                },
            );
            self.stats.resident_bytes += bytes;
            self.stats.peak_bytes = self.stats.peak_bytes.max(self.stats.resident_bytes);
        }
        // Oversized transforms and budget=0 bypass storage, never comparison.
        transform
    }
    pub(super) fn scored_visible_blocks(&mut self, count: u64) {
        self.stats.scored_visible_blocks += count;
    }
    pub(super) fn stats(&self) -> TileCacheStats {
        self.stats.clone()
    }
}

/// Fingerprints are coarse descriptors. Cache revisions use exact content.
pub(super) fn same_template(a: Option<&CardTemplate>, b: Option<&CardTemplate>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.image == b.image
                && a.mask == b.mask
                && a.core == b.core
                && a.bounds.x == b.bounds.x
                && a.bounds.y == b.bounds.y
                && a.bounds.w == b.bounds.w
                && a.bounds.h == b.bounds.h
        }
        _ => false,
    }
}

/// Align each revealed anchor with every possible local tile. Deduplication
/// retains all legal rectangles; (y,x) ordering matches the direct search.
pub(super) fn positions(anchors: &[usize], columns: usize, rows: usize) -> Vec<(usize, usize)> {
    if columns == 0 || rows == 0 || columns > COLS || rows > ROWS {
        return Vec::new();
    }
    let mut positions = BTreeSet::new();
    for &anchor in anchors {
        if anchor >= COLS * ROWS {
            continue;
        }
        for row in 0..rows {
            for column in 0..columns {
                let (Some(x), Some(y)) = (
                    (anchor % COLS).checked_sub(column),
                    (anchor / COLS).checked_sub(row),
                ) else {
                    continue;
                };
                if x + columns <= COLS && y + rows <= ROWS {
                    positions.insert((y, x));
                }
            }
        }
    }
    let result: Vec<_> = positions.into_iter().map(|(y, x)| (x, y)).collect();
    #[cfg(debug_assertions)]
    {
        let old: Vec<_> = (0..=ROWS - rows)
            .flat_map(|y| {
                (0..=COLS - columns).filter_map(move |x| {
                    anchors
                        .iter()
                        .any(|&a| {
                            a < COLS * ROWS
                                && a % COLS >= x
                                && a % COLS < x + columns
                                && a / COLS >= y
                                && a / COLS < y + rows
                        })
                        .then_some((x, y))
                })
            })
            .collect();
        debug_assert_eq!(result, old);
    }
    result
}

pub(super) fn evaluate(
    image: &RgbaImage,
    transformed: &Transformed,
    c: &Candidate,
    cells: &[String],
) -> (Option<Candidate>, u64) {
    evaluate_with_policy(image, transformed, c, cells, false)
}

pub(super) fn evaluate_with_policy(
    image: &RgbaImage,
    transformed: &Transformed,
    c: &Candidate,
    cells: &[String],
    threshold_free_search: bool,
) -> (Option<Candidate>, u64) {
    let p = &c.placement;
    let (mut sum, mut count, mut bad, mut visible) = (0.0, 0, 0, 0);
    let mut counts = [0; 45];
    // Do not sum each block first: that changes floating-point association.
    // tileRow -> localY -> tileCol -> localX is exactly full row-major order.
    for row in 0..transformed.rows {
        for local_y in 0..MATCH_CELL {
            for column in 0..transformed.columns {
                let idx = (p.y + row) * COLS + p.x + column;
                if cells[idx] == "unknown" {
                    continue;
                }
                visible += u64::from(local_y == 0);
                let tile = &transformed.tiles[row * transformed.columns + column];
                for local_x in 0..MATCH_CELL {
                    let pixel = tile.pixels[local_y * MATCH_CELL + local_x];
                    if !pixel.core {
                        continue;
                    }
                    let observed = rgb(
                        image,
                        (p.x + column) * MATCH_CELL + local_x,
                        (p.y + row) * MATCH_CELL + local_y,
                    );
                    let error = distance(observed, pixel.expected);
                    sum += error;
                    count += 1;
                    bad += usize::from(error > 35.0);
                    counts[idx] += 1;
                }
            }
        }
    }
    if count < 55 || (!threshold_free_search && bad as f64 / count as f64 > 0.40) {
        return (None, visible);
    }
    let mut result = c.clone();
    result.score = sum / count as f64;
    result.counts = counts;
    (Some(result), visible)
}

pub(super) fn push_pose(
    out: &mut Vec<Pose>,
    image: &RgbaImage,
    t: &CardTemplate,
    c: &Candidate,
    o: &Observation,
    item: usize,
    cells: &[String],
    cfg: &Config,
    transformed: &Transformed,
    revision: u64,
) {
    if (!cfg.threshold_free_search && c.score > cfg.max_color_error)
        || c.counts[o.anchor] < cfg.min_anchor_pixels
    {
        return;
    }
    let p = &c.placement;
    let (w, h) = (p.width * MATCH_CELL, p.height * MATCH_CELL);
    let (
        mut edge,
        mut edge_n,
        mut bad,
        mut expected_n,
        mut unexplained,
        mut actual_n,
        mut effective,
    ) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for row in 0..transformed.rows {
        for local_y in 0..MATCH_CELL {
            let y = row * MATCH_CELL + local_y;
            if y == 0 || y == h - 1 {
                continue;
            }
            for column in 0..transformed.columns {
                let idx = (p.y + row) * COLS + p.x + column;
                if cells[idx] == "unknown" {
                    continue;
                }
                let tile = &transformed.tiles[row * transformed.columns + column];
                for local_x in 0..MATCH_CELL {
                    let x = column * MATCH_CELL + local_x;
                    if x == 0 || x == w - 1 {
                        continue;
                    }
                    let (bx, by) = (p.x * MATCH_CELL + x, p.y * MATCH_CELL + y);
                    let observed = rgb(image, bx, by);
                    let border = x % MATCH_CELL < cfg.reliable_border
                        || x % MATCH_CELL >= MATCH_CELL - cfg.reliable_border
                        || y % MATCH_CELL < cfg.reliable_border
                        || y % MATCH_CELL >= MATCH_CELL - cfg.reliable_border;
                    let weight = if border || super::super::super::selection_green(observed) {
                        cfg.low_confidence_weight
                    } else {
                        1.0
                    };
                    let pixel = tile.pixels[local_y * MATCH_CELL + local_x];
                    let error = if pixel.mask {
                        distance(observed, pixel.expected)
                    } else {
                        255.0
                    };
                    if pixel.core {
                        expected_n += weight;
                        bad += weight * f64::from(error > cfg.mismatch_error);
                        effective += weight;
                    }
                    if reliable(observed, cfg) {
                        actual_n += weight;
                        if !pixel.mask || error > cfg.mismatch_error {
                            unexplained += weight;
                        }
                    }
                    if pixel.core {
                        let right = transformed.at(x + 1, y);
                        if right.core && (bx + 1) / MATCH_CELL == bx / MATCH_CELL {
                            edge += (distance(rgb(image, bx + 1, by), observed)
                                - distance(right.expected, pixel.expected))
                            .abs()
                                * weight;
                            edge_n += weight;
                        }
                        let down = transformed.at(x, y + 1);
                        if down.core && (by + 1) / MATCH_CELL == by / MATCH_CELL {
                            edge += (distance(rgb(image, bx, by + 1), observed)
                                - distance(down.expected, pixel.expected))
                            .abs()
                                * weight;
                            edge_n += weight;
                        }
                    }
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
        scale: transformed.scale,
        offset: [c.dx, c.dy],
        evidence_pixels: c.counts[o.anchor],
        effective_evidence: effective,
        color_error: c.score,
        edge_error,
        predicted_mismatch,
        unexplained_foreground,
        scores,
        accepted: scores.map(|s| s <= cfg.max_color_error),
        tile_template: Some(TileDiagnostic {
            id: String::new(),
            image: String::new(),
            mask: String::new(),
            core: String::new(),
            columns: transformed.columns,
            rows: transformed.rows,
            cell_pixels: MATCH_CELL,
            anchor_tile: [o.anchor % COLS - p.x, o.anchor / COLS - p.y],
            visible_tiles: evidence(image, transformed, c, cells),
            key: Some(Key::new(item, revision, c)),
        }),
        visible_evidence: Some(visible::measure(image, c, cells, cfg, |x, y| {
            let pixel = transformed.at(x, y);
            (pixel.mask, pixel.expected)
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
                    let pixel = transformed.at(x, y);
                    (pixel.core, pixel.expected)
                },
            )
        }),
    });
}

fn evidence(
    image: &RgbaImage,
    transformed: &Transformed,
    c: &Candidate,
    cells: &[String],
) -> Vec<TileEvidence> {
    let p = &c.placement;
    transformed
        .tiles
        .iter()
        .filter_map(|tile| {
            let board_cell = (p.y + tile.row) * COLS + p.x + tile.column;
            if cells[board_cell] == "unknown" {
                return None;
            }
            let (mut sum, mut core_pixels) = (0.0, 0);
            for local_y in 0..MATCH_CELL {
                for local_x in 0..MATCH_CELL {
                    let pixel = tile.pixels[local_y * MATCH_CELL + local_x];
                    if pixel.core {
                        sum += distance(
                            rgb(
                                image,
                                (p.x + tile.column) * MATCH_CELL + local_x,
                                (p.y + tile.row) * MATCH_CELL + local_y,
                            ),
                            pixel.expected,
                        );
                        core_pixels += 1;
                    }
                }
            }
            Some(TileEvidence {
                column: tile.column,
                row: tile.row,
                board_cell,
                core_pixels,
                color_error: (core_pixels > 0).then(|| sum / core_pixels as f64),
            })
        })
        .collect()
}

/// Called only after the matching timer stops. Rebuilding here has no access
/// to Cache, so PNG generation cannot warm or alter the search cache/stats.
pub(super) fn export(
    candidates: &mut [Pose],
    templates: &[Option<CardTemplate>; 3],
    out: &Path,
) -> Result<(), String> {
    let mut exported = BTreeMap::<Key, usize>::new();
    std::fs::create_dir_all(out.join("tiles")).map_err(|e| e.to_string())?;
    for pose in candidates {
        let Some(diag) = &mut pose.tile_template else {
            continue;
        };
        let key = diag.key.ok_or("tile diagnostic missing transform key")?;
        let next = exported.len();
        let number = if let Some(&number) = exported.get(&key) {
            number
        } else {
            let t = templates[key.item]
                .as_ref()
                .ok_or("tile template unavailable")?;
            let c = key.candidate();
            let transformed = Transformed::new(t, Rotation::new(t, c.angle), &c);
            let (w, h) = (
                transformed.columns * MATCH_CELL,
                transformed.rows * MATCH_CELL,
            );
            let mut image = RgbaImage::new(w as u32, h as u32);
            let mut mask = image.clone();
            let mut core = image.clone();
            for y in 0..h {
                for x in 0..w {
                    let p = transformed.at(x, y);
                    image.put_pixel(
                        x as u32,
                        y as u32,
                        Rgba([
                            p.expected[0],
                            p.expected[1],
                            p.expected[2],
                            if p.mask { 255 } else { 0 },
                        ]),
                    );
                    let m = if p.mask { 255 } else { 0 };
                    let k = if p.core { 255 } else { 0 };
                    mask.put_pixel(x as u32, y as u32, Rgba([m, m, m, 255]));
                    core.put_pixel(x as u32, y as u32, Rgba([k, k, k, 255]));
                }
            }
            for (suffix, asset) in [("", image), ("-mask", mask), ("-core", core)] {
                asset
                    .save(out.join(format!("tiles/pose-{next}{suffix}.png")))
                    .map_err(|e| e.to_string())?;
            }
            exported.insert(key, next);
            next
        };
        diag.id = format!("pose-{number}");
        diag.image = format!("tiles/pose-{number}.png");
        diag.mask = format!("tiles/pose-{number}-mask.png");
        diag.core = format!("tiles/pose-{number}-core.png");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template() -> CardTemplate {
        let mut image = RgbaImage::new(CARD_W as u32, CARD_H as u32);
        let mut mask = vec![false; CARD_W * CARD_H];
        let bounds = Bounds {
            x: 12,
            y: 19,
            w: 121,
            h: 81,
        };
        for y in 0..CARD_H {
            for x in 0..CARD_W {
                image.put_pixel(
                    x as u32,
                    y as u32,
                    Rgba([
                        (35 + (3 * x + y) % 150) as u8,
                        (25 + (x + 5 * y) % 135) as u8,
                        (50 + (7 * x + 3 * y) % 145) as u8,
                        255,
                    ]),
                );
                mask[y * CARD_W + x] = x >= bounds.x
                    && x < bounds.x + bounds.w
                    && y >= bounds.y
                    && y < bounds.y + bounds.h
                    && !(x > 42 && x < 49 && y > 48 && y < 67);
            }
        }
        let core = morph(&mask, CARD_W, CARD_H, false);
        CardTemplate {
            image,
            mask,
            core,
            bounds,
            fingerprint: "coarse-same".into(),
        }
    }
    fn candidate(
        width: usize,
        height: usize,
        angle: f64,
        fill: f64,
        dx: f64,
        dy: f64,
    ) -> Candidate {
        Candidate {
            placement: GridPlacement {
                x: 0,
                y: 0,
                width,
                height,
            },
            angle,
            fill,
            dx,
            dy,
            score: 255.0,
            counts: [0; 45],
        }
    }
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
            reliable_border: 2,
            low_confidence_weight: 0.2,
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
    // An independent renderer uses the direct formula, rather than reading
    // cached pixels, so score parity also checks coordinate/sampling accuracy.
    fn board(t: &CardTemplate, c: &Candidate, noisy: bool) -> RgbaImage {
        let mut board = RgbaImage::new((COLS * MATCH_CELL) as u32, (ROWS * MATCH_CELL) as u32);
        render(&mut board, t, c, noisy);
        board
    }
    fn render(board: &mut RgbaImage, t: &CardTemplate, c: &Candidate, noisy: bool) {
        let p = &c.placement;
        let (w, h) = (p.width * MATCH_CELL, p.height * MATCH_CELL);
        let r = Rotation::new(t, c.angle);
        let scale = (w as f64 / r.w).min(h as f64 / r.h) * c.fill;
        let (ox, oy) = (
            (w as f64 - r.w * scale) / 2.0 + c.dx,
            (h as f64 - r.h * scale) / 2.0 + c.dy,
        );
        for y in 0..h {
            for x in 0..w {
                let (sx, sy) =
                    r.source((x as f64 + 0.5 - ox) / scale, (y as f64 + 0.5 - oy) / scale);
                let mut expected = if mask_at(&t.mask, sx, sy) {
                    sample(&t.image, sx, sy)
                } else {
                    [238, 244, 249]
                };
                if noisy && mask_at(&t.mask, sx, sy) {
                    let delta = if (x + 3 * y) % 23 == 0 {
                        80
                    } else {
                        ((x + 2 * y) % 11) as i16 - 5
                    };
                    expected = expected.map(|p| (p as i16 + delta).clamp(0, 255) as u8);
                    if (x + y) % 41 == 0 {
                        expected = [45, 160, 65];
                    }
                }
                board.put_pixel(
                    (p.x * MATCH_CELL + x) as u32,
                    (p.y * MATCH_CELL + y) as u32,
                    Rgba([expected[0], expected[1], expected[2], 255]),
                );
            }
        }
    }
    fn cells_for(c: &Candidate) -> Vec<String> {
        let mut cells = vec!["unknown".to_string(); COLS * ROWS];
        let p = &c.placement;
        for row in 0..p.height {
            for col in 0..p.width {
                if (col + row) % 3 != 2 {
                    cells[(p.y + row) * COLS + p.x + col] = "uncertain".into();
                }
            }
        }
        cells
    }
    fn observation(anchor: usize) -> Observation {
        Observation {
            id: format!("test-{anchor}"),
            anchor,
            texture_std: 10.0,
            reliable_foreground_pixels: 100,
            border_reliable_foreground_pixels: 0,
            effective_foreground_pixels: 100.0,
            sufficient_evidence: true,
        }
    }
    fn without_tile(mut pose: Pose) -> serde_json::Value {
        pose.tile_template = None;
        serde_json::to_value(pose).unwrap()
    }
    fn equal_candidate(a: Option<&Candidate>, b: Option<&Candidate>) {
        assert_eq!(a.is_some(), b.is_some());
        if let (Some(a), Some(b)) = (a, b) {
            assert_eq!(a.score.to_bits(), b.score.to_bits());
            assert_eq!(a.counts, b.counts);
        }
    }

    fn foreground_case() -> (CardTemplate, RgbaImage, Candidate, Vec<String>, Config) {
        let color = [150, 40, 80, 255];
        let bounds = Bounds {
            x: 40,
            y: 30,
            w: 32,
            h: 32,
        };
        let image = RgbaImage::from_pixel(CARD_W as u32, CARD_H as u32, Rgba(color));
        let mut mask = vec![false; CARD_W * CARD_H];
        for y in bounds.y..bounds.y + bounds.h {
            for x in bounds.x..bounds.x + bounds.w {
                mask[y * CARD_W + x] = true;
            }
        }
        let t = CardTemplate {
            image,
            core: morph(&mask, CARD_W, CARD_H, false),
            mask,
            bounds,
            fingerprint: "coverage-versus-rgb".into(),
        };
        let small = candidate(1, 1, 0.0, 0.60, 0.0, 0.0);
        let truth = Candidate {
            fill: 0.88,
            ..small.clone()
        };
        let mut observed = board(&t, &truth, false);
        let transformed = Transformed::new(&t, Rotation::new(&t, small.angle), &small);
        // The central fragment has a perfect RGB fit. The rest of the same
        // foreground is only 20 RGB levels different, so explaining it is a
        // better visible-foreground fit but a worse single-direction RGB fit.
        for y in 0..MATCH_CELL {
            for x in 0..MATCH_CELL {
                if !transformed.at(x, y).mask && observed.get_pixel(x as u32, y as u32).0 == color {
                    observed.put_pixel(x as u32, y as u32, Rgba([170, 60, 100, 255]));
                }
            }
        }
        let mut cfg = config();
        cfg.max_color_error = 38.0;
        cfg.min_anchor_pixels = 45;
        cfg.edge_weight = 0.12;
        cfg.reverse_weight = 10.0;
        cfg.foreground_refinement = true;
        cfg.refine = vec![Refine {
            angles: vec![-4.0, 0.0, 4.0],
            fills: vec![0.0, 0.32],
            offsets: vec![0.0],
        }];
        (t, observed, small.clone(), cells_for(&small), cfg)
    }

    #[test]
    fn threshold_free_search_scores_poor_colors_with_exact_direct_tiled_parity() {
        let t = template();
        let mut cfg = config();
        cfg.max_color_error = 38.0;
        cfg.min_anchor_pixels = 45;
        cfg.confidence_weighted_scoring = true;
        let image = RgbaImage::from_pixel(
            (COLS * MATCH_CELL) as u32,
            (ROWS * MATCH_CELL) as u32,
            Rgba([0, 0, 0, 255]),
        );
        for (w, h, angle, fill, dx, dy) in [
            (3, 2, 17.25, 0.92, 0.75, -0.33),
            (2, 3, 90.0, 1.0, 0.0, 0.0),
            (1, 1, 173.5, 0.92, -0.25, 0.5),
        ] {
            let c = candidate(w, h, angle, fill, dx, dy);
            let cells = cells_for(&c);
            let r = Rotation::new(&t, angle);
            let tiles = Transformed::new(&t, r, &c);
            assert!(super::super::evaluate(&image, &t, r, &c, &cells).is_none());
            assert!(evaluate(&image, &tiles, &c, &cells).0.is_none());
            assert!(evaluate_direct(&image, &t, r, &c, &cells, false).is_none());
            let direct = evaluate_direct(&image, &t, r, &c, &cells, true).unwrap();
            let tiled = evaluate_with_policy(&image, &tiles, &c, &cells, true)
                .0
                .unwrap();
            equal_candidate(Some(&direct), Some(&tiled));
            assert!(direct.score > cfg.max_color_error);
            let anchor = direct
                .counts
                .iter()
                .position(|&n| n >= cfg.min_anchor_pixels)
                .unwrap();
            let o = observation(anchor);
            let mut direct_poses = Vec::new();
            let mut tiled_poses = Vec::new();
            super::super::push_pose(&mut direct_poses, &image, &t, &direct, &o, 0, &cells, &cfg);
            push_pose(
                &mut tiled_poses,
                &image,
                &t,
                &tiled,
                &o,
                0,
                &cells,
                &cfg,
                &tiles,
                1,
            );
            assert!(direct_poses.is_empty() && tiled_poses.is_empty());
            cfg.threshold_free_search = true;
            super::super::push_pose(&mut direct_poses, &image, &t, &direct, &o, 0, &cells, &cfg);
            push_pose(
                &mut tiled_poses,
                &image,
                &t,
                &tiled,
                &o,
                0,
                &cells,
                &cfg,
                &tiles,
                1,
            );
            assert_eq!(direct_poses.len(), 1);
            assert_eq!(tiled_poses.len(), 1);
            assert_eq!(direct_poses[0].accepted, [false; 3]);
            assert!(foreground_objective(&direct_poses[0]).is_some());
            assert_eq!(
                without_tile(direct_poses.remove(0)),
                without_tile(tiled_poses.remove(0))
            );
            cfg.threshold_free_search = false;
        }

        // The max_color_error gate is independent of the production bad-ratio gate.
        let c = candidate(3, 2, 0.0, 0.92, 0.0, 0.0);
        let noisy = board(&t, &c, true);
        let cells = cells_for(&c);
        let r = Rotation::new(&t, 0.0);
        let production = super::super::evaluate(&noisy, &t, r, &c, &cells).unwrap();
        assert!(production.score > 1.0 && production.score < 35.0);
        equal_candidate(
            Some(&production),
            evaluate_direct(&noisy, &t, r, &c, &cells, false).as_ref(),
        );
        equal_candidate(
            Some(&production),
            evaluate_direct(&noisy, &t, r, &c, &cells, true).as_ref(),
        );
        cfg.max_color_error = 1.0;
        let o = observation(
            production
                .counts
                .iter()
                .position(|&n| n >= cfg.min_anchor_pixels)
                .unwrap(),
        );
        let mut poses = Vec::new();
        super::super::push_pose(&mut poses, &noisy, &t, &production, &o, 0, &cells, &cfg);
        assert!(poses.is_empty());
        cfg.threshold_free_search = true;
        super::super::push_pose(&mut poses, &noisy, &t, &production, &o, 0, &cells, &cfg);
        assert_eq!(poses.len(), 1);
    }

    #[test]
    fn threshold_free_search_keeps_evidence_minimums_and_stable_metric_minima() {
        let (t, image, small, cells, mut cfg) = foreground_case();
        cfg.threshold_free_search = true;
        let o = observation(0);
        let mut poses = Vec::new();
        for c in [
            small.clone(),
            Candidate {
                fill: 0.92,
                ..small.clone()
            },
        ] {
            let evaluated =
                evaluate_direct(&image, &t, Rotation::new(&t, c.angle), &c, &cells, true).unwrap();
            super::super::push_pose(&mut poses, &image, &t, &evaluated, &o, 0, &cells, &cfg);
        }
        let (rgb_best, composite_best) = (&poses[0], &poses[1]);
        assert!(rgb_best.scores[0] < composite_best.scores[0]);
        assert!(
            foreground_objective(composite_best).unwrap() < foreground_objective(rgb_best).unwrap()
        );
        let mut rgb_tie = rgb_best.clone();
        rgb_tie.angle = 123.0;
        let mut composite_tie = composite_best.clone();
        composite_tie.angle = 234.0;
        let mut other_rect = rgb_best.clone();
        other_rect.rect.width = 2;
        let mut minima = ThresholdFreeMinima::new();
        retain_threshold_free_poses(
            &mut minima,
            [
                composite_best.clone(),
                rgb_best.clone(),
                rgb_tie,
                composite_tie,
                other_rect,
            ],
        );
        assert_eq!(minima.len(), 2);
        let retained = minima.get(&(0, 0, 0, 0, 1, 1)).unwrap();
        assert_eq!(
            without_tile(retained[0].clone().unwrap()),
            without_tile(rgb_best.clone())
        );
        assert_eq!(
            without_tile(retained[1].clone().unwrap()),
            without_tile(composite_best.clone())
        );

        let covered = vec!["unknown".to_string(); COLS * ROWS];
        let r = Rotation::new(&t, 0.0);
        let tiles = Transformed::new(&t, r, &small);
        assert!(evaluate_direct(&image, &t, r, &small, &covered, true).is_none());
        assert!(evaluate_with_policy(&image, &tiles, &small, &covered, true)
            .0
            .is_none());
        let tiny = Candidate {
            fill: 0.15,
            ..small.clone()
        };
        let tiles = Transformed::new(&t, r, &tiny);
        assert!(evaluate_direct(&image, &t, r, &tiny, &cells, true).is_none());
        assert!(evaluate_with_policy(&image, &tiles, &tiny, &cells, true)
            .0
            .is_none());
        let evaluated = evaluate_direct(&image, &t, r, &small, &cells, true).unwrap();
        cfg.min_anchor_pixels = evaluated.counts[0] + 1;
        let mut insufficient = Vec::new();
        super::super::push_pose(
            &mut insufficient,
            &image,
            &t,
            &evaluated,
            &o,
            0,
            &cells,
            &cfg,
        );
        assert!(insufficient.is_empty());
    }

    #[test]
    fn threshold_free_foreground_refinement_keeps_above_threshold_non_improving_pose() {
        let (t, _, small, cells, mut cfg) = foreground_case();
        let image = RgbaImage::from_pixel(
            (COLS * MATCH_CELL) as u32,
            (ROWS * MATCH_CELL) as u32,
            Rgba([0, 0, 0, 255]),
        );
        cfg.threshold_free_search = true;
        cfg.refine = vec![Refine {
            angles: vec![0.0],
            fills: vec![0.0],
            offsets: vec![0.0],
        }];
        let o = observation(0);
        let small =
            evaluate_direct(&image, &t, Rotation::new(&t, 0.0), &small, &cells, true).unwrap();
        let mut initial = Vec::new();
        super::super::push_pose(&mut initial, &image, &t, &small, &o, 0, &cells, &cfg);
        let score = foreground_objective(&initial[0]).unwrap();
        assert!(small.score > cfg.max_color_error && score > cfg.max_color_error);
        let run = |backend, enabled| {
            let mut cfg = cfg.clone();
            cfg.threshold_free_search = enabled;
            let mut cache = Cache::default();
            cache.begin_run(cfg.tile_cache_bytes);
            let mut poses = Vec::new();
            let mut minima = ThresholdFreeMinima::new();
            let mut evaluations = 0;
            let mut extra = 0;
            assert!(super::super::refine_foreground(
                &mut cache,
                backend,
                0,
                1,
                &image,
                &t,
                &[&o],
                &cells,
                &cfg,
                ForegroundSeed {
                    candidate: small.clone(),
                    score
                },
                &mut poses,
                &mut minima,
                &mut evaluations,
                &mut extra,
                None,
            ));
            assert_eq!((evaluations, extra), (1, 1));
            assert!(poses.is_empty());
            let retained = minima
                .into_values()
                .flatten()
                .flatten()
                .map(without_tile)
                .collect::<Vec<_>>();
            assert_eq!(retained.len(), if enabled { 2 } else { 0 });
            retained
        };
        assert_eq!(run(Backend::Direct, false), run(Backend::Tiled, false));
        assert_eq!(run(Backend::Direct, true), run(Backend::Tiled, true));
    }

    #[test]
    fn foreground_seed_and_refinement_keep_rgb_worse_coverage_and_supported_angles() {
        let (t, image, small, cells, cfg) = foreground_case();
        let o = observation(0);
        let small =
            super::super::evaluate(&image, &t, Rotation::new(&t, 0.0), &small, &cells).unwrap();
        let broad = Candidate {
            fill: small.fill + 0.32,
            ..small.clone()
        };
        let broad =
            super::super::evaluate(&image, &t, Rotation::new(&t, 0.0), &broad, &cells).unwrap();
        assert_eq!(small.score, 0.0);
        assert!(broad.score > small.score && broad.score < cfg.max_color_error);
        let mut coarse = Vec::new();
        super::super::push_pose(&mut coarse, &image, &t, &small, &o, 0, &cells, &cfg);
        let small_score = super::super::foreground_objective(&coarse[0]).unwrap();
        let mut seed = None;
        super::super::consider_foreground_seed(&mut seed, &small, &coarse);
        let mut broad_pose = Vec::new();
        super::super::push_pose(&mut broad_pose, &image, &t, &broad, &o, 0, &cells, &cfg);
        assert!(super::super::foreground_objective(&broad_pose[0]).unwrap() < small_score);
        super::super::consider_foreground_seed(&mut seed, &broad, &broad_pose);
        assert_eq!(seed.unwrap().candidate.fill, broad.fill);

        let run = |backend| {
            let mut cache = Cache::default();
            cache.begin_run(cfg.tile_cache_bytes);
            let mut poses = Vec::new();
            let mut minima = ThresholdFreeMinima::new();
            let mut evaluations = 0;
            let mut extra = 0;
            let complete = super::super::refine_foreground(
                &mut cache,
                backend,
                0,
                1,
                &image,
                &t,
                &[&o],
                &cells,
                &cfg,
                super::super::ForegroundSeed {
                    candidate: small.clone(),
                    score: small_score,
                },
                &mut poses,
                &mut minima,
                &mut evaluations,
                &mut extra,
                None,
            );
            assert!(complete);
            assert_eq!(evaluations, 6);
            assert_eq!(extra, evaluations);
            let retained: Vec<_> = poses
                .iter()
                .filter(|p| {
                    p.fill > 0.9
                        && super::super::foreground_objective(p).unwrap() <= cfg.max_color_error
                })
                .collect();
            assert!(retained.iter().all(|p| p.color_error > small.score));
            let angles: std::collections::HashSet<_> =
                retained.iter().map(|p| p.angle.to_bits()).collect();
            assert!(angles.len() >= 2, "supported intermediate angles were lost");
            poses.into_iter().map(without_tile).collect::<Vec<_>>()
        };
        assert_eq!(run(Backend::Direct), run(Backend::Tiled));
    }

    #[test]
    fn foreground_refinement_exhausts_shared_budget_identically_on_both_backends() {
        let (t, image, small, cells, mut cfg) = foreground_case();
        let o = observation(0);
        let small =
            super::super::evaluate(&image, &t, Rotation::new(&t, 0.0), &small, &cells).unwrap();
        let mut coarse = Vec::new();
        super::super::push_pose(&mut coarse, &image, &t, &small, &o, 0, &cells, &cfg);
        let score = super::super::foreground_objective(&coarse[0]).unwrap();
        cfg.max_evaluations = 7;
        let run = |backend| {
            let mut cache = Cache::default();
            cache.begin_run(cfg.tile_cache_bytes);
            let mut poses = Vec::new();
            let mut minima = ThresholdFreeMinima::new();
            let mut evaluations = 5;
            let mut extra = 0;
            let complete = super::super::refine_foreground(
                &mut cache,
                backend,
                0,
                1,
                &image,
                &t,
                &[&o],
                &cells,
                &cfg,
                super::super::ForegroundSeed {
                    candidate: small.clone(),
                    score,
                },
                &mut poses,
                &mut minima,
                &mut evaluations,
                &mut extra,
                None,
            );
            assert!(!complete);
            assert_eq!(evaluations, 7);
            assert_eq!(extra, 2);
            poses.into_iter().map(without_tile).collect::<Vec<_>>()
        };
        assert_eq!(run(Backend::Direct), run(Backend::Tiled));
    }

    #[test]
    fn direct_and_tiled_base_edge_reverse_are_exact_at_varied_transforms() {
        let t = template();
        let cfg = config();
        let cases = [
            (3, 2, 0.0, 0.92, 0.0, 0.0),
            (2, 3, 90.0, 1.0, 0.0, 0.0),
            (4, 1, 17.0, 0.84, 1.5, -0.75),
            (1, 4, -6.0, 0.99, -1.5, 0.75),
            (1, 1, 173.5, 0.55, 0.33, -0.2),
            (3, 2, 359.125, 0.92, -0.75, 1.5),
            (4, 3, 360.0, 0.99, 0.0, 0.0),
            (5, 2, 37.3, 0.84, 2.3, -1.1),
            (9, 5, 0.0, 1.0, 0.0, 0.0),
            (3, 2, 0.0, 0.0, 0.0, 0.0),
            (2, 2, -360.0, 0.92, -0.0, 0.0),
            (2, 3, 47.25, 0.73, 0.15, -0.55),
        ];
        let mut accepted = 0;
        let mut nonzero_extra = 0;
        for (w, h, angle, fill, dx, dy) in cases {
            let c = candidate(w, h, angle, fill, dx, dy);
            let board = board(&t, &c, true);
            let cells = cells_for(&c);
            let r = Rotation::new(&t, angle);
            let direct = super::super::evaluate(&board, &t, r, &c, &cells);
            let transformed = Transformed::new(&t, r, &c);
            let (tiled, visited) = evaluate(&board, &transformed, &c, &cells);
            equal_candidate(direct.as_ref(), tiled.as_ref());
            assert_eq!(
                visited,
                cells.iter().filter(|s| s.as_str() != "unknown").count() as u64
            );
            if let (Some(direct), Some(tiled)) = (direct, tiled) {
                accepted += 1;
                for anchor in 0..COLS * ROWS {
                    if direct.counts[anchor] == 0 {
                        continue;
                    }
                    let o = observation(anchor);
                    let mut a = Vec::new();
                    let mut b = Vec::new();
                    super::super::push_pose(&mut a, &board, &t, &direct, &o, 0, &cells, &cfg);
                    push_pose(
                        &mut b,
                        &board,
                        &t,
                        &tiled,
                        &o,
                        0,
                        &cells,
                        &cfg,
                        &transformed,
                        1,
                    );
                    assert!(a[0].visible_evidence.is_some());
                    assert_eq!(
                        serde_json::to_value(&a[0].visible_evidence).unwrap(),
                        serde_json::to_value(&b[0].visible_evidence).unwrap(),
                    );
                    assert_eq!(a[0].scores.map(f64::to_bits), b[0].scores.map(f64::to_bits));
                    nonzero_extra +=
                        usize::from(a[0].edge_error > 0.0 && a[0].predicted_mismatch > 0.0);
                    assert_eq!(without_tile(a.remove(0)), without_tile(b.remove(0)));
                }
            }
        }
        assert!(accepted >= 9, "parity must exercise accepted candidates");
        assert!(
            nonzero_extra > 0,
            "edge/reverse checks must exercise nonzero error"
        );
    }

    #[test]
    fn unknown_pixels_never_change_base_extra_scores_or_tile_evidence() {
        let t = template();
        let c = candidate(4, 3, 17.25, 0.92, 0.75, -0.33);
        let original = board(&t, &c, true);
        let mut changed = original.clone();
        let cells = cells_for(&c);
        for y in 0..ROWS * MATCH_CELL {
            for x in 0..COLS * MATCH_CELL {
                if cells[y / MATCH_CELL * COLS + x / MATCH_CELL] == "unknown" {
                    changed.put_pixel(
                        x as u32,
                        y as u32,
                        Rgba([(x % 256) as u8, (y % 256) as u8, 0, 255]),
                    );
                }
            }
        }
        let transformed = Transformed::new(&t, Rotation::new(&t, c.angle), &c);
        let a = evaluate(&original, &transformed, &c, &cells).0.unwrap();
        let b = evaluate(&changed, &transformed, &c, &cells).0.unwrap();
        equal_candidate(Some(&a), Some(&b));
        let anchor = a.counts.iter().position(|&n| n > 0).unwrap();
        let mut pa = Vec::new();
        let mut pb = Vec::new();
        push_pose(
            &mut pa,
            &original,
            &t,
            &a,
            &observation(anchor),
            0,
            &cells,
            &config(),
            &transformed,
            1,
        );
        push_pose(
            &mut pb,
            &changed,
            &t,
            &b,
            &observation(anchor),
            0,
            &cells,
            &config(),
            &transformed,
            1,
        );
        assert_eq!(
            serde_json::to_value(&pa).unwrap(),
            serde_json::to_value(&pb).unwrap()
        );
        for tile in &pa[0].tile_template.as_ref().unwrap().visible_tiles {
            assert_ne!(cells[tile.board_cell], "unknown");
        }
    }

    #[test]
    fn anchor_minus_local_tile_enumerates_exact_original_domain_and_order() {
        for width in 1..=COLS {
            for height in 1..=ROWS {
                for anchor in 0..COLS * ROWS {
                    let anchors = [anchor, (anchor + 13) % (COLS * ROWS), anchor];
                    let old: Vec<_> = (0..=ROWS - height)
                        .flat_map(|y| {
                            (0..=COLS - width).filter_map(move |x| {
                                anchors
                                    .iter()
                                    .any(|&a| {
                                        a % COLS >= x
                                            && a % COLS < x + width
                                            && a / COLS >= y
                                            && a / COLS < y + height
                                    })
                                    .then_some((x, y))
                            })
                        })
                        .collect();
                    assert_eq!(positions(&anchors, width, height), old);
                }
            }
        }
        assert!(positions(&[], 3, 2).is_empty());
        assert!(positions(&[0], COLS + 1, 2).is_empty());
    }

    #[test]
    fn exact_pose_keys_reuse_across_positions_and_preserve_all_transform_bits() {
        let t = template();
        let mut cache = Cache::default();
        cache.begin_run(default_tile_cache_bytes());
        let c = candidate(3, 2, 17.0, 0.92, 0.0, 0.0);
        let a = cache.prepare(0, 1, &t, Rotation::new(&t, c.angle), &c);
        let mut shifted = c.clone();
        shifted.placement.x = 4;
        shifted.placement.y = 2;
        let b = cache.prepare(0, 1, &t, Rotation::new(&t, c.angle), &shifted);
        assert!(Arc::ptr_eq(&a, &b));
        let mut variants = Vec::new();
        let mut v = c.clone();
        v.angle = 377.0;
        variants.push(v);
        let mut v = c.clone();
        v.fill = f64::from_bits(c.fill.to_bits() + 1);
        variants.push(v);
        let mut v = c.clone();
        v.dx = -0.0;
        variants.push(v);
        let mut v = c.clone();
        v.dy = 0.125;
        variants.push(v);
        let mut v = c.clone();
        v.placement.width = 2;
        variants.push(v);
        for v in variants {
            let fresh = cache.prepare(0, 1, &t, Rotation::new(&t, v.angle), &v);
            assert!(!Arc::ptr_eq(&a, &fresh));
        }
        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 6);
        assert!(stats.preparation_ms > 0.0);
    }

    #[test]
    fn bounded_and_zero_cache_evict_or_bypass_without_pruning_comparisons() {
        let t = template();
        let base = candidate(3, 2, 0.0, 0.92, 0.0, 0.0);
        let mut measure = Cache::default();
        measure.begin_run(default_tile_cache_bytes());
        measure.prepare(0, 1, &t, Rotation::new(&t, 0.0), &base);
        let one_transform = measure.stats().resident_bytes;
        for budget in [0, one_transform, default_tile_cache_bytes()] {
            let mut cache = Cache::default();
            cache.begin_run(budget);
            let angles = [0.0, 0.0, 17.0, 17.0, 90.0, 0.0];
            for angle in angles {
                let c = Candidate {
                    angle,
                    ..base.clone()
                };
                let image = board(&t, &c, true);
                let cells = cells_for(&c);
                let r = Rotation::new(&t, angle);
                let direct = super::super::evaluate(&image, &t, r, &c, &cells);
                let (result, _) = evaluate_backend(
                    &mut cache,
                    Backend::Tiled,
                    0,
                    1,
                    &image,
                    &t,
                    r,
                    &c,
                    &cells,
                    false,
                );
                equal_candidate(direct.as_ref(), result.as_ref());
            }
            let stats = cache.stats();
            assert_eq!(stats.hits + stats.misses, angles.len() as u64);
            assert!(stats.resident_bytes <= budget && stats.peak_bytes <= budget);
            assert_eq!(stats.scored_visible_blocks, 4 * angles.len() as u64);
            if budget == 0 {
                assert_eq!(stats.hits, 0);
                assert_eq!(stats.misses, 6);
                assert_eq!(stats.resident_bytes, 0);
            } else if budget == one_transform {
                assert_eq!(stats.hits, 2);
                assert_eq!(stats.evictions, 3);
            } else {
                assert_eq!(stats.hits, 3);
                assert_eq!(stats.evictions, 0);
            }
        }
    }

    #[test]
    fn exact_source_change_and_reset_invalidate_but_finish_retains_cache() {
        let mut session = Session::default();
        let empty = session.templates.clone();
        session.templates[0] = Some(template());
        session.refresh_template_revisions(&empty);
        session.tile_cache.begin_run(default_tile_cache_bytes());
        let c = candidate(3, 2, 0.0, 0.92, 0.0, 0.0);
        let first = session.tile_cache.prepare(
            0,
            session.source_revisions[0],
            session.templates[0].as_ref().unwrap(),
            Rotation::new(session.templates[0].as_ref().unwrap(), 0.0),
            &c,
        );
        // A genuine Finish banner is the only update_cards path retaining an
        // occluded card. Its retained template/revision must keep cache hits.
        let mut frame = RgbaImage::new(1920, 1080);
        for y in 880..1004 {
            for x in 182..336 {
                frame.put_pixel(x, y, Rgba([40, 40, 40, 255]));
            }
        }
        for y in 930..934 {
            for x in 215..310 {
                frame.put_pixel(x, y, Rgba([235, 220, 45, 255]));
            }
        }
        let previous = session.templates.clone();
        let finish = update_cards(
            &frame,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1920.0,
                h: 1080.0,
            },
            &mut session.templates,
        );
        assert!(finish[0]);
        session.refresh_template_revisions(&previous);
        let second = session.tile_cache.prepare(
            0,
            session.source_revisions[0],
            session.templates[0].as_ref().unwrap(),
            Rotation::new(session.templates[0].as_ref().unwrap(), 0.0),
            &c,
        );
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(session.tile_cache.stats().hits, 1);
        let previous = session.templates.clone();
        let current = session.templates[0].as_mut().unwrap();
        current.image.put_pixel(60, 60, Rgba([255, 0, 0, 255]));
        assert_eq!(
            current.fingerprint,
            previous[0].as_ref().unwrap().fingerprint
        );
        assert!(!same_template(Some(&*current), previous[0].as_ref()));
        session.refresh_template_revisions(&previous);
        assert_eq!(session.tile_cache.stats().resident_bytes, 0);
        let third = session.tile_cache.prepare(
            0,
            session.source_revisions[0],
            session.templates[0].as_ref().unwrap(),
            Rotation::new(session.templates[0].as_ref().unwrap(), 0.0),
            &c,
        );
        assert!(!Arc::ptr_eq(&first, &third));
        assert_eq!(session.tile_cache.stats().misses, 2);
        session.reset();
        assert_eq!(session.tile_cache.stats().resident_bytes, 0);
        assert!(session.templates.iter().all(Option::is_none));
        assert_eq!(session.source_revisions, [0; 3]);
    }

    #[test]
    fn two_visible_blocks_share_one_pose_and_jointly_reject_incoherent_art() {
        let t = template();
        let c = candidate(3, 2, 0.0, 0.92, 0.0, 0.0);
        let transformed = Transformed::new(&t, Rotation::new(&t, c.angle), &c);
        let correct = board(&t, &c, false);
        let mut cells = vec!["unknown".to_string(); COLS * ROWS];
        cells[1] = "uncertain".into();
        cells[10] = "uncertain".into();
        let joint = evaluate(&correct, &transformed, &c, &cells).0.unwrap();
        assert!(joint.counts[1] >= 55 && joint.counts[10] >= 55);
        let mut poses = Vec::new();
        for anchor in [1, 10] {
            push_pose(
                &mut poses,
                &correct,
                &t,
                &joint,
                &observation(anchor),
                0,
                &cells,
                &config(),
                &transformed,
                1,
            );
        }
        assert_eq!(poses.len(), 2);
        assert_eq!(
            poses[0].scores.map(f64::to_bits),
            poses[1].scores.map(f64::to_bits)
        );
        assert_eq!(
            poses[0].tile_template.as_ref().unwrap().visible_tiles.len(),
            2
        );
        assert_eq!(poses[0].tile_template.as_ref().unwrap().anchor_tile, [1, 0]);
        assert_eq!(poses[1].tile_template.as_ref().unwrap().anchor_tile, [1, 1]);
        let mut wrong = correct.clone();
        for y in MATCH_CELL..2 * MATCH_CELL {
            for x in MATCH_CELL..2 * MATCH_CELL {
                wrong.put_pixel(x as u32, y as u32, Rgba([255, 255, 255, 255]));
            }
        }
        assert!(evaluate(&wrong, &transformed, &c, &cells).0.is_none());
        cells[10] = "unknown".into();
        let single = evaluate(&wrong, &transformed, &c, &cells).0.unwrap();
        assert_eq!(single.score.to_bits(), 0.0f64.to_bits());
    }

    #[test]
    fn diagnostic_png_uses_exact_sampled_pose_and_deduplicates_without_cache_access() {
        let t = template();
        let mut c = candidate(2, 2, -17.0, 0.92, 0.3, -0.7);
        c.placement.x = 1;
        c.placement.y = 1;
        let mut image = board(&t, &c, false);
        let mut shifted = c.clone();
        shifted.placement.x = 5;
        render(&mut image, &t, &shifted, false);
        let mut cells = vec!["unknown".to_string(); COLS * ROWS];
        for p in [&c.placement, &shifted.placement] {
            for y in p.y..p.y + p.height {
                for x in p.x..p.x + p.width {
                    cells[y * COLS + x] = "uncertain".into();
                }
            }
        }
        let mut cache = Cache::default();
        cache.begin_run(default_tile_cache_bytes());
        let tiles = cache.prepare(0, 1, &t, Rotation::new(&t, c.angle), &c);
        let mut poses = Vec::new();
        for original in [&c, &shifted] {
            let scored = evaluate(&image, &tiles, original, &cells).0.unwrap();
            let anchor = scored.counts.iter().position(|&n| n > 0).unwrap();
            push_pose(
                &mut poses,
                &image,
                &t,
                &scored,
                &observation(anchor),
                0,
                &cells,
                &config(),
                &tiles,
                1,
            );
        }
        let before = serde_json::to_value(cache.stats()).unwrap();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let out = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.artifacts/partial-recognition/tile-engineering")
            .join(format!("module-assets-{stamp}"));
        export(&mut poses, &[Some(t), None, None], &out).unwrap();
        assert_eq!(before, serde_json::to_value(cache.stats()).unwrap());
        let a = poses[0].tile_template.as_ref().unwrap();
        let b = poses[1].tile_template.as_ref().unwrap();
        assert_eq!(a.image, b.image);
        assert_eq!(a.id, b.id);
        assert_eq!(std::fs::read_dir(out.join("tiles")).unwrap().count(), 3);
        let png = image::open(out.join(&a.image)).unwrap().to_rgba8();
        for y in 0..64 {
            for x in 0..64 {
                let p = tiles.at(x, y);
                let actual = png.get_pixel(x as u32, y as u32);
                assert_eq!(actual[3], if p.mask { 255 } else { 0 });
                if p.mask {
                    assert_eq!([actual[0], actual[1], actual[2]], p.expected);
                }
            }
        }
    }
}
