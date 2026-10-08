// Contract checks on saved evidence. Recognition failures are reported, not hidden.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
const dir=path.resolve(process.argv[2]);
const report=JSON.parse(fs.readFileSync(path.join(dir,'results.json'),'utf8'));
const frames=new Map(report.frames.map(f=>[f.id,f]));
let poses=0, geometryResults=0, tileFrames=0;
const atlasAssets=new Map();
const matcherEvidence=m=>{const v=structuredClone(m);for(const k of ['backend','tile_cache','extraction_ms','matching_ms','asset_export_ms'])delete v[k];for(const p of v.candidates){delete p.tile_template;delete p.global_feasibility;}return v;};
const decisionEvidence=stages=>stages.map(s=>{const v=structuredClone(s);for(const k of ['extraction_ms','matching_ms','constraints_ms'])delete v.metrics[k];if(v.constraints?.stats)delete v.constraints.stats.elapsed_ms;if(v.competition?.constraints?.stats)delete v.competition.constraints.stats.elapsed_ms;return v;});
const geometryOf=p=>({item_index:p.item_index,x:p.rect.x,y:p.rect.y,w:p.rect.width,h:p.rect.height});
const geometryKey=g=>JSON.stringify([g.item_index,g.x,g.y,g.w,g.h]);
const compareGeometry=(a,b)=>{for(const key of ['item_index','x','y','w','h'])if(a[key]!==b[key])return a[key]-b[key];return 0;};
const uniqueGeometries=geometries=>[...new Map(geometries.map(g=>[geometryKey(g),g])).values()].sort(compareGeometry);
for(const f of frames.values()){
 assert(!f.production.cells.some(s=>s.startsWith('item')),`${f.id}: production must preserve partial pause`);
 assert.deepEqual(f.production.candidate_constraints,[],`${f.id}: no experimental predictions in production`);
 if(f.error)continue;
 const m=f.matching;
 for(const o of m.observations)if(o.effective_foreground_pixels!==undefined){
  assert(Number.isInteger(o.border_reliable_foreground_pixels)&&o.border_reliable_foreground_pixels>=0);
  const expected=o.reliable_foreground_pixels+o.border_reliable_foreground_pixels*report.config.matching.low_confidence_weight;
  assert.equal(o.effective_foreground_pixels,expected,`${f.id}: observed foreground weight`);
  assert.equal(o.sufficient_evidence,expected>=report.config.matching.min_anchor_pixels&&o.texture_std>=report.config.matching.min_texture_std);
 }
 if(f.tile_comparison){
  tileFrames++;
  assert.equal(m.backend,'tiled');assert.equal(f.matching_direct.backend,'direct');
  assert.deepEqual(matcherEvidence(m),matcherEvidence(f.matching_direct),`${f.id}: exact matcher evidence parity`);
  assert.deepEqual(decisionEvidence(f.stages),decisionEvidence(f.stages_direct),`${f.id}: all visual/global decisions`);
  assert.equal(f.tile_comparison.equivalent,true);assert.equal(f.tile_comparison.decisions_equal,true);
  assert.equal(f.tile_comparison.missing_candidates,0);assert.equal(f.tile_comparison.extra_candidates,0);assert.equal(f.tile_comparison.max_score_delta,0);
  assert(m.tile_cache.resident_bytes<=report.config.matching.tile_cache_bytes);
  assert(m.tile_cache.peak_bytes<=report.config.matching.tile_cache_bytes);
 }
 for(const card of m.cards){if(card.reference_source){const source=frames.get(card.reference_source);assert(source,`${f.id}: missing source`);assert.equal(source.sequence,f.sequence);assert.equal(source.round,f.round);assert(source.order<=f.order,`${f.id}: future template leakage`);}}
 for(const p of m.candidates){
  poses++;const r=p.rect;assert(r.x>=0&&r.y>=0&&r.x+r.width<=9&&r.y+r.height<=5);
  assert(p.anchor%9>=r.x&&p.anchor%9<r.x+r.width&&Math.floor(p.anchor/9)>=r.y&&Math.floor(p.anchor/9)<r.y+r.height);
  assert.equal(f.production.cells[p.anchor],'uncertain');
  const [w,h]=f.production.shapes[p.item_index];assert((r.width===w&&r.height===h)||(r.width===h&&r.height===w));
  if(report.config.scoring){
   const v=p.visible_evidence;assert(v,`${f.id}: missing observed-foreground evidence`);
   for(const e of [v,...v.tiles]){
    assert(e.reliable_pixels>=0&&e.reliable_weight>=0&&e.reliable_weight<=e.reliable_pixels+1e-8);
    if(e.uncapped_foreground_color_error!==undefined){
     assert(e.capped_pixels>=0&&e.capped_pixels<=e.low_confidence_pixels&&e.low_confidence_pixels<=e.reliable_pixels);
     assert(e.reliable_weight===0?e.uncapped_foreground_color_error===null:e.uncapped_foreground_color_error+1e-8>=e.foreground_color_error);
     if(report.config.matching.low_confidence_error_cap==null){assert.equal(e.capped_pixels,0);assert.equal(e.uncapped_foreground_color_error,e.foreground_color_error);}
    }
    for(const key of ['foreground_color_error','missing_foreground_fraction','contradicted_foreground_fraction']){
     if(e.reliable_weight===0)assert.equal(e[key],null);
     else assert(Number.isFinite(e[key])&&e[key]>=0&&e[key]<=(key==='foreground_color_error'?255:1)+1e-8);
    }
   }
   assert.equal(v.reliable_pixels,v.tiles.reduce((n,t)=>n+t.reliable_pixels,0));
   assert(Math.abs(v.reliable_weight-v.tiles.reduce((n,t)=>n+t.reliable_weight,0))<1e-7);
   for(const t of v.tiles){assert.notEqual(f.production.cells[t.board_cell],'unknown');assert.equal(t.board_cell,(r.y+t.row)*9+r.x+t.column);}
  }
  if(f.tile_comparison){
   const t=p.tile_template;assert(t,`${f.id}: tile diagnostics missing`);
   assert.equal(t.columns,r.width);assert.equal(t.rows,r.height);assert.equal(t.cell_pixels,32);
   assert.deepEqual(t.anchor_tile,[p.anchor%9-r.x,Math.floor(p.anchor/9)-r.y]);
   const expected=[];
   for(let row=0;row<t.rows;row++)for(let column=0;column<t.columns;column++){
    const cell=(r.y+row)*9+r.x+column;
    if(f.production.cells[cell]!=='unknown')expected.push(cell);
   }
   assert.deepEqual(t.visible_tiles.map(v=>v.board_cell),expected,`${f.id}: only observed tiles contribute`);
   for(const v of t.visible_tiles){assert.equal(v.board_cell,(r.y+v.row)*9+r.x+v.column);assert(v.core_pixels>=0&&v.core_pixels<=1024);assert(v.color_error===null||Number.isFinite(v.color_error));}
   for(const key of ['image','mask','core']){
    const file=path.join(dir,f.asset_dir??f.id,t[key]);
    let size=atlasAssets.get(file);
    if(!size){const png=fs.readFileSync(file);assert.equal(png.subarray(0,8).toString('hex'),'89504e470d0a1a0a');size=[png.readUInt32BE(16),png.readUInt32BE(20)];atlasAssets.set(file,size);}
    assert.deepEqual(size,[t.columns*t.cell_pixels,t.rows*t.cell_pixels]);
   }
  }
  for(const state of Object.values(p.global_feasibility??{})){geometryResults++;if(state.status==='feasible'){
   for(let y=r.y;y<r.y+r.height;y++)for(let x=r.x;x<r.x+r.width;x++)assert(!['empty','completed'].includes(f.production.cells[y*9+x]));
  }}
 }
 for(const s of f.stages){
  if(s.scoring){
   assert.equal(s.candidate_checks.length,m.candidates.length);
   for(const [i,p] of m.candidates.entries()){
    const base=s.scoring.kind==='visible_foreground'&&p.confidence_weighted?p.confidence_weighted.bidirectional_score:p.scores[s.scoring.legacy_score_index],fg=p.visible_evidence?.foreground_color_error;
    const expected=s.scoring.kind==='visible_foreground'?(base==null||fg==null?null:Math.max(base,fg)):base;
    assert.equal(s.candidate_checks[i].score,expected);
    assert.equal(s.candidate_checks[i].accepted,expected!==null&&expected<=s.scoring.threshold);
   }
   if(s.constraint_input)for(const o of s.constraint_input.observations){
    const expected=m.candidates.flatMap((p,i)=>p.observation_id===o.id&&s.candidate_checks[i].accepted?[{item_index:p.item_index,x:p.rect.x,y:p.rect.y,w:p.rect.width,h:p.rect.height}]:[]);
    assert.deepEqual(o.candidates,expected,`${f.id}: solver must use this score's candidates`);
   }
  }
  if(s.constraint_input){
   assert.deepEqual(s.constraint_input.empty_cells,f.production.cells.flatMap((v,i)=>v==='empty'?[i]:[]));
   assert.deepEqual(s.constraint_input.completed_cells,f.production.cells.flatMap((v,i)=>v==='completed'?[i]:[]));
  }
  if(s.competition){
   const policy=report.config.scoring.geometry_competition,competition=s.competition;
   assert(policy&&s.name==='visible_foreground_global');
   assert.deepEqual(Object.keys(policy),['error_tolerance']);
   assert(Number.isFinite(policy.error_tolerance)&&policy.error_tolerance>=0);
   assert.deepEqual(competition.observations,s.observations.map(o=>o.competition));
   if(competition.observations.some(o=>o.proposed)){
    assert(competition.constraint_input&&competition.constraints);
    const expected={...s.constraint_input,observations:competition.observations.map(o=>({
     id:o.id,
     candidates:uniqueGeometries((o.proposed?o.groups.filter(g=>g.score<=o.cutoff).flatMap(g=>g.indices):o.baseline_retained_indices).map(i=>geometryOf(m.candidates[i])))
    }))};
    assert.deepEqual(competition.constraint_input,expected,`${f.id}: check every score-band proposal together with untouched observations`);
    assert.equal(competition.applied,competition.constraints.complete&&competition.constraints.status==='feasible');
    assert.deepEqual(competition.constraints.observations.map(o=>({id:o.observation_id,candidates:uniqueGeometries(o.candidates.map(c=>c.geometry))})),expected.observations,`${f.id}: joint constraint evidence must cover the attempted bands`);
   }else{
    assert.equal(competition.applied,false);
    assert.equal(competition.constraint_input,null);
    assert.equal(competition.constraints,null);
   }
  }
  for(const o of s.observations){
   for(const index of o.retained_indices){assert.equal(m.candidates[index].observation_id,o.id);if(s.candidate_checks)assert.equal(s.candidate_checks[index].accepted,true);}
   if(!m.complete || (s.name.endsWith('_global')&&!s.constraints?.complete))assert(!o.category_certain&&!o.occupancy_certain&&!o.direction_certain);
   const ranking=o.competition;
   if(ranking){
    const policy=report.config.scoring.geometry_competition;
    assert(policy&&s.name==='visible_foreground_global');
    assert.deepEqual(ranking.retained_indices,o.retained_indices);
    const baseline=m.candidates.flatMap((p,i)=>p.observation_id===o.id&&s.candidate_checks[i].accepted&&!s.constraints?.observations.find(r=>r.observation_id===o.id)?.candidates.some(c=>c.status==='infeasible'&&c.geometry.item_index===p.item_index&&c.geometry.x===p.rect.x&&c.geometry.y===p.rect.y&&c.geometry.w===p.rect.width&&c.geometry.h===p.rect.height)?[i]:[]);
    assert.deepEqual(ranking.baseline_retained_indices,baseline);
    const grouped=new Map();
    for(const i of baseline){
     const geometry=geometryOf(m.candidates[i]),key=geometryKey(geometry),rawScore=s.candidate_checks[i].score,score=rawScore===0?0:rawScore;
     assert(Number.isFinite(score)&&score>=0);
     const group=grouped.get(key)??{geometry,score,indices:[]};
     group.score=Math.min(group.score,score);group.indices.push(i);grouped.set(key,group);
    }
    const groups=[...grouped.values()].sort((a,b)=>a.score-b.score||compareGeometry(a.geometry,b.geometry));
    assert.deepEqual(ranking.groups,groups,`${f.id}: use the minimum score per category and rectangle`);
    const best=groups[0]?.score??null,runner=groups[1]?.score??null,cutoff=best===null?null:Math.min(best+policy.error_tolerance,Number.MAX_VALUE);
    assert.equal(ranking.best_score,best);assert.equal(ranking.runner_up_score,runner);
    assert.equal(ranking.absolute_gap,best===null||runner===null?null:runner-best);
    assert.equal(ranking.cutoff,cutoff);
    const matchingObservation=m.observations.find(row=>row.id===o.id);
    const referenceReady=f.production.reference_ready.every((ready,item)=>ready||(s.constraints?.complete&&f.production.finish[item]&&f.hud.counts?.[item]===0));
    const eligible=m.complete&&Boolean(s.constraints?.complete)&&matchingObservation.sufficient_evidence&&referenceReady;
    assert.equal(ranking.proposed,eligible&&groups.some(g=>g.score>cutoff),`${f.id}: propose only when an eligible observation loses at least one geometry`);
    const band=groups.filter(g=>g.score<=cutoff).flatMap(g=>g.indices).sort((a,b)=>a-b);
    const applied=s.competition.applied&&ranking.proposed;
    if(applied){
     assert(m.complete&&s.constraints.complete);
     assert(s.competition.constraints.complete&&s.competition.constraints.status==='feasible');
     assert.deepEqual(o.retained_indices,band,`${f.id}: retain all poses of every geometry at or below best plus tolerance`);
     assert.equal(o.category_certain,new Set(band.map(i=>m.candidates[i].item_index)).size===1);
     assert.equal(o.occupancy_certain,new Set(band.map(i=>geometryKey({...geometryOf(m.candidates[i]),item_index:0}))).size===1);
     assert.equal(o.decision_basis,'score_margin_and_joint_feasibility');
    }else assert.deepEqual(o.retained_indices,baseline,`${f.id}: no unverified pruning`);
   }
  }
 }
}
assert.equal(report.metadata.config_fnv1a64,report.metadata.validation.config_fnv1a64);
assert.equal(report.metadata.real_partial_validation_observations,0,'Keep known real partial validation gap explicit');
const strap=frames.get('new-20261006-capture02-cell5');
if(strap){
 assert.equal(strap.split,'tuning','The diagnosed strap sequence is tuning/regression evidence');
 assert.deepEqual(strap.truth[0].rect,{x:3,y:0,width:3,height:1});
 if(report.config.matching.coarse_offset_rescue&&report.config.matching.low_confidence_error_cap!=null){
  assert(strap.matching.rescue_evaluations>0&&strap.matching.recovered_basins>0);
  const result=strap.stages.find(s=>s.name==='visible_foreground_global').observations;
  assert(result.some(o=>o.correct_candidate_retained===true),'Combined changes must retain the user-confirmed phone footprint');
  assert(result.every(o=>!o.false_unique),'Strap recovery must not create false certainty');
 }
}
assert(report.failures.some(f=>f.frame==='r1-negative-symmetric-fragment'),'Symmetric failure must be visible');
const pollution=frames.get('r1-negative-background-pollution');
if(report.config.matching.extraction?.enabled){
 const d=pollution.matching.cards[0].extraction;
 assert.equal(d.status,'cleaned');assert(d.removed_raw_pixels>0);assert(d.repaired_mask_pixels<d.legacy_mask_pixels);
 const result=pollution.stages.find(s=>s.name==='visible_foreground_global').observations;
 assert(result.some(o=>o.correct_candidate_retained===true),'Boundary cleanup must recover the known true candidate');
 assert(result.every(o=>!o.false_unique),'Recovered candidates must not introduce false certainty');
}else assert(report.failures.some(f=>f.frame==='r1-negative-background-pollution'),'Disabled cleanup must retain the diagnosed failure');
for(const s of report.summary){
 const observations=report.frames.filter(f=>f.split===s.split&&f.kind===s.kind).flatMap(f=>f.stages?.find(stage=>stage.name===s.stage)?.observations??[]);
 if(s.geometry_undecided!==undefined){
  assert.equal(s.geometry_undecided,observations.filter(o=>!(o.category_certain&&o.occupancy_certain)).length);
  assert.equal(s.angle_only_undecided,observations.filter(o=>o.category_certain&&o.occupancy_certain&&!o.direction_certain).length);
  assert.equal(s.ambiguous,s.geometry_undecided+s.angle_only_undecided);
 }
}
if(report.tile_comparison_summary){assert.equal(tileFrames,report.tile_comparison_summary.frames);assert.equal(report.tile_comparison_summary.failed,0);}
const checked={status:'passed',frames:frames.size,poses,geometryResults,tileFrames,atlas_assets:atlasAssets.size,checks:['production pause contract','legal rectangles cover observed anchors','reference source cannot be future or another round','global feasible candidates exclude observed empty/completed','prediction input separated from actual observation','interruption and unknown cannot establish certainty','frozen config matches','known failures and validation gap retained','direct/tiled exact ordered candidate evidence and decisions','tile coordinate mapping, known-only evidence, atlas dimensions and cache budget']};
if(report.config.scoring)checked.checks.push('observed-foreground denominators and null evidence','separate final scoring decisions and per-stage solver inputs');
if(report.config.matching.extraction?.enabled)checked.checks.push('current-card boundary cleanup recovers known polluted candidate without false uniqueness');
if(report.config.scoring?.geometry_competition)checked.checks.push('inclusive geometric score bands retain all poses','all band proposals jointly checked or atomically rolled back');
checked.checks.push('separate geometry uncertainty and angle-only uncertainty preserve raw totals');
fs.writeFileSync(path.join(dir,'integrity-check.json'),JSON.stringify(checked,null,2)+'\n');console.log(JSON.stringify(checked));
