import {
  BOARD_SIZE,
  BOARD_COLUMNS,
  BOARD_ROWS,
  type CaptureState,
  type CaptureVersion,
  type CellState,
  type InferredPlacement,
  type ItemSpec,
  type OverlayState,
  type RefreshControlState,
  type Rect,
  type SolverResult,
  type WindowChoice,
} from './contracts.ts';

const knownCells = new Set<CellState>([
  'unknown',
  'empty',
  'completed',
]);

export function chooseWindow(choices: WindowChoice[], previous: string): string {
  if (choices.some(({ hwnd }) => String(hwnd) === previous)) return previous;
  const game = choices.find(({ title }) =>
    (/MuMu安卓设备|MuMuPlayer/i.test(title) && !title.includes('MuMu模拟器')) ||
    /^(?:Blue\s?Archive|ブルーアーカイブ)$/i.test(title.trim()),
  );
  return game === undefined ? '' : String(game.hwnd);
}

function itemFitsBoard(width: number, height: number): boolean {
  return Number.isInteger(width) && width >= 1 &&
    Number.isInteger(height) && height >= 1 &&
    ((width <= BOARD_COLUMNS && height <= BOARD_ROWS) ||
      (height <= BOARD_COLUMNS && width <= BOARD_ROWS));
}

export function validItems(items: ItemSpec[]): boolean {
  return (
    items.length === 3 &&
    items.every(
      ({ width, height, remaining_count }) =>
        itemFitsBoard(width, height) &&
        Number.isInteger(remaining_count) &&
        remaining_count >= 0 &&
        remaining_count <= 7,
    ) &&
    items.reduce((area, item) => area + item.width * item.height * item.remaining_count, 0) <=
      BOARD_SIZE
  );
}

export function canCalculate(state: CaptureState | null): state is CaptureState {
  return (
    state !== null &&
    state.status === 'ready' &&
    // Native busy also covers solving after the ready snapshot was captured.
    state.confirmed &&
    state.board !== null &&
    state.cells.length === BOARD_SIZE &&
    state.cells.every((cell) => knownCells.has(cell)) &&
    validItems(state.items)
  );
}

/** A repeated preview is not a new solver job. */
export function calculationKey(state: CaptureState | null): string | null {
  return canCalculate(state)
    ? `${state.session_id}:${state.round_epoch}:${state.revision}`
    : null;
}

/** Manual mode consumes each refreshed snapshot once, including cancelled jobs. */
export function canStartCalculation(
  state: CaptureState | null,
  last_manual_job: string | null,
): boolean {
  return canCalculate(state) &&
    (state.update_mode !== 'manual' || calculationKey(state) !== last_manual_job);
}

export function isCurrentResult(
  result: CaptureVersion,
  state: CaptureState | null,
): boolean {
  return (
    canCalculate(state) &&
    result.session_id === state.session_id &&
    result.round_epoch === state.round_epoch &&
    result.revision === state.revision
  );
}

export function filterMask(selected: readonly boolean[]): number {
  return selected.reduce(
    (mask, enabled, index) => (enabled && index < 3 ? mask | (1 << index) : mask),
    0,
  );
}

export function selectedProbabilities(
  probs: number[][],
  mask: number,
): number[] | null {
  const row = probs[mask];
  if (
    !Number.isInteger(mask) ||
    mask < 1 ||
    mask > 7 ||
    row?.length !== BOARD_SIZE ||
    !row.every((prob) => Number.isFinite(prob) && prob >= 0 && prob <= 1)
  ) {
    return null;
  }
  return row;
}

export function bestCells(
  probabilities: readonly number[],
  cells: readonly CellState[],
): Set<number> {
  const unknown = probabilities
    .map((probability, index) => ({ probability, index }))
    .filter(({ probability, index }) =>
      cells[index] === 'unknown' && Number.isFinite(probability),
    );
  if (unknown.length === 0) return new Set();
  const max = Math.max(...unknown.map(({ probability }) => probability));
  if (max <= 0) return new Set();
  return new Set(
    unknown
      .filter(({ probability }) => Math.abs(probability - max) <= 1e-9)
      .map(({ index }) => index),
  );
}

export function percent(probability: number): string {
  return `${(Math.min(1, Math.max(0, probability)) * 100).toFixed(1)}%`;
}

export function selectionRect(
  start: { x: number; y: number },
  end: { x: number; y: number },
): Rect | null {
  const x = Math.max(0, Math.min(1, Math.min(start.x, end.x)));
  const y = Math.max(0, Math.min(1, Math.min(start.y, end.y)));
  const right = Math.max(0, Math.min(1, Math.max(start.x, end.x)));
  const bottom = Math.max(0, Math.min(1, Math.max(start.y, end.y)));
  if (right - x < 0.025 || bottom - y < 0.025) return null;
  return { x, y, width: right - x, height: bottom - y };
}

export function shouldAcceptCapture(
  current: CaptureState | null,
  incoming: CaptureState,
  active_session: number | null,
): boolean {
  if (active_session !== null && incoming.session_id !== active_session) {
    return false;
  }
  return current === null || compareVersion(incoming, current) >= 0;
}

/** Sessions and round epochs are monotonic; revisions may restart in a new round. */
export function compareVersion(left: CaptureVersion, right: CaptureVersion): number {
  for (const field of ['session_id', 'round_epoch', 'revision'] as const) {
    if (left[field] !== right[field]) return left[field] < right[field] ? -1 : 1;
  }
  return 0;
}

function validInferredPlacement(placement: InferredPlacement): boolean {
  return Number.isInteger(placement.item_index) && placement.item_index >= 0 && placement.item_index < 3 &&
    Number.isInteger(placement.x) && placement.x >= 0 &&
    Number.isInteger(placement.y) && placement.y >= 0 &&
    Number.isInteger(placement.width) && placement.width >= 1 &&
    Number.isInteger(placement.height) && placement.height >= 1 &&
    placement.x + placement.width <= BOARD_COLUMNS && placement.y + placement.height <= BOARD_ROWS;
}

export function validSolverResult(result: SolverResult): boolean {
  return result.error === '' && (result.precision === 'exact' || result.precision === 'sampled') &&
    result.probs.length === 8 && result.probs.every((row) =>
      row.length === BOARD_SIZE && row.every((prob) => Number.isFinite(prob) && prob >= 0 && prob <= 1),
    ) && Array.isArray(result.inferred_placements) && result.inferred_placements.every(validInferredPlacement);
}

/** Only the solver can prove a unique placement, including when probabilities are sampled. */
export function selectedInferredPlacements(result: SolverResult, mask: number): InferredPlacement[] {
  if (!validSolverResult(result) || !Number.isInteger(mask) || mask < 1 || mask > 7) return [];
  return result.inferred_placements.filter((placement) => Boolean(mask & (1 << placement.item_index)));
}

export function mergeOverlayState(
  current: OverlayState,
  incoming: Omit<OverlayState, 'emphasize_best'> & { emphasize_best?: boolean },
): OverlayState {
  if (compareVersion(incoming, current) < 0) return current;
  const emphasizeBest = incoming.emphasize_best === true;
  const identical = compareVersion(incoming, current) === 0 &&
    current.visible === incoming.visible && current.precision === incoming.precision && current.message === incoming.message &&
    current.emphasize_best === emphasizeBest &&
    current.probabilities.length === incoming.probabilities.length && current.cells.length === incoming.cells.length &&
    current.inferred_placements.length === incoming.inferred_placements.length &&
    current.probabilities.every((prob, index) => prob === incoming.probabilities[index]) &&
    current.cells.every((cell, index) => cell === incoming.cells[index]) &&
    current.inferred_placements.every((placement, index) => {
      const other = incoming.inferred_placements[index];
      return placement.item_index === other.item_index && placement.x === other.x && placement.y === other.y &&
        placement.width === other.width && placement.height === other.height;
    });
  if (identical) return current;
  return incoming.emphasize_best === emphasizeBest
    ? incoming as OverlayState
    : { ...incoming, emphasize_best: emphasizeBest };
}

export function mergeRefreshState(current: RefreshControlState | null, incoming: RefreshControlState): RefreshControlState {
  if (current === null) return incoming;
  const order = compareVersion(incoming, current);
  if (order < 0) return current;
  if (order === 0 && current.refreshing === incoming.refreshing && current.enabled === incoming.enabled &&
    current.attention === incoming.attention && current.message === incoming.message) return current;
  return incoming;
}

export function canRequestRefresh(native: boolean, state: RefreshControlState | null, pending: boolean): boolean {
  return native && state !== null && state.enabled && !state.refreshing && !pending;
}

/** An idle watchdog echo cannot release a request before its busy/completion event. */
export function refreshProgressObserved(request: CaptureVersion, state: RefreshControlState): boolean {
  const order = compareVersion(state, request);
  return order > 0 || (order === 0 && state.refreshing);
}
