import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import init, { solve_observed } from '../public/wasm/wasm_solver.js';

await init(await readFile(new URL('../public/wasm/wasm_solver_bg.wasm', import.meta.url)));
const items = [{ width: 3, height: 2, count: 2 }, { width: 3, height: 1, count: 5 }, { width: 2, height: 1, count: 2 }];
const results = [];
for (const name of ['initial', 'partial', 'contradiction']) {
  const cells = Array(45).fill('unknown');
  if (name !== 'initial') {
    for (const index of [11, 15, 29]) cells[index] = 'empty';
    cells[33] = 'item0';
  }
  if (name === 'contradiction') { cells.fill('empty'); cells[0] = 'item0'; }
  const started = performance.now();
  const result = solve_observed({ items, cells });
  if (name === 'contradiction') {
    assert.equal(result.precision, null);
    assert.match(result.error, /no_valid_configuration/);
    assert.deepEqual(result.probs, []);
  } else {
    assert.equal(result.error, '');
    assert.equal(result.probs.length, 8);
    assert(result.probs.every(row => row.length === 45 && row.every(p => Number.isFinite(p) && p >= 0 && p <= 1)));
    assert.equal(result.samples, 100000);
    if (name === 'partial') {
      assert.equal(result.probs[7][33], 1);
      assert.equal(result.probs[7][15], 0);
    }
  }
  results.push({ name, ms: Math.round(performance.now() - started), precision: result.precision, patterns: result.total_patterns });
}
assert.equal(solve_observed(null).precision, null);
assert.match(solve_observed(null).error, /input_error/);
console.log(JSON.stringify({ runtime: 'Node WASM; not live capture latency', results }, null, 2));
