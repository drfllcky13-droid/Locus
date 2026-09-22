// Adaptive point budget (spike item 1): start at 10M points and follow measured frame time,
// so a fast GPU draws more and integrated graphics settle near what they can hold at 30 fps.

export interface BudgetState {
  points: number;
  /** Exponential moving average of frame time while rendering continuously, ms. */
  avgMs: number;
}

export const BUDGET_START = 10_000_000;
export const BUDGET_MIN = 1_000_000;
export const BUDGET_MAX = 30_000_000;
/** Above this average the budget shrinks; the target is to stay above 30 fps with margin. */
export const SLOW_MS = 28;
/** Below this, and only if the budget is actually limiting, it grows. */
export const FAST_MS = 16;

export function initialBudget(): BudgetState {
  return { points: BUDGET_START, avgMs: 0 };
}

/**
 * Feed one frame interval (only intervals between consecutive rendered frames).
 * `limited` says whether the last node selection stopped because of the budget.
 */
export function updateBudget(s: BudgetState, frameMs: number, limited: boolean): BudgetState {
  const avgMs = s.avgMs === 0 ? frameMs : s.avgMs * 0.9 + frameMs * 0.1;
  let points = s.points;
  if (avgMs > SLOW_MS) points *= 0.97;
  else if (avgMs < FAST_MS && limited) points *= 1.02;
  points = Math.round(Math.min(BUDGET_MAX, Math.max(BUDGET_MIN, points)));
  return { points, avgMs };
}
