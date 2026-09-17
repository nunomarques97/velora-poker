import type { PlayerStats } from "../data/types";

export const STAT_LABELS: Record<keyof PlayerStats, string> = {
  vpip: "VPIP",
  pfr: "PFR",
  threeBet: "3-Bet",
  foldToThreeBet: "Fold 3B",
  cBet: "C-Bet",
  foldToCBet: "Fold CB",
  aggressionFactor: "AF",
  wtsd: "WTSD",
  wsd: "W$SD",
};

/** Fixed per-column accent colors for the stat row, cycling by position —
 * mirrors the multi-colored stat readout in the HUD reference material. */
export const STAT_COLUMN_COLORS = ["#5fd3a0", "#e0b84a", "#e0724f", "#6f9fff"];

const NON_PERCENT_STATS = new Set<keyof PlayerStats>(["aggressionFactor"]);

/** Shown in place of a percentage/ratio whose denominator was zero — a real
 * "no opportunity yet", never a fabricated 0. */
export const NO_OPPORTUNITY = "–";

export function formatStatValue(key: keyof PlayerStats, stats: PlayerStats): string {
  const value = stats[key];
  if (value === null) {
    return NO_OPPORTUNITY;
  }
  if (NON_PERCENT_STATS.has(key)) {
    return value.toFixed(1);
  }
  return `${value.toFixed(1)}%`;
}

export function formatChips(stack: number, currency: string, format: "cash" | "tournament"): string {
  if (format === "tournament" || currency === "CHIPS") {
    return Math.round(stack).toLocaleString();
  }
  const symbol = currency === "EUR" ? "€" : currency === "GBP" ? "£" : currency === "USD" ? "$" : "";
  return `${symbol}${stack.toFixed(2)}`;
}

export function formatBb(stackBb: number): string {
  return stackBb.toFixed(1);
}

/**
 * Stack-depth color tier for the BB readout, same cutoffs on every HUD
 * model (user decision): under 20bb is short/red, 20-35bb is medium/blue,
 * 35bb or more is deep/green.
 */
export type BbDepthTier = "bbShort" | "bbMedium" | "bbDeep";

export function bbDepthTier(stackBb: number): BbDepthTier {
  if (stackBb < 20) return "bbShort";
  if (stackBb < 35) return "bbMedium";
  return "bbDeep";
}
