export const SOLVER_TIMEOUT_MS = 15_000;

/** A queued old timeout cannot fail or terminate a replacement worker. */
export function armSolverTimeout(isCurrent: () => boolean, onTimeout: () => void): () => void {
  let active = true;
  const timer = setTimeout(() => {
    if (!active || !isCurrent()) return;
    active = false;
    onTimeout();
  }, SOLVER_TIMEOUT_MS);
  return () => {
    active = false;
    clearTimeout(timer);
  };
}
