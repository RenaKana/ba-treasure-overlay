export const BOARD_COLUMNS = 9;
export const BOARD_ROWS = 5;
export const BOARD_SIZE = BOARD_COLUMNS * BOARD_ROWS;

/** Coordinates are normalized against the complete captured frame. */
export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface ItemSpec {
  width: number;
  height: number;
  remaining_count: number;
}

/** Coordinates are zero-based cells within the 9 by 5 board. */
export interface GridPlacement {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface CompletedObject extends GridPlacement {
  item_index: number | null;
}

export interface PlacementConstraint {
  anchor: number;
  item_index: number;
  placements: GridPlacement[];
}

export interface InferredPlacement extends GridPlacement {
  item_index: number;
}

export interface CaptureVersion {
  session_id: number;
  round_epoch: number;
  revision: number;
}

export type CellState =
  | 'unknown'
  | 'empty'
  | 'item0'
  | 'item1'
  | 'item2'
  | 'completed'
  | 'uncertain';

export type ObservedCell = Exclude<CellState, 'uncertain'>;
export type UpdateMode = 'manual' | 'auto';
export type CaptureStatus =
  | 'manual'
  | 'searching'
  | 'ready'
  | 'uncertain'
  | 'waiting_item'
  | 'paused'
  | 'away'
  | 'error';

export interface CaptureState extends CaptureVersion {
  status: CaptureStatus;
  update_mode: UpdateMode;
  refreshing: boolean;
  captured_at_ms: number | null;
  message: string;
  frame_url: string;
  width: number;
  height: number;
  board: Rect | null;
  /** Absolute source-frame pixels, shared by native vision and OCR. */
  content_rect_px?: [number, number, number, number] | null;
  cells: CellState[];
  items: ItemSpec[];
  completed_objects: CompletedObject[];
  candidate_constraints: PlacementConstraint[];
  reference_ready: [boolean, boolean, boolean];
  card_fingerprints: [string | null, string | null, string | null];
  finish: [boolean, boolean, boolean];
  remaining: number | null;
  round: string | null;
  confirmed: boolean;
}

export interface WindowChoice {
  hwnd: number;
  title: string;
}

export type Precision = 'exact' | 'sampled';

export interface SolverInput {
  items: ItemSpec[];
  cells: ObservedCell[];
  candidate_constraints: PlacementConstraint[];
}

export interface SolverResult {
  probs: number[][];
  error: string;
  precision: Precision | null;
  total_patterns: string;
  samples: number;
  inferred_placements: InferredPlacement[];
}

export interface SolverRequest extends CaptureVersion {
  input: SolverInput;
}

export interface SolverResponse extends SolverResult, CaptureVersion {}

export interface OverlayResult extends CaptureVersion {
  probabilities: number[];
  cells: CellState[];
  precision: Precision;
  message: string;
  inferred_placements: InferredPlacement[];
  emphasize_best: boolean;
}

export interface OverlayState extends CaptureVersion {
  probabilities: number[];
  cells: CellState[];
  precision: string;
  message: string;
  visible: boolean;
  inferred_placements: InferredPlacement[];
  emphasize_best: boolean;
}

export interface RefreshControlState extends CaptureVersion {
  refreshing: boolean;
  enabled: boolean;
  attention: boolean;
  message: string;
}
