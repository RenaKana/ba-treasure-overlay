// Offline forced-choice evaluation. Truth is consulted only after selection.
// node evaluate-nearest.mjs <baseline/results.json> <replay-directory> <new-output-directory>
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';

const [baselinePath, replayPath, outputPath] = process.argv.slice(2);
if (!baselinePath || !replayPath || !outputPath) {
  throw new Error('Usage: node evaluate-nearest.mjs <baseline/results.json> <replay-directory> <new-output-directory>');
}
const read = file => JSON.parse(fs.readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
const baseline = read(baselinePath);
const tuning = read(path.join(replayPath, 'tuning/results.json'));
const validation = read(path.join(replayPath, 'validation/results.json'));
assert.equal(tuning.config.matching.threshold_free_search, true);
assert.deepEqual(tuning.config, validation.config);
assert.equal(validation.metadata.freeze.config_fnv1a64, tuning.metadata.config_fnv1a64);
const expanded = [...tuning.frames, ...validation.frames];
assert.equal(new Set(expanded.map(f => f.id)).size, expanded.length);
assert.deepEqual(expanded.map(f => f.id).sort(), baseline.frames.map(f => f.id).sort());
const baselineById = new Map(baseline.frames.map(f => [f.id, f]));
const originalMatching = {...tuning.config.matching};
delete originalMatching.threshold_free_search;
const baselineMatching = {...baseline.config.matching};
delete baselineMatching.threshold_free_search;
assert.deepEqual(originalMatching, baselineMatching, 'Only the search policy may change in this comparison');
for (const f of expanded) {
  const before = baselineById.get(f.id);
  assert.equal(f.input_fnv1a64, before.input_fnv1a64, `${f.id}: source differs`);
  assert.equal(f.transform, before.transform, `${f.id}: transform differs`);
  assert.deepEqual(f.truth, before.truth, `${f.id}: truth differs`);
  assert.deepEqual(f.production, before.production, `${f.id}: production output differs`);
  assert.equal(f.matching.complete, true, `${f.id}: interrupted search`);
  assert.equal(f.tile_comparison.equivalent, true, `${f.id}: backend mismatch`);
}

const rectKey = r => r ? [r.x, r.y, r.width, r.height].join(',') : '';
const objectKey = c => `${c.item_index}:${rectKey(c.rect)}`;
const score = (c, metric) => {
  if (metric === 'rgb') return c.scores[0];
  const base = c.confidence_weighted ? c.confidence_weighted.bidirectional_score : c.scores[2];
  const observed = c.visible_evidence?.foreground_color_error;
  return Number.isFinite(base) && Number.isFinite(observed) ? Math.max(base, observed) : null;
};
for (const f of baseline.frames) {
  const checks = f.stages.find(s => s.name === 'visible_foreground').candidate_checks;
  f.matching.candidates.forEach((c, i) => assert.equal(score(c, 'composite'), checks[i].score,
    `${f.id}: evaluator score differs from existing Rust scoring`));
}
// No truth, accepted flags, final thresholds, solver result or texture cutoff here.
// Stable original index breaks exact ties, and geometry ties remain visible in the report.
function nearest(matching, anchor, metric) {
  if (!matching?.complete) return {prediction: null, reason: 'incomplete_search'};
  const objects = new Map();
  matching.candidates.forEach((c, index) => {
    const error = score(c, metric);
    if (c.anchor !== anchor || !Number.isFinite(error)) return;
    const key = objectKey(c);
    const old = objects.get(key);
    if (!old || error < old.error) objects.set(key, {c, index, error});
  });
  const ranked = [...objects.values()].sort((a, b) => a.error - b.error || a.index - b.index);
  const first = ranked[0];
  if (!first) return {prediction: null, reason: 'no_scorable_candidate'};
  const {c, index, error} = first;
  return {
    prediction: {item_index: c.item_index, rect: c.rect, angle: c.angle, error, candidate_index: index},
    object_candidates: ranked.length,
    tied_objects: ranked.filter(r => r.error === error).length,
    next_object_error: ranked[1]?.error ?? null,
    margin: ranked[1] ? ranked[1].error - error : null,
    reason: 'forced_nearest_without_score_cutoff',
  };
}
function threshold(f, anchor) {
  const stage = f.stages.find(s => s.name === 'visible_foreground_global');
  const o = stage.observations.find(o => o.anchor === anchor);
  if (!o?.category_certain || !o?.occupancy_certain) {
    return {prediction: null, reason: o?.reason ?? 'observation_missing'};
  }
  const c = f.matching.candidates[o.retained_indices[0]];
  return {prediction: {item_index: c.item_index, rect: c.rect, angle: null, error: null}, reason: 'threshold_geometry_determined'};
}

const modes = [
  {id: 'threshold_baseline', name: '现有阈值 + 全局约束', frames: baseline.frames, select: threshold},
  {id: 'final_cutoff_removed', name: '仅取消最终阈值，原候选池取综合分最近', frames: baseline.frames, select: (f, a) => nearest(f.matching, a, 'composite')},
  {id: 'threshold_free_composite', name: '取消搜索及最终分数门槛，取综合分最近', frames: expanded, select: (f, a) => nearest(f.matching, a, 'composite')},
  {id: 'threshold_free_rgb', name: '取消搜索及最终分数门槛，仅取 RGB 最近', frames: expanded, select: (f, a) => nearest(f.matching, a, 'rgb')},
];
const rows = [];
for (const mode of modes) {
  for (const f of mode.frames) {
    // Denominator includes every labeled target, even if detection/search has no result.
    const anchors = [...new Set([...f.truth.map(t => t.anchor), ...f.must_remain_ambiguous,
      ...(f.matching?.observations ?? []).map(o => o.anchor)])];
    for (const anchor of anchors) {
      const selected = mode.select(f, anchor);
      if (mode.id.startsWith('threshold_free_')) {
        const metric = mode.id.endsWith('_rgb') ? 'rgb' : 'composite';
        assert.deepEqual(selected, nearest(f.matching_direct, anchor, metric), `${f.id}: selected backend result differs`);
      }
      // Selection has already completed. Labels are evaluator-only.
      const truth = f.truth.find(t => t.anchor === anchor) ?? null;
      const negative = f.must_remain_ambiguous.includes(anchor);
      const categoryCorrect = truth ? selected.prediction?.item_index === truth.item_index : null;
      const occupancyCorrect = truth ? rectKey(selected.prediction?.rect) === rectKey(truth.rect) : null;
      const observation = f.matching?.observations.find(o => o.anchor === anchor);
      rows.push({mode: mode.id, frame: f.id, kind: f.kind, split: f.split, group: f.group,
        anchor, instance_id: truth?.instance_id ?? null, truth, negative, ...selected,
        category_correct: categoryCorrect, occupancy_correct: occupancyCorrect,
        joint_correct: truth ? categoryCorrect && occupancyCorrect : null,
        negative_forced_assertion: negative && selected.prediction !== null,
        original_sufficient_evidence: observation?.sufficient_evidence ?? false,
        source_image: path.resolve(mode.id.startsWith('threshold_free_') ? replayPath : path.dirname(baselinePath), f.split, f.id, 'frame.png'),
      });
    }
  }
}
const summaries = [];
for (const mode of modes) for (const kind of ['real', 'derived', 'synthetic']) for (const split of ['tuning', 'validation']) {
  const selected = rows.filter(r => r.mode === mode.id && r.kind === kind && r.split === split);
  const labeled = selected.filter(r => r.truth);
  const negative = selected.filter(r => r.negative);
  const correct = labeled.filter(r => r.joint_correct).length;
  const instances = [...new Set(labeled.map(r => r.instance_id))];
  summaries.push({mode: mode.id, name: mode.name, kind, split,
    truth_observations: labeled.length, predicted: labeled.filter(r => r.prediction).length,
    category_correct: labeled.filter(r => r.category_correct).length,
    occupancy_correct: labeled.filter(r => r.occupancy_correct).length,
    joint_correct: correct, accuracy: labeled.length ? correct / labeled.length : null,
    missing_predictions: labeled.filter(r => !r.prediction).length,
    wrong_predictions: labeled.filter(r => r.prediction && !r.joint_correct).length,
    truth_frames: new Set(labeled.map(r => r.frame)).size,
    registered_instances: instances.length,
    instances_correct_in_all_observations: instances.filter(id => labeled.filter(r => r.instance_id === id).every(r => r.joint_correct)).length,
    exact_geometry_ties: labeled.filter(r => r.tied_objects > 1).length,
    negative_checks: negative.length, negative_forced_assertions: negative.filter(r => r.negative_forced_assertion).length,
    unlabeled_observations: selected.filter(r => !r.truth && !r.negative).length,
  });
}
const report = {schema: 1, baseline: path.resolve(baselinePath), replay: path.resolve(replayPath),
  policy: {
    decision: 'Deterministic top-1 by minimum score; exact ties use existing candidate order, independent of truth.',
    composite: 'max(confidence-weighted core RGB + edge/reverse penalties, observed foreground MAE)',
    removed: ['search RGB MAE cutoff', 'bad-color-pixel ratio rejection', 'final score cutoff', 'texture certainty gate for forced guesses'],
    preserved: ['legal in-board geometry/shape/aspect', 'known empty/completed cell exclusion', 'minimum 55 template core pixels', 'minimum 45 anchor pixels', 'available references', 'configured coarse/refinement grid'],
    global_constraints: 'Raw nearest modes do not apply solver constraints. The threshold baseline includes its existing global constraints.',
    evidence: 'Correlated tuning/replay observations; derived and synthetic results separate. Independent real partial validation denominator is zero. Sprite-angle correctness unavailable.',
    negative_note: 'Negative samples require abstention/ambiguity, so a forced claim is a policy error, not a measurable category error without a positive label.',
  },
  verification: {frames: expanded.length, backend_evidence_equal: expanded.filter(f => f.tile_comparison.equivalent).length,
    completed_searches: expanded.filter(f => f.matching.complete).length, production_matches_baseline: true,
    direct_matching_ms: expanded.reduce((n, f) => n + f.matching_direct.matching_ms, 0),
    tiled_matching_ms: expanded.reduce((n, f) => n + f.matching.matching_ms, 0)},
  summaries, rows};
fs.mkdirSync(outputPath, {recursive: false});
fs.writeFileSync(path.join(outputPath, 'comparison.json'), JSON.stringify(report, null, 2) + '\n');
const esc = text => String(text).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
const ratio = (n, d) => d ? `${n}/${d} (${(100 * n / d).toFixed(1)}%)` : 'N/A';
const rect = r => r ? `L${r.y + 1}C${r.x + 1}–L${r.y + r.height}C${r.x + r.width}` : '无输出';
const link = file => esc(path.relative(outputPath, file).replaceAll('\\', '/'));
const summaryRows = summaries.filter(s => s.truth_observations || s.negative_checks).map(s => `<tr><td>${esc(s.name)}</td><td>${s.kind}</td><td>${ratio(s.category_correct, s.truth_observations)}</td><td>${ratio(s.joint_correct, s.truth_observations)}</td><td>${s.missing_predictions}</td><td>${s.negative_forced_assertions}/${s.negative_checks}</td></tr>`).join('');
const detailRows = rows.filter(r => r.mode === 'threshold_free_composite').map(r => `<tr class="${r.joint_correct === false || r.negative_forced_assertion ? 'error' : ''}"><td><a href="${link(r.source_image)}">${esc(r.frame)}</a></td><td>${r.kind} / 格 ${r.anchor + 1}</td><td>${r.prediction ? `类别 ${r.prediction.item_index + 1}; ${rect(r.prediction.rect)}` : '无输出'}</td><td>${r.truth ? `类别 ${r.truth.item_index + 1}; ${rect(r.truth.rect)}` : r.negative ? '负例：应保持不确定' : '无标注'}</td><td>${r.prediction?.error?.toFixed(2) ?? '—'}</td><td>${r.negative ? r.negative_forced_assertion ? '强行认定' : '未认定' : r.joint_correct ? '正确' : '未正确识别'}</td></tr>`).join('');
fs.writeFileSync(path.join(outputPath, 'index.html'), `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>最近匹配离线对照</title><style>body{font:15px/1.65 system-ui;margin:36px auto;max-width:1280px;padding:0 24px;color:#20313c;background:#f6f8fa}h1{font-size:28px}table{border-collapse:collapse;background:white;width:100%;margin:20px 0}th,td{text-align:left;border-bottom:1px solid #dce3e8;padding:10px 12px}th{background:#e8eef2}a{color:#165d9b}.error{background:#fff0eb}p{max-width:1000px}</style><h1>取消分数门槛，直接取最近匹配</h1><p>采用现有综合误差最小的候选。移除搜索颜色门槛、坏像素比例淘汰和最终阈值；保留合法占格、最少有效像素和既有搜索网格。强制认定模式不调用全局求解器。类别与占格同时正确才记为正确，图案角度没有可靠标注。</p><p>所有真实局部样本均为相关调参/回归数据；独立真实局部验证分母为 <strong>0</strong>。负例单独计算强行认定数，不混入正例正确率。Direct/Tiled 证据一致 ${report.verification.backend_evidence_equal}/${report.verification.frames} 帧，搜索完成 ${report.verification.completed_searches}/${report.verification.frames} 帧。</p><table><thead><tr><th>策略</th><th>样本</th><th>类别正确</th><th>类别 + 占格正确</th><th>无输出</th><th>负例强行认定</th></tr></thead><tbody>${summaryRows}</tbody></table><h2>综合分最近：逐项结果</h2><table><thead><tr><th>截图</th><th>观测</th><th>预测</th><th>标注</th><th>误差</th><th>结果</th></tr></thead><tbody>${detailRows}</tbody></table><p><a href="comparison.json">完整 JSON、方法与逐项分数</a></p></html>`);
console.log(JSON.stringify({report: path.resolve(outputPath, 'index.html'), verification: report.verification,
  summaries: summaries.filter(s => s.truth_observations || s.negative_checks)}, null, 2));
