#![cfg(feature = "partial-recognition")]

#[path = "../src/vision.rs"]
mod vision;

use image::RgbaImage;
use std::{path::PathBuf, time::{Duration, Instant}};
use vision::experiment::{Config, Observation, Output, Pose, Session};
use vision::partial::{decide_matches, Placement, Recognizer as PartialRecognizer};
use vision::{Analysis, GridPlacement, Recognizer};

fn fixture(name: &str) -> RgbaImage {
    image::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(format!("vision-{name}.png")),
    )
    .unwrap()
    .to_rgba8()
}

fn matching_config() -> Config {
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../examples/partial_recognition/config.json")).unwrap();
    serde_json::from_value(config["matching"].clone()).unwrap()
}

fn analyze(
    recognizer: &mut Recognizer,
    frame: &RgbaImage,
    remaining: u32,
    counts: [Option<u32>; 3],
) -> (Analysis, [f64; 4]) {
    let content = recognizer.locate_content(frame).unwrap();
    let analysis =
        recognizer.analyze_completed_in_content(frame, content, None, Some(remaining), counts);
    assert!(analysis.present, "{}", analysis.message);
    (analysis, content)
}

fn phone_evidence() -> (Analysis, Output) {
    let frame = fixture("user-20261006-phone-strap");
    let (analysis, content) = analyze(
        &mut Recognizer::new(),
        &frame,
        30,
        [Some(0), Some(5), Some(2)],
    );
    let output = Session::default()
        .run_live(
            &frame,
            &analysis,
            content,
            "phone",
            &matching_config(),
            None,
        )
        .unwrap();
    assert!(output.complete);
    assert_eq!(output.observations.len(), 1);
    assert_eq!(output.observations[0].anchor, 5);
    (analysis, output)
}

fn scored_pose(original: &Pose, anchor: usize, rect: GridPlacement, score: f64) -> Pose {
    let mut pose = original.clone();
    pose.anchor = anchor;
    pose.observation_id = format!("phone:cell-{anchor}");
    pose.item_index = 1;
    pose.rect = rect;
    pose.confidence_weighted
        .as_mut()
        .unwrap()
        .bidirectional_score = Some(score);
    pose.visible_evidence
        .as_mut()
        .unwrap()
        .foreground_color_error = Some(score);
    pose
}

fn observation(original: &Observation, anchor: usize) -> Observation {
    Observation {
        id: format!("phone:cell-{anchor}"),
        anchor,
        texture_std: original.texture_std,
        reliable_foreground_pixels: original.reliable_foreground_pixels,
        border_reliable_foreground_pixels: original.border_reliable_foreground_pixels,
        effective_foreground_pixels: original.effective_foreground_pixels,
        sufficient_evidence: original.sufficient_evidence,
    }
}

fn phone_rect(x: usize) -> GridPlacement {
    GridPlacement {
        x,
        y: 0,
        width: 3,
        height: 1,
    }
}

fn phone_placement(x: usize, y: usize, width: usize, height: usize) -> Placement {
    Placement {
        item_index: 1,
        x,
        y,
        width,
        height,
    }
}

#[test]
fn runtime_real_single_cell_phones_resolve_and_preserve_raw_observations() {
    // These are recorded tuning/regression fixtures, not independent holdout.
    for (name, remaining, counts, anchor, expected) in [
        (
            "user-20261006-phone-strap",
            30,
            [Some(0), Some(5), Some(2)],
            5,
            phone_placement(3, 0, 3, 1),
        ),
        (
            "user-20261006-phone-partial-finish",
            24,
            [Some(0), Some(3), Some(2)],
            32,
            phone_placement(5, 1, 1, 3),
        ),
    ] {
        let frame = fixture(name);
        let mut recognizer = Recognizer::new();
        let (analysis, content) = analyze(&mut recognizer, &frame, remaining, counts);
        assert_eq!(analysis.cells[anchor], "uncertain");
        assert!(analysis.candidate_constraints.is_empty());
        let original_cells = analysis.cells.clone();
        let original_completed = analysis.completed_objects.clone();
        let result = recognizer.resolve_partial(&frame, &analysis, Some(content), counts);
        assert!(result.complete, "{name}: {}", result.message);
        assert_eq!(result.placements, vec![expected]);
        assert_eq!(analysis.cells, original_cells);
        assert_eq!(analysis.completed_objects, original_completed);
        assert!(analysis.candidate_constraints.is_empty());
    }
}

#[test]
fn runtime_manual_item_type_remains_a_restriction() {
    let frame = fixture("user-20261006-phone-strap");
    let mut recognizer = Recognizer::new();
    let counts = [Some(0), Some(5), Some(2)];
    let (mut analysis, content) = analyze(&mut recognizer, &frame, 30, counts);
    analysis.cells[5] = "item1".into();
    let result = recognizer.resolve_partial(&frame, &analysis, Some(content), counts);
    assert!(result.complete, "{}", result.message);
    assert_eq!(result.placements, vec![phone_placement(3, 0, 3, 1)]);
    assert_eq!(analysis.cells[5], "item1");

    analysis.cells[5] = "item2".into();
    let result = recognizer.resolve_partial(&frame, &analysis, Some(content), counts);
    assert!(!result.complete);
    assert!(result.placements.is_empty());
    assert_eq!(analysis.cells[5], "item2");
}

#[test]
fn runtime_cache_reuses_complete_evidence_and_invalidates_type_and_failed_input() {
    let frame = fixture("user-20261006-phone-strap");
    let counts = [Some(0), Some(5), Some(2)];
    let (mut analysis, content) = analyze(&mut Recognizer::new(), &frame, 30, counts);
    let original_cells = analysis.cells.clone();
    let mut partial = PartialRecognizer::default();
    let started = Instant::now();
    let first = partial.resolve(&frame, &analysis, Some(content), counts);
    let first_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert!(first.complete, "{}", first.message);
    assert_eq!(first.placements, vec![phone_placement(3, 0, 3, 1)]);
    assert_eq!(partial.cache_hits(), 0);
    let started = Instant::now();
    let repeated = partial.resolve(&frame, &analysis, Some(content), counts);
    let repeated_ms = started.elapsed().as_secs_f64() * 1000.0;
    eprintln!("partial-cache fixture=user-20261006-phone-strap first_ms={first_ms:.2} repeated_ms={repeated_ms:.2}");
    assert_eq!(repeated, first);
    assert_eq!(partial.cache_hits(), 1);
    assert_eq!(analysis.cells, original_cells);

    analysis.cells[5] = "item1".into();
    let restricted = partial.resolve(&frame, &analysis, Some(content), counts);
    assert!(restricted.complete, "{}", restricted.message);
    assert_eq!(restricted.placements, first.placements);
    assert_eq!(partial.cache_hits(), 1, "raw type restriction must miss");
    assert_eq!(
        partial.resolve(&frame, &analysis, Some(content), counts),
        restricted
    );
    assert_eq!(partial.cache_hits(), 2);

    analysis.cells[5] = "item2".into();
    for _ in 0..2 {
        let pending = partial.resolve(&frame, &analysis, Some(content), counts);
        assert!(!pending.complete);
        assert!(pending.placements.is_empty());
        assert_eq!(
            partial.cache_hits(),
            2,
            "unsupported evidence must not cache"
        );
    }
    analysis.cells[5] = "item1".into();
    assert!(
        partial.resolve(&frame, &analysis, Some(content), counts).complete
    );
    assert_eq!(
        partial.cache_hits(),
        2,
        "failed new input must retire the old success"
    );
    partial.reset();
    assert_eq!(partial.cache_hits(), 0);
    assert!(
        partial.resolve(&frame, &analysis, Some(content), counts).complete
    );
    assert_eq!(
        partial.cache_hits(),
        0,
        "round reset must retire cached success"
    );
}

#[test]
fn runtime_cache_compares_actual_board_cards_and_all_effective_inputs() {
    let frame = fixture("initial");
    let counts = [Some(2), Some(5), Some(2)];
    let (analysis, content) = analyze(&mut Recognizer::new(), &frame, 45, counts);
    assert!(analysis.cells.iter().all(|cell| cell == "unknown"));
    let mut partial = PartialRecognizer::default();

    for change in [
        "board pixels", "card pixels", "cells", "shapes", "counts", "board", "content",
        "dimensions", "Finish",
    ] {
        let baseline = partial.resolve(&frame, &analysis, Some(content), counts);
        assert!(baseline.complete);
        let hits = partial.cache_hits();
        assert_eq!(
            partial.resolve(&frame, &analysis, Some(content), counts),
            baseline
        );
        assert_eq!(partial.cache_hits(), hits + 1);

        let mut changed_frame = frame.clone();
        let mut changed_analysis = analysis.clone();
        let mut changed_content = content;
        let mut changed_counts = counts;
        match change {
            "board pixels" => {
                let board = analysis.board.unwrap();
                let x = ((board[0] + board[2] / (2.0 * 9.0 * 32.0))
                    * frame.width() as f64)
                    .floor() as u32;
                let y = ((board[1] + board[3] / (2.0 * 5.0 * 32.0))
                    * frame.height() as f64)
                    .floor() as u32;
                for yy in y..=y + 1 {
                    for xx in x..=x + 1 {
                        changed_frame.put_pixel(xx, yy, image::Rgba([255, 0, 255, 255]));
                    }
                }
            }
            "card pixels" => {
                let [x, y, w, h] = vision::layout_region(content, vision::LayoutAnchor::Bottom);
                let x = (x + 183.0 * w / 1920.0).floor() as u32;
                let y = (y + 881.0 * h / 1080.0).floor() as u32;
                for yy in y..=y + 1 {
                    for xx in x..=x + 1 {
                        changed_frame.put_pixel(xx, yy, image::Rgba([255, 0, 255, 255]));
                    }
                }
            }
            "cells" => changed_analysis.cells[0] = "empty".into(),
            "shapes" => {
                changed_analysis.shapes[0] = if analysis.shapes[0] == [2, 2] {
                    [1, 1]
                } else {
                    [2, 2]
                };
            }
            "counts" => changed_counts[0] = Some(3),
            "board" => changed_analysis.board.as_mut().unwrap()[0] += 1e-12,
            "content" => {
                changed_content[0] += 1e-12;
                changed_content[2] -= 1e-12;
            }
            "dimensions" => {
                changed_frame = RgbaImage::new(frame.width() + 1, frame.height());
                image::imageops::replace(&mut changed_frame, &frame, 0, 0);
            }
            "Finish" => changed_analysis.finish[0] = true,
            _ => unreachable!(),
        }
        let _ = partial.resolve(
            &changed_frame,
            &changed_analysis,
            Some(changed_content),
            changed_counts,
        );
        assert_eq!(partial.cache_hits(), hits + 1, "{change} must invalidate");
        let restored = partial.resolve(&frame, &analysis, Some(content), counts);
        assert!(restored.complete);
        assert_eq!(
            partial.cache_hits(),
            hits + 1,
            "{change} must retire the old entry"
        );
    }

    // Pixels outside the matcher's board/cards are irrelevant to this result.
    let hits = partial.cache_hits();
    let mut outside = frame.clone();
    outside.put_pixel(0, 0, image::Rgba([255, 0, 255, 255]));
    assert!(
        partial.resolve(&outside, &analysis, Some(content), counts).complete
    );
    assert_eq!(partial.cache_hits(), hits + 1);

    for invalid_content in [None, Some([f64::NAN, 0.0, 1.0, 1.0])] {
        let hits = partial.cache_hits();
        assert!(
            !partial.resolve(&frame, &analysis, invalid_content, counts).complete
        );
        assert_eq!(partial.cache_hits(), hits);
        assert!(
            partial.resolve(&frame, &analysis, Some(content), counts).complete
        );
        assert_eq!(
            partial.cache_hits(),
            hits,
            "invalid geometry must clear memo"
        );
    }

    let mut absent = analysis.clone();
    absent.present = false;
    let hits = partial.cache_hits();
    assert!(
        !partial.resolve(&frame, &absent, Some(content), counts).complete
    );
    assert!(
        partial.resolve(&frame, &analysis, Some(content), counts).complete
    );
    assert_eq!(partial.cache_hits(), hits, "absent frame must clear memo");
}

#[test]
fn runtime_cache_never_reuses_interrupted_search_or_missing_inventory() {
    let frame = fixture("user-20261006-phone-strap");
    let counts = [Some(0), Some(5), Some(2)];
    let (analysis, content) = analyze(&mut Recognizer::new(), &frame, 30, counts);
    let mut interrupted = PartialRecognizer::with_test_matching_budget(Duration::ZERO);
    for _ in 0..2 {
        let pending = interrupted.resolve(&frame, &analysis, Some(content), counts);
        assert!(!pending.complete);
        assert!(pending.placements.is_empty());
        assert!(pending.message.contains("局部匹配未完成"));
        assert_eq!(interrupted.cache_hits(), 0);
    }

    let mut missing = PartialRecognizer::default();
    for _ in 0..2 {
        let pending = missing.resolve(&frame, &analysis, Some(content), [Some(0), None, Some(2)]);
        assert!(!pending.complete);
        assert!(pending.placements.is_empty());
        assert!(pending.message.contains("剩余件数未识别"));
        assert_eq!(missing.cache_hits(), 0);
    }
}

#[test]
fn runtime_delta5_and_joint_constraints_deduplicate_physical_items() {
    // The image matcher is exercised above. Here controlled scores/observations
    // isolate the joint decision contract using an actually extracted reference.
    let (mut analysis, mut output) = phone_evidence();
    let original = output
        .candidates
        .iter()
        .find(|pose| pose.item_index == 1)
        .unwrap()
        .clone();
    let counts = [Some(0), Some(1), Some(0)];
    output.candidates = vec![
        scored_pose(&original, 5, phone_rect(3), 10.0),
        scored_pose(&original, 5, phone_rect(4), 16.0),
    ];
    let selected = decide_matches(&analysis, &output, counts);
    assert!(selected.complete, "{}", selected.message);
    assert_eq!(selected.placements, vec![phone_placement(3, 0, 3, 1)]);

    // The band is inclusive; a runner at best + 5 stays ambiguous.
    output.candidates[1] = scored_pose(&original, 5, phone_rect(4), 15.0);
    let tied = decide_matches(&analysis, &output, counts);
    assert!(!tied.complete);
    assert!(tied.placements.is_empty());

    analysis.cells[4] = "uncertain".into();
    output.observations = vec![
        observation(&output.observations[0], 4),
        observation(&output.observations[0], 5),
    ];
    output.candidates = vec![
        scored_pose(&original, 4, phone_rect(3), 10.0),
        scored_pose(&original, 5, phone_rect(3), 11.0),
    ];
    let cells = analysis.cells.clone();
    let shared = decide_matches(&analysis, &output, counts);
    assert!(shared.complete, "{}", shared.message);
    assert_eq!(shared.placements, vec![phone_placement(3, 0, 3, 1)]);
    assert_eq!(analysis.cells, cells);

    output.candidates[1] = scored_pose(&original, 5, phone_rect(4), 11.0);
    let overlapping = decide_matches(&analysis, &output, [Some(0), Some(2), Some(0)]);
    assert!(!overlapping.complete);
    assert!(overlapping.placements.is_empty());
    output.candidates[1] = scored_pose(&original, 5, phone_rect(3), 11.0);
    let exhausted = decide_matches(&analysis, &output, [Some(0); 3]);
    assert!(!exhausted.complete);
    assert!(exhausted.placements.is_empty());
}

#[test]
fn runtime_missing_hud_reference_and_incomplete_evidence_never_confirm() {
    let (analysis, mut output) = phone_evidence();
    for counts in [[None; 3], [Some(0), None, Some(2)]] {
        let result = decide_matches(&analysis, &output, counts);
        assert!(!result.complete);
        assert!(result.placements.is_empty());
    }
    let counts = [Some(0), Some(5), Some(2)];
    output.complete = false;
    assert!(!decide_matches(&analysis, &output, counts).complete);
    output.complete = true;
    output.observations[0].sufficient_evidence = false;
    assert!(!decide_matches(&analysis, &output, counts).complete);
    output.observations[0].sufficient_evidence = true;
    output.cards[1].fingerprint = None;
    assert!(!decide_matches(&analysis, &output, counts).complete);
}

#[test]
fn runtime_session_warms_without_fragments_survives_invalid_frame_and_resets() {
    let config = matching_config();
    let mut recognizer = Recognizer::new();
    let initial = fixture("initial");
    let (analysis, content) = analyze(&mut recognizer, &initial, 45, [Some(2), Some(5), Some(2)]);
    let mut session = Session::default();
    let warm = session
        .run_live(&initial, &analysis, content, "initial", &config, None)
        .unwrap();
    assert!(warm.observations.is_empty());
    assert!(warm.cards[0].fingerprint.is_some());
    let no_fragments = recognizer.resolve_partial(&initial, &analysis, Some(content), [None; 3]);
    assert!(no_fragments.complete);
    assert!(no_fragments.placements.is_empty());

    let mut absent = analysis.clone();
    absent.present = false;
    absent.board = None;
    assert!(session
        .run_live(&initial, &absent, content, "absent", &config, None)
        .is_err());
    let pending = recognizer.resolve_partial(&initial, &absent, Some(content), [None; 3]);
    assert!(!pending.complete);
    assert!(pending.placements.is_empty());

    let finished = fixture("waterguns-finished");
    let counts = [Some(0), Some(4), Some(1)];
    let (analysis, content) = analyze(&mut recognizer, &finished, 19, counts);
    let reused = session
        .run_live(&finished, &analysis, content, "finish", &config, None)
        .unwrap();
    assert!(reused.cards[0].finished);
    assert_eq!(reused.cards[0].fingerprint, warm.cards[0].fingerprint);
    assert_eq!(reused.cards[0].reference_source.as_deref(), Some("initial"));

    session.reset();
    recognizer.reset();
    let (analysis, content) = analyze(&mut recognizer, &finished, 19, counts);
    let cold = session
        .run_live(&finished, &analysis, content, "cold", &config, None)
        .unwrap();
    assert!(cold.cards[0].finished);
    assert!(cold.cards[0].fingerprint.is_none());
    assert!(cold.cards[0].reference_source.is_none());
    let result = recognizer.resolve_partial(&finished, &analysis, Some(content), counts);
    assert!(
        result.complete,
        "missing exhausted Finish reference: {}",
        result.message
    );
    assert_eq!(result.placements.len(), 3);
}

#[test]
fn runtime_live_and_offline_direct_poses_are_identical_without_live_png_exports() {
    let frame = fixture("user-20261006-phone-strap");
    let (analysis, content) = analyze(
        &mut Recognizer::new(),
        &frame,
        30,
        [Some(0), Some(5), Some(2)],
    );
    let config = matching_config();
    let temp = std::env::temp_dir();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let out = temp.join(format!(
        "ba-partial-runtime-{}-{unique}",
        std::process::id()
    ));
    std::fs::create_dir(&out).unwrap();
    let live = Session::default()
        .run_live(&frame, &analysis, content, "parity", &config, None)
        .unwrap();
    assert_eq!(live.asset_export_ms, 0.0);
    assert!(live.tile_cache.is_none());
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
    let mut offline = Session::default()
        .run(&frame, &analysis, "parity", &config, &out)
        .unwrap();
    assert!(out.join("board.png").is_file());
    assert!(out.join("card-1.png").is_file());
    assert_eq!(live.complete, offline.complete);
    assert!(live.evaluations <= offline.evaluations);
    assert!(live.rescue_evaluations <= offline.rescue_evaluations);
    assert!(live.foreground_evaluations <= offline.foreground_evaluations);
    let legal = |pose: &Pose, cells: &[String]| {
        !(pose.rect.y..pose.rect.y + pose.rect.height).any(|y| {
            (pose.rect.x..pose.rect.x + pose.rect.width).any(|x| {
                matches!(cells[y * 9 + x].as_str(), "empty" | "completed")
            })
        })
    };
    offline.candidates.retain(|pose| legal(pose, &analysis.cells));
    // The raw matcher need not retain an impossible pose. Start with the known
    // supported phone geometry and prove that adding either blocker invalidates
    // the same otherwise feasible full-layout constraint.
    use wasm_solver::snapshot::{check_snapshot_feasibility_native, GridPlacement,
        PlacementConstraint, SnapshotInput, SnapshotItem};
    let pose = offline.candidates.iter().find(|pose| {
        pose.item_index == 1 && pose.rect.x == 3 && pose.rect.y == 0
            && pose.rect.width == 3 && pose.rect.height == 1
    }).expect("known phone geometry must be retained");
    let blocked_cell = (pose.rect.y..pose.rect.y + pose.rect.height)
        .flat_map(|y| (pose.rect.x..pose.rect.x + pose.rect.width).map(move |x| y * 9 + x))
        .find(|&cell| cell != pose.anchor && analysis.cells[cell] == "unknown")
        .expect("known phone geometry must contain a covered non-anchor cell");
    let mut cells: Vec<_> = analysis.cells.iter().map(|cell| match cell.as_str() {
        "empty" => "empty".to_owned(),
        "completed" => "completed".to_owned(),
        _ => "unknown".to_owned(),
    }).collect();
    cells[pose.anchor] = format!("item{}", pose.item_index);
    let baseline = SnapshotInput {
        items: analysis.shapes.iter().zip([0, 5, 2]).map(|(shape, count)| SnapshotItem {
            width: shape[0] as i32, height: shape[1] as i32, remaining_count: count,
        }).collect(),
        cells,
        candidate_constraints: vec![PlacementConstraint {
            anchor: pose.anchor,
            item_index: pose.item_index,
            placements: vec![GridPlacement { x: pose.rect.x, y: pose.rect.y,
                width: pose.rect.width, height: pose.rect.height }],
        }],
    };
    let feasible = check_snapshot_feasibility_native(baseline.clone());
    assert_eq!(feasible.error, "");
    assert!(feasible.samples > 0);
    assert!(feasible.total_patterns.parse::<u64>().unwrap() > 0);
    for state in ["empty", "completed"] {
        let mut blocked = baseline.clone();
        blocked.cells[blocked_cell] = state.into();
        let result = check_snapshot_feasibility_native(blocked);
        assert_eq!(result.error,
            "no_valid_configuration: observations, remaining counts, and candidates are inconsistent");
        assert_eq!(result.total_patterns, "0");
        assert_eq!(result.samples, 0);
        assert!(result.precision.is_none());

        let mut blocked_analysis = analysis.clone();
        blocked_analysis.cells[blocked_cell] = state.into();
        let id = format!("blocked-{state}");
        let filtered = Session::default().run_live(
            &frame, &blocked_analysis, content, &id, &config, None,
        ).unwrap();
        let folder = out.join(&id);
        std::fs::create_dir(&folder).unwrap();
        let mut exhaustive = Session::default().run(
            &frame, &blocked_analysis, &id, &config, &folder,
        ).unwrap();
        assert!(filtered.complete && exhaustive.complete);
        assert!(filtered.evaluations < exhaustive.evaluations,
            "{state} pruning must reduce search work: live={}, offline={}",
            filtered.evaluations, exhaustive.evaluations);
        assert!(filtered.candidates.iter().all(|candidate| {
            candidate.item_index != pose.item_index || candidate.rect != pose.rect
        }));
        exhaustive.candidates.retain(|candidate| legal(candidate, &blocked_analysis.cells));
        assert_eq!(serde_json::to_value(filtered.candidates).unwrap(),
            serde_json::to_value(exhaustive.candidates).unwrap());
        eprintln!("blocked-{state} live_evaluations={} offline_evaluations={}",
            filtered.evaluations, exhaustive.evaluations);
    }
    assert_eq!(
        serde_json::to_value(live.cards).unwrap(),
        serde_json::to_value(offline.cards).unwrap()
    );
    assert_eq!(
        serde_json::to_value(live.observations).unwrap(),
        serde_json::to_value(offline.observations).unwrap()
    );
    assert_eq!(
        serde_json::to_value(live.candidates).unwrap(),
        serde_json::to_value(offline.candidates).unwrap()
    );
    assert!(out.starts_with(&temp));
    std::fs::remove_dir_all(&out).unwrap();
}

#[test]
fn runtime_deadline_keeps_search_incomplete_without_pruning_candidates() {
    let frame = fixture("user-20261006-phone-strap");
    let (analysis, content) = analyze(
        &mut Recognizer::new(),
        &frame,
        30,
        [Some(0), Some(5), Some(2)],
    );
    let output = Session::default()
        .run_live(
            &frame,
            &analysis,
            content,
            "deadline",
            &matching_config(),
            Some(Duration::ZERO),
        )
        .unwrap();
    assert!(!output.complete);
    assert_eq!(output.evaluations, 0);
    assert!(output
        .interruption
        .as_ref()
        .unwrap()
        .contains("elapsed budget"));
    let result = decide_matches(&analysis, &output, [Some(0), Some(5), Some(2)]);
    assert!(!result.complete);
    assert!(result.placements.is_empty());
}


fn tracking_phone() -> (
    RgbaImage,
    Analysis,
    [f64; 4],
    PartialRecognizer,
    [Option<u32>; 3],
) {
    let frame = fixture("user-20261006-phone-strap");
    let counts = [Some(0), Some(5), Some(2)];
    let (analysis, content) = analyze(&mut Recognizer::new(), &frame, 30, counts);
    let mut partial = PartialRecognizer::default();
    let result = partial.resolve(&frame, &analysis, Some(content), counts);
    assert!(result.complete, "{}", result.message);
    assert_eq!(result.placements, vec![phone_placement(3, 0, 3, 1)]);
    partial.set_test_matching_budget(Duration::ZERO);
    (frame, analysis, content, partial, counts)
}

fn repaint_tracking_cell(frame: &mut RgbaImage, analysis: &Analysis, cell: usize) {
    let [x, y, w, h] = analysis.board.unwrap();
    let x0 = ((x + (cell % 9) as f64 * w / 9.0) * frame.width() as f64).ceil() as u32 + 2;
    let x1 = ((x + (cell % 9 + 1) as f64 * w / 9.0) * frame.width() as f64).floor() as u32 - 2;
    let y0 = ((y + (cell / 9) as f64 * h / 5.0) * frame.height() as f64).ceil() as u32 + 2;
    let y1 = ((y + (cell / 9 + 1) as f64 * h / 5.0) * frame.height() as f64).floor() as u32 - 2;
    for y in y0..y1 {
        for x in x0..x1 {
            frame.put_pixel(x, y, image::Rgba([120, 80, 150, 255]));
        }
    }
}

#[test]
fn tracking_real_ring_31_to_30_keeps_confirmed_footprints() {
    let first = fixture("pc-user-20261008-tracked-ring-31");
    let next = fixture("pc-user-20261008-tracked-ring-30");
    let counts = [Some(1), Some(4), Some(3)];
    let mut observer = Recognizer::new();
    let (before, content) = analyze(&mut observer, &first, 31, counts);
    let mut partial = PartialRecognizer::default();
    let confirmed = partial.resolve(&first, &before, Some(content), counts);
    assert!(confirmed.complete, "{}", confirmed.message);
    assert_eq!(confirmed.placements.len(), 8);
    assert!(confirmed.placements.contains(&Placement {
        item_index: 0,
        x: 3,
        y: 2,
        width: 3,
        height: 3
    }));
    let (after, content) = analyze(&mut observer, &next, 30, counts);
    assert_eq!(before.cells[31], "unknown");
    assert_eq!(after.cells[31], "uncertain");
    let raw = after.cells.clone();
    partial.set_test_matching_budget(Duration::ZERO);
    let tracked = partial.resolve(&next, &after, Some(content), counts);
    assert!(tracked.complete, "{}", tracked.message);
    assert_eq!(tracked.placements, confirmed.placements);
    assert_eq!(after.cells, raw);
    let cold = PartialRecognizer::default().resolve(&next, &after, Some(content), counts);
    assert!(
        !cold.complete,
        "the new pixels alone still do not clear the original threshold"
    );
}

#[test]
fn tracking_internal_observation_and_external_empty_preserve_raw_cells() {
    let (mut frame, mut analysis, content, mut partial, counts) = tracking_phone();
    analysis.cells[4] = "uncertain".into();
    repaint_tracking_cell(&mut frame, &analysis, 4);
    let original = analysis.cells.clone();
    let result = partial.resolve(&frame, &analysis, Some(content), counts);
    assert!(result.complete, "{}", result.message);
    assert_eq!(result.placements, vec![phone_placement(3, 0, 3, 1)]);
    assert_eq!(analysis.cells, original);
    analysis.cells[4] = "item1".into();
    analysis.cells[44] = "empty".into();
    let raw = analysis.cells.clone();
    let result = partial.resolve(&frame, &analysis, Some(content), counts);
    assert!(result.complete, "{}", result.message);
    assert_eq!(result.placements.len(), 1);
    assert_eq!(analysis.cells, raw);
    for (cell, value) in analysis.cells.iter_mut().enumerate() {
        if !(3..=5).contains(&cell) {
            *value = "empty".into();
        }
    }
    let impossible = partial.resolve(&frame, &analysis, Some(content), counts);
    assert!(
        !impossible.complete,
        "current external blockers must still pass full-layout feasibility"
    );
}

#[test]
fn tracking_missing_reads_pause_without_erasing_confirmations() {
    for missing in [
        "fingerprint",
        "reference",
        "shape",
        "counts",
        "absent",
        "content",
    ] {
        let (mut frame, analysis, content, mut partial, counts) = tracking_phone();
        let mut unavailable = analysis.clone();
        let mut unavailable_counts = counts;
        let mut unavailable_content = Some(content);
        match missing {
            "fingerprint" => unavailable.card_fingerprints[1] = None,
            "reference" => unavailable.reference_ready[1] = false,
            "shape" => unavailable.shapes[1] = [0, 0],
            "counts" => unavailable_counts[1] = None,
            "absent" => unavailable.present = false,
            "content" => unavailable_content = None,
            _ => unreachable!(),
        }
        let pending = partial.resolve(
            &frame,
            &unavailable,
            unavailable_content,
            unavailable_counts,
        );
        assert!(!pending.complete, "{missing}");
        repaint_tracking_cell(&mut frame, &analysis, 5);
        let recovered = partial.resolve(&frame, &analysis, Some(content), counts);
        assert!(recovered.complete, "{missing}: {}", recovered.message);
        assert_eq!(recovered.placements, vec![phone_placement(3, 0, 3, 1)]);
    }
}

#[test]
fn tracking_explicit_conflicts_geometry_identity_and_reset_release_tracks() {
    for conflict in [
        "empty",
        "wrong type",
        "mixed completed conflict",
        "anchor lost",
        "shape",
        "card",
        "board",
        "dimensions",
        "reset",
        "invalidate",
    ] {
        let (mut frame, mut analysis, content, mut partial, counts) = tracking_phone();
        // Force any current full search to miss the old pixel cache. Tracking
        // itself does not need to rescore an already-confirmed object's pixels.
        repaint_tracking_cell(&mut frame, &analysis, 5);
        match conflict {
            "empty" => analysis.cells[4] = "empty".into(),
            "wrong type" => analysis.cells[4] = "item2".into(),
            "mixed completed conflict" => {
                analysis.cells[3] = "completed".into();
                analysis.cells[4] = "item2".into();
            }
            "anchor lost" => analysis.cells[5] = "unknown".into(),
            "shape" => analysis.shapes[1] = [2, 1],
            "card" => analysis.card_fingerprints[1] = Some("different-valid-card".into()),
            "board" => analysis.board.as_mut().unwrap()[0] += 1e-12,
            "dimensions" => {
                let mut changed = RgbaImage::new(frame.width() + 1, frame.height());
                image::imageops::replace(&mut changed, &frame, 0, 0);
                frame = changed;
            }
            "reset" => {
                partial.reset();
                partial.set_test_matching_budget(Duration::ZERO);
            }
            "invalidate" => partial.invalidate_confirmations(),
            _ => unreachable!(),
        }
        let result = partial.resolve(&frame, &analysis, Some(content), counts);
        assert!(
            result.placements.is_empty(),
            "{conflict}: {}",
            result.message
        );
        if conflict != "anchor lost" {
            assert!(!result.complete, "{conflict}");
        }
    }
}

#[test]
fn tracking_completed_object_waits_for_inventory_and_retires_once() {
    let (frame, mut analysis, content, mut partial, mut counts) = tracking_phone();
    for cell in 3..=5 {
        analysis.cells[cell] = "completed".into();
    }
    let unsynced = partial.resolve(&frame, &analysis, Some(content), counts);
    assert!(!unsynced.complete && unsynced.placements.is_empty());
    counts[1] = Some(4);
    let retired = partial.resolve(&frame, &analysis, Some(content), counts);
    assert!(retired.complete, "{}", retired.message);
    assert!(retired.placements.is_empty());
    for _ in 0..2 {
        let repeated = partial.resolve(&frame, &analysis, Some(content), counts);
        assert_eq!(
            repeated, retired,
            "retirement must not subtract inventory again"
        );
    }
    assert_eq!(partial.cache_hits(), 2);
    assert!(analysis.cells[3..=5].iter().all(|cell| cell == "completed"));
}

#[test]
fn tracking_external_fragment_pending_preserves_ledger_but_conflict_falls_back() {
    let (mut frame, analysis, content, mut partial, counts) = tracking_phone();
    let mut new_fragment = analysis.clone();
    new_fragment.cells[44] = "uncertain".into();
    let raw = new_fragment.cells.clone();
    let pending = partial.resolve(&frame, &new_fragment, Some(content), counts);
    assert!(!pending.complete && pending.placements.is_empty());
    assert_eq!(
        new_fragment.cells, raw,
        "matching/solver completion marks belong only to the copy"
    );
    let mut internal = analysis.clone();
    internal.cells[4] = "uncertain".into();
    repaint_tracking_cell(&mut frame, &internal, 4);
    let restored = partial.resolve(&frame, &internal, Some(content), counts);
    assert!(
        restored.complete,
        "new uncertain evidence outside must not erase the ledger: {}",
        restored.message
    );

    let mut impossible = internal.clone();
    impossible.cells[0] = "item0".into(); // Current inventory explicitly has zero item0.
    let result = partial.resolve(&frame, &impossible, Some(content), counts);
    assert!(!result.complete && result.placements.is_empty());
    repaint_tracking_cell(&mut frame, &internal, 5);
    let released = partial.resolve(&frame, &internal, Some(content), counts);
    assert!(
        !released.complete,
        "a proved current contradiction must release old tracks before fallback"
    );
}

#[test]
fn tracking_manual_uncertain_correction_explicitly_revokes_confirmation() {
    let first = fixture("pc-user-20261008-tracked-ring-31");
    let next = fixture("pc-user-20261008-tracked-ring-30");
    let counts = [Some(1), Some(4), Some(3)];
    let mut recognizer = Recognizer::new();
    let (before, content) = analyze(&mut recognizer, &first, 31, counts);
    assert!(
        recognizer
            .resolve_partial(&first, &before, Some(content), counts)
            .complete
    );
    let (after, content) = analyze(&mut recognizer, &next, 30, counts);
    assert!(
        recognizer
            .resolve_partial(&next, &after, Some(content), counts)
            .complete
    );
    recognizer.correct(31, "uncertain").unwrap();
    let corrected = recognizer.resolve_partial(&next, &after, Some(content), counts);
    assert!(
        !corrected.complete,
        "an explicit uncertain correction must re-enter the current visual decision"
    );
}
