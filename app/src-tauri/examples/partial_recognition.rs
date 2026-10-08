//! Offline only. No capture, desktop host, OCR side effects, or game input.
#![allow(dead_code)]
#[path = "partial_recognition/competition.rs"]
mod competition;
#[path = "partial_recognition/constraints.rs"]
mod constraints;
#[path = "../src/vision.rs"]
mod vision;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use vision::experiment;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Truth {
    anchor: usize,
    item_index: usize,
    rect: vision::GridPlacement,
    angle: Option<f64>,
    instance_id: String,
    source: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hud {
    remaining: Option<u32>,
    counts: Option<Vec<u32>>,
    source: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Frame {
    id: String,
    source: String,
    group: String,
    sequence: String,
    order: u32,
    split: String,
    kind: String,
    round: String,
    notes: String,
    hud: Hud,
    #[serde(default)]
    truth: Vec<Truth>,
    #[serde(default)]
    transform: Option<String>,
    #[serde(default)]
    must_remain_ambiguous: Vec<usize>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    notes: Vec<String>,
    frames: Vec<Frame>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    schema: u32,
    parameter_policy: String,
    matching: experiment::Config,
    constraints: constraints::Config,
    direction_tolerance_degrees: f64,
    #[serde(default)]
    scoring: Scoring,
}

#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Scoring {
    /// Final score limits never prune the coarse/refinement candidate search.
    retention_max_error: f64,
    threshold_controls: Vec<f64>,
    visible_foreground_max_error: f64,
    /// Optional offline geometry competition after the complete global check.
    geometry_competition: Option<competition::Config>,
}
impl Default for Scoring {
    fn default() -> Self {
        Self {
            retention_max_error: 38.0,
            threshold_controls: vec![21.0, 20.0],
            visible_foreground_max_error: 38.0,
            geometry_competition: None,
        }
    }
}
impl Scoring {
    fn validate(&self) -> Result<(), String> {
        if let Some(config) = &self.geometry_competition {
            config.validate()?;
        }
        let limits = [self.retention_max_error, self.visible_foreground_max_error];
        if limits
            .iter()
            .chain(&self.threshold_controls)
            .any(|x| !x.is_finite() || *x < 0.0)
            || self
                .threshold_controls
                .iter()
                .enumerate()
                .any(|(i, x)| self.threshold_controls[..i].contains(x))
        {
            return Err("invalid or duplicate final scoring thresholds".into());
        }
        Ok(())
    }
}
struct StageSpec {
    name: String,
    threshold: f64,
    legacy_index: usize,
    visible_foreground: bool,
}
fn stage_specs(cfg: &Config) -> Vec<StageSpec> {
    let mut specs: Vec<_> = ["color", "color_edge", "color_edge_bidirectional"]
        .iter()
        .enumerate()
        .map(|(i, name)| StageSpec {
            name: (*name).into(),
            threshold: cfg.scoring.retention_max_error,
            legacy_index: i,
            visible_foreground: false,
        })
        .collect();
    specs.extend(
        cfg.scoring
            .threshold_controls
            .iter()
            .map(|&threshold| StageSpec {
                name: format!("bidirectional_threshold_{threshold}"),
                threshold,
                legacy_index: 2,
                visible_foreground: false,
            }),
    );
    specs.push(StageSpec {
        name: "visible_foreground".into(),
        threshold: cfg.scoring.visible_foreground_max_error,
        legacy_index: 2,
        visible_foreground: true,
    });
    specs
}
fn candidate_check(p: &experiment::Pose, spec: &StageSpec) -> Value {
    let base = if spec.visible_foreground {
        p.visible_base_score()
    } else {
        Some(p.scores[spec.legacy_index])
    };
    let mut reasons = Vec::new();
    if base.is_none() {
        reasons.push("no_weighted_template_evidence");
    } else if base.is_some_and(|score| score > spec.threshold) {
        reasons.push(
            if spec.visible_foreground && p.confidence_weighted.is_some() {
                "confidence_weighted_score_exceeds_final_threshold"
            } else {
                "legacy_score_exceeds_final_threshold"
            },
        );
    }
    let score = if spec.visible_foreground {
        let foreground = p
            .visible_evidence
            .as_ref()
            .and_then(|e| e.foreground_color_error);
        match foreground {
            Some(error) => {
                if error > spec.threshold {
                    reasons.push("observed_foreground_error_exceeds_final_threshold");
                }
                base.map(|base| base.max(error))
            }
            None => {
                reasons.push("no_reliable_observed_foreground");
                None
            }
        }
    } else {
        base
    };
    json!({"score":score,"accepted":score.is_some_and(|s| s <= spec.threshold),"reasons":reasons})
}

fn fnv(bytes: &[u8]) -> String {
    let mut h = 0xcbf29ce484222325u64;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}
fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T, String> {
    serde_json::from_slice(&fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?)
        .map_err(|e| format!("{}: {e}", p.display()))
}
fn write_json(p: &Path, v: &impl Serialize) -> Result<(), String> {
    fs::write(p, serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}
fn analysis(a: &vision::Analysis) -> Value {
    json!({"present":a.present,"board":a.board,"cells":a.cells,"shapes":a.shapes,"finish":a.finish,"reference_ready":a.reference_ready,"card_fingerprints":a.card_fingerprints,"completed_objects":a.completed_objects,"candidate_constraints":a.candidate_constraints,"message":a.message})
}
fn geometry(p: &experiment::Pose) -> constraints::Geometry {
    constraints::Geometry {
        item_index: p.item_index,
        x: p.rect.x as u32,
        y: p.rect.y as u32,
        w: p.rect.width as u32,
        h: p.rect.height as u32,
    }
}
fn validate_manifest(m: &Manifest) -> Result<(), String> {
    if m.schema != 1 {
        return Err("unsupported manifest schema".into());
    }
    let mut ids = BTreeSet::new();
    let mut splits = HashMap::new();
    let mut orders = BTreeSet::new();
    for f in &m.frames {
        if f.id.is_empty()
            || !f
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !ids.insert(&f.id)
        {
            return Err("frame IDs must be unique safe directory names".into());
        }
        if !["tuning", "validation"].contains(&f.split.as_str())
            || !["real", "derived", "synthetic"].contains(&f.kind.as_str())
        {
            return Err(format!("invalid split/kind: {}", f.id));
        }
        for key in [
            format!("group:{}", f.group),
            format!("source:{}", f.source),
            format!("sequence:{}", f.sequence),
        ] {
            if splits
                .insert(key, f.split.clone())
                .is_some_and(|s| s != f.split)
            {
                return Err("source/sequence/group crosses tuning-validation boundary".into());
            }
        }
        if !orders.insert((&f.sequence, f.order)) {
            return Err("duplicate order in sequence".into());
        }
        if (f.hud.counts.is_some() || f.hud.remaining.is_some()) && f.hud.source.trim().is_empty() {
            return Err("HUD values require independent provenance".into());
        }
        if f.hud.counts.as_ref().is_some_and(|v| v.len() != 3) {
            return Err("expected three card counts".into());
        }
        if f.truth.iter().any(|t| {
            t.anchor >= 45
                || t.item_index >= 3
                || t.rect.width == 0
                || t.rect.height == 0
                || t.rect.x + t.rect.width > 9
                || t.rect.y + t.rect.height > 5
                || t.source.is_empty()
        }) {
            return Err("invalid truth annotation".into());
        }
    }
    Ok(())
}
#[derive(Default, Serialize, Clone)]
struct Metrics {
    frames: usize,
    observations: usize,
    truth_observations: usize,
    correct_candidate_retained: usize,
    false_unique: usize,
    category_certain: usize,
    category_correct: usize,
    occupancy_certain: usize,
    occupancy_correct: usize,
    direction_certain: usize,
    direction_evaluable: usize,
    direction_correct: usize,
    ambiguous: usize,
    negative_checks: usize,
    negative_false_unique: usize,
    incomplete_frames: usize,
    extraction_ms: f64,
    matching_ms: f64,
    constraints_ms: f64,
}
impl Metrics {
    fn add(&mut self, o: &Self) {
        self.frames += o.frames;
        self.observations += o.observations;
        self.truth_observations += o.truth_observations;
        self.correct_candidate_retained += o.correct_candidate_retained;
        self.false_unique += o.false_unique;
        self.category_certain += o.category_certain;
        self.category_correct += o.category_correct;
        self.occupancy_certain += o.occupancy_certain;
        self.occupancy_correct += o.occupancy_correct;
        self.direction_certain += o.direction_certain;
        self.direction_evaluable += o.direction_evaluable;
        self.direction_correct += o.direction_correct;
        self.ambiguous += o.ambiguous;
        self.negative_checks += o.negative_checks;
        self.negative_false_unique += o.negative_false_unique;
        self.incomplete_frames += o.incomplete_frames;
        self.extraction_ms += o.extraction_ms;
        self.matching_ms += o.matching_ms;
        self.constraints_ms += o.constraints_ms;
    }
}
fn stage(
    f: &Frame,
    a: &vision::Analysis,
    m: &experiment::Output,
    cfg: &Config,
    stage: usize,
    apply_constraints: bool,
) -> (Value, Metrics) {
    let specs = stage_specs(cfg);
    let spec = &specs[stage];
    let checks: Vec<_> = m
        .candidates
        .iter()
        .map(|p| candidate_check(p, spec))
        .collect();
    let mut metrics = Metrics {
        frames: 1,
        extraction_ms: m.extraction_ms,
        matching_ms: m.matching_ms,
        ..Metrics::default()
    };
    let input = constraints::ConstraintInput {
        width: 9,
        height: 5,
        shapes: a.shapes.clone(),
        counts: f.hud.counts.clone().unwrap_or_default(),
        empty_cells: a
            .cells
            .iter()
            .enumerate()
            .filter_map(|(i, s)| (s == "empty").then_some(i))
            .collect(),
        completed_cells: a
            .cells
            .iter()
            .enumerate()
            .filter_map(|(i, s)| (s == "completed").then_some(i))
            .collect(),
        observations: m
            .observations
            .iter()
            .map(|o| constraints::Observation {
                id: o.id.clone(),
                candidates: m
                    .candidates
                    .iter()
                    .enumerate()
                    .filter(|(i, p)| p.anchor == o.anchor && checks[*i]["accepted"] == true)
                    .map(|(_, p)| geometry(p))
                    .collect(),
            })
            .collect(),
    };
    let report = if apply_constraints && f.hud.counts.is_some() && !m.observations.is_empty() {
        Some(constraints::check_candidates(&input, &cfg.constraints))
    } else {
        None
    };
    metrics.constraints_ms = report.as_ref().map_or(0.0, |r| r.stats.elapsed_ms);
    let solver_complete = !apply_constraints
        || m.observations.is_empty()
        || report.as_ref().is_some_and(|r| r.complete);
    metrics.incomplete_frames = usize::from(!m.complete || !solver_complete);
    // Missing references are only harmless for explicitly exhausted Finish cards.
    let ref_ready = a.reference_ready.iter().enumerate().all(|(item, ready)| {
        *ready
            || (apply_constraints
                && solver_complete
                && a.finish.get(item) == Some(&true)
                && f.hud.counts.as_ref().and_then(|counts| counts.get(item)) == Some(&0))
    });
    let baseline_retained: Vec<Vec<_>> = m
        .observations
        .iter()
        .map(|o| {
            m.candidates
                .iter()
                .enumerate()
                .filter(|(i, p)| {
                    p.anchor == o.anchor
                        && checks[*i]["accepted"] == true
                        && !report.as_ref().is_some_and(|r| {
                            r.observations
                                .iter()
                                .filter(|r| r.observation_id == o.id)
                                .flat_map(|r| &r.candidates)
                                .any(|c| {
                                    c.geometry == geometry(p)
                                        && c.status == constraints::Status::Infeasible
                                })
                        })
                })
                .collect()
        })
        .collect();
    let competition = cfg
        .scoring
        .geometry_competition
        .as_ref()
        .filter(|_| spec.visible_foreground && apply_constraints)
        .map(|config| {
            let observations: Vec<_> = m
                .observations
                .iter()
                .zip(&baseline_retained)
                .map(|(o, retained)| competition::Observation {
                    id: o.id.clone(),
                    eligible: m.complete && solver_complete && o.sufficient_evidence && ref_ready,
                    candidates: retained
                        .iter()
                        .map(|(i, p)| competition::ScoredCandidate {
                            index: *i,
                            geometry: geometry(p),
                            score: checks[*i]["score"].as_f64().unwrap(),
                        })
                        .collect(),
                })
                .collect();
            competition::select(&input, &observations, config, &cfg.constraints)
        });
    metrics.constraints_ms += competition
        .as_ref()
        .and_then(|r| r.constraints.as_ref())
        .map_or(0.0, |r| r.stats.elapsed_ms);
    let mut observed = Vec::new();
    for (o, baseline) in m.observations.iter().zip(&baseline_retained) {
        let ranking = competition
            .as_ref()
            .and_then(|r| r.observations.iter().find(|row| row.id == o.id));
        let retained: Vec<_> = baseline
            .iter()
            .copied()
            .filter(|(i, _)| ranking.is_none_or(|r| r.retained_indices.contains(i)))
            .collect();
        let types: BTreeSet<_> = retained.iter().map(|(_, p)| p.item_index).collect();
        let rects: BTreeSet<_> = retained
            .iter()
            .map(|(_, p)| (p.rect.x, p.rect.y, p.rect.width, p.rect.height))
            .collect();
        let allowed = m.complete
            && solver_complete
            && o.sufficient_evidence
            && ref_ready
            && !retained.is_empty();
        let category = allowed && types.len() == 1;
        let occupancy = allowed && rects.len() == 1;
        let angular_span = retained.first().map(|(_, first)| {
            retained
                .iter()
                .map(|(_, p)| ((p.angle - first.angle + 180.0).rem_euclid(360.0) - 180.0).abs())
                .fold(0.0, f64::max)
        });
        let direction = category
            && occupancy
            && angular_span.is_some_and(|s| s <= cfg.direction_tolerance_degrees);
        let ambiguous = !(category && occupancy && direction);
        let truth = f.truth.iter().find(|t| t.anchor == o.anchor);
        let correct = truth.map(|t| {
            retained
                .iter()
                .any(|(_, p)| p.item_index == t.item_index && p.rect == t.rect)
        });
        let category_correct = truth.is_some_and(|t| category && types.contains(&t.item_index));
        let occupancy_correct = truth.is_some_and(|t| {
            occupancy && rects.contains(&(t.rect.x, t.rect.y, t.rect.width, t.rect.height))
        });
        let direction_correct = truth.and_then(|t| t.angle).is_some_and(|angle| {
            direction
                && retained.iter().all(|(_, p)| {
                    ((p.angle - angle + 180.0).rem_euclid(360.0) - 180.0).abs()
                        <= cfg.direction_tolerance_degrees
                })
        });
        let negative = f.must_remain_ambiguous.contains(&o.anchor);
        let negative_false_unique = negative && (category || occupancy || direction);
        let false_unique = negative_false_unique
            || (truth.is_some()
                && ((category && !category_correct)
                    || (occupancy && !occupancy_correct)
                    || (direction && truth.and_then(|t| t.angle).is_some() && !direction_correct)));
        metrics.observations += 1;
        metrics.truth_observations += usize::from(truth.is_some());
        metrics.correct_candidate_retained += usize::from(correct == Some(true));
        metrics.false_unique += usize::from(false_unique);
        metrics.category_certain += usize::from(category);
        metrics.category_correct += usize::from(category_correct);
        metrics.occupancy_certain += usize::from(occupancy);
        metrics.occupancy_correct += usize::from(occupancy_correct);
        metrics.direction_certain += usize::from(direction);
        metrics.direction_evaluable += usize::from(truth.and_then(|t| t.angle).is_some());
        metrics.direction_correct += usize::from(direction_correct);
        metrics.ambiguous += usize::from(ambiguous);
        metrics.negative_checks += usize::from(negative);
        metrics.negative_false_unique +=
            usize::from(negative && (category || occupancy || direction));
        let margin_applied =
            competition.as_ref().is_some_and(|r| r.applied) && ranking.is_some_and(|r| r.proposed);
        observed.push(json!({"id":o.id,"anchor":o.anchor,"retained_indices":retained.iter().map(|(i,_)|*i).collect::<Vec<_>>(),"category_certain":category,"occupancy_certain":occupancy,"direction_certain":direction,"ambiguous":ambiguous,"correct_candidate_retained":correct,"false_unique":false_unique,"negative_false_unique":negative_false_unique,"angle_span_from_first":angular_span,"competition":ranking,"decision_basis":if margin_applied {"score_margin_and_joint_feasibility"}else {"threshold_and_constraints"},"reason":if !m.complete {"search interrupted"}else if !solver_complete {"global feasibility unresolved"}else if !o.sufficient_evidence {"insufficient foreground texture"}else if !ref_ready {"some reference identities unavailable"}else if retained.is_empty(){"no retained candidate; recognition failure, not uniqueness"}else if margin_applied {"score-margin preference; jointly feasible, not a proof of uniqueness"}else {"conditional on configured finite search and observed references"}}));
    }
    // Truth remains evaluator-only. Missed observations count as failed retention.
    for t in &f.truth {
        if !m.observations.iter().any(|o| o.anchor == t.anchor) {
            metrics.truth_observations += 1;
        }
    }
    for anchor in &f.must_remain_ambiguous {
        if !m.observations.iter().any(|o| o.anchor == *anchor) {
            metrics.negative_checks += 1;
        }
    }
    let name = spec.name.clone() + if apply_constraints { "_global" } else { "" };
    let formula = if !spec.visible_foreground {
        "legacy_score <= final_threshold"
    } else if cfg.matching.confidence_weighted_scoring {
        "max(confidence_weighted_core_rgb + legacy_edge_and_reverse_penalties, observed_foreground_mae_with_missing_255)"
    } else {
        "max(legacy_bidirectional, observed_foreground_mae_with_missing_255)"
    };
    (
        json!({"name":name,"observations":observed,"constraints":report,"competition":competition,"constraint_input":if apply_constraints{Some(input)}else{None},"metrics":metrics,
            "scoring":{"kind":if spec.visible_foreground {"visible_foreground"} else {"legacy"},"formula":formula,"threshold":spec.threshold,"legacy_score_index":spec.legacy_index},"candidate_checks":checks}),
        metrics,
    )
}
fn matcher_evidence(m: &experiment::Output) -> Value {
    let mut value = serde_json::to_value(m).unwrap();
    for key in [
        "backend",
        "tile_cache",
        "extraction_ms",
        "matching_ms",
        "asset_export_ms",
    ] {
        value.as_object_mut().unwrap().remove(key);
    }
    for candidate in value["candidates"].as_array_mut().unwrap() {
        candidate.as_object_mut().unwrap().remove("tile_template");
    }
    value
}

fn pose_key(p: &Value) -> String {
    serde_json::to_string(&json!([
        p["observation_id"],
        p["anchor"],
        p["reference_id"],
        p["item_index"],
        p["rect"],
        p["angle"],
        p["fill"],
        p["scale"],
        p["offset"]
    ]))
    .unwrap()
}

// Count multisets as well as ordered evidence: duplicated or reordered poses must
// never make the comparison pass accidentally or corrupt retained_indices.
fn compare_matchers(direct: &Value, tiled: &Value) -> Value {
    let mut groups = BTreeMap::<String, (Vec<&Value>, Vec<&Value>)>::new();
    for p in direct["candidates"].as_array().unwrap() {
        groups.entry(pose_key(p)).or_default().0.push(p);
    }
    for p in tiled["candidates"].as_array().unwrap() {
        groups.entry(pose_key(p)).or_default().1.push(p);
    }
    let mut missing = 0;
    let mut extra = 0;
    let mut max_score_delta = 0.0_f64;
    let mut details = Vec::new();
    for (key, (a, b)) in groups {
        missing += a.len().saturating_sub(b.len());
        extra += b.len().saturating_sub(a.len());
        for (left, right) in a.iter().zip(&b) {
            for field in [
                "effective_evidence",
                "color_error",
                "edge_error",
                "predicted_mismatch",
                "unexplained_foreground",
            ] {
                if let (Some(x), Some(y)) = (left[field].as_f64(), right[field].as_f64()) {
                    max_score_delta = max_score_delta.max((x - y).abs());
                }
            }
            for (x, y) in left["scores"]
                .as_array()
                .unwrap()
                .iter()
                .zip(right["scores"].as_array().unwrap())
            {
                max_score_delta =
                    max_score_delta.max((x.as_f64().unwrap() - y.as_f64().unwrap()).abs());
            }
            for field in [
                "foreground_color_error",
                "uncapped_foreground_color_error",
                "missing_foreground_fraction",
                "contradicted_foreground_fraction",
            ] {
                if let (Some(x), Some(y)) = (
                    left["visible_evidence"][field].as_f64(),
                    right["visible_evidence"][field].as_f64(),
                ) {
                    max_score_delta = max_score_delta.max((x - y).abs());
                }
            }
        }
        if a != b {
            details.push(json!({"pose":key,"direct":a,"tiled":b}));
        }
    }
    json!({"equivalent":direct==tiled,"candidate_count_direct":direct["candidates"].as_array().unwrap().len(),
        "candidate_count_tiled":tiled["candidates"].as_array().unwrap().len(),"missing_candidates":missing,
        "extra_candidates":extra,"max_score_delta":max_score_delta,"details":details,
        "observations_equal":direct["observations"]==tiled["observations"],
        "cards_equal":direct["cards"]==tiled["cards"],
        "search_equal":direct["complete"]==tiled["complete"] && direct["interruption"]==tiled["interruption"] && direct["evaluations"]==tiled["evaluations"]})
}

fn decision_evidence(stages: &[Value]) -> Value {
    let mut value = json!(stages);
    for stage in value.as_array_mut().unwrap() {
        for key in ["extraction_ms", "matching_ms", "constraints_ms"] {
            stage["metrics"].as_object_mut().unwrap().remove(key);
        }
        if let Some(stats) = stage["constraints"]["stats"].as_object_mut() {
            stats.remove("elapsed_ms");
        }
        if let Some(stats) = stage["competition"]["constraints"]["stats"].as_object_mut() {
            stats.remove("elapsed_ms");
        }
    }
    value
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help") {
        println!("partial-recognition --manifest FILE --config FILE --output DIR --split tuning|validation|all [--freeze-record FILE]\nPaths in the manifest resolve from its directory. Validation requires a matching freeze record, created by run.ps1 after tuning. No game/window access.");
        return Ok(());
    }
    if args.len() % 2 != 0 {
        return Err("options require values; see --help".into());
    }
    let mut options = HashMap::new();
    for p in args.chunks(2) {
        if ![
            "--manifest",
            "--config",
            "--output",
            "--split",
            "--freeze-record",
        ]
        .contains(&p[0].as_str())
            || options.insert(p[0].clone(), p[1].clone()).is_some()
        {
            return Err("unknown or duplicate option".into());
        }
    }
    let get = |k: &str| {
        options
            .get(k)
            .map(PathBuf::from)
            .ok_or_else(|| format!("missing {k}"))
    };
    let manifest_path = get("--manifest")?;
    let config_path = get("--config")?;
    let out = get("--output")?;
    let split = options.get("--split").map(String::as_str).unwrap_or("all");
    if !["tuning", "validation", "all"].contains(&split) {
        return Err("invalid split".into());
    }
    let m: Manifest = read_json(&manifest_path)?;
    validate_manifest(&m)?;
    let cfg: Config = read_json(&config_path)?;
    cfg.matching.validate()?;
    cfg.scoring.validate()?;
    if cfg.schema != 1
        || !cfg.direction_tolerance_degrees.is_finite()
        || cfg.direction_tolerance_degrees < 0.0
    {
        return Err("invalid config schema/direction tolerance".into());
    }
    let config_bytes = fs::read(&config_path).map_err(|e| e.to_string())?;
    let digest = fnv(&config_bytes);
    let freeze = if split != "tuning" {
        let record: Value = read_json(&get("--freeze-record")?)?;
        if record["config_fnv1a64"] != digest {
            return Err("config differs from frozen tuning configuration".into());
        }
        Some(record)
    } else {
        None
    };
    if out.join("results.json").exists() {
        return Err("output already contains results.json; choose a new run directory".into());
    }
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    fs::write(out.join("config.json"), &config_bytes).map_err(|e| e.to_string())?;
    write_json(&out.join("manifest.json"), &m)?;
    let mut rows: BTreeMap<String, Metrics> = BTreeMap::new();
    let mut results = Vec::new();
    let mut sequences: BTreeMap<String, Vec<&Frame>> = BTreeMap::new();
    for f in &m.frames {
        if split == "all" || f.split == split {
            sequences.entry(f.sequence.clone()).or_default().push(f);
        }
    }
    for frames in sequences.values_mut() {
        frames.sort_by_key(|f| f.order);
        let mut prod = vision::Recognizer::new();
        let mut legacy = vision::Recognizer::new();
        let mut session = experiment::Session::default();
        let mut direct_session = experiment::Session::default();
        let mut round = None;
        for &f in frames.iter() {
            if round.as_ref() != Some(&f.round) {
                prod.reset();
                legacy.reset();
                session.reset();
                direct_session.reset();
                round = Some(f.round.clone());
            }
            let source = manifest_path
                .parent()
                .unwrap_or(Path::new("."))
                .join(&f.source);
            let bytes = fs::read(&source).map_err(|e| format!("{}: {e}", source.display()))?;
            let mut frame = image::load_from_memory(&bytes)
                .map_err(|e| e.to_string())?
                .to_rgba8();
            if let Some(transform) = &f.transform {
                frame = experiment::derive(&frame, transform)?;
            }
            let folder = out.join(&f.id);
            fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
            frame
                .save(folder.join("frame.png"))
                .map_err(|e| e.to_string())?;
            let counts =
                std::array::from_fn(|i| f.hud.counts.as_ref().and_then(|v| v.get(i).copied()));
            let start = Instant::now();
            let a = prod.analyze_completed_snapshot(&frame, None, f.hud.remaining, counts);
            let production_ms = start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            let b = legacy.analyze_snapshot(&frame, None, f.hud.remaining, counts);
            let legacy_ms = start.elapsed().as_secs_f64() * 1000.0;
            let mut record = serde_json::to_value(f).map_err(|e| e.to_string())?;
            record["input_fnv1a64"] = json!(fnv(&bytes));
            record["production"] = analysis(&a);
            record["legacy"] = analysis(&b);
            record["production_ms"] = json!(production_ms);
            record["legacy_ms"] = json!(legacy_ms);
            if a.present {
                let direct_folder = folder.join("direct");
                fs::create_dir_all(&direct_folder).map_err(|e| e.to_string())?;
                // Alternate backend order without sharing templates or cache state.
                let direct_first = results.len() % 2 == 0;
                let (direct, matched) = if direct_first {
                    let direct = direct_session.run_backend(
                        &frame,
                        &a,
                        &f.id,
                        &cfg.matching,
                        &direct_folder,
                        experiment::Backend::Direct,
                    )?;
                    let tiled = session.run_backend(
                        &frame,
                        &a,
                        &f.id,
                        &cfg.matching,
                        &folder,
                        experiment::Backend::Tiled,
                    )?;
                    (direct, tiled)
                } else {
                    let tiled = session.run_backend(
                        &frame,
                        &a,
                        &f.id,
                        &cfg.matching,
                        &folder,
                        experiment::Backend::Tiled,
                    )?;
                    let direct = direct_session.run_backend(
                        &frame,
                        &a,
                        &f.id,
                        &cfg.matching,
                        &direct_folder,
                        experiment::Backend::Direct,
                    )?;
                    (direct, tiled)
                };
                let mut comparison =
                    compare_matchers(&matcher_evidence(&direct), &matcher_evidence(&matched));
                let equivalent = comparison["equivalent"] == true;
                let mut stages = Vec::new();
                let mut direct_stages = Vec::new();
                // Threshold-free replay exports score evidence for evaluate-nearest.mjs.
                // Existing global stages filter by thresholds and cannot validate raw argmin.
                let stage_count = if cfg.matching.threshold_free_search {
                    0
                } else {
                    stage_specs(&cfg).len()
                };
                for stage_index in 0..stage_count {
                    for global in [false, true] {
                        let (result, metrics) = stage(f, &a, &matched, &cfg, stage_index, global);
                        let key = format!(
                            "{} / {} / {}",
                            f.split,
                            f.kind,
                            result["name"].as_str().unwrap()
                        );
                        rows.entry(key).or_default().add(&metrics);
                        // Identical matcher evidence gives identical solver inputs.
                        // Reuse that evidence, with reuse explicitly recorded.
                        let direct_result = if equivalent {
                            let mut reused = result.clone();
                            reused["metrics"]["extraction_ms"] = json!(direct.extraction_ms);
                            reused["metrics"]["matching_ms"] = json!(direct.matching_ms);
                            reused
                        } else {
                            stage(f, &a, &direct, &cfg, stage_index, global).0
                        };
                        direct_stages.push(direct_result);
                        stages.push(result);
                    }
                }
                eprintln!(
                    "{}: {} observations, {} poses, complete={}, direct {:.0} / tiled {:.0} ms, parity={}",
                    f.id,
                    matched.observations.len(),
                    matched.candidates.len(),
                    matched.complete,
                    direct.matching_ms,
                    matched.matching_ms,
                    equivalent
                );
                comparison["decisions_equal"] =
                    json!(decision_evidence(&direct_stages) == decision_evidence(&stages));
                comparison["threshold_decisions_compared"] =
                    json!(!cfg.matching.threshold_free_search);
                comparison["solver_evidence_reused"] =
                    json!(equivalent && !cfg.matching.threshold_free_search);
                comparison["execution_order"] = json!(if direct_first {
                    ["direct", "tiled"]
                } else {
                    ["tiled", "direct"]
                });
                comparison["direct_matching_ms"] = json!(direct.matching_ms);
                comparison["tiled_matching_ms"] = json!(matched.matching_ms);
                comparison["speedup"] = if matched.matching_ms > 0.0 {
                    json!(direct.matching_ms / matched.matching_ms)
                } else {
                    Value::Null
                };
                record["tile_comparison"] = comparison;
                record["matching_direct"] =
                    serde_json::to_value(&direct).map_err(|e| e.to_string())?;
                record["stages_direct"] = json!(direct_stages);
                record["matching"] = serde_json::to_value(&matched).map_err(|e| e.to_string())?;
                for (index, pose) in matched.candidates.iter().enumerate() {
                    let mut statuses = serde_json::Map::new();
                    for s in &stages {
                        let name = s["name"].as_str().unwrap();
                        if !name.ends_with("_global") {
                            continue;
                        }
                        let observed =
                            s["constraints"]["observations"]
                                .as_array()
                                .and_then(|rows| {
                                    rows.iter()
                                        .find(|r| r["observation_id"] == pose.observation_id)
                                });
                        let found =
                            observed
                                .and_then(|r| r["candidates"].as_array())
                                .and_then(|cs| {
                                    cs.iter().find(|c| c["geometry"] == json!(geometry(pose)))
                                });
                        statuses.insert(name.into(),found.cloned().unwrap_or_else(||json!({"status":"not_checked","reason":"not retained by this visual score, or missing HUD/observation evidence"})));
                    }
                    record["matching"]["candidates"][index]["global_feasibility"] =
                        Value::Object(statuses);
                }
                record["stages"] = json!(stages);
            } else {
                record["error"] = json!("production board not present; no experimental claims");
                eprintln!("{}: board absent", f.id);
            }
            write_json(&folder.join("result.json"), &record)?;
            results.push(record);
        }
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    };
    let diff = git(&["diff", "--binary"]);
    fs::write(out.join("working-tree.patch"), &diff).map_err(|e| e.to_string())?;
    let summary: Vec<_> = rows
        .iter()
        .map(|(key, v)| {
            let mut row = serde_json::to_value(v).unwrap();
            let keys: Vec<_> = key.split(" / ").collect();
            row["split"] = json!(keys[0]);
            row["kind"] = json!(keys[1]);
            row["stage"] = json!(keys[2]);
            row["correct_candidate_retention_rate"] = if v.truth_observations > 0 {
                json!(v.correct_candidate_retained as f64 / v.truth_observations as f64)
            } else {
                Value::Null
            };
            row["ambiguity_rate"] = if v.observations > 0 {
                json!(v.ambiguous as f64 / v.observations as f64)
            } else {
                Value::Null
            };
            row
        })
        .collect();
    let report = json!({"metadata":{"schema":1,"created_unix_seconds":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),"git_head":git(&["rev-parse","HEAD"]).trim(),"git_status":git(&["status","--short"]),"working_diff_fnv1a64":fnv(diff.as_bytes()),"config_fnv1a64":digest,"freeze":freeze,"manifest_notes":m.notes,"source_count":results.iter().filter_map(|f|f["source"].as_str()).collect::<BTreeSet<_>>().len(),"finite_search_only":true,"production_enablement":false,"notes":["Legacy evaluator thresholds remain production constants, documented per frame.","Ground truth is read only by evaluation, never matching, caches, or solver input.","Derived/synthetic evidence is not a new real capture; angles without truth are N/A.","Search completion is completion of the configured coarse/refine grid, not mathematical exhaustiveness over continuous poses."]},"config":cfg,"summary":summary,"frames":results});
    write_json(&out.join("results.json"), &report)?;
    if split == "tuning" {
        write_json(
            &out.join("freeze.json"),
            &json!({"config_fnv1a64":digest,"frozen_unix_seconds":SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),"tuning_results_fnv1a64":fnv(&fs::read(out.join("results.json")).map_err(|e|e.to_string())?),"policy":"Configuration fixed after exploratory run and before validation; no true partial holdout is available"}),
        )?;
    }
    let encoded = serde_json::to_string(&report)
        .map_err(|e| e.to_string())?
        .replace('<', "\\u003c");
    fs::write(
        out.join("index.html"),
        include_str!("partial_recognition/report.html").replace("__REPORT_JSON__", &encoded),
    )
    .map_err(|e| e.to_string())?;
    println!("{}", out.join("index.html").display());
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("partial-recognition: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tile_comparison_detects_score_search_and_multiplicity_changes() {
        let p = json!({"observation_id":"x","anchor":0,"reference_id":"r","item_index":0,
            "rect":{"x":0,"y":0,"width":2,"height":3},"angle":0.0,"fill":0.92,
            "scale":1.0,"offset":[0.0,0.0],"scores":[1.0,2.0,3.0],"accepted":[true,true,true]});
        let evidence = json!({"cards":[],"observations":[],"candidates":[p],"complete":true,"interruption":null,"evaluations":7});
        assert_eq!(compare_matchers(&evidence, &evidence)["equivalent"], true);
        let mut changed = evidence.clone();
        changed["candidates"][0]["scores"][2] = json!(3.25);
        let comparison = compare_matchers(&evidence, &changed);
        assert_eq!(comparison["equivalent"], false);
        assert_eq!(comparison["max_score_delta"], 0.25);
        changed = evidence.clone();
        changed["candidates"].as_array_mut().unwrap().push(p);
        assert_eq!(compare_matchers(&evidence, &changed)["extra_candidates"], 1);
        assert_eq!(
            compare_matchers(&changed, &evidence)["missing_candidates"],
            1
        );
        changed = evidence.clone();
        changed["complete"] = json!(false);
        assert_eq!(compare_matchers(&evidence, &changed)["search_equal"], false);
    }
    #[test]
    fn split_validation_rejects_derived_leakage() {
        let mut m: Manifest =
            serde_json::from_str(include_str!("partial_recognition/manifest.json")).unwrap();
        validate_manifest(&m).unwrap();
        let mut duplicate = m.frames[0].clone();
        duplicate.id = "leak".into();
        duplicate.order += 100;
        duplicate.split = if duplicate.split == "tuning" {
            "validation"
        } else {
            "tuning"
        }
        .into();
        m.frames.push(duplicate);
        assert!(validate_manifest(&m).is_err());
    }
    #[test]
    fn frozen_config_and_manifest_are_valid() {
        let cfg: Config =
            serde_json::from_str(include_str!("partial_recognition/config.json")).unwrap();
        cfg.matching.validate().unwrap();
        cfg.scoring.validate().unwrap();
    }

    #[test]
    fn exhausted_finish_reference_allows_only_globally_checked_confirmation() {
        let cfg: Config =
            serde_json::from_str(include_str!("partial_recognition/config.json")).unwrap();
        let manifest: Manifest =
            serde_json::from_str(include_str!("partial_recognition/manifest.json")).unwrap();
        let mut f = manifest
            .frames
            .iter()
            .find(|f| f.id == "r1-finish-cold")
            .unwrap()
            .clone();
        let frame = image::load_from_memory(include_bytes!(
            "../tests/fixtures/vision-waterguns-finished.png"
        ))
        .unwrap()
        .to_rgba8();
        let mut a = vision::Recognizer::new().analyze_completed_snapshot(
            &frame,
            None,
            f.hud.remaining,
            std::array::from_fn(|i| {
                f.hud
                    .counts
                    .as_ref()
                    .and_then(|counts| counts.get(i).copied())
            }),
        );
        assert_eq!(a.reference_ready, [false, true, true]);
        assert_eq!(a.finish, [true, false, false]);
        assert_eq!(f.hud.counts.as_ref().unwrap()[0], 0);
        let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.artifacts/partial-recognition/reference-readiness-test");
        fs::create_dir_all(&folder).unwrap();
        let mut m = experiment::Session::default()
            .run(&frame, &a, &f.id, &cfg.matching, &folder)
            .unwrap();
        let before = matcher_evidence(&m);
        let index = stage_specs(&cfg)
            .iter()
            .position(|s| s.name == "visible_foreground")
            .unwrap();
        let check = |result: &Value, expected: bool| {
            let observations = result["observations"].as_array().unwrap();
            assert_eq!(observations.len(), 3);
            for o in observations {
                assert_eq!(o["category_certain"], expected);
                assert_eq!(o["occupancy_certain"], expected);
                assert_eq!(o["false_unique"], false);
            }
        };
        let visual = stage(&f, &a, &m, &cfg, index, false).0;
        check(&visual, false);
        let global = stage(&f, &a, &m, &cfg, index, true).0;
        check(&global, true);
        assert_eq!(global["constraints"]["complete"], true);
        for o in global["observations"].as_array().unwrap() {
            assert_eq!(o["correct_candidate_retained"], true);
            assert_eq!(
                o["direction_certain"], false,
                "angle ambiguity must survive"
            );
        }
        // A present reference yields the same candidates and geometric result.
        a.reference_ready[0] = true;
        let with_reference = stage(&f, &a, &m, &cfg, index, true).0;
        assert_eq!(global["observations"], with_reference["observations"]);
        assert_eq!(
            global["candidate_checks"],
            with_reference["candidate_checks"]
        );
        assert_eq!(
            global["constraint_input"],
            with_reference["constraint_input"]
        );
        a.reference_ready[0] = false;
        a.finish[0] = false;
        check(&stage(&f, &a, &m, &cfg, index, true).0, false);
        a.finish[0] = true;
        f.hud.counts.as_mut().unwrap()[0] = 1;
        check(&stage(&f, &a, &m, &cfg, index, true).0, false);
        f.hud.counts = None;
        check(&stage(&f, &a, &m, &cfg, index, true).0, false);
        assert_eq!(
            matcher_evidence(&m),
            before,
            "confirmation cannot delete competitors"
        );
        f.hud.counts = Some(vec![0, 4, 1]);
        m.complete = false;
        check(&stage(&f, &a, &m, &cfg, index, true).0, false);
    }

    #[test]
    fn final_threshold_preserves_coarse_seed_for_watergun_refinement() {
        let cfg: Config =
            serde_json::from_str(include_str!("partial_recognition/config.json")).unwrap();
        let manifest: Manifest =
            serde_json::from_str(include_str!("partial_recognition/manifest.json")).unwrap();
        let f = manifest
            .frames
            .iter()
            .find(|f| f.id == "r1-partial-watergun")
            .unwrap();
        let frame = image::load_from_memory(include_bytes!(
            "../tests/fixtures/vision-partial-watergun.png"
        ))
        .unwrap()
        .to_rgba8();
        let a = vision::Recognizer::new().analyze_completed_snapshot(
            &frame,
            None,
            Some(41),
            [Some(2), Some(5), Some(2)],
        );
        let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.artifacts/partial-recognition/scoring-engineering/watergun-test");
        fs::create_dir_all(&folder).unwrap();
        let m = experiment::Session::default()
            .run(&frame, &a, &f.id, &cfg.matching, &folder)
            .unwrap();
        assert!(m.complete);
        let truth = &f.truth[0];
        // A real coarse seed lies above final 21, yet its refined pose is valid.
        assert!(m.candidates.iter().any(|p| p.rect == truth.rect
            && p.angle == 60.0
            && p.offset == [0.0, 0.0]
            && p.color_error > 21.0));
        let index = stage_specs(&cfg)
            .iter()
            .position(|s| s.name == "bidirectional_threshold_21")
            .unwrap();
        let (result, _) = stage(f, &a, &m, &cfg, index, false);
        assert_eq!(
            result["observations"][0]["correct_candidate_retained"],
            true
        );
        assert!(m
            .candidates
            .iter()
            .any(|p| p.rect == truth.rect && p.scores[2] < 21.0));
    }

    #[test]
    fn repaired_reference_retains_only_through_finish_and_reset_clears_it() {
        let mut cfg: Config =
            serde_json::from_str(include_str!("partial_recognition/config.json")).unwrap();
        cfg.matching.extraction.enabled = true;
        cfg.matching.max_evaluations = 1;
        let original =
            image::load_from_memory(include_bytes!("../tests/fixtures/vision-other-items.png"))
                .unwrap()
                .to_rgba8();
        let polluted = experiment::derive(&original, "background-pollution").unwrap();
        let finish = image::load_from_memory(include_bytes!(
            "../tests/fixtures/vision-waterguns-finished.png"
        ))
        .unwrap()
        .to_rgba8();
        let mut session = experiment::Session::default();
        let out = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.artifacts/partial-recognition/extraction-engineering/cache-test");
        let mut saved = None;
        for (id, frame, reset) in [
            ("polluted", &polluted, false),
            ("finish", &finish, false),
            ("reset-finish", &finish, true),
        ] {
            if reset {
                session.reset();
            }
            let analysis =
                vision::Recognizer::new().analyze_completed_snapshot(frame, None, None, [None; 3]);
            let folder = out.join(id);
            fs::create_dir_all(&folder).unwrap();
            let matched = session
                .run(frame, &analysis, id, &cfg.matching, &folder)
                .unwrap();
            let card = &matched.cards[0];
            if id == "polluted" {
                assert_eq!(card.extraction.as_ref().unwrap().status, "cleaned");
                assert!(!card.touches_crop);
                saved = Some((card.fingerprint.clone(), card.mask_pixels));
            } else if !reset {
                assert!(card.finished);
                assert!(card.extraction.as_ref().unwrap().reused_on_finish);
                assert_eq!(card.reference_source.as_deref(), Some("polluted"));
                assert_eq!(
                    (card.fingerprint.clone(), card.mask_pixels),
                    saved.clone().unwrap()
                );
            } else {
                assert!(card.finished);
                assert!(card.fingerprint.is_none());
                assert!(card.reference_source.is_none());
                assert!(card.extraction.is_none());
            }
        }
    }

    #[test]
    fn interrupted_matching_cannot_claim_certainty_or_change_observations() {
        let mut cfg: Config =
            serde_json::from_str(include_str!("partial_recognition/config.json")).unwrap();
        cfg.matching.max_evaluations = 1;
        let manifest: Manifest =
            serde_json::from_str(include_str!("partial_recognition/manifest.json")).unwrap();
        let f = manifest
            .frames
            .iter()
            .find(|f| f.id == "r1-partial-watergun")
            .unwrap();
        let frame = image::load_from_memory(include_bytes!(
            "../tests/fixtures/vision-partial-watergun.png"
        ))
        .unwrap()
        .to_rgba8();
        let a = vision::Recognizer::new().analyze_completed_snapshot(
            &frame,
            None,
            Some(41),
            [Some(2), Some(5), Some(2)],
        );
        let before = a.cells.clone();
        let out = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.artifacts/partial-recognition/entry-budget-test");
        fs::create_dir_all(&out).unwrap();
        let mut evidence = Vec::new();
        for backend in [experiment::Backend::Direct, experiment::Backend::Tiled] {
            let folder = out.join(format!("{backend:?}"));
            fs::create_dir_all(&folder).unwrap();
            let m = experiment::Session::default()
                .run_backend(&frame, &a, &f.id, &cfg.matching, &folder, backend)
                .unwrap();
            assert!(!m.complete);
            assert_eq!(m.evaluations, 1);
            for i in 0..stage_specs(&cfg).len() {
                let (result, _) = stage(f, &a, &m, &cfg, i, true);
                for o in result["observations"].as_array().unwrap() {
                    assert_eq!(o["category_certain"], false);
                    assert_eq!(o["occupancy_certain"], false);
                    assert_eq!(o["direction_certain"], false);
                }
            }
            evidence.push(matcher_evidence(&m));
        }
        assert_eq!(
            compare_matchers(&evidence[0], &evidence[1])["equivalent"],
            true
        );
        assert_eq!(a.cells, before);
    }
}
