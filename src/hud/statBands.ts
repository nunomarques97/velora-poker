import type { PlayerStats } from "../data/types";

/**
 * Per-stat value bands, used by the Jivaro visual model to colour a stat and
 * to fill its ring arc.
 *
 * **These thresholds are a PLACEHOLDER awaiting a product decision.** Velora
 * had no value-band colour convention before this model: `STAT_COLUMN_COLORS`
 * in `statFormat.ts` is a *positional* palette (column 1 is always green,
 * column 2 always amber) and carries no meaning about the value itself, and
 * nothing else in the app colours a stat by where it falls. So there was
 * nothing to reuse.
 *
 * What is here was read off the Jivaro reference screenshots rather than
 * invented: the real HUD renders VPIP 14 and 15 red, PFR 6 and 10 amber, and
 * fold-to-3-bet 64-71 green, which fixes the ramp's *direction* (low = red,
 * high = green) but not its cut-offs — a screenshot cannot yield those. The
 * numbers below are one defensible reading of that ramp, not a statistical
 * claim, and should be replaced with real ones.
 *
 * Purely cosmetic in both uses: a band never hides a value, never changes a
 * stat, and never feeds classification — the archetype colour still comes
 * from `classification::resolve_for_player` alone.
 */
export interface StatBand {
  /** Below this value the stat reads as low. */
  low: number;
  /** Above this value the stat reads as high. */
  high: number;
  /** Full-scale value for the ring arc's fill; values above it clamp. */
  scale: number;
}

export const STAT_BANDS: Record<keyof PlayerStats, StatBand> = {
  vpip: { low: 18, high: 30, scale: 60 },
  pfr: { low: 5, high: 18, scale: 45 },
  threeBet: { low: 4, high: 9, scale: 20 },
  foldToThreeBet: { low: 40, high: 60, scale: 100 },
  cBet: { low: 45, high: 70, scale: 100 },
  foldToCBet: { low: 40, high: 60, scale: 100 },
  aggressionFactor: { low: 1.5, high: 3.5, scale: 8 },
  wtsd: { low: 24, high: 32, scale: 60 },
  wsd: { low: 46, high: 56, scale: 100 },
};

/** Band colours, matching the accepted reference mock. */
export const BAND_LOW = "#e03a2b";
export const BAND_MID = "#f5a623";
export const BAND_HIGH = "#6fbf44";
/** Every arc tick and value below the profile's `min_hands` gate. */
export const BAND_UNKNOWN = "#3a3d42";

export function bandColor(key: keyof PlayerStats, value: number | null): string {
  if (value === null) return BAND_UNKNOWN;
  const band = STAT_BANDS[key];
  if (value < band.low) return BAND_LOW;
  if (value > band.high) return BAND_HIGH;
  return BAND_MID;
}

/** 0..1 position of `value` on the stat's arc, clamped at both ends. `null`
 * ("no opportunity" yet) reads as empty rather than a fabricated low value. */
export function bandFraction(key: keyof PlayerStats, value: number | null): number {
  if (value === null) return 0;
  const { scale } = STAT_BANDS[key];
  if (scale <= 0) return 0;
  return Math.min(1, Math.max(0, value / scale));
}

/**
 * The Jivaro model prints integers with no percent sign, which is what the
 * reference does and what keeps three values legible inside a 168px card.
 * Aggression factor is a ratio, not a percentage, so it keeps one decimal.
 * A `null` value (denominator was zero — no opportunity yet) prints as an
 * em dash rather than a fabricated 0.
 */
export function formatBandValue(key: keyof PlayerStats, stats: PlayerStats): string {
  const value = stats[key];
  if (value === null) return "–";
  return key === "aggressionFactor" ? value.toFixed(1) : String(Math.round(value));
}
