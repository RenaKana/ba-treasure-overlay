import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import init, { solve, solve_observed, solve_snapshot } from '../../public/wasm/wasm_solver.js';

await init(await readFile(new URL('../../public/wasm/wasm_solver_bg.wasm', import.meta.url)));

function input(shapes, width = 3, height = 3) {
  return {
    items: shapes.map(([width, height, remaining_count]) => ({ width, height, remaining_count })),
    cells: Array.from({ length: 45 }, (_, cell) => cell % 9 < width && Math.floor(cell / 9) < height ? 'unknown' : 'empty'),
  };
}

function exact(result, patterns) {
  assert.equal(result.error, '');
  assert.equal(result.precision, 'exact');
  assert.equal(result.total_patterns, String(patterns));
  assert.equal(result.samples, patterns);
  assert.equal(result.probs.length, 8);
  assert(result.probs.every(row => row.length === 45 && row.every(p => Number.isFinite(p) && p >= 0 && p <= 1)));
}

function failure(result, prefix = /input_error/) {
  assert.match(result.error, prefix);
  assert.equal(result.precision, null);
  assert.equal(result.total_patterns, '0');
  assert.equal(result.samples, 0);
  assert.deepEqual(result.probs, []);
  assert.deepEqual(result.inferred_placements, []);
}

const partial = input([[2, 1, 1], [1, 1, 0], [1, 1, 0]]);
partial.cells[10] = 'item0';
const partialResult = solve_snapshot(partial); // candidate_constraints defaults to [].
exact(partialResult, 4);
assert.equal(partialResult.probs[1][10], 1);
assert.equal(partialResult.probs[1].reduce((a, b) => a + b), 2);
assert.deepEqual(partialResult.inferred_placements, []);
const oldObserved = solve_observed({ items: partial.items.map(({ width, height, remaining_count: count }) => ({ width, height, count })), cells: partial.cells });
assert.deepEqual(partialResult.probs, oldObserved.probs);
assert.equal(partialResult.total_patterns, oldObserved.total_patterns);

const completed = input([[2, 1, 1], [1, 1, 0], [1, 1, 0]], 4, 1);
completed.cells[0] = 'completed';
completed.cells[1] = 'completed';
completed.cells[3] = 'item0';
const completedResult = solve_snapshot(completed);
exact(completedResult, 1);
for (const row of completedResult.probs) { assert.equal(row[0], 0); assert.equal(row[1], 0); }
assert.equal(completedResult.probs[1][2], 1);

const placement = { x: 0, y: 0, width: 2, height: 2 };
const multiHit = input([[2, 2, 1], [1, 1, 0], [1, 1, 0]]);
multiHit.cells[0] = 'item0';
multiHit.cells[10] = 'item0';
multiHit.candidate_constraints = [
  { anchor: 0, item_index: 0, placements: [placement, placement] },
  { anchor: 10, item_index: 0, placements: [placement] },
];
const multiHitResult = solve_snapshot(multiHit);
exact(multiHitResult, 1);
assert.deepEqual(multiHitResult.inferred_placements, [{ item_index: 0, ...placement }]);

const rotated = structuredClone(partial);
rotated.candidate_constraints = [{ anchor: 10, item_index: 0, placements: [{ x: 1, y: 0, width: 1, height: 2 }] }];
const rotatedResult = solve_snapshot(rotated);
exact(rotatedResult, 1);
assert.deepEqual(rotatedResult.inferred_placements, [{ item_index: 0, x: 1, y: 0, width: 1, height: 2 }]);

const globallyUnique = input([[2, 1, 1], [1, 1, 0], [1, 1, 0]]);
globallyUnique.cells[0] = 'item0';
globallyUnique.candidate_constraints = [{ anchor: 0, item_index: 0, placements: [
  { x: 0, y: 0, width: 2, height: 1 }, { x: 0, y: 0, width: 1, height: 2 },
] }];
globallyUnique.cells[9] = 'empty';
const globallyUniqueResult = solve_snapshot(globallyUnique);
exact(globallyUniqueResult, 1);
assert.equal(globallyUniqueResult.probs[1][1], 1);
assert.deepEqual(globallyUniqueResult.inferred_placements, []);

const impossible = structuredClone(rotated);
impossible.cells[1] = 'completed';
failure(solve_snapshot(impossible), /no_valid_configuration/);

const sampled = input([[2, 1, 1], [1, 1, 3], [1, 1, 2]], 9, 5);
sampled.cells[0] = 'item0';
sampled.candidate_constraints = globallyUnique.candidate_constraints;
const sampledResult = solve_snapshot(sampled);
assert.equal(sampledResult.error, '');
assert.equal(sampledResult.precision, 'sampled');
assert.equal(sampledResult.samples, 100000);
assert.equal(sampledResult.probs[1][0], 1);
assert.deepEqual(sampledResult.inferred_placements, []);

const invalid = [null, {}, { ...partial, unexpected: true }];
function changed(mutator) { const value = structuredClone(rotated); mutator(value); invalid.push(value); }
for (const field of ['width', 'height', 'remaining_count']) {
  for (const value of [0.5, '1', null, Infinity, NaN, 4294967296]) changed(board => { board.items[0][field] = value; });
}
changed(board => { board.items[0].count = 1; });
changed(board => { board.cells[0] = 'uncertain'; });
changed(board => { board.cells[0] = 0; });
changed(board => { board.candidate_constraints = null; });
changed(board => { board.candidate_constraints[0].extra = true; });
changed(board => { board.candidate_constraints[0].placements[0].rotated = true; });
changed(board => { board.candidate_constraints[0].placements = []; });
changed(board => { board.candidate_constraints.push(structuredClone(board.candidate_constraints[0])); });
for (const field of ['anchor', 'item_index']) {
  for (const value of [-1, 0.5, '0', null, NaN, Infinity, 4294967296, Number.MAX_SAFE_INTEGER]) {
    changed(board => { board.candidate_constraints[0][field] = value; });
  }
}
for (const field of ['x', 'y', 'width', 'height']) {
  for (const value of [-1, 0.5, '1', null, NaN, Infinity, 4294967295, 4294967296, Number.MAX_SAFE_INTEGER]) {
    changed(board => { board.candidate_constraints[0].placements[0][field] = value; });
  }
}
for (const [index, value] of invalid.entries()) {
  const result = solve_snapshot(value);
  assert(result.error, `invalid input ${index} was accepted: ${JSON.stringify(value)}`);
  failure(result);
}

const legacyResult = solve({ item_and_placement: [0, 1, 2].map(index => ({ item: { item: { width: 1, height: 1, index }, count: 0 }, placements: [] })), open_map: Array(45).fill(false) });
assert.equal(legacyResult.error, '');
assert.equal(legacyResult.probs.length, 8);
assert(legacyResult.probs.every(row => row.length === 45 && row.every(p => p === 0)));

// Exercise the actual three exported WASM entrypoints with wide rectangles.
for (const [width, height, patterns] of [[5, 1, 34], [1, 5, 34], [9, 1, 5], [5, 5, 5], [9, 5, 1], [5, 9, 1]]) {
  const board = input([[width, height, 1], [1, 1, 0], [1, 1, 0]], 9, 5);
  const snapshot = solve_snapshot(board);
  exact(snapshot, patterns);
  const observed = solve_observed({ items: board.items.map(({ width, height, remaining_count: count }) => ({ width, height, count })), cells: board.cells });
  exact(observed, patterns);
  assert.deepEqual(snapshot.probs, observed.probs);
  const legacy = solve({ item_and_placement: board.items.map(({ width, height, remaining_count: count }, index) => ({ item: { item: { width, height, index }, count }, placements: [] })), open_map: Array(45).fill(false) });
  assert.equal(legacy.error, '');
  assert.deepEqual(legacy.probs, observed.probs);
}
for (const [width, height] of [[6, 6], [10, 1], [1, 10]]) {
  const board = input([[width, height, 0], [1, 1, 0], [1, 1, 0]], 9, 5);
  failure(solve_snapshot(board));
  assert.match(solve_observed({ items: board.items.map(({ width, height, remaining_count: count }) => ({ width, height, count })), cells: board.cells }).error, /input_error/);
  assert.match(solve({ item_and_placement: board.items.map(({ width, height, remaining_count: count }, index) => ({ item: { item: { width, height, index }, count }, placements: [] })), open_map: Array(45).fill(false) }).error, /input_error/);
}
const wide = input([[5, 1, 1], [1, 1, 0], [1, 1, 0]], 9, 1);
wide.cells[0] = 'completed';
wide.cells[1] = 'completed';
wide.cells[4] = 'item0';
wide.candidate_constraints = [{ anchor: 4, item_index: 0, placements: [{ x: 2, y: 0, width: 5, height: 1 }] }];
const wideResult = solve_snapshot(wide);
exact(wideResult, 1);
assert.equal(wideResult.probs[1][0], 0);
assert.equal(wideResult.probs[1][4], 1);
assert.equal(wideResult.inferred_placements[0].width, 5);

console.log(JSON.stringify({ runtime: 'Node WASM; synthetic fixtures, not live capture', export: 'solve_snapshot', exact_patterns: partialResult.total_patterns, sampled_patterns: sampledResult.total_patterns, invalid_inputs_rejected: invalid.length, legacy_exports: ['solve', 'solve_observed'], wide_rectangle_cases_per_export: 6 }, null, 2));
