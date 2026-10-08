//! Optional offline score competition between distinct geometric candidates.
//! Scores propose a restriction; only a complete joint feasibility check applies it.

use super::constraints;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub error_tolerance: f64,
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !self.error_tolerance.is_finite() || self.error_tolerance < 0.0 {
            return Err("error_tolerance must be finite and non-negative".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub index: usize,
    pub geometry: constraints::Geometry,
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct Observation {
    pub id: String,
    pub eligible: bool,
    pub candidates: Vec<ScoredCandidate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Group {
    pub geometry: constraints::Geometry,
    pub score: f64,
    pub indices: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservationReport {
    pub id: String,
    pub retained_indices: Vec<usize>,
    pub baseline_retained_indices: Vec<usize>,
    /// A local score proposal remains a proposal if the joint check rolls back.
    pub proposed: bool,
    pub groups: Vec<Group>,
    pub best_score: Option<f64>,
    pub runner_up_score: Option<f64>,
    pub absolute_gap: Option<f64>,
    /// Inclusive score-band limit. Every pose of an in-band geometry is retained.
    pub cutoff: Option<f64>,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub applied: bool,
    pub reason: String,
    pub observations: Vec<ObservationReport>,
    /// The attempted joint restriction, including on rollback; never observed data.
    pub constraint_input: Option<constraints::ConstraintInput>,
    pub constraints: Option<constraints::ConstraintReport>,
}

/// `observations` must contain every observation, including ineligible baselines.
/// Candidate indices refer to the caller's original visual candidates.
pub fn select(
    input: &constraints::ConstraintInput,
    observations: &[Observation],
    config: &Config,
    solver_config: &constraints::Config,
) -> Report {
    let invalid_config = config.validate().err();
    let mut reports: Vec<_> = observations
        .iter()
        .map(|observation| assess(observation, config, invalid_config.as_deref()))
        .collect();
    if !reports.iter().any(|report| report.proposed) {
        return Report {
            applied: false,
            reason: invalid_config
                .map(|reason| format!("invalid configuration: {reason}"))
                .unwrap_or_else(|| "no eligible score-band restriction".into()),
            observations: reports,
            constraint_input: None,
            constraints: None,
        };
    }

    let mut restricted = input.clone();
    restricted.observations = observations
        .iter()
        .zip(&reports)
        .map(|(observation, report)| {
            let mut candidates: Vec<_> = if report.proposed {
                report
                    .groups
                    .iter()
                    .filter(|group| group.score <= report.cutoff.unwrap())
                    .map(|group| group.geometry)
                    .collect()
            } else {
                observation
                    .candidates
                    .iter()
                    .map(|candidate| candidate.geometry)
                    .collect()
            };
            candidates.sort_unstable();
            candidates.dedup();
            constraints::Observation {
                id: observation.id.clone(),
                candidates,
            }
        })
        .collect();
    let checked = constraints::check_candidates(&restricted, solver_config);
    let applied = checked.complete && checked.status == constraints::Status::Feasible;
    let reason = if applied {
        "score-band restrictions are jointly feasible".into()
    } else {
        format!(
            "score-band proposals rolled back: {:?} (complete={})",
            checked.status, checked.complete
        )
    };
    if !applied {
        for report in &mut reports {
            report.retained_indices = report.baseline_retained_indices.clone();
            if report.proposed {
                report.reason = reason.clone();
            }
        }
    }
    Report {
        applied,
        reason,
        observations: reports,
        constraint_input: Some(restricted),
        constraints: Some(checked),
    }
}

fn assess(
    observation: &Observation,
    config: &Config,
    invalid_config: Option<&str>,
) -> ObservationReport {
    let mut baseline: Vec<_> = observation
        .candidates
        .iter()
        .map(|candidate| candidate.index)
        .collect();
    baseline.sort_unstable();
    baseline.dedup();
    let invalid_score = observation
        .candidates
        .iter()
        .any(|candidate| !candidate.score.is_finite() || candidate.score < 0.0);
    let mut grouped = BTreeMap::<constraints::Geometry, Group>::new();
    if !invalid_score {
        for candidate in &observation.candidates {
            // Canonical zero also makes +/-0 ties independent of traversal order.
            let score = if candidate.score == 0.0 {
                0.0
            } else {
                candidate.score
            };
            let group = grouped.entry(candidate.geometry).or_insert_with(|| Group {
                geometry: candidate.geometry,
                score,
                indices: vec![],
            });
            group.score = group.score.min(score);
            group.indices.push(candidate.index);
        }
    }
    let mut groups: Vec<_> = grouped.into_values().collect();
    for group in &mut groups {
        group.indices.sort_unstable();
        group.indices.dedup();
    }
    groups.sort_by(|left, right| {
        left.score
            .total_cmp(&right.score)
            .then(left.geometry.cmp(&right.geometry))
    });
    let best_score = groups.first().map(|group| group.score);
    let runner_up_score = groups.get(1).map(|group| group.score);
    let absolute_gap = best_score
        .zip(runner_up_score)
        .map(|(best, runner)| runner - best);
    let cutoff = if invalid_config.is_none() {
        // A finite score cannot exceed an overflowed sum; keep the diagnostic
        // finite and serializable without excluding any candidate in that case.
        best_score.map(|best| (best + config.error_tolerance).min(f64::MAX))
    } else {
        None
    };
    let (proposed, reason) = if let Some(reason) = invalid_config {
        (false, format!("invalid configuration: {reason}"))
    } else if invalid_score {
        (false, "invalid candidate score".into())
    } else if !observation.eligible {
        (false, "observation is ineligible".into())
    } else if groups.len() < 2 {
        (false, "fewer than two distinct geometries".into())
    } else {
        let proposed = groups.iter().any(|group| group.score > cutoff.unwrap());
        (
            proposed,
            if proposed {
                "local geometric score-band restriction"
            } else {
                "all geometric scores are within error tolerance"
            }
            .into(),
        )
    };
    let mut retained_indices = if proposed {
        groups
            .iter()
            .filter(|group| group.score <= cutoff.unwrap())
            .flat_map(|group| group.indices.iter().copied())
            .collect()
    } else {
        baseline.clone()
    };
    retained_indices.sort_unstable();
    retained_indices.dedup();
    ObservationReport {
        id: observation.id.clone(),
        retained_indices,
        baseline_retained_indices: baseline,
        proposed,
        groups,
        best_score,
        runner_up_score,
        absolute_gap,
        cutoff,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(item_index: usize, x: u32) -> constraints::Geometry {
        constraints::Geometry {
            item_index,
            x,
            y: 0,
            w: 1,
            h: 1,
        }
    }

    fn observation(id: &str, candidates: &[(usize, constraints::Geometry, f64)]) -> Observation {
        Observation {
            id: id.into(),
            eligible: true,
            candidates: candidates
                .iter()
                .map(|&(index, geometry, score)| ScoredCandidate {
                    index,
                    geometry,
                    score,
                })
                .collect(),
        }
    }

    fn board(observations: &[Observation], counts: [u32; 3]) -> constraints::ConstraintInput {
        constraints::ConstraintInput {
            width: 4,
            height: 1,
            shapes: vec![[1, 1]; 3],
            counts: counts.to_vec(),
            empty_cells: vec![],
            completed_cells: vec![],
            observations: observations
                .iter()
                .map(|o| constraints::Observation {
                    id: o.id.clone(),
                    candidates: o.candidates.iter().map(|c| c.geometry).collect(),
                })
                .collect(),
        }
    }

    fn config() -> Config {
        Config {
            error_tolerance: 5.0,
        }
    }

    fn solver_config() -> constraints::Config {
        constraints::Config {
            max_search_nodes: 100_000,
            max_solver_calls: 1_000,
            max_elapsed_ms: 60_000,
        }
    }

    fn run(observations: &[Observation]) -> Report {
        select(
            &board(observations, [1, 0, 0]),
            observations,
            &config(),
            &solver_config(),
        )
    }

    #[test]
    fn score_band_is_applied_only_after_joint_feasibility() {
        let observations = vec![observation("o", &[(2, g(0, 1), 12.0), (1, g(0, 0), 4.0)])];
        let input = board(&observations, [1, 0, 0]);
        let before = serde_json::to_value(&input).unwrap();
        let report = select(&input, &observations, &config(), &solver_config());
        assert!(report.applied, "{report:?}");
        assert_eq!(report.observations[0].retained_indices, vec![1]);
        assert_eq!(report.observations[0].baseline_retained_indices, vec![1, 2]);
        assert_eq!(report.observations[0].absolute_gap, Some(8.0));
        assert_eq!(report.observations[0].cutoff, Some(9.0));
        let checked = report.constraints.as_ref().unwrap();
        assert!(checked.complete);
        assert_eq!(checked.status, constraints::Status::Feasible);
        assert!(checked.stats.solver_calls > 0);
        assert_eq!(serde_json::to_value(&input).unwrap(), before);
    }

    #[test]
    fn scores_within_tolerance_keep_baseline_without_solver() {
        for (best, runner) in [(11.0, 14.0), (35.0, 37.0), (2.0, 2.0)] {
            let report = run(&[observation(
                "o",
                &[(1, g(0, 0), best), (2, g(0, 1), runner)],
            )]);
            assert!(!report.applied);
            assert!(!report.observations[0].proposed);
            assert_eq!(report.observations[0].retained_indices, vec![1, 2]);
            assert_eq!(
                report.observations[0].reason,
                "all geometric scores are within error tolerance"
            );
            assert!(report.constraints.is_none());
            assert!(report.constraint_input.is_none());
        }
    }

    #[test]
    fn pruning_depends_only_on_error_difference_without_absolute_or_ratio_gate() {
        for (best, runner) in [(4.0, 12.0), (18.0, 30.0), (25.0, 37.0)] {
            let report = run(&[observation(
                "o",
                &[(1, g(0, 0), best), (2, g(0, 1), runner)],
            )]);
            assert!(report.applied, "{report:?}");
            assert!(report.observations[0].proposed);
            assert_eq!(report.observations[0].retained_indices, vec![1]);
            assert_eq!(report.observations[0].cutoff, Some(best + 5.0));
        }
    }

    #[test]
    fn zero_and_inclusive_cutoffs_are_handled_without_division() {
        for (best, runner, applied) in [
            (0.0, 6.0, true),
            (0.0, 0.0, false),
            (-0.0, 0.0, false),
            (0.0, 5.0, false),
            (11.0, 16.0, false),
            (11.0, 16.000001, true),
        ] {
            let report = run(&[observation(
                "o",
                &[(1, g(0, 0), best), (2, g(0, 1), runner)],
            )]);
            assert_eq!(report.applied, applied, "{report:?}");
        }
        let zero_tolerance = Config {
            error_tolerance: 0.0,
        };
        assert!(zero_tolerance.validate().is_ok());
        let observations = vec![observation(
            "o",
            &[(1, g(0, 0), 1.0), (2, g(0, 1), 1.0), (3, g(0, 2), 2.0)],
        )];
        let report = select(
            &board(&observations, [1, 0, 0]),
            &observations,
            &zero_tolerance,
            &solver_config(),
        );
        assert!(report.applied, "{report:?}");
        assert_eq!(report.observations[0].retained_indices, vec![1, 2]);
    }

    #[test]
    fn three_groups_keep_the_entire_band_including_multiple_types_and_ties() {
        for second_score in [11.0, 14.0, 16.0] {
            let observations = vec![observation(
                "o",
                &[
                    (3, g(0, 0), 11.0),
                    (1, g(1, 1), second_score),
                    (2, g(2, 2), 30.0),
                ],
            )];
            let report = select(
                &board(&observations, [1, 1, 1]),
                &observations,
                &config(),
                &solver_config(),
            );
            assert!(report.applied, "{report:?}");
            assert!(report.observations[0].proposed);
            assert_eq!(report.observations[0].retained_indices, vec![1, 3]);
            assert_eq!(report.observations[0].cutoff, Some(16.0));
            assert_eq!(
                report.constraint_input.unwrap().observations[0].candidates,
                vec![g(0, 0), g(1, 1)]
            );
        }
    }

    #[test]
    fn overflowing_cutoff_keeps_all_finite_scores_and_a_serializable_report() {
        let observations = vec![observation(
            "o",
            &[(1, g(0, 0), f64::MAX / 2.0), (2, g(0, 1), f64::MAX)],
        )];
        let report = select(
            &board(&observations, [1, 0, 0]),
            &observations,
            &Config {
                error_tolerance: f64::MAX,
            },
            &solver_config(),
        );
        assert!(!report.applied);
        assert!(!report.observations[0].proposed);
        assert_eq!(report.observations[0].retained_indices, vec![1, 2]);
        assert_eq!(report.observations[0].cutoff, Some(f64::MAX));
        assert!(serde_json::to_string(&report).is_ok());
    }

    #[test]
    fn geometry_minimum_keeps_every_pose_and_does_not_vote() {
        let report = run(&[observation(
            "o",
            &[
                (3, g(0, 0), 3.0),
                (1, g(0, 0), 1.0),
                (2, g(0, 0), 100.0),
                (4, g(0, 1), 7.0),
                (5, g(0, 1), 7.0),
                (6, g(0, 1), 8.0),
                (7, g(0, 1), 9.0),
            ],
        )]);
        assert!(report.applied);
        let observation = &report.observations[0];
        assert_eq!(observation.groups.len(), 2);
        assert_eq!(observation.best_score, Some(1.0));
        assert_eq!(observation.runner_up_score, Some(7.0));
        assert_eq!(observation.retained_indices, vec![1, 2, 3]);
        assert_eq!(
            report.constraint_input.unwrap().observations[0].candidates,
            vec![g(0, 0)]
        );
    }

    #[test]
    fn type_is_part_of_geometry_and_one_geometry_is_not_a_competition() {
        let observations = vec![observation("o", &[(1, g(0, 0), 1.0), (2, g(1, 0), 7.0)])];
        let report = select(
            &board(&observations, [1, 1, 0]),
            &observations,
            &config(),
            &solver_config(),
        );
        assert!(report.applied);
        assert_eq!(report.observations[0].groups.len(), 2);
        let report = run(&[observation("o", &[(1, g(0, 0), 1.0), (2, g(0, 0), 6.0)])]);
        assert!(!report.applied);
        assert_eq!(report.observations[0].retained_indices, vec![1, 2]);
        assert!(report.constraints.is_none());
    }

    #[test]
    fn reversing_candidate_order_cannot_break_ties_or_change_the_band() {
        for runner in [1.0, 7.0] {
            let forward = observation(
                "o",
                &[(2, g(0, 1), runner), (3, g(0, 0), 3.0), (1, g(0, 0), 1.0)],
            );
            let mut reverse = forward.clone();
            reverse.candidates.reverse();
            let left = run(&[forward]);
            let right = run(&[reverse]);
            assert_eq!(left.applied, right.applied);
            assert_eq!(
                serde_json::to_value(&left.observations).unwrap(),
                serde_json::to_value(&right.observations).unwrap()
            );
        }
    }

    #[test]
    fn invalid_scores_and_ineligible_observations_keep_all_indices() {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            let report = run(&[observation(
                "o",
                &[(1, g(0, 0), 1.0), (2, g(0, 1), invalid)],
            )]);
            assert!(!report.applied);
            assert_eq!(report.observations[0].retained_indices, vec![1, 2]);
            assert!(report.observations[0].groups.is_empty());
            assert_eq!(report.observations[0].best_score, None);
            assert!(report.constraints.is_none());
            assert!(serde_json::to_string(&report).is_ok());
        }
        let mut ineligible = observation("o", &[(1, g(0, 0), 1.0), (2, g(0, 1), 7.0)]);
        ineligible.eligible = false;
        let report = run(&[ineligible]);
        assert!(!report.applied);
        assert_eq!(report.observations[0].retained_indices, vec![1, 2]);
        assert_eq!(report.observations[0].best_score, Some(1.0));
        assert!(report.constraints.is_none());
    }

    #[test]
    fn config_has_no_implicit_values_and_rejects_invalid_tolerance() {
        assert!(serde_json::from_str::<Config>("{}").is_err());
        assert!(serde_json::from_str::<Config>(
            r#"{"error_tolerance":5,"extra":1}"#
        )
        .is_err());
        assert!(serde_json::from_str::<Config>(
            r#"{"max_best_error":20,"min_absolute_gap":10,"min_score_ratio":2}"#
        )
        .is_err());
        for tolerance in [0.0, 5.0] {
            assert!(serde_json::from_str::<Config>(&format!(
                r#"{{"error_tolerance":{tolerance}}}"#
            ))
            .unwrap()
            .validate()
            .is_ok());
        }
        let observations = vec![observation("o", &[(1, g(0, 0), 1.0), (2, g(0, 1), 7.0)])];
        for error_tolerance in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            let invalid = Config { error_tolerance };
            assert!(invalid.validate().is_err());
            let report = select(
                &board(&observations, [1, 0, 0]),
                &observations,
                &invalid,
                &solver_config(),
            );
            assert!(!report.applied);
            assert!(!report.observations[0].proposed);
            assert_eq!(report.observations[0].retained_indices, vec![1, 2]);
            assert!(report.constraints.is_none());
        }
    }

    #[test]
    fn individually_feasible_bands_roll_back_when_jointly_incompatible() {
        let observations = vec![
            observation("a", &[(1, g(0, 0), 1.0), (2, g(0, 1), 7.0)]),
            observation("b", &[(3, g(0, 1), 1.0), (4, g(0, 0), 7.0)]),
        ];
        let input = board(&observations, [1, 0, 0]);
        let baseline = constraints::check_candidates(&input, &solver_config());
        assert!(baseline.complete);
        assert!(baseline
            .observations
            .iter()
            .flat_map(|o| &o.candidates)
            .all(|c| c.status == constraints::Status::Feasible));
        let report = select(&input, &observations, &config(), &solver_config());
        assert!(!report.applied);
        for observation in &report.observations {
            assert!(observation.proposed);
            assert_eq!(
                observation.retained_indices,
                observation.baseline_retained_indices
            );
        }
        let checked = report.constraints.unwrap();
        assert!(checked.complete);
        assert_eq!(checked.status, constraints::Status::Infeasible);
        let attempted = report.constraint_input.unwrap();
        assert_eq!(attempted.observations[0].candidates, vec![g(0, 0)]);
        assert_eq!(attempted.observations[1].candidates, vec![g(0, 1)]);
    }

    #[test]
    fn jointly_feasible_bands_keep_alternatives_instead_of_forcing_local_best() {
        let observations = vec![
            observation(
                "a",
                &[(1, g(0, 0), 11.0), (2, g(0, 1), 14.0), (3, g(0, 2), 30.0)],
            ),
            observation(
                "b",
                &[(4, g(0, 1), 11.0), (5, g(0, 0), 14.0), (6, g(0, 2), 30.0)],
            ),
        ];
        let report = run(&observations);
        assert!(report.applied, "{report:?}");
        assert_eq!(report.observations[0].retained_indices, vec![1, 2]);
        assert_eq!(report.observations[1].retained_indices, vec![4, 5]);
        let attempted = report.constraint_input.unwrap();
        for observation in attempted.observations {
            assert_eq!(observation.candidates, vec![g(0, 0), g(0, 1)]);
        }
    }

    #[test]
    fn duplicate_item_observations_do_not_consume_duplicate_inventory() {
        let observations = vec![
            observation("a", &[(1, g(0, 0), 1.0), (2, g(0, 1), 7.0)]),
            observation("b", &[(3, g(0, 0), 1.0), (4, g(0, 1), 7.0)]),
        ];
        let report = run(&observations);
        assert!(report.applied, "{report:?}");
        assert_eq!(report.observations[0].retained_indices, vec![1]);
        assert_eq!(report.observations[1].retained_indices, vec![3]);
        assert_eq!(report.constraints.unwrap().stats.solver_calls, 1);
    }

    #[test]
    fn applied_band_preserves_ineligible_observation_candidates() {
        let mut untouched = observation("b", &[(3, g(0, 0), 1.0), (4, g(0, 1), 7.0)]);
        untouched.eligible = false;
        let observations = vec![
            observation("a", &[(1, g(0, 0), 1.0), (2, g(0, 1), 7.0)]),
            untouched,
        ];
        let report = select(
            &board(&observations, [2, 0, 0]),
            &observations,
            &config(),
            &solver_config(),
        );
        assert!(report.applied, "{report:?}");
        assert_eq!(report.observations[0].retained_indices, vec![1]);
        assert!(!report.observations[1].proposed);
        assert_eq!(report.observations[1].retained_indices, vec![3, 4]);
        assert_eq!(
            report.constraint_input.unwrap().observations[1].candidates,
            vec![g(0, 0), g(0, 1)]
        );
    }

    #[test]
    fn zero_solver_budgets_roll_back_every_proposal() {
        let observations = vec![observation("o", &[(1, g(0, 0), 1.0), (2, g(0, 1), 7.0)])];
        for budget in [
            constraints::Config {
                max_solver_calls: 0,
                ..solver_config()
            },
            constraints::Config {
                max_search_nodes: 0,
                ..solver_config()
            },
            constraints::Config {
                max_elapsed_ms: 0,
                ..solver_config()
            },
        ] {
            let report = select(
                &board(&observations, [1, 0, 0]),
                &observations,
                &config(),
                &budget,
            );
            assert!(!report.applied);
            assert!(report.observations[0].proposed);
            assert_eq!(report.observations[0].retained_indices, vec![1, 2]);
            let checked = report.constraints.unwrap();
            assert_eq!(checked.status, constraints::Status::Interrupted);
            assert!(!checked.complete);
            assert_eq!(checked.stats.solver_calls, 0);
        }
    }

    #[test]
    fn feasible_but_incomplete_joint_evidence_still_rolls_back() {
        let mut uncertain = observation("b", &[(3, g(1, 2), 1.0), (4, g(1, 3), 7.0)]);
        uncertain.eligible = false;
        let observations = vec![
            observation("a", &[(1, g(0, 0), 1.0), (2, g(0, 1), 7.0)]),
            uncertain,
        ];
        let report = select(
            &board(&observations, [1, 1, 0]),
            &observations,
            &config(),
            &constraints::Config {
                max_solver_calls: 1,
                ..solver_config()
            },
        );
        assert!(!report.applied);
        assert_eq!(report.observations[0].retained_indices, vec![1, 2]);
        assert_eq!(report.observations[1].retained_indices, vec![3, 4]);
        let checked = report.constraints.unwrap();
        assert_eq!(checked.status, constraints::Status::Feasible);
        assert!(!checked.complete);
    }

    #[test]
    fn missing_evidence_or_invalid_board_rolls_back() {
        let mut observations = vec![
            observation("a", &[(1, g(0, 0), 1.0), (2, g(0, 1), 7.0)]),
            observation("missing", &[]),
        ];
        let report = run(&observations);
        assert!(!report.applied);
        assert_eq!(report.observations[0].retained_indices, vec![1, 2]);
        assert_eq!(
            report.constraints.unwrap().status,
            constraints::Status::InsufficientEvidence
        );
        observations.pop();
        let mut invalid = board(&observations, [1, 0, 0]);
        invalid.width = 0;
        let report = select(&invalid, &observations, &config(), &solver_config());
        assert!(!report.applied);
        assert_eq!(report.observations[0].retained_indices, vec![1, 2]);
        assert_eq!(
            report.constraints.unwrap().status,
            constraints::Status::InvalidInput
        );
    }
}
