import init, { solve_snapshot } from '../../public/wasm/wasm_solver';
import type { SolverRequest, SolverResult } from './contracts.ts';

// One initialization promise per worker, including concurrent queued messages.
let initialization: Promise<unknown> | null = null;

self.addEventListener('message', async (event: MessageEvent<SolverRequest>) => {
  const { session_id, round_epoch, revision, input } = event.data;
  try {
    initialization ??= init();
    await initialization;
    const result = solve_snapshot(input) as SolverResult;
    self.postMessage({ ...result, session_id, round_epoch, revision });
  } catch (cause) {
    self.postMessage({
      session_id,
      round_epoch,
      revision,
      probs: [],
      error: cause instanceof Error ? cause.message : String(cause),
      precision: null,
      total_patterns: '0',
      samples: 0,
      inferred_placements: [],
    });
  }
});

export {};
