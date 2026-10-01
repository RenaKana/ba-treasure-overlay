//! Per-round references extracted from the three visible cards. No item atlas.
//! Masks, never the card/board background, are rotated into a candidate.
use super::{
    anchored_viewport, sample, viewport_sample, CompletedObject, GridPlacement, LayoutAnchor,
    PlacementConstraint, Rect, COLS, ROWS,
};
use image::{Rgba, RgbaImage};
use std::collections::VecDeque;

const CARD_W: usize = 154;
const CARD_H: usize = 124;
const MATCH_CELL: usize = 32;
const GRAY_CELL: usize = 64;

#[derive(Clone)]
pub(super) struct CardTemplate {
    image: RgbaImage,
    mask: Vec<bool>,
    core: Vec<bool>,
    bounds: Bounds,
    pub fingerprint: String,
}
#[derive(Clone, Copy, Debug)]
struct Bounds {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}
struct Component {
    bounds: Bounds,
    pixels: Vec<usize>,
}

fn components(mask: &[bool], w: usize, h: usize) -> Vec<Component> {
    let mut seen = vec![false; mask.len()];
    let mut out = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] || seen[start] {
            continue;
        }
        let mut queue = VecDeque::from([start]);
        seen[start] = true;
        let mut pixels = Vec::new();
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
        while let Some(i) = queue.pop_front() {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            pixels.push(i);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (xx, yy) = (x as isize + dx, y as isize + dy);
                    if xx < 0 || yy < 0 || xx >= w as isize || yy >= h as isize {
                        continue;
                    }
                    let j = yy as usize * w + xx as usize;
                    if mask[j] && !seen[j] {
                        seen[j] = true;
                        queue.push_back(j);
                    }
                }
            }
        }
        out.push(Component {
            bounds: Bounds {
                x: x0,
                y: y0,
                w: x1 - x0 + 1,
                h: y1 - y0 + 1,
            },
            pixels,
        });
    }
    out
}
fn morph(mask: &[bool], w: usize, h: usize, dilate: bool) -> Vec<bool> {
    let mut out = vec![false; mask.len()];
    for y in 0..h {
        for x in 0..w {
            let mut value = !dilate;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (xx, yy) = (
                        (x as isize + dx).clamp(0, w as isize - 1) as usize,
                        (y as isize + dy).clamp(0, h as isize - 1) as usize,
                    );
                    if dilate {
                        value |= mask[yy * w + xx];
                    } else {
                        value &= mask[yy * w + xx];
                    }
                }
            }
            out[y * w + x] = value;
        }
    }
    out
}

fn open_inside(mask: &[bool], w: usize, h: usize, radius: usize) -> Vec<bool> {
    // Outside a cropped artwork region is background, not a continuation of
    // its last foreground row. Padding prevents a clipped divider from being
    // replicated by erosion while retaining the same morphology predicate.
    let (pw, ph) = (w + 2 * radius, h + 2 * radius);
    let mut padded = vec![false; pw * ph];
    for y in 0..h {
        padded[(y + radius) * pw + radius..(y + radius) * pw + radius + w]
            .copy_from_slice(&mask[y * w..(y + 1) * w]);
    }
    for _ in 0..radius {
        padded = morph(&padded, pw, ph, false);
    }
    for _ in 0..radius {
        padded = morph(&padded, pw, ph, true);
    }
    (0..h)
        .flat_map(|y| {
            padded[(y + radius) * pw + radius..(y + radius) * pw + radius + w]
                .iter()
                .copied()
        })
        .collect()
}

fn without_frame_spurs(comp: &Component, board_width: usize) -> Option<Component> {
    let b = comp.bounds;
    let mut local = vec![false; b.w * b.h];
    for pixel in &comp.pixels {
        local[(pixel / board_width - b.y) * b.w + pixel % board_width - b.x] = true;
    }
    // At 64 samples per cell a narrow frame shadow can survive the ordinary
    // 3x3 opening. This 5x5 retry is only for otherwise rejected edge components.
    let cleaned = open_inside(&local, b.w, b.h, 2);
    let mut next = components(&cleaned, b.w, b.h)
        .into_iter()
        .max_by_key(|c| c.pixels.len())?;
    next.bounds.x += b.x;
    next.bounds.y += b.y;
    for pixel in &mut next.pixels {
        *pixel = (*pixel / b.w + b.y) * board_width + *pixel % b.w + b.x;
    }
    (next.pixels.len() >= GRAY_CELL * GRAY_CELL / 7).then_some(next)
}
fn distance(a: [u8; 3], b: [u8; 3]) -> f64 {
    (0..3)
        .map(|i| (a[i] as f64 - b[i] as f64).abs())
        .sum::<f64>()
        / 3.0
}
fn rgb(image: &RgbaImage, x: usize, y: usize) -> [u8; 3] {
    let p = image.get_pixel(x as u32, y as u32);
    [p[0], p[1], p[2]]
}
fn read_card(frame: &RgbaImage, viewport: Rect, index: usize) -> RgbaImage {
    let viewport = anchored_viewport(viewport, LayoutAnchor::Bottom);
    let mut image = RgbaImage::new(CARD_W as u32, CARD_H as u32);
    for y in 0..CARD_H {
        for x in 0..CARD_W {
            let p = viewport_sample(
                frame,
                viewport,
                [182.0, 389.0, 595.0][index] + x as f64,
                880.0 + y as f64,
            );
            image.put_pixel(x as u32, y as u32, Rgba([p[0], p[1], p[2], 255]));
        }
    }
    image
}
fn finish_banner(card: &RgbaImage) -> bool {
    let mut yellow = 0;
    let mut dark = 0;
    for y in 40..85 {
        for x in 0..CARD_W {
            let [r, g, b] = rgb(card, x, y);
            yellow += usize::from(r > 205 && g > 180 && b < 90 && x > 20 && x < 135);
            dark += usize::from(r < 70 && g < 90 && b < 135);
        }
    }
    yellow > 35 && dark as f64 / (45 * CARD_W) as f64 > 0.65
}
fn extract(card: RgbaImage) -> Option<CardTemplate> {
    // Learn the patterned card background from both side margins. Palette
    // agreement plus connected components prevents cyan sprite pixels from
    // being removed merely because the background is also blue.
    let mut margin = Vec::new();
    for y in (22..105).step_by(2) {
        for x in [0, 2, 4, CARD_W - 5, CARD_W - 3, CARD_W - 1] {
            margin.push(rgb(&card, x, y));
        }
    }
    let median: [u8; 3] = std::array::from_fn(|c| {
        let mut a: Vec<_> = margin.iter().map(|p| p[c]).collect();
        a.sort_unstable();
        a[a.len() / 2]
    });
    let palette: Vec<_> = margin
        .into_iter()
        .filter(|p| distance(*p, median) < 16.0)
        .collect();
    if palette.len() < 60 {
        return None;
    }
    let mut mask = vec![false; CARD_W * CARD_H];
    for y in 0..CARD_H {
        for x in 0..CARD_W {
            let p = rgb(&card, x, y);
            mask[y * CARD_W + x] = palette.iter().all(|bg| distance(p, *bg) > 12.0);
        }
    }
    mask = morph(&morph(&mask, CARD_W, CARD_H, true), CARD_W, CARD_H, false);
    let mut component = components(&mask, CARD_W, CARD_H)
        .into_iter()
        .max_by_key(|c| c.pixels.len())?;
    let b = component.bounds;
    if b.x == 0 || b.y == 0 || b.x + b.w == CARD_W || b.y + b.h == CARD_H {
        // A divider at the crop edge can connect to the sprite after closing.
        // Clean only this boundary-contaminated path; interior masks are unchanged.
        mask = open_inside(&mask, CARD_W, CARD_H, 1);
        component = components(&mask, CARD_W, CARD_H)
            .into_iter()
            .max_by_key(|c| c.pixels.len())?;
    }
    if component.pixels.len() < 450
        || component.pixels.len() > CARD_W * CARD_H * 3 / 4
        || component.bounds.w < 22
        || component.bounds.h < 22
    {
        return None;
    }
    mask.fill(false);
    for i in component.pixels {
        mask[i] = true;
    }
    let core = morph(&mask, CARD_W, CARD_H, false);
    // Coarse foreground descriptor avoids incorporating text and quantities.
    let mut hash = 0xcbf29ce484222325_u64;
    for y in 0..12 {
        for x in 0..16 {
            let (xx, yy) = (x * CARD_W / 16, y * CARD_H / 12);
            let p = if mask[yy * CARD_W + xx] {
                rgb(&card, xx, yy)
            } else {
                [0; 3]
            };
            for v in p {
                hash ^= (v / 32) as u64;
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
    }
    Some(CardTemplate {
        image: card,
        mask,
        core,
        bounds: component.bounds,
        fingerprint: format!("{hash:016x}"),
    })
}
pub(super) fn finished_cards(frame: &RgbaImage, viewport: Rect) -> [bool; 3] {
    std::array::from_fn(|i| finish_banner(&read_card(frame, viewport, i)))
}

pub(super) fn update_cards(
    frame: &RgbaImage,
    viewport: Rect,
    templates: &mut [Option<CardTemplate>; 3],
) -> [bool; 3] {
    let mut finished = [false; 3];
    for i in 0..3 {
        let card = read_card(frame, viewport, i);
        finished[i] = finish_banner(&card);
        if finished[i] {
            continue;
        }
        let next = extract(card);
        // Invalid/occluded art is not an identity match. Finish is the only
        // overlay allowed to retain an otherwise obscured round reference.
        if let Some(next) = next {
            let unchanged = templates[i].as_ref().map_or(false, |old| {
                let mut n = 0;
                let mut sum = 0.0;
                for p in 0..next.mask.len() {
                    if old.core[p] && next.core[p] {
                        n += 1;
                        sum += distance(
                            rgb(&old.image, p % CARD_W, p / CARD_W),
                            rgb(&next.image, p % CARD_W, p / CARD_W),
                        );
                    }
                }
                n > 400
                    && sum / (n as f64) < 7.0
                    && old
                        .mask
                        .iter()
                        .zip(&next.mask)
                        .filter(|(a, b)| a != b)
                        .count()
                        < next.mask.len() / 20
            });
            if !unchanged {
                templates[i] = Some(next);
            }
        } else {
            templates[i] = None;
        }
    }
    finished
}

#[derive(Clone, Copy)]
struct Rotation {
    c: f64,
    s: f64,
    x0: f64,
    y0: f64,
    w: f64,
    h: f64,
}
impl Rotation {
    fn new(t: &CardTemplate, angle: f64) -> Self {
        let (s, c) = angle.to_radians().sin_cos();
        let (mut x0, mut y0, mut x1, mut y1) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for y in t.bounds.y..t.bounds.y + t.bounds.h {
            for x in t.bounds.x..t.bounds.x + t.bounds.w {
                if !t.mask[y * CARD_W + x] {
                    continue;
                }
                let (xx, yy) = (c * x as f64 + s * y as f64, -s * x as f64 + c * y as f64);
                x0 = x0.min(xx);
                x1 = x1.max(xx);
                y0 = y0.min(yy);
                y1 = y1.max(yy);
            }
        }
        Self {
            c,
            s,
            x0,
            y0,
            w: x1 - x0 + 1.0,
            h: y1 - y0 + 1.0,
        }
    }
    fn source(self, x: f64, y: f64) -> (f64, f64) {
        let (x, y) = (x + self.x0, y + self.y0);
        (self.c * x - self.s * y, self.s * x + self.c * y)
    }
}
fn mask_at(mask: &[bool], x: f64, y: f64) -> bool {
    let (x, y) = (x.round() as isize, y.round() as isize);
    x >= 0
        && y >= 0
        && x < CARD_W as isize
        && y < CARD_H as isize
        && mask[y as usize * CARD_W + x as usize]
}
fn gray_iou(t: &CardTemplate, observed: &[bool], w: usize, b: Bounds) -> f64 {
    let mut best: f64 = 0.0;
    for angle in (0..360).step_by(2) {
        let r = Rotation::new(t, angle as f64);
        if ((r.w / r.h) / (b.w as f64 / b.h as f64)).ln().abs() > 0.16 {
            continue;
        }
        let (mut intersection, mut union) = (0, 0);
        for y in 0..48 {
            for x in 0..48 {
                let (sx, sy) =
                    r.source((x as f64 + 0.5) * r.w / 48.0, (y as f64 + 0.5) * r.h / 48.0);
                let a = mask_at(&t.mask, sx, sy);
                let xx = (b.x + ((x as f64 + 0.5) * b.w as f64 / 48.0) as usize).min(b.x + b.w - 1);
                let yy = (b.y + ((y as f64 + 0.5) * b.h as f64 / 48.0) as usize).min(b.y + b.h - 1);
                let z = observed[yy * w + xx];
                intersection += usize::from(a && z);
                union += usize::from(a || z);
            }
        }
        best = best.max(intersection as f64 / union.max(1) as f64);
    }
    best
}
fn read_board(frame: &RgbaImage, board: Rect, cell: usize) -> RgbaImage {
    let mut out = RgbaImage::new((COLS * cell) as u32, (ROWS * cell) as u32);
    for y in 0..ROWS * cell {
        for x in 0..COLS * cell {
            let p = sample(
                frame,
                board.x + (x as f64 + 0.5) * board.w / (COLS * cell) as f64,
                board.y + (y as f64 + 0.5) * board.h / (ROWS * cell) as f64,
            );
            out.put_pixel(x as u32, y as u32, Rgba([p[0], p[1], p[2], 255]));
        }
    }
    out
}
fn orientations(shape: [u32; 2]) -> Vec<(usize, usize)> {
    let (w, h) = (shape[0] as usize, shape[1] as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let mut result = Vec::with_capacity(2);
    if w <= COLS && h <= ROWS {
        result.push((w, h));
    }
    if w != h && h <= COLS && w <= ROWS {
        result.push((h, w));
    }
    result
}
fn completed_placements(
    comp: &Component,
    cells: &[String],
    shapes: &[[u32; 2]],
    w: usize,
) -> Vec<GridPlacement> {
    let b = comp.bounds;
    let mut placements = Vec::new();
    for shape in shapes {
        for (cw, ch) in orientations(*shape) {
            // Thin or tilted complete art need not fill a fixed fraction
            // of its footprint. Containment and positive pixels in every
            // footprint cell, checked below, establish its full extent.
            if b.w as f64 > (cw * GRAY_CELL) as f64 * 1.07
                || b.h as f64 > (ch * GRAY_CELL) as f64 * 1.07
            {
                continue;
            }
            let xx = ((b.x as f64 + b.w as f64 / 2.0) / GRAY_CELL as f64 - cw as f64 / 2.0).round()
                as isize;
            let yy = ((b.y as f64 + b.h as f64 / 2.0) / GRAY_CELL as f64 - ch as f64 / 2.0).round()
                as isize;
            if xx < 0 || yy < 0 || xx as usize + cw > COLS || yy as usize + ch > ROWS {
                continue;
            }
            let (x, y) = (xx as usize, yy as usize);
            if b.x + 3 < x * GRAY_CELL
                || b.y + 3 < y * GRAY_CELL
                || b.x + b.w > (x + cw) * GRAY_CELL + 3
                || b.y + b.h > (y + ch) * GRAY_CELL + 3
            {
                continue;
            }
            if (y..y + ch).any(|yy| {
                (x..x + cw).any(|xx| {
                    cells[yy * COLS + xx] == "unknown" || cells[yy * COLS + xx] == "empty"
                })
            }) {
                continue;
            }
            let each_cell = (y..y + ch).all(|yy| {
                (x..x + cw).all(|xx| {
                    comp.pixels
                        .iter()
                        .filter(|p| (**p % w) / GRAY_CELL == xx && (**p / w) / GRAY_CELL == yy)
                        .count()
                        > GRAY_CELL * GRAY_CELL / 50
                })
            });
            if each_cell {
                placements.push(GridPlacement {
                    x,
                    y,
                    width: cw,
                    height: ch,
                });
            }
        }
    }
    placements.sort_by_key(|p| p.width * p.height);
    placements.dedup();
    placements
}

pub(super) fn completed_objects(
    frame: &RgbaImage,
    board: Rect,
    templates: &[Option<CardTemplate>; 3],
    shapes: &[[u32; 2]],
    cells: &mut [String],
) -> Vec<CompletedObject> {
    let image = read_board(frame, board, GRAY_CELL);
    let (w, h) = (COLS * GRAY_CELL, ROWS * GRAY_CELL);
    let mut mask = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let p = rgb(&image, x, y);
            let (max, min) = (*p.iter().max().unwrap(), *p.iter().min().unwrap());
            let cell = (y / GRAY_CELL) * COLS + x / GRAY_CELL;
            // Fully found objects share the neutral dark overlay across events.
            // The cover gate is independent, so a dark selected tile is never an object.
            mask[y * w + x] = cells[cell] != "unknown"
                && cells[cell] != "empty"
                && max - min < 40
                && max < 180
                && min > 35;
        }
    }
    mask = morph(&morph(&mask, w, h, false), w, h, true);
    mask = morph(&morph(&mask, w, h, true), w, h, false);
    let mut out = Vec::new();
    for mut comp in components(&mask, w, h) {
        if comp.pixels.len() < GRAY_CELL * GRAY_CELL / 7 {
            continue;
        }
        let b = comp.bounds;
        let mut placements = completed_placements(&comp, cells, shapes, w);
        // A component next to the outer board frame may acquire a narrow
        // connected shadow spur. Retry only when its ordinary footprint failed;
        // successful components and every acceptance bound stay unchanged.
        if placements.is_empty()
            && (b.x <= 3 || b.y <= 3 || b.x + b.w + 3 >= w || b.y + b.h + 3 >= h)
        {
            if let Some(cleaned) = without_frame_spurs(&comp, w) {
                let next = completed_placements(&cleaned, cells, shapes, w);
                if !next.is_empty() {
                    comp = cleaned;
                    placements = next;
                }
            }
        }
        let b = comp.bounds;
        let Some(p) = placements.first() else {
            continue;
        };
        // A colorful partial screen is not the complete gray overlay: its
        // colored casing inside this rectangle supplies contradictory pixels.
        let mut saturated = 0;
        let mut dark = 0;
        for yy in p.y * GRAY_CELL..(p.y + p.height) * GRAY_CELL {
            for xx in p.x * GRAY_CELL..(p.x + p.width) * GRAY_CELL {
                let q = rgb(&image, xx, yy);
                let (max, min) = (*q.iter().max().unwrap(), *q.iter().min().unwrap());
                saturated += usize::from(max - min > 65 && min < 160);
                dark += usize::from(max < 185);
            }
        }
        let mut scores: Vec<_> = templates
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                t.as_ref()
                    .filter(|_| {
                        shapes
                            .get(i)
                            .map_or(false, |s| orientations(*s).contains(&(p.width, p.height)))
                    })
                    .map(|t| (i, gray_iou(t, &mask, w, b)))
            })
            .collect();
        scores.sort_by(|a, b| b.1.total_cmp(&a.1));
        let typed = scores
            .first()
            .filter(|(_, s)| *s > 0.78 && scores.get(1).map_or(true, |b| *s - b.1 > 0.09))
            .map(|(i, _)| *i);
        let missing_matching_shape = templates.iter().enumerate().any(|(i, t)| {
            t.is_none()
                && shapes
                    .get(i)
                    .map_or(false, |s| orientations(*s).contains(&(p.width, p.height)))
        });
        if typed.is_none() && saturated > dark / 12 + 8 {
            continue;
        }
        if typed.is_none() && !missing_matching_shape {
            continue;
        }
        let levels: Vec<f64> = comp
            .pixels
            .iter()
            .map(|i| {
                let q = rgb(&image, i % w, i / w);
                q.iter().map(|v| *v as f64).sum::<f64>() / 3.0
            })
            .collect();
        let mean = levels.iter().sum::<f64>() / levels.len() as f64;
        let variance = levels.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / levels.len() as f64;
        if variance.sqrt() < 2.0
            || comp.pixels.len() * 100 > p.width * p.height * GRAY_CELL * GRAY_CELL * 88
        {
            continue;
        }
        // Uncached Finish cannot yield a color template. The complete neutral
        // component still provides an untyped occupied rectangle.
        for yy in p.y..p.y + p.height {
            for xx in p.x..p.x + p.width {
                cells[yy * COLS + xx] = "completed".to_owned();
            }
        }
        out.push(CompletedObject {
            item_index: typed,
            x: p.x,
            y: p.y,
            width: p.width,
            height: p.height,
        });
    }
    out
}

#[derive(Clone)]
struct Candidate {
    placement: GridPlacement,
    angle: f64,
    fill: f64,
    dx: f64,
    dy: f64,
    score: f64,
    counts: [usize; 45],
}
fn evaluate(
    image: &RgbaImage,
    t: &CardTemplate,
    r: Rotation,
    c: &Candidate,
    cells: &[String],
) -> Option<Candidate> {
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
    let mut bad = 0;
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
            let error = distance(observed, expected);
            sum += error;
            count += 1;
            bad += usize::from(error > 35.0);
            counts[idx] += 1;
        }
    }
    if count < 55 || bad as f64 / count as f64 > 0.40 {
        return None;
    }
    let mut result = c.clone();
    result.score = sum / count as f64;
    result.counts = counts;
    Some(result)
}
pub(super) fn colored_observations(
    frame: &RgbaImage,
    board: Rect,
    templates: &[Option<CardTemplate>; 3],
    shapes: &[[u32; 2]],
    cells: &mut [String],
) -> Vec<PlacementConstraint> {
    let active: Vec<_> = cells
        .iter()
        .enumerate()
        .filter_map(|(i, s)| (s == "uncertain").then_some(i))
        .collect();
    if active.is_empty() {
        return Vec::new();
    }
    let image = read_board(frame, board, MATCH_CELL);
    let mut best: Vec<Vec<Option<Candidate>>> = vec![vec![None; 45]; 3];
    let mut alternatives: Vec<Vec<std::collections::HashSet<(usize, usize, usize, usize)>>> =
        vec![vec![std::collections::HashSet::new(); 45]; 3];
    for (item, template) in templates.iter().enumerate() {
        let Some(t) = template else {
            continue;
        };
        let dims = orientations(shapes.get(item).copied().unwrap_or([0, 0]));
        for angle in (0..360).step_by(10) {
            let r = Rotation::new(t, angle as f64);
            for &(cw, ch) in &dims {
                if cw > COLS
                    || ch > ROWS
                    || ((r.w / r.h) / (cw as f64 / ch as f64)).ln().abs() > 0.40
                {
                    continue;
                }
                for y in 0..=ROWS - ch {
                    for x in 0..=COLS - cw {
                        let contains = |i: usize| {
                            i % COLS >= x && i % COLS < x + cw && i / COLS >= y && i / COLS < y + ch
                        };
                        if !active.iter().any(|i| contains(*i))
                            || (y..y + ch).any(|yy| {
                                (x..x + cw).any(|xx| cells[yy * COLS + xx] == "completed")
                            })
                        {
                            continue;
                        }
                        for fill in [0.84, 0.92, 0.99] {
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
                            if let Some(c) = evaluate(&image, t, r, &c, cells) {
                                for &idx in &active {
                                    if c.counts[idx] >= 45 && c.score < 38.0 {
                                        alternatives[item][idx].insert((
                                            c.placement.x,
                                            c.placement.y,
                                            c.placement.width,
                                            c.placement.height,
                                        ));
                                    }
                                    if c.counts[idx] >= 45
                                        && best[item][idx]
                                            .as_ref()
                                            .map_or(true, |b| c.score < b.score)
                                    {
                                        best[item][idx] = Some(c.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        // Refine rotation and scale without widening the acceptance bounds.
        for &idx in &active {
            for (angles, scales, offsets) in [
                (
                    vec![-6.0, -4.0, -2.0, 0.0, 2.0, 4.0, 6.0],
                    vec![-0.04, -0.02, 0.0, 0.02, 0.04],
                    vec![-1.5, 0.0, 1.5],
                ),
                (
                    vec![-1.0, 0.0, 1.0],
                    vec![-0.01, 0.0, 0.01],
                    vec![-0.75, 0.0, 0.75],
                ),
            ] {
                let Some(seed) = best[item][idx].clone() else {
                    continue;
                };
                if seed.score > 38.0 {
                    continue;
                }
                for da in angles {
                    let r = Rotation::new(t, seed.angle + da);
                    for df in &scales {
                        for dy in &offsets {
                            for dx in &offsets {
                                let c = Candidate {
                                    angle: seed.angle + da,
                                    fill: seed.fill + df,
                                    dx: seed.dx + dx,
                                    dy: seed.dy + dy,
                                    ..seed.clone()
                                };
                                if let Some(c) = evaluate(&image, t, r, &c, cells) {
                                    if c.counts[idx] >= 45
                                        && best[item][idx]
                                            .as_ref()
                                            .map_or(true, |b| c.score < b.score)
                                    {
                                        best[item][idx] = Some(c);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut constraints = Vec::new();
    for idx in active {
        let mut ranked: Vec<_> = (0..3)
            .filter_map(|i| best[i][idx].as_ref().map(|c| (i, c)))
            .collect();
        ranked.sort_by(|a, b| a.1.score.total_cmp(&b.1.score));
        let Some((item, candidate)) = ranked.first() else {
            continue;
        };
        if candidate.score > 18.0
            || ranked
                .get(1)
                .map_or(false, |(_, b)| b.score - candidate.score < 6.0)
        {
            continue;
        }
        cells[idx] = format!("item{item}");
        // One strong pose supplies a geometric candidate, never observations
        // for its covered neighbors. Omit weak/ambiguous fits entirely.
        if candidate.score < 11.0
            && candidate.counts[idx] >= 90
            && alternatives[*item][idx].len() == 1
        {
            constraints.push(PlacementConstraint {
                anchor: idx,
                item_index: *item,
                placements: vec![candidate.placement.clone()],
            });
        }
    }
    constraints
}

/// Learn a color/texture model from the exposed corners in this frame. The
/// dominant repeated corner cluster is independent of a particular event's
/// hue. Brightness variation is fitted per channel, so texture/shadows are
/// positive evidence; an unrecognized foreground is never the empty test.
pub(super) fn background_cells(patches: &[super::Patch], cells: &mut [String]) {
    let mut corners = Vec::new();
    for (index, p) in patches.iter().enumerate() {
        if cells[index] == "unknown" {
            continue;
        }
        for y0 in [4, 22] {
            for x0 in [4, 22] {
                for y in y0..y0 + 6 {
                    for x in x0..x0 + 6 {
                        corners.push(p.rgb[y * 32 + x]);
                    }
                }
            }
        }
    }
    if corners.len() < 100 {
        return;
    }
    let mut bins = std::collections::HashMap::<[u8; 3], usize>::new();
    for p in &corners {
        *bins.entry(p.map(|v| v / 16)).or_default() += 1;
    }
    let Some((mode, count)) = bins.into_iter().max_by_key(|(_, n)| *n) else {
        return;
    };
    if count < corners.len() / 8 {
        return;
    }
    let center = mode.map(|v| v as f64 * 16.0 + 8.0);
    let mut background: Vec<_> = corners
        .into_iter()
        .filter(|p| (0..3).all(|c| (p[c] as f64 - center[c]).abs() < 55.0))
        .collect();
    if background.len() < 100 {
        return;
    }
    let lum = |p: &[u8; 3]| p.iter().map(|v| *v as f64).sum::<f64>() / 3.0;
    let fit_line = |points: &Vec<[u8; 3]>| {
        let n = points.len() as f64;
        let mean = points.iter().map(lum).sum::<f64>() / n;
        let variance = points.iter().map(|p| (lum(p) - mean).powi(2)).sum::<f64>() / n;
        let color: [f64; 3] =
            std::array::from_fn(|c| points.iter().map(|p| p[c] as f64).sum::<f64>() / n);
        let gain: [f64; 3] = std::array::from_fn(|c| {
            points
                .iter()
                .map(|p| (lum(p) - mean) * (p[c] as f64 - color[c]))
                .sum::<f64>()
                / (n * variance.max(0.001))
        });
        (mean, variance, color, gain)
    };
    let original_count = background.len();
    for _ in 0..2 {
        let (mean, _, color, gain) = fit_line(&background);
        background.retain(|p| {
            (0..3).all(|c| (p[c] as f64 - color[c] - gain[c] * (lum(p) - mean)).abs() < 3.5)
        });
        if background.len() < 100 || background.len() * 2 < original_count {
            return;
        }
    }
    let (mean, variance, color, gain) = fit_line(&background);
    if !(3.0..22.0).contains(&variance.sqrt()) {
        return;
    }
    // Automatic bootstrap is deliberately restricted to a light, near-neutral
    // exposed surface. A dark gray completed sprite or saturated foreground
    // cannot teach itself as empty. Other themes need a trusted empty
    // correction, which is retained only for this round.
    if mean < 185.0
        || color.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            - color.iter().copied().fold(f64::INFINITY, f64::min)
            > 35.0
    {
        return;
    }
    let mut levels: Vec<_> = background.iter().map(lum).collect();
    levels.sort_by(f64::total_cmp);
    let low = levels[levels.len() / 100] - 15.0;
    let high = levels[levels.len() * 99 / 100] + 3.0;
    let fit = |p: [u8; 3]| {
        let level = lum(&p);
        level > low
            && level < high
            && (0..3).all(|c| (p[c] as f64 - color[c] - gain[c] * (level - mean)).abs() < 2.7)
    };
    // Require the model to actually describe a shared background population.
    if background.iter().filter(|p| fit(**p)).count() * 100 < background.len() * 94 {
        return;
    }
    for (index, p) in patches.iter().enumerate() {
        if cells[index] != "uncertain" {
            continue;
        }
        let mut agrees = 0;
        let mut values = Vec::new();
        for y in 4..28 {
            for x in 4..28 {
                let q = p.rgb[y * 32 + x];
                agrees += usize::from(fit(q));
                values.push(lum(&q));
            }
        }
        values.sort_by(f64::total_cmp);
        let spread = values[values.len() * 9 / 10] - values[values.len() / 10];
        // A flat cutout can share background color; it does not share the
        // observed texture distribution. Both bounds are data-relative.
        let mut flat_cutout = false;
        for y0 in 4..22 {
            for x0 in 4..22 {
                let mut sum = 0.0;
                let mut sum2 = 0.0;
                for y in y0..y0 + 7 {
                    for x in x0..x0 + 7 {
                        let v = lum(&p.rgb[y * 32 + x]);
                        sum += v;
                        sum2 += v * v;
                    }
                }
                let local_variance = (sum2 / 49.0 - (sum / 49.0).powi(2)).max(0.0);
                if local_variance < variance * 0.04 {
                    flat_cutout = true;
                }
            }
        }
        if !flat_cutout
            && agrees * 100 >= values.len() * 94
            && spread > variance.sqrt() * 1.0
            && spread < variance.sqrt() * 4.0
        {
            cells[index] = "empty".to_owned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn orientations_use_board_geometry_for_each_direction() {
        for shape in [[9, 1], [1, 9]] {
            assert_eq!(orientations(shape), vec![(9, 1)]);
        }
        for shape in [[9, 5], [5, 9]] {
            assert_eq!(orientations(shape), vec![(9, 5)]);
        }
        assert_eq!(orientations([2, 3]), vec![(2, 3), (3, 2)]);
        assert_eq!(orientations([5, 5]), vec![(5, 5)]);
        for shape in [[0, 1], [1, 0], [10, 1], [1, 10], [6, 6]] {
            assert!(orientations(shape).is_empty(), "shape {shape:?}");
        }
    }

    fn fixture(name: &str) -> RgbaImage {
        image::open(format!("tests/fixtures/vision-{name}.png"))
            .unwrap()
            .to_rgba8()
    }
    #[test]
    fn card_rotation_and_replaced_card_background_keep_real_board_identity() {
        let mut frame = fixture("other-items");
        let vp = super::super::game_viewport(&frame).unwrap();
        for item in 0..3 {
            let original = extract(read_card(&frame, vp, item)).unwrap();
            let r = Rotation::new(&original, 17.0);
            let scale = ((CARD_W as f64 - 10.0) / r.w).min((CARD_H as f64 - 10.0) / r.h) * 0.91;
            let (ox, oy) = (
                (CARD_W as f64 - r.w * scale) / 2.0,
                (CARD_H as f64 - r.h * scale) / 2.0,
            );
            for y in 0..CARD_H {
                for x in 0..CARD_W {
                    let (sx, sy) = r.source((x as f64 - ox) / scale, (y as f64 - oy) / scale);
                    let p = if mask_at(&original.mask, sx, sy) {
                        sample(&original.image, sx, sy)
                    } else {
                        [55, 105, 165]
                    };
                    frame.put_pixel(
                        [184, 391, 597][item] + x as u32,
                        940 + y as u32,
                        Rgba([p[0], p[1], p[2], 255]),
                    );
                }
            }
        }
        let mut rec = super::super::Recognizer::new();
        let a = rec.analyze(&frame, None, Some(25));
        assert_eq!(a.reference_ready, [true; 3]);
        assert_eq!(a.cells[13], "item0");
        assert_eq!(a.cells[26], "item1");
        assert_eq!(a.cells[39], "item1");
        assert_eq!(a.completed_objects.len(), 3);
        assert!(a.completed_objects.iter().all(|o| o.item_index.is_some()));
        assert_eq!(
            a.cells.iter().filter(|s| s.as_str() == "unknown").count(),
            25
        );
    }
    #[test]
    fn neutral_or_saturated_foreground_cannot_teach_itself_as_empty() {
        for base in [[110_u8, 112, 115], [175, 60, 80]] {
            let mut patches = Vec::new();
            for i in 0..45 {
                let pixels = (0..1024)
                    .map(|j| {
                        let variation = ((j * 7 + i * 11) % 17) as u8;
                        base.map(|v| v.saturating_add(variation))
                    })
                    .collect();
                patches.push(super::super::Patch::from_rgb(pixels));
            }
            let mut cells = vec!["uncertain".to_owned(); 45];
            background_cells(&patches, &mut cells);
            assert!(cells.iter().all(|s| s == "uncertain"));
        }
    }
    #[test]
    fn flat_gray_rectangle_is_not_a_completed_object_or_empty() {
        let mut frame = fixture("other-items");
        for y in 346..450 {
            for x in 1534..1742 {
                frame.put_pixel(x, y, Rgba([112, 114, 116, 255]));
            }
        }
        let a = super::super::Recognizer::new().analyze(&frame, None, Some(25));
        assert_eq!(a.cells[6], "uncertain");
        assert_eq!(a.cells[7], "uncertain");
        assert!(!a.completed_objects.iter().any(|o| o.x == 6 && o.y == 0));
    }
}

#[cfg(test)]
mod generated_round_tests {
    use super::super::Recognizer;
    use super::*;
    use image::imageops;

    fn frame(name: &str) -> RgbaImage {
        image::open(format!("tests/fixtures/vision-{name}.png"))
            .unwrap()
            .to_rgba8()
    }

    // A new asymmetric purple satellite, defined by geometry rather than any
    // captured game icon. Both targets are independently rendered from it.
    fn generated_satellite() -> CardTemplate {
        let mut art =
            RgbaImage::from_pixel(CARD_W as u32, CARD_H as u32, Rgba([38, 104, 198, 255]));
        for y in 0..CARD_H {
            for x in 0..CARD_W {
                let (xx, yy) = (x as f64 - 77.0, y as f64 - 62.0);
                let body = (xx / 34.0).powi(2) + (yy / 42.0).powi(2) < 1.0;
                let wings = (20..135).contains(&x) && (48..76).contains(&y);
                let notch = x > 91 && y < 39;
                if (body || wings) && !notch {
                    let color = if (x + y * 2) % 37 < 9 {
                        [248, 188, 52]
                    } else if x < 40 || x > 118 {
                        [77, 35, 119]
                    } else {
                        [173, 58, 203]
                    };
                    art.put_pixel(
                        x as u32,
                        y as u32,
                        Rgba([color[0], color[1], color[2], 255]),
                    );
                }
            }
        }
        extract(art).unwrap()
    }

    fn render_foreground(
        target: &mut RgbaImage,
        art: &CardTemplate,
        rect: Bounds,
        angle: f64,
        fill: f64,
    ) {
        let rotation = Rotation::new(art, angle);
        let scale = (rect.w as f64 / rotation.w).min(rect.h as f64 / rotation.h) * fill;
        let (ox, oy) = (
            (rect.w as f64 - rotation.w * scale) / 2.0,
            (rect.h as f64 - rotation.h * scale) / 2.0,
        );
        for y in 0..rect.h {
            for x in 0..rect.w {
                let (sx, sy) =
                    rotation.source((x as f64 + 0.5 - ox) / scale, (y as f64 + 0.5 - oy) / scale);
                if mask_at(&art.mask, sx, sy) {
                    let c = sample(&art.image, sx, sy);
                    target.put_pixel(
                        (rect.x + x) as u32,
                        (rect.y + y) as u32,
                        Rgba([c[0], c[1], c[2], 255]),
                    );
                }
            }
        }
    }

    #[test]
    fn generated_new_item_and_two_by_two_shape_replace_previous_round_reference() {
        let initial = frame("initial");
        let opened = frame("opened");
        let mut derived = initial.clone();
        let satellite = generated_satellite();
        // Slot 0 has new artwork, 23 degrees at card scale. Its quantity comes
        // from the host's snapshot input; this is not an OCR digit test.
        for y in 940..1064 {
            for x in 184..338 {
                derived.put_pixel(x, y, Rgba([38, 104, 198, 255]));
            }
        }
        render_foreground(
            &mut derived,
            &satellite,
            Bounds {
                x: 184,
                y: 940,
                w: CARD_W,
                h: CARD_H,
            },
            23.0,
            0.84,
        );
        // Replace the former 3x2 mini-mask with a positively visible 2x2 mask.
        for y in 5..26 {
            for x in 5..37 {
                derived.put_pixel(168 + x, 1068 + y, Rgba([199, 224, 246, 255]));
            }
        }
        for row in 0..2 {
            for col in 0..2 {
                for y in 0..6 {
                    for x in 0..6 {
                        derived.put_pixel(
                            168 + 10 + col * 8 + x,
                            1068 + 9 + row * 8 + y,
                            Rgba([69, 98, 158, 255]),
                        );
                    }
                }
            }
        }
        // Independent board rendering at 73 degrees and larger scale. Leave
        // one of the 2x2 footprint's cells covered, as in a real partial find.
        let ice = imageops::crop_imm(&opened, 1534, 450, 104, 104).to_image();
        for row in 1..3 {
            for col in 4..6 {
                imageops::overlay(&mut derived, &ice, 910 + col * 104, 346 + row * 104);
            }
        }
        render_foreground(
            &mut derived,
            &satellite,
            Bounds {
                x: 1326,
                y: 450,
                w: 208,
                h: 208,
            },
            73.0,
            0.94,
        );
        let still_covered = imageops::crop_imm(&initial, 1430, 554, 104, 104).to_image();
        imageops::overlay(&mut derived, &still_covered, 1430, 554);

        let mut recognizer = Recognizer::new();
        let previous =
            recognizer.analyze_snapshot(&initial, None, Some(45), [Some(2), Some(5), Some(2)]);
        assert_eq!(previous.shapes[0], [3, 2]);
        recognizer.reset();
        let next =
            recognizer.analyze_snapshot(&derived, None, Some(42), [Some(3), Some(1), Some(2)]);
        assert!(next.present, "{}", next.message);
        assert_eq!(next.shapes[0], [2, 2]);
        assert_ne!(next.card_fingerprints[0], previous.card_fingerprints[0]);
        assert_eq!(next.reference_ready, [true; 3]);
        for index in [13, 14, 22] {
            assert_eq!(
                next.cells[index], "item0",
                "generated satellite cell {index}"
            );
        }
        assert_eq!(next.cells[23], "unknown");
        assert_eq!(
            next.cells
                .iter()
                .filter(|s| s.as_str() == "unknown")
                .count(),
            42
        );
        assert!(next.completed_objects.is_empty());
        for constraint in &next.candidate_constraints {
            if constraint.item_index == 0 {
                assert!(constraint
                    .placements
                    .iter()
                    .all(|p| p.width == 2 && p.height == 2));
            }
        }
        let other_counts =
            recognizer.analyze_snapshot(&derived, None, Some(42), [Some(7), Some(9), Some(4)]);
        assert_eq!(
            other_counts.cells, next.cells,
            "remaining object counts never supply observed pixels"
        );
        recognizer.reset();
        let restored = recognizer.analyze(&initial, None, Some(45));
        assert_eq!(restored.shapes[0], [3, 2]);
        assert_eq!(restored.card_fingerprints[0], previous.card_fingerprints[0]);
    }
}
