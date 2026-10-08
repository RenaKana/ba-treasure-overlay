//! Experimental correction at the raw-mask boundary, before legacy morphology.
//! Only current-card evidence is used. Production extract/update_cards are unchanged.
use super::*;
use std::collections::BTreeSet;

// The normalized card sampling domain is identical to production extract().
const SAMPLE_START: usize = 22;
const SAMPLE_END: usize = 105;
const SAMPLE_STEP: usize = 2;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub color_error_max: f64,
    pub vertical_bands: usize,
    pub min_rows_per_band: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            color_error_max: 12.0,
            vertical_bands: 3,
            min_rows_per_band: 3,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        let rows = (SAMPLE_START..SAMPLE_END).step_by(SAMPLE_STEP).count();
        if !self.color_error_max.is_finite()
            || !(0.0..=255.0).contains(&self.color_error_max)
            || self.vertical_bands == 0
            || self.vertical_bands > rows
            || self.min_rows_per_band == 0
            || self.min_rows_per_band > rows / self.vertical_bands
        {
            return Err("invalid experimental boundary-background evidence configuration".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub enabled: bool,
    pub status: String,
    pub repeated_colors: Vec<[u8; 3]>,
    pub removed_raw_pixels: usize,
    pub legacy_mask_pixels: Option<usize>,
    pub repaired_mask_pixels: Option<usize>,
    pub changed_mask_pixels: usize,
    pub reused_on_finish: bool,
}

/// Outer Option means a correction was attempted; inner None revokes an invalid
/// corrected template instead of retaining an older identity as a fallback.
pub(super) fn repair(card: &RgbaImage, cfg: &Config) -> (Option<Option<CardTemplate>>, Diagnostic) {
    let original = extract(card.clone());
    let old_count = original
        .as_ref()
        .map(|t| t.mask.iter().filter(|&&v| v).count());
    let edge = original.as_ref().is_some_and(|t| touches(t.bounds));
    let mut diagnostic = Diagnostic {
        enabled: cfg.enabled,
        status: if !cfg.enabled {
            "disabled"
        } else if edge {
            "edge_unresolved"
        } else {
            "unchanged"
        }
        .into(),
        repeated_colors: Vec::new(),
        removed_raw_pixels: 0,
        legacy_mask_pixels: old_count,
        repaired_mask_pixels: old_count,
        changed_mask_pixels: 0,
        reused_on_finish: false,
    };
    if !cfg.enabled {
        return (None, diagnostic);
    }
    let (Some(mut raw), samples) = raw_mask(card) else {
        diagnostic.status = "legacy_background_unavailable".into();
        return (None, diagnostic);
    };
    let seeds: BTreeSet<_> = samples
        .iter()
        .flat_map(|row| row.iter())
        .filter_map(|&(x, y, p)| raw[y * CARD_W + x].then_some(p))
        .collect();
    let mut removal = vec![false; raw.len()];
    for seed in seeds {
        let supported = (0..2).all(|side| {
            (0..cfg.vertical_bands).all(|band| {
                // Equal-sized row bands, with remainder assigned to the first bands.
                let base = samples.len() / cfg.vertical_bands;
                let extra = samples.len() % cfg.vertical_bands;
                let start = band * base + band.min(extra);
                let end = start + base + usize::from(band < extra);
                samples[start..end]
                    .iter()
                    .filter(|row| {
                        row[side * 3..side * 3 + 3]
                            .iter()
                            .any(|&(_, _, p)| distance(p, seed) <= cfg.color_error_max)
                    })
                    .count()
                    >= cfg.min_rows_per_band
            })
        });
        if !supported {
            continue;
        }
        diagnostic.repeated_colors.push(seed);
        let colored: Vec<_> = raw
            .iter()
            .enumerate()
            .map(|(i, &fg)| {
                fg && distance(rgb(card, i % CARD_W, i / CARD_W), seed) <= cfg.color_error_max
            })
            .collect();
        for component in components(&colored, CARD_W, CARD_H)
            .into_iter()
            .filter(|c| touches(c.bounds))
        {
            for i in component.pixels {
                removal[i] = true;
            }
        }
    }
    diagnostic.removed_raw_pixels = removal.iter().filter(|&&v| v).count();
    if diagnostic.removed_raw_pixels == 0 {
        return (None, diagnostic);
    }
    for (pixel, remove) in raw.iter_mut().zip(removal) {
        *pixel &= !remove;
    }
    let repaired = finish_mask(card.clone(), raw);
    diagnostic.repaired_mask_pixels = repaired
        .as_ref()
        .map(|t| t.mask.iter().filter(|&&v| v).count());
    diagnostic.changed_mask_pixels = (0..CARD_W * CARD_H)
        .filter(|&i| {
            original.as_ref().is_some_and(|t| t.mask[i])
                != repaired.as_ref().is_some_and(|t| t.mask[i])
        })
        .count();
    diagnostic.status = if repaired.is_none() {
        "invalid_after_cleanup"
    } else if repaired.as_ref().is_some_and(|t| touches(t.bounds)) {
        "cleaned_edge_unresolved"
    } else {
        "cleaned"
    }
    .into();
    (Some(repaired), diagnostic)
}

fn touches(b: Bounds) -> bool {
    b.x == 0 || b.y == 0 || b.x + b.w == CARD_W || b.y + b.h == CARD_H
}
type MarginRow = [(usize, usize, [u8; 3]); 6];
fn raw_mask(card: &RgbaImage) -> (Option<Vec<bool>>, Vec<MarginRow>) {
    let samples: Vec<MarginRow> = (SAMPLE_START..SAMPLE_END)
        .step_by(SAMPLE_STEP)
        .map(|y| [0, 2, 4, CARD_W - 5, CARD_W - 3, CARD_W - 1].map(|x| (x, y, rgb(card, x, y))))
        .collect();
    let margin: Vec<_> = samples
        .iter()
        .flat_map(|row| row.iter().map(|&(_, _, p)| p))
        .collect();
    let median = std::array::from_fn(|c| {
        let mut values: Vec<_> = margin.iter().map(|p| p[c]).collect();
        values.sort_unstable();
        values[values.len() / 2]
    });
    // Keep production palette unchanged. Repainting margins would relearn a
    // different background and change unrelated sprite pixels.
    let palette: Vec<_> = margin
        .into_iter()
        .filter(|p| distance(*p, median) < 16.0)
        .collect();
    if palette.len() < 60 {
        return (None, samples);
    }
    let mask = (0..CARD_W * CARD_H)
        .map(|i| {
            let p = rgb(card, i % CARD_W, i / CARD_W);
            palette.iter().all(|&bg| distance(p, bg) > 12.0)
        })
        .collect();
    (Some(mask), samples)
}

// This tail mirrors extract() exactly, using its existing morphology/component
// helpers and validity checks. Only the raw mask supplied above is changed.
fn finish_mask(card: RgbaImage, mut mask: Vec<bool>) -> Option<CardTemplate> {
    mask = morph(&morph(&mask, CARD_W, CARD_H, true), CARD_W, CARD_H, false);
    let mut component = components(&mask, CARD_W, CARD_H)
        .into_iter()
        .max_by_key(|c| c.pixels.len())?;
    if touches(component.bounds) {
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

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            enabled: true,
            ..Config::default()
        }
    }
    fn frame() -> RgbaImage {
        image::load_from_memory(include_bytes!(
            "../../tests/fixtures/vision-other-items.png"
        ))
        .unwrap()
        .to_rgba8()
    }
    fn cards(frame: &RgbaImage) -> [RgbaImage; 3] {
        let vp = super::super::super::super::content_rect(
            frame,
            super::super::super::super::locate_content(frame).unwrap(),
        )
        .unwrap();
        std::array::from_fn(|i| read_card(frame, vp, i))
    }
    fn equal(a: &CardTemplate, b: &CardTemplate) {
        assert_eq!(a.mask, b.mask);
        assert_eq!(a.core, b.core);
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(
            [a.bounds.x, a.bounds.y, a.bounds.w, a.bounds.h],
            [b.bounds.x, b.bounds.y, b.bounds.w, b.bounds.h]
        );
    }
    #[test]
    fn legacy_tail_and_clean_rotated_cards_are_exact_noops() {
        let original = frame();
        for frame in [
            original.clone(),
            derive(&original, "rotate-cards-17").unwrap(),
        ] {
            for card in cards(&frame) {
                let expected = extract(card.clone()).unwrap();
                equal(
                    &expected,
                    &finish_mask(card.clone(), raw_mask(&card).0.unwrap()).unwrap(),
                );
                let (change, d) = repair(&card, &config());
                assert!(change.is_none());
                assert_eq!(d.removed_raw_pixels, 0);
            }
        }
    }
    #[test]
    fn polluted_reference_recovers_shape_without_losing_clean_foreground() {
        let frame = frame();
        let clean = extract(cards(&frame)[0].clone()).unwrap();
        let card = cards(&derive(&frame, "background-pollution").unwrap())[0].clone();
        let (change, d) = repair(&card, &config());
        let fixed = change.unwrap().unwrap();
        assert_eq!(d.status, "cleaned");
        assert!(d.removed_raw_pixels > 0);
        assert!(clean.mask.iter().zip(&fixed.mask).all(|(&a, &b)| !a || b));
        assert_eq!(
            [
                fixed.bounds.x,
                fixed.bounds.y,
                fixed.bounds.w,
                fixed.bounds.h
            ],
            [
                clean.bounds.x,
                clean.bounds.y,
                clean.bounds.w,
                clean.bounds.h
            ]
        );
        assert!(repair(&card, &Config::default()).0.is_none());
    }
    #[test]
    fn single_edge_or_short_repeated_color_is_not_erased() {
        for (sides, range) in [(1, 0..CARD_H), (2, 55..65)] {
            let mut card = cards(&frame())[0].clone();
            for side in 0..sides {
                for y in range.clone() {
                    for dx in 0..7 {
                        let x = if side == 0 { dx } else { CARD_W - 1 - dx };
                        card.put_pixel(x as u32, y as u32, Rgba([210, 65, 180, 255]));
                    }
                }
            }
            let (change, d) = repair(&card, &config());
            assert!(change.is_none());
            assert_eq!(d.removed_raw_pixels, 0);
        }
    }
    #[test]
    fn same_color_internal_thin_detail_survives_boundary_cleanup() {
        let mut card = cards(&derive(&frame(), "background-pollution").unwrap())[0].clone();
        for y in 60..62 {
            for x in 65..95 {
                card.put_pixel(x, y, Rgba([210, 65, 180, 255]));
            }
        }
        let fixed = repair(&card, &config()).0.unwrap().unwrap();
        for y in 60..62 {
            for x in 65..95 {
                assert!(fixed.mask[y * CARD_W + x]);
                assert_eq!(rgb(&fixed.image, x, y), [210, 65, 180]);
            }
        }
    }
    #[test]
    fn evidence_config_rejects_invalid_or_impossible_support() {
        assert!(config().validate().is_ok());
        for bad in [
            Config {
                vertical_bands: 0,
                ..config()
            },
            Config {
                min_rows_per_band: 43,
                ..config()
            },
            Config {
                color_error_max: f64::NAN,
                ..config()
            },
        ] {
            assert!(bad.validate().is_err());
        }
    }
}
