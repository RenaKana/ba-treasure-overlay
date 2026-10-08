import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const dir=path.resolve(process.argv[2]);
const read=phase=>JSON.parse(fs.readFileSync(path.join(dir,phase,'results.json'),'utf8'));
const tuning=read('tuning'), validation=read('validation');
if(tuning.metadata.config_fnv1a64!==validation.metadata.config_fnv1a64) throw new Error('Frozen config differs across phases');
const frames=[...tuning.frames.map(f=>({...f,asset_dir:`tuning/${f.id}`})),...validation.frames.map(f=>({...f,asset_dir:`validation/${f.id}`}))];
const summary=[...tuning.summary,...validation.summary].map(s=>{
 const observations=frames.filter(f=>f.split===s.split&&f.kind===s.kind).flatMap(f=>f.stages?.find(stage=>stage.name===s.stage)?.observations??[]);
 const geometry_undecided=observations.filter(o=>!(o.category_certain&&o.occupancy_certain)).length;
 const angle_only_undecided=observations.filter(o=>o.category_certain&&o.occupancy_certain&&!o.direction_certain).length;
 return {...s,geometry_undecided,angle_only_undecided};
});
const truthInstances=[...new Set(frames.filter(f=>f.kind==='real').flatMap(f=>f.truth.map(t=>t.instance_id)))];
const realSources=[...new Set(frames.filter(f=>f.kind==='real').map(f=>f.source))];
const failures=[];
const comparisons=frames.flatMap(f=>f.tile_comparison?[f]:[]);
const total=fn=>comparisons.reduce((n,f)=>n+fn(f),0);
const passed=comparisons.filter(f=>f.tile_comparison.equivalent&&f.tile_comparison.decisions_equal).length;
const tile_comparison_summary={frames:comparisons.length,passed,failed:comparisons.length-passed,
 direct_matching_ms:total(f=>f.matching_direct.matching_ms),tiled_matching_ms:total(f=>f.matching.matching_ms),
 preparation_ms:total(f=>f.matching.tile_cache?.preparation_ms??0),asset_export_ms:total(f=>f.matching.asset_export_ms??0),
 hits:total(f=>f.matching.tile_cache?.hits??0),misses:total(f=>f.matching.tile_cache?.misses??0),evictions:total(f=>f.matching.tile_cache?.evictions??0),
 peak_bytes:Math.max(0,...comparisons.map(f=>f.matching.tile_cache?.peak_bytes??0)),
 scored_visible_blocks:total(f=>f.matching.tile_cache?.scored_visible_blocks??0),
 missing_candidates:total(f=>f.tile_comparison.missing_candidates),extra_candidates:total(f=>f.tile_comparison.extra_candidates),
 max_score_delta:Math.max(0,...comparisons.map(f=>f.tile_comparison.max_score_delta))};
tile_comparison_summary.speedup=tile_comparison_summary.tiled_matching_ms?tile_comparison_summary.direct_matching_ms/tile_comparison_summary.tiled_matching_ms:null;
tile_comparison_summary.timing_note='One descriptive replay; backend order alternates by frame. Tiled matching includes preparation. PNG export is separate and cannot warm matching caches. No statistical speed guarantee.';
for(const f of frames){
 if(f.error) failures.push({frame:f.id,reason:f.error});
 for(const s of f.stages??[]){
  for(const o of s.observations){if(o.correct_candidate_retained===false || o.false_unique) failures.push({frame:f.id,stage:s.name,anchor:o.anchor,retained:o.retained_indices.length,reason:o.reason,correct_candidate_retained:o.correct_candidate_retained,false_unique:o.false_unique,negative_false_unique:o.negative_false_unique,diagnosis:f.transform==='background-pollution'?'Card-margin pixels connect to foreground and touch crop edge; wrong rectangle survives RGB/edge, bidirectional rejects it after true candidate was already lost.':f.transform==='symmetric-fragment'?'A symmetric patch is falsely certain in category: reliable foreground is unexplained but its additive penalty is insufficient.':null});}
 }
}
const scoring_summary=summary.filter(s=>s.stage==='color_edge_bidirectional_global'||s.stage.startsWith('bidirectional_threshold_')&&s.stage.endsWith('_global')||s.stage==='visible_foreground_global');
const scoring_changes=frames.flatMap(f=>{
 const before=f.stages?.find(s=>s.name==='color_edge_bidirectional_global'),after=f.stages?.find(s=>s.name==='visible_foreground_global');
 const fields=['ambiguous','false_unique','correct_candidate_retained','category_certain','occupancy_certain','direction_certain'];
 return (before?.observations??[]).flatMap(b=>{const a=after?.observations.find(o=>o.id===b.id);return a&&fields.some(key=>b[key]!==a[key])?[{frame:f.id,kind:f.kind,split:f.split,anchor:b.anchor,instance_id:f.truth.find(t=>t.anchor===b.anchor)?.instance_id??null,before:b,after:a}]:[];});
});
const report={metadata:{...tuning.metadata,validation:validation.metadata,real_source_frames:realSources.length,real_partial_truth_instances:truthInstances,real_partial_validation_observations:validation.summary.filter(s=>s.kind==='real'&&s.stage==='color').reduce((n,s)=>n+s.truth_observations,0),code_state_note:'SHA-256 inventory is in sha256.json. Per-phase working-tree patches capture pre-existing dirty state; source_snapshot preserves the experimental source.'},config:tuning.config,summary,tile_comparison_summary,scoring_summary,scoring_changes,frames,failures};
fs.writeFileSync(path.join(dir,'results.json'),JSON.stringify(report,null,2)+'\n');
const here=path.dirname(fileURLToPath(import.meta.url));
fs.writeFileSync(path.join(dir,'index.html'),fs.readFileSync(path.join(here,'report.html'),'utf8').replace('__REPORT_JSON__',JSON.stringify(report).replaceAll('<','\\u003c')));
const sourceDir=path.join(dir,'source_snapshot');fs.mkdirSync(sourceDir,{recursive:true});
for(const file of fs.readdirSync(here))if(fs.statSync(path.join(here,file)).isFile())fs.copyFileSync(path.join(here,file),path.join(sourceDir,file));
fs.copyFileSync(path.join(here,'../partial_recognition.rs'),path.join(sourceDir,'entry.rs'));
const recognizerDir=path.resolve(here,'../../src');
const savedRecognizer=path.join(sourceDir,'recognizer');fs.mkdirSync(savedRecognizer,{recursive:true});
for(const file of fs.readdirSync(recognizerDir))if(fs.statSync(path.join(recognizerDir,file)).isFile())fs.copyFileSync(path.join(recognizerDir,file),path.join(savedRecognizer,file));
for(const file of ['Cargo.toml','Cargo.lock'])fs.copyFileSync(path.resolve(here,'../..',file),path.join(sourceDir,file));
const ratio=(n,d)=>d?`${n}/${d} (${(n/d*100).toFixed(1)}%)`:'N/A (0 samples)';
let md=`# Partial recognition offline experiment\n\nReal source frames: ${realSources.length}; distinct partial truth instances: ${truthInstances.length} (${truthInstances.join(', ')}).\n\nIndependent real partial validation observations: **${report.metadata.real_partial_validation_observations}**. Rounds 2/7 are completed-object controls only. This does not establish production readiness.\n\nConfig fingerprint: \`${tuning.metadata.config_fnv1a64}\`. Frozen before validation.\n\nFailure records are stage-specific; retained historical baselines are documented in source_snapshot/README.md. Geometry undecided means category or occupied rectangle is not certain. Angle-only undecided means both are certain but sprite angle has not converged; it does not block the geometric conclusion. Angle convergence does not establish angle correctness.\n\n| Split / kind / stage | Retention | False unique | Category certain/correct | Occupancy certain/correct | Direction evaluable | Geometry undecided | Angle-only undecided | Original all-dimension undecided | Incomplete frames | Extract/match/constraint ms |\n|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n`;
for(const s of summary)md+=`| ${s.split} / ${s.kind} / ${s.stage} | ${ratio(s.correct_candidate_retained,s.truth_observations)} | ${s.false_unique} | ${s.category_certain}/${s.category_correct} | ${s.occupancy_certain}/${s.occupancy_correct} | ${s.direction_evaluable||'N/A'} | ${ratio(s.geometry_undecided,s.observations)} | ${ratio(s.angle_only_undecided,s.observations)} | ${ratio(s.ambiguous,s.observations)} | ${s.incomplete_frames} | ${s.extraction_ms.toFixed(0)}/${s.matching_ms.toFixed(0)}/${s.constraints_ms.toFixed(0)} |\n`;
md+='\nCounts above are correlated frame-observations, not independent trials. Matching time is shared candidate-pool construction (including legacy scores and observed-foreground evidence), repeated in each stage row for context. All filters reuse that pool. The original refinement uses RGB; the optional foreground route is recorded separately below. Constraint timings are separately measured.\n\nFailures and complete raw observations are in `results.json`. Incomplete or insufficient evidence never establishes uniqueness. Incomplete-frame counts also include insufficient solver inputs (such as no retained candidates), not only interrupted searches; see per-stage status for the distinction. Prediction rectangles remain separate from observed cell labels. Real sprite angles have no reliable ground truth.\n';
fs.writeFileSync(path.join(dir,'conclusions.md'),md);
let tiles=`\n## Direct versus transformed cell tiles\n\nParity: ${passed}/${comparisons.length} frames; missing/extra candidates: ${tile_comparison_summary.missing_candidates}/${tile_comparison_summary.extra_candidates}; maximum score delta: ${tile_comparison_summary.max_score_delta}.\n\nFull matching: direct ${tile_comparison_summary.direct_matching_ms.toFixed(0)} ms; tiled ${tile_comparison_summary.tiled_matching_ms.toFixed(0)} ms (includes ${tile_comparison_summary.preparation_ms.toFixed(0)} ms preparation); ratio direct/tiled ${tile_comparison_summary.speedup?.toFixed(3)??'N/A'}×. Tile PNG export: ${tile_comparison_summary.asset_export_ms.toFixed(0)} ms, separately measured. ${tile_comparison_summary.timing_note}\n\nCache budget: ${report.config.matching.tile_cache_bytes} bytes; charged peak: ${tile_comparison_summary.peak_bytes} bytes; hits/misses/evictions: ${tile_comparison_summary.hits}/${tile_comparison_summary.misses}/${tile_comparison_summary.evictions}. Eviction recomputes transforms; it never removes candidates.\n\n| Frame | Equal evidence/decisions | Poses | Direct ms | Tiled ms | Preparation ms | Cache hits/misses |\n|---|---|---:|---:|---:|---:|---:|---:|\n`;
for(const f of comparisons){const c=f.tile_comparison,t=f.matching.tile_cache;tiles+=`| ${f.id} | ${c.equivalent}/${c.decisions_equal} | ${c.candidate_count_tiled} | ${c.direct_matching_ms.toFixed(0)} | ${c.tiled_matching_ms.toFixed(0)} | ${(t?.preparation_ms??0).toFixed(0)} | ${t?.hits??0}/${t?.misses??0} |\n`;}
fs.appendFileSync(path.join(dir,'conclusions.md'),tiles);
if(report.config.scoring){
 let scoring=`\n## Scoring optimization (same candidate search)\n\nSearch RGB gate: ${report.config.matching.max_color_error}. Legacy final limit: ${report.config.scoring.retention_max_error}; threshold-only controls: ${report.config.scoring.threshold_controls.join(', ')}; observed-foreground final limit: ${report.config.scoring.visible_foreground_max_error}. Final limits never prune coarse seeds.\n\nObserved-foreground MAE measures every reliable observed foreground pixel in known cells, including whole-rectangle boundaries. Missing predicted foreground has error 255; zero reliable weight is unavailable, not a perfect match. New score = max(selected bidirectional score, observed-foreground MAE), so unexplained content cannot be hidden by a small additive coefficient. Reliability still uses brightness/chroma; white sprite details can be missed. No semantic part detector or independent contour model is claimed.\n\nOnly tuning data informs this comparison. All stages recompute their geometry constraints after visual filtering; identical Direct/Tiled inputs share a solver result. Direction correctness remains N/A; the true partial holdout denominator remains zero.\n\n| Split / kind / score + global | Correct candidate retained | False unique | Category certain/correct | Occupancy certain/correct | Geometry undecided | Angle-only undecided |\n|---|---:|---:|---:|---:|---:|---:|\n`;
 for(const s of scoring_summary)scoring+=`| ${s.split} / ${s.kind} / ${s.stage} | ${ratio(s.correct_candidate_retained,s.truth_observations)} | ${s.false_unique} | ${s.category_certain}/${s.category_correct} | ${s.occupancy_certain}/${s.occupancy_correct} | ${ratio(s.geometry_undecided,s.observations)} | ${ratio(s.angle_only_undecided,s.observations)} |\n`;
 scoring+='\nBaseline failures remain in the report for comparison. A rejected wrong pose is not recovery of a correct candidate already lost during extraction/search. Detailed changed observations are saved as scoring_changes in results.json.\n';
 fs.appendFileSync(path.join(dir,'conclusions.md'),scoring);
}
if(report.config.matching.coarse_offset_rescue||report.config.matching.low_confidence_error_cap!=null){
 fs.appendFileSync(path.join(dir,'conclusions.md'),`\n## Sparse-fragment experiment\n\nCoarse offset rescue: ${report.config.matching.coarse_offset_rescue??false}. Low-confidence border pixel error cap: ${report.config.matching.low_confidence_error_cap??'disabled'}. The raw missing-prediction penalty is still 255; when enabled, this cap changes border pixel losses before weighting. Interior errors and observed denominators do not change. Uncapped MAE and capped-pixel counts are saved. This is bounded influence for uncertain borders, not a background segmentation model.\n\nRescue evaluations: ${frames.reduce((n,f)=>n+(f.matching?.rescue_evaluations??0),0)}; recovered geometry/angle basins: ${frames.reduce((n,f)=>n+(f.matching?.recovered_basins??0),0)}. Each unsuccessful basin tries the existing first-refinement offsets, preserving all passing candidates and the evaluation budget.\n\nThe two 2026-10-06 user captures are one tuning/regression sequence. Their retrospective category/rectangle truth never enters matching or caches; neither has sprite-angle truth.\n`);
}
if(report.config.matching.foreground_refinement){
 fs.appendFileSync(path.join(dir,'conclusions.md'),`\n## Foreground-guided candidate refinement\n\nThe original RGB route and coarse candidates are preserved. Each eligible rectangle/angle basin also refines an independent seed minimizing max(selected bidirectional score, observed-foreground error). Improving poses and all explored poses meeting the search retention bound are retained; no top-K is used. All extra evaluations share the configured interruption budget.\n\nForeground evaluations: ${frames.reduce((n,f)=>n+(f.matching?.foreground_evaluations??0),0)}; foreground basins: ${frames.reduce((n,f)=>n+(f.matching?.foreground_basins??0),0)}. This changes the searched candidate pool, not the final score thresholds or production recognition. The 2026-10-07 ring and its full sequence are tuning/regression data after informing this change.\n`);
}
console.log(JSON.stringify({report:path.join(dir,'index.html'),frames:frames.length,real_sources:realSources.length,partial_instances:truthInstances.length,failures:failures.length}));

if(report.config.matching.confidence_weighted_scoring){
 fs.appendFileSync(path.join(dir,"conclusions.md"),"\n## Consistent border confidence\n\nThe visible-foreground stage and refinement use confidence-weighted template-core RGB plus the unchanged edge/reverse penalties. Core RGB errors are not capped; the existing border confidence is reused. Original RGB gates, scores and stages remain recorded. Missing observed foreground retains its 255 penalty. No mask, threshold or search grid was changed. This remains tuning/regression evidence, with zero independent real partial holdout observations.\n");
}
