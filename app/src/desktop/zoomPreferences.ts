// Display presets for the control window. These do not change game geometry.
export const CONTROL_ZOOM_LEVELS = [0.75, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2] as const;
export const CONTROL_ZOOM_STORAGE_KEY = 'ba-control-zoom';
export const DEFAULT_CONTROL_ZOOM = 1;

export function normalizeControlZoom(value: unknown): number {
  const numeric = typeof value === 'string' && value.trim() !== '' ? Number(value) : value;
  return typeof numeric === 'number' && CONTROL_ZOOM_LEVELS.some((level) => level === numeric)
    ? numeric
    : DEFAULT_CONTROL_ZOOM;
}

export function stepControlZoom(value: number, direction: -1 | 1): number {
  const index = CONTROL_ZOOM_LEVELS.findIndex((level) => level === normalizeControlZoom(value));
  return CONTROL_ZOOM_LEVELS[Math.max(0, Math.min(CONTROL_ZOOM_LEVELS.length - 1, index + direction))];
}
