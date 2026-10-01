import assert from 'node:assert/strict';
import test from 'node:test';
import { CONTROL_ZOOM_LEVELS, normalizeControlZoom, stepControlZoom } from './zoomPreferences.ts';

test('valid saved zoom is restored, malformed or unsupported preferences use 100%', () => {
  for (const level of CONTROL_ZOOM_LEVELS) assert.equal(normalizeControlZoom(String(level)), level);
  for (const value of [null, undefined, '', 'broken', NaN, Infinity, -1, 0, 7, {}, []]) {
    assert.equal(normalizeControlZoom(value), 1);
  }
});

test('keyboard zoom steps follow the same display presets and stop at their ends', () => {
  assert.equal(stepControlZoom(1, 1), 1.1);
  assert.equal(stepControlZoom(1, -1), 0.9);
  assert.equal(stepControlZoom(0.75, -1), 0.75);
  assert.equal(stepControlZoom(2, 1), 2);
  assert.equal(stepControlZoom(NaN, 1), 1.1);
});
