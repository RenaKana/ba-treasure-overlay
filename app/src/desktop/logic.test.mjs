import assert from 'node:assert/strict';
import test from 'node:test';
import { armSolverTimeout, SOLVER_TIMEOUT_MS } from './solverTimeout.ts';
import {
  bestCells,
  calculationKey,
  chooseWindow,
  canCalculate,
  canStartCalculation,
  filterMask,
  isCurrentResult,
  projectSolverSnapshot,
  selectedProbabilities,
  selectionRect,
  shouldAcceptCapture,
  validItems,
  compareVersion,
  validSolverResult,
  uniquePartialPlacements,
  mergeOverlayState,
  mergeRefreshState,
  canRequestRefresh,
  refreshProgressObserved,
} from './logic.ts';

const ready = (changes = {}) => ({
  session_id: 12,
  round_epoch: 2,
  revision: 4,
  status: 'ready',
  update_mode: 'auto',
  refreshing: false,
  captured_at_ms: 1790766000000,
  message: '',
  frame_url: '',
  width: 1280,
  height: 720,
  board: { x: 0.1, y: 0.2, width: 0.6, height: 0.5 },
  cells: Array(45).fill('unknown'),
  items: [
    { width: 2, height: 1, remaining_count: 2 },
    { width: 1, height: 3, remaining_count: 1 },
    { width: 2, height: 2, remaining_count: 1 },
  ],
  completed_objects: [],
  candidate_constraints: [],
  partial_placements: [],
  reference_ready: [true, true, true],
  card_fingerprints: ['a', 'b', 'c'],
  finish: [false, false, false],
  remaining: null,
  round: '1',
  confirmed: true,
  ...changes,
});

const version = (changes = {}) => ({ session_id: 12, round_epoch: 2, revision: 4, ...changes });
const solverResult = (changes = {}) => ({
  error: '', precision: 'exact', total_patterns: '1', samples: 0,
  probs: Array.from({ length: 8 }, () => Array(45).fill(0.25)),
  inferred_placements: [], ...changes,
});
const overlayState = (changes = {}) => ({
  ...version(), visible: true, precision: 'exact', message: '',
  probabilities: Array(45).fill(0.25), cells: Array(45).fill('unknown'),
  inferred_placements: [], emphasize_best: false, ...changes,
});
const refreshState = (changes = {}) => ({
  ...version(), refreshing: false, enabled: true, attention: false, message: '手动快照', ...changes,
});

test('window selection prefers the game device and keeps unrelated windows unselected', () => {
  const choices = [
    { hwnd: 10, title: 'ChatGPT' },
    { hwnd: 20, title: 'MuMu模拟器' },
    { hwnd: 30, title: 'MuMu安卓设备' },
  ];
  assert.equal(chooseWindow(choices, ''), '30');
  assert.equal(chooseWindow(choices, '10'), '10');
  assert.equal(chooseWindow(choices.slice(0, 2), ''), '');
  assert.equal(chooseWindow([{ hwnd: 40, title: 'BlueArchive' }], ''), '40');
});

test('PC selection skips the Japanese launcher and selects the game window', () => {
  const launcher = { hwnd: 10, title: 'BlueArchive_JP_Gamelauncher' };
  const game = { hwnd: 20, title: 'Blue Archive' };
  assert.equal(chooseWindow([launcher, game], ''), '20');
  assert.equal(chooseWindow([launcher], ''), '');
  const japaneseGame = { hwnd: 30, title: 'ブルーアーカイブ' };
  assert.equal(chooseWindow([{ hwnd: 11, title: 'ブルアカ' }, japaneseGame], ''), '30');
  assert.equal(chooseWindow([japaneseGame], '999'), '30');
  // An explicit user selection remains available for unrecognized titles.
  assert.equal(chooseWindow([launcher, game], '10'), '10');
});

test('only confirmed ready boards with a valid solver projection can calculate', () => {
  assert.equal(canCalculate(ready()), true);
  for (const status of ['manual', 'searching', 'uncertain', 'waiting_item', 'paused', 'away', 'error']) {
    assert.equal(canCalculate(ready({ status })), false);
  }
  assert.equal(canCalculate(ready({ confirmed: false })), false);
  assert.equal(canCalculate(ready({ board: null })), false);
  assert.equal(canCalculate(ready({ cells: Array(45).fill('uncertain') })), false);
  assert.equal(canCalculate(ready({ cells: ['unknown'] })), false);
  const completedCells = Array(45).fill('unknown');
  completedCells[8] = 'completed';
  assert.equal(canCalculate(ready({ cells: completedCells })), true);
});

test('duplicate fragment footprints retire one item without mutating the captured board or counts', () => {
  const placement = { item_index: 0, x: 0, y: 0, width: 2, height: 1 };
  const cells = Array(45).fill('unknown');
  cells[0] = 'item0';
  cells[1] = 'uncertain';
  const snapshot = ready({ cells, partial_placements: [placement, { ...placement }], remaining: 2,
    candidate_constraints: [{ anchor: 0, item_index: 0, placements: [placement] }] });
  const original = structuredClone(snapshot);
  const input = projectSolverSnapshot(snapshot);
  assert.ok(input);
  assert.equal(input.items[0].remaining_count, 1);
  assert.deepEqual(input.cells.slice(0, 3), ['completed', 'completed', 'unknown']);
  assert.deepEqual(input.candidate_constraints, [], 'local candidate guesses never enter the projected solver input');
  assert.deepEqual(uniquePartialPlacements(snapshot), [placement]);
  assert.notStrictEqual(input.cells, snapshot.cells);
  for (let item = 0; item < 3; item += 1) assert.notStrictEqual(input.items[item], snapshot.items[item]);
  assert.deepEqual(snapshot, original, 'raw observations, HUD counts, items and candidates remain intact');
  assert.equal(canCalculate(snapshot), true, 'resolved raw uncertain and item fragments may calculate');
});

test('two distinct footprints of the same item type decrement twice, including a rotated footprint', () => {
  const cells = Array(45).fill('unknown');
  cells[0] = 'uncertain';
  cells[22] = 'item0';
  const placements = [
    { item_index: 0, x: 0, y: 0, width: 2, height: 1 },
    { item_index: 0, x: 4, y: 2, width: 1, height: 2 },
  ];
  const input = projectSolverSnapshot(ready({ cells, partial_placements: placements }));
  assert.ok(input);
  assert.equal(input.items[0].remaining_count, 0);
  assert.deepEqual([0, 1, 22, 31].map((index) => input.cells[index]), Array(4).fill('completed'));
  assert.equal(input.items[1].remaining_count, 1);
});

test('already completed Finish items use the HUD count without another decrement', () => {
  const cells = Array(45).fill('unknown');
  cells[0] = cells[1] = 'completed';
  const snapshot = ready({ cells, finish: [true, false, false],
    items: ready().items.map((item, index) => index === 0 ? { ...item, remaining_count: 0 } : item),
    completed_objects: [{ item_index: 0, x: 0, y: 0, width: 2, height: 1 }] });
  const input = projectSolverSnapshot(snapshot);
  assert.ok(input);
  assert.equal(input.items[0].remaining_count, 0);
  assert.deepEqual(input.cells, cells);
  assert.equal(canCalculate(snapshot), true);
  assert.equal(projectSolverSnapshot({ ...snapshot,
    partial_placements: [{ item_index: 0, x: 0, y: 0, width: 2, height: 1 }] }), null,
  'a stale partial footprint cannot double-count a completed object');
});

test('projection rejects insufficient counts, incompatible pixels, and incomplete fragment coverage', () => {
  const placement = { item_index: 0, x: 0, y: 0, width: 2, height: 1 };
  const cells = Array(45).fill('unknown');
  cells[0] = 'uncertain';
  const partial = ready({ cells, partial_placements: [placement] });
  const changedCell = (index, cell) => cells.map((current, position) => position === index ? cell : current);
  const cases = [
    ['no remaining item', { items: partial.items.map((item, index) => index === 0 ? { ...item, remaining_count: 0 } : item) }],
    ['empty inside footprint', { cells: changedCell(1, 'empty') }],
    ['completed inside footprint', { cells: changedCell(1, 'completed') }],
    ['manually corrected other item inside footprint', { cells: changedCell(1, 'item1') }],
    ['no observed fragment', { cells: changedCell(0, 'unknown') }],
    ['unresolved uncertain outside footprint', { cells: changedCell(2, 'uncertain') }],
    ['unresolved typed item outside footprint', { cells: changedCell(2, 'item0') }],
    ['shape differs from the item', { partial_placements: [{ ...placement, width: 3 }] }],
    ['out of bounds', { partial_placements: [{ ...placement, x: 8 }] }],
    ['invalid item index', { partial_placements: [{ ...placement, item_index: 3 }] }],
    ['fractional position', { partial_placements: [{ ...placement, x: 0.5 }] }],
    ['malformed placement', { partial_placements: [null] }],
    ['malformed placement list', { partial_placements: null }],
    ['invalid raw cell', { cells: changedCell(2, 'invalid') }],
    ['invalid raw items', { items: [null, ...partial.items.slice(1)] }],
  ];
  for (const [reason, change] of cases) {
    const snapshot = { ...partial, ...change };
    assert.equal(projectSolverSnapshot(snapshot), null, reason);
    assert.equal(canCalculate(snapshot), false, reason);
  }
  const second = { ...placement, x: 3 };
  const twoItems = { ...partial, cells: changedCell(3, 'item0'), partial_placements: [placement, second],
    items: partial.items.map((item, index) => index === 0 ? { ...item, remaining_count: 1 } : item) };
  assert.equal(projectSolverSnapshot(twoItems), null, 'one remaining item cannot retire two distinct footprints');
});

test('distinct overlapping footprints are rejected instead of merged as one item', () => {
  const cells = Array(45).fill('unknown');
  cells[0] = cells[2] = 'uncertain';
  const partial_placements = [
    { item_index: 0, x: 0, y: 0, width: 2, height: 1 },
    { item_index: 0, x: 1, y: 0, width: 2, height: 1 },
  ];
  assert.equal(projectSolverSnapshot(ready({ cells, partial_placements })), null);
});

test('legacy snapshots without partial placements preserve completed-only calculation', () => {
  const { partial_placements: _legacyMissing, ...snapshot } = ready();
  assert.ok(projectSolverSnapshot(snapshot));
  assert.equal(canCalculate(snapshot), true);
  assert.deepEqual(uniquePartialPlacements(snapshot), []);
  snapshot.cells[0] = 'uncertain';
  assert.equal(projectSolverSnapshot(snapshot), null);
});

test('partial items stop calculation and invalidate old results until completed in both modes', () => {
  for (const update_mode of ['manual', 'auto']) {
    for (const observed of ['item0', 'item1', 'item2', 'uncertain']) {
      const cells = Array(45).fill('unknown');
      cells[8] = observed;
      const partial = ready({ update_mode, status: 'waiting_item', cells, revision: 5 });
      assert.equal(canCalculate(partial), false);
      assert.equal(isCurrentResult(version(), partial), false);
      assert.equal(calculationKey(partial), null);
      assert.equal(canCalculate({ ...partial, status: 'ready' }), false, 'backend ready alone cannot bypass the partial-item gate');
      cells[8] = 'completed';
      const complete = ready({ update_mode, cells, revision: 6 });
      assert.equal(canCalculate(complete), true);
      assert.equal(isCurrentResult(version(), complete), false);
      assert.equal(isCurrentResult(version({ revision: 6 }), complete), true);
    }
  }
});

test('manual snapshots calculate only after a completed refresh returns ready', () => {
  const waiting = ready({ update_mode: 'manual', status: 'manual', captured_at_ms: null });
  assert.equal(calculationKey(waiting), null);
  const refreshing = ready({ update_mode: 'manual', status: 'searching', refreshing: true, revision: 5 });
  assert.equal(canCalculate(refreshing), false);
  assert.equal(isCurrentResult(version(), refreshing), false);
  const refreshed = ready({ update_mode: 'manual', revision: 6 });
  assert.equal(calculationKey(refreshed), '12:2:6');
  assert.equal(canCalculate(refreshed), true);
  assert.equal(canCalculate(ready({ refreshing: true })), true, 'ready may remain busy until the solver publishes');
});

test('manual parameter and cell corrections require a refresh and reject cached results', () => {
  const snapshot = ready({ update_mode: 'manual' });
  assert.equal(isCurrentResult(version(), snapshot), true);
  const corrected = ready({ update_mode: 'manual', status: 'manual', revision: 5 });
  assert.equal(calculationKey(corrected), null);
  assert.equal(isCurrentResult(version(), corrected), false);
});

test('resizing retires the old solver result and requires a new stable snapshot', () => {
  for (const update_mode of ['manual', 'auto']) {
    const resizing = ready({ update_mode, status: update_mode === 'manual' ? 'manual' : 'searching',
      revision: 5, board: null, content_rect_px: null });
    assert.equal(canCalculate(resizing), false);
    assert.equal(isCurrentResult(version(), resizing), false);
    assert.equal(canStartCalculation(resizing, null), false);
    const stable = ready({ update_mode, revision: 6, width: 960, height: 570 });
    assert.equal(canStartCalculation(stable, calculationKey(ready({ update_mode }))), true);
    assert.equal(isCurrentResult(version(), stable), false);
  }
});

test('repeated manual snapshots keep the solver job and completed filter matrix', () => {
  const snapshot = ready({ update_mode: 'manual' });
  const repeated = { ...snapshot, message: '手动快照', frame_url: 'same-captured-frame' };
  assert.equal(calculationKey(repeated), calculationKey(snapshot));
  const matrix = Array.from({ length: 8 }, (_, mask) => Array(45).fill(mask / 8));
  assert.strictEqual(selectedProbabilities(matrix, filterMask([true, false, true])), matrix[5]);
  assert.equal(isCurrentResult(version(), repeated), true);
});

test('manual jobs do not restart after cancellation or a rejected local edit', () => {
  const snapshot = ready({ update_mode: 'manual' });
  assert.equal(canStartCalculation(snapshot, null), true);
  assert.equal(canStartCalculation(snapshot, calculationKey(snapshot)), false);
  const refreshed = ready({ update_mode: 'manual', revision: 5 });
  assert.equal(canStartCalculation(refreshed, calculationKey(snapshot)), true);
  assert.equal(canStartCalculation(ready({ update_mode: 'manual', round_epoch: 3 }), calculationKey(snapshot)), true);
  assert.equal(canStartCalculation(ready({ update_mode: 'auto' }), calculationKey(snapshot)), true);
});

test('preview-only changes retain the job key and old revisions are rejected', () => {
  const state = ready();
  assert.equal(calculationKey(state), calculationKey(ready({ frame_url: 'new' })));
  assert.equal(isCurrentResult(version(), state), true);
  assert.equal(isCurrentResult(version({ revision: 3 }), state), false);
  assert.equal(isCurrentResult(version({ session_id: 11 }), state), false);
  assert.equal(isCurrentResult(version({ round_epoch: 1 }), state), false);
  assert.equal(isCurrentResult(version(), ready({ status: 'paused' })), false);
  assert.equal(shouldAcceptCapture(state, ready({ revision: 3 }), 12), false);
  assert.equal(shouldAcceptCapture(state, ready({ session_id: 13 }), 12), false);
  assert.equal(shouldAcceptCapture(state, ready({ round_epoch: 1, revision: 100 }), 12), false);
  assert.equal(shouldAcceptCapture(state, ready({ round_epoch: 3, revision: 0, round: '1' }), 12), true);
  assert.equal(shouldAcceptCapture(state, ready({ session_id: 11, revision: 100 }), null), false);
  assert.equal(shouldAcceptCapture(state, ready({ session_id: 13, round_epoch: 0, revision: 0 }), null), true);
});

test('all seven filters select their matching WASM probability row', () => {
  const matrix = Array.from({ length: 8 }, (_, mask) => Array(45).fill(mask / 8));
  for (let mask = 1; mask <= 7; mask += 1) {
    const selected = [0, 1, 2].map((index) => Boolean(mask & (1 << index)));
    assert.equal(filterMask(selected), mask);
    assert.equal(selectedProbabilities(matrix, mask)?.[0], mask / 8);
  }
  assert.equal(selectedProbabilities(matrix, 0), null);
  assert.equal(selectedProbabilities([[NaN]], 1), null);
});

test('the live nine-item round remains valid within per-group solver bounds', () => {
  assert.equal(validItems([
    { width: 3, height: 2, remaining_count: 2 },
    { width: 3, height: 1, remaining_count: 5 },
    { width: 2, height: 1, remaining_count: 2 },
  ]), true);
  assert.equal(validItems([
    { width: 5, height: 1, remaining_count: 1 },
    { width: 1, height: 1, remaining_count: 0 },
    { width: 1, height: 1, remaining_count: 0 },
  ]), true);
});

test('item sizes are bounded by board geometry including rotation, not an activity maximum', () => {
  for (const [width, height] of [[5, 1], [1, 5], [9, 1], [1, 9], [5, 5], [9, 5], [5, 9]]) {
    const items = [
      { width, height, remaining_count: 1 },
      { width: 1, height: 1, remaining_count: 0 },
      { width: 1, height: 1, remaining_count: 0 },
    ];
    assert.equal(validItems(items), true, `${width}x${height}`);
    assert.equal(canCalculate(ready({ items })), true);
  }
  for (const [width, height] of [[6, 6], [10, 1], [1, 10], [0, 1], [-1, 1], [1.5, 1], [Infinity, 1], [NaN, 1]]) {
    assert.equal(validItems([
      { width, height, remaining_count: 1 },
      { width: 1, height: 1, remaining_count: 0 },
      { width: 1, height: 1, remaining_count: 0 },
    ]), false, `${width}x${height}`);
  }
  assert.equal(validSolverResult(solverResult({ inferred_placements: [
    { item_index: 0, x: 0, y: 0, width: 9, height: 5 },
  ] })), true);
  assert.equal(validSolverResult(solverResult({ inferred_placements: [
    { item_index: 0, x: 0, y: 0, width: 5, height: 9 },
  ] })), false, 'a concrete placement must fit without further rotation');
});

test('unread and invalid remaining cards block solving while finished groups remain valid', () => {
  const finished = Array.from({ length: 3 }, () => ({ width: 1, height: 1, remaining_count: 0 }));
  assert.equal(validItems(finished), true);
  for (const change of [{ width: 0 }, { remaining_count: -1 }, { remaining_count: 8 }, { height: 1.5 }]) {
    const items = finished.map((item, index) => index === 0 ? { ...item, ...change } : item);
    assert.equal(validItems(items), false);
    assert.equal(canCalculate(ready({ items })), false);
  }
  assert.equal(validItems(finished.slice(1)), false);
  assert.equal(validItems(finished.map(() => ({ width: 4, height: 4, remaining_count: 1 }))), false);
});

test('all highest-probability unopened cells are highlighted', () => {
  const cells = Array(45).fill('unknown');
  const probabilities = Array(45).fill(0.1);
  probabilities[0] = 1;
  cells[0] = 'item0';
  probabilities[1] = probabilities[2] = 0.7;
  assert.deepEqual([...bestCells(probabilities, cells)], [1, 2]);
  cells[1] = 'completed';
  assert.deepEqual([...bestCells(probabilities, cells)], [2]);
  assert.deepEqual([...bestCells(Array(45).fill(0), cells)], []);
});

test('native footprints remain visible across probability filters and never become best cells', () => {
  const cells = Array(45).fill('unknown');
  cells[0] = 'uncertain';
  const placement = { item_index: 0, x: 0, y: 0, width: 2, height: 1 };
  const snapshot = ready({ cells, partial_placements: [placement] });
  const input = projectSolverSnapshot(snapshot);
  assert.ok(input);
  const result = solverResult({ precision: 'sampled', samples: 100000,
    inferred_placements: [{ item_index: 2, x: 7, y: 3, width: 2, height: 2 }] });
  for (const probabilities of result.probs) {
    probabilities[0] = probabilities[1] = 1;
    probabilities[2] = 0.75;
  }
  for (let mask = 1; mask <= 7; mask += 1) {
    assert.deepEqual([...bestCells(selectedProbabilities(result.probs, mask), input.cells)], [2]);
    assert.deepEqual(uniquePartialPlacements(snapshot), [placement]);
  }
  assert.deepEqual(uniquePartialPlacements(ready()), [], 'WASM placements or sampled 100 percent alone never create native footprints');
});

test('incomplete probability matrices and out-of-board inference are rejected', () => {
  assert.equal(validSolverResult(solverResult()), true);
  assert.equal(validSolverResult(solverResult({ probs: Array(8).fill([0.5]) })), false);
  for (const change of [{ item_index: 3 }, { x: -1 }, { x: 8 }, { y: 4 }, { width: 1.5 }]) {
    assert.equal(validSolverResult(solverResult({ inferred_placements: [
      { item_index: 0, x: 0, y: 0, width: 2, height: 2, ...change },
    ] })), false);
  }
});

test('overlay repeats retain their object and reject old session, round and revision events', () => {
  const current = overlayState({ inferred_placements: [{ item_index: 0, x: 0, y: 0, width: 2, height: 1 }] });
  assert.strictEqual(mergeOverlayState(current, structuredClone(current)), current);
  for (const older of [{ session_id: 11, revision: 100 }, { round_epoch: 1, revision: 100 }, { revision: 3 }]) {
    assert.strictEqual(mergeOverlayState(current, overlayState(older)), current);
  }
  const filtered = overlayState({ inferred_placements: [] });
  assert.strictEqual(mergeOverlayState(current, filtered), filtered);
  const hidden = { ...current, visible: false };
  assert.strictEqual(mergeOverlayState(current, hidden), hidden);
});

test('best-cell emphasis redraws the current version without reviving old or hidden overlays', () => {
  const current = overlayState();
  const emphasized = overlayState({ emphasize_best: true });
  assert.deepEqual(mergeOverlayState(current, emphasized), emphasized);
  assert.strictEqual(mergeOverlayState(emphasized, structuredClone(emphasized)), emphasized);
  for (const older of [{ session_id: 11, revision: 100 }, { round_epoch: 1, revision: 100 }, { revision: 3 }]) {
    assert.strictEqual(mergeOverlayState(emphasized, overlayState({ ...older, emphasize_best: false })), emphasized);
  }
  const hidden = overlayState({ visible: false, emphasize_best: true });
  assert.equal(mergeOverlayState(current, hidden).visible, false);
  assert.equal(mergeOverlayState(hidden, overlayState({ visible: false })).visible, false);
});

test('legacy overlay events default best-cell emphasis to off', () => {
  const { emphasize_best: _legacyMissing, ...legacy } = overlayState();
  const current = overlayState();
  assert.strictEqual(mergeOverlayState(current, legacy), current);
  const changed = mergeOverlayState(overlayState({ emphasize_best: true }), legacy);
  assert.equal(changed.emphasize_best, false);
  assert.deepEqual(changed, current);
});

test('version ordering allows revisions to restart only in newer rounds or sessions', () => {
  assert.equal(compareVersion(version(), version()), 0);
  assert.equal(compareVersion(version({ round_epoch: 3, revision: 0 }), version()), 1);
  assert.equal(compareVersion(version({ session_id: 13, round_epoch: 0, revision: 0 }), version()), 1);
  assert.equal(compareVersion(version({ round_epoch: 1, revision: 100 }), version()), -1);
});

test('refresh watchdog echoes preserve identity and cannot revive old versions', () => {
  const current = refreshState();
  assert.strictEqual(mergeRefreshState(null, current), current);
  assert.strictEqual(mergeRefreshState(current, { ...current }), current);
  for (const older of [{ session_id: 11, revision: 100 }, { round_epoch: 1, revision: 100 }, { revision: 3 }]) {
    assert.strictEqual(mergeRefreshState(current, refreshState(older)), current);
  }
  for (const change of [{ refreshing: true }, { enabled: false }, { attention: true }, { message: '读取失败' }]) {
    const incoming = refreshState(change);
    assert.strictEqual(mergeRefreshState(current, incoming), incoming);
  }
});

test('refresh locks before native progress but permits a manual retry after errors', () => {
  const state = refreshState();
  assert.equal(canRequestRefresh(false, state, false), false);
  assert.equal(canRequestRefresh(true, null, false), false);
  assert.equal(canRequestRefresh(true, state, true), false);
  assert.equal(canRequestRefresh(true, refreshState({ refreshing: true }), false), false);
  assert.equal(canRequestRefresh(true, refreshState({ enabled: false }), false), false);
  assert.equal(canRequestRefresh(true, refreshState({ attention: true, message: '读取失败' }), false), true);
  assert.equal(refreshProgressObserved(version(), state), false);
  assert.equal(refreshProgressObserved(version(), refreshState({ revision: 3, refreshing: true })), false);
  assert.equal(refreshProgressObserved(version(), refreshState({ refreshing: true })), true);
  assert.equal(refreshProgressObserved(version(), refreshState({ revision: 5 })), true);
  assert.equal(refreshProgressObserved(version(), refreshState({ round_epoch: 3, revision: 0 })), true);
});

test('reverse drag stays normalized to the full frame and tiny selections are rejected', () => {
  assert.deepEqual(selectionRect({ x: 0.8, y: 0.9 }, { x: 0.2, y: 0.3 }), {
    x: 0.2, y: 0.3, width: 0.6000000000000001, height: 0.6000000000000001,
  });
  assert.equal(selectionRect({ x: 0.4, y: 0.4 }, { x: 0.41, y: 0.5 }), null);
});

test('an old worker timeout cannot terminate its replacement and completed jobs cancel their timers', (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let generation = 1;
  const oldWorker = { terminated: 0 };
  const newWorker = { terminated: 0 };
  let activeWorker = oldWorker;
  let failures = 0;
  const cancelOld = armSolverTimeout(
    () => generation === 1 && activeWorker === oldWorker,
    () => { oldWorker.terminated += 1; failures += 1; },
  );
  t.mock.timers.tick(1000);
  generation = 2;
  activeWorker = newWorker;
  const cancelNew = armSolverTimeout(
    () => generation === 2 && activeWorker === newWorker,
    () => { newWorker.terminated += 1; failures += 1; },
  );
  t.mock.timers.tick(SOLVER_TIMEOUT_MS - 1000);
  assert.equal(oldWorker.terminated, 0);
  assert.equal(newWorker.terminated, 0);
  assert.equal(failures, 0);
  cancelNew();
  t.mock.timers.tick(1000);
  assert.equal(newWorker.terminated, 0, 'a completed calculation is not reported as timed out');
  cancelOld();
  armSolverTimeout(() => activeWorker === newWorker, () => { newWorker.terminated += 1; failures += 1; });
  t.mock.timers.tick(SOLVER_TIMEOUT_MS);
  assert.equal(newWorker.terminated, 1);
  assert.equal(failures, 1, 'a current timed-out calculation fails exactly once');
});
