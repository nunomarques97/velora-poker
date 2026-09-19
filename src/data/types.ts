/**
 * `null` means "no opportunity" — the stat's denominator was zero (e.g. a
 * player who never faced a 3-bet), not a real 0% — and must render as
 * insufficient-data (an em dash), never as a fabricated 0.
 */
export interface PlayerStats {
  vpip: number | null;
  pfr: number | null;
  threeBet: number | null;
  foldToThreeBet: number | null;
  cBet: number | null;
  foldToCBet: number | null;
  aggressionFactor: number | null;
  wtsd: number | null;
  wsd: number | null;
}

export type PlayerClassificationKind =
  | "unknown"
  | "loosePassive"
  | "looseAggressive"
  | "tightAggressive"
  | "maniac"
  | "nittyRock"
  | "recreational";

export type RuleCategory = "tendency" | "exploit";

export type ConfidenceTier = "high" | "medium" | "low" | "insufficientData";

/** One stat that fed a `RuleResult`'s conclusion. */
export interface Evidence {
  statName: string;
  value: number | null;
  opportunities: number;
}

/**
 * Structured rule result for the HUD click-popup / player profile drawer.
 * Replaces the old bare
 * `{ text, confidence }` pair — `confidencePct`/`confidenceTier` are
 * per-conclusion, never a single global player score.
 */
export interface RuleResult {
  ruleId: string;
  category: RuleCategory;
  conclusion: string;
  confidencePct: number | null;
  confidenceTier: ConfidenceTier;
  evidence: Evidence[];
}

export interface ClassificationResult {
  classification: PlayerClassificationKind;
  label: string;
  color: string;
  isOverride: boolean;
  /**
   * False only when the automatic classifier isn't compiled into this build
   * (`auto-classification` off) and there's no manual override — distinct
   * from a genuine Unknown (below `minHands`, or no rule matched). The
   * PROFILE section must render "Classification unavailable in this build"
   * rather than a normal Unknown badge when this is false. TENDENCIES/
   * EXPLOITS/CONFIDENCE never read this field (they are independent of the build flag).
   */
  available: boolean;
}

export interface PlayerSnapshot {
  stack: number;
  bigBlind: number;
  stackBb: number;
  currency: string;
  format: "cash" | "tournament";
}

export interface Player {
  id: string;
  name: string;
  hands: number;
  stats: PlayerStats;
  /**
   * Structured rule results for the player profile drawer's TENDENCIES/
   * EXPLOITS/CONFIDENCE sections, sorted by confidence
   * descending. Empty when the `strategic-analysis` build flag is off, or
   * when no rule cleared its opportunity floor — each section renders its
   * own empty state for either case, never a blank or fabricated line.
   * Independent of `classification` below (see `ClassificationResult.available`).
   */
  descriptions?: RuleResult[];
  /** Free-text note the user wrote about this player, or `null` when none. Not shown on the HUD overlay. */
  note?: string | null;
  classification?: ClassificationResult;
  snapshot?: PlayerSnapshot | null;
  /** This player's absolute PokerStars seat at the currently active table. `null`/absent outside that context (e.g. the Players view). */
  seat?: number | null;
  /**
   * The same seat rotated so the hero sits at offset 0 — the key a
   * saved HUD card position is stored under, because PokerStars' "Auto-Center
   * me" makes the hero, not any absolute seat number, the fixed screen
   * anchor. `null` when the active hand has no recorded hero or table size.
   */
  seatOffset?: number | null;
}

export interface SessionsTodaySummary {
  sessionCount: number;
  totalDurationSecs: number;
  totalHands: number;
  /** `null` unless at least one cash session played today has a known net result. */
  netResultCash: number | null;
  currency: string | null;
}

export interface DashboardSummary {
  handsPlayed: number;
  playersTracked: number;
  currentHudProfile: string;
  /** `null` when no session has been played today. */
  sessionsToday: SessionsTodaySummary | null;
}

export interface Session {
  startAt: string;
  endAt: string;
  durationSecs: number;
  handCount: number;
  tableCount: number;
  hasCash: boolean;
  hasTournament: boolean;
  /**
   * Net cash result for this session, or `null` when unknown — always
   * `null` for a tournament-only session, since PokerStars hand-history text
   * has no buy-in/finish/payout to compute one from.
   */
  netResultCash: number | null;
  currency: string | null;
}

export interface MockHudProfile {
  id: string;
  name: string;
  description: string;
  active: boolean;
}

export interface StatPage {
  id: string;
  label: string;
  statKeys: (keyof PlayerStats)[];
}

export type HudVisualModel = "velora_hud" | "velora_classic" | "minimal" | "jivaro";

export interface HudProfile {
  id: string;
  name: string;
  visualModel: HudVisualModel;
  statPages: StatPage[];
  minHands: number;
  isBuiltin: boolean;
}

/**
 * `x`/`y` are fractions (0..1) of the overlay window, not
 * absolute screen pixels — the overlay window itself tracks the PokerStars
 * table window, so a saved fraction stays correct as the table moves/resizes.
 */
export interface HudPosition {
  playerId: string;
  x: number;
  y: number;
}

/**
 * One calibrated card position for a given table size (2/6/9-max), reused
 * automatically on every future table of that size. Same 0..1 fraction scheme
 * as `HudPosition`.
 *
 * Keyed by the seat's offset from the hero, not its absolute PokerStars seat
 * number — "Auto-Center me" rotates the display so the hero is the
 * fixed screen anchor, which makes distance-from-hero the only stable key.
 */
export interface SeatTemplate {
  seatOffset: number;
  x: number;
  y: number;
}

export interface AppSettings {
  onboardingComplete: boolean;
  pokerRoom: string | null;
  overlayEnabled: boolean;
  /** User-declared confirmation that PokerStars' "Auto-Center" table option is on — required for automatic seat-mapping templates. */
  autoCenterEnabled: boolean;
}

export interface DetectedDir {
  path: string;
  handFileCount: number;
  screenNames: string[];
}

export interface DirValidation {
  isValid: boolean;
  handFileCount: number;
  message: string;
}

export interface TableSeat {
  seat: number;
  player: Player;
}

export interface ImportStatus {
  configuredDir: string | null;
  handsImported: number;
  lastImportAt: string | null;
  parserStatus: string;
}

/**
 * One import-integrity finding, mirrors `commands::IngestionProblemPayload`.
 * `severity` is `"reject"` or `"warn"`; `count` and the
 * `firstSeenAt`/`lastSeenAt` bracket are aggregated across every hand that hit
 * this exact `code`, recomputed from `import_problems` on every call — never
 * an in-memory counter that a restart could lose or reset.
 */
export interface IngestionProblem {
  severity: string;
  code: string;
  detail: string;
  explanation: string;
  count: number;
  firstSeenAt: string;
  lastSeenAt: string;
}

/**
 * Mirrors `commands::IngestionHealthPayload`, the `get_ingestion_health`
 * response. Every count is a real, always-present number computed
 * from `hands`/`import_problems` — zero is a genuine "nothing rejected", not
 * a placeholder. `lastImportAt` is the only nullable field: `null` means no
 * import activity (hand or problem) has ever been recorded.
 */
export interface IngestionHealth {
  handsImported: number;
  handsRejected: number;
  handsWithWarnings: number;
  problems: IngestionProblem[];
  lastImportAt: string | null;
}

/**
 * Onboarding readiness gate. Mirrors
 * `commands::OnboardingFolderReadiness` field-for-field — `parsedHandCount`
 * is a real test-parse result, never the file count.
 */
export interface OnboardingFolderReadiness {
  path: string | null;
  exists: boolean;
  handFileCount: number;
  parsedHandCount: number;
  message: string;
}

/** Mirrors `commands::OnboardingClientLanguageReadiness`. `checked: false` means no verdict was possible yet (no folder/file), never a guessed pass. */
export interface OnboardingClientLanguageReadiness {
  checked: boolean;
  isEnglish: boolean;
  sampleFile: string | null;
  reason: string;
}

/** Mirrors `commands::OnboardingAutoCenterReadiness` — self-declared, PokerStars gives no way to detect this from the app. */
export interface OnboardingAutoCenterReadiness {
  enabled: boolean;
}

/** Mirrors `commands::OnboardingReadinessPayload`, the `get_onboarding_readiness` response. */
export interface OnboardingReadinessPayload {
  folder: OnboardingFolderReadiness;
  clientLanguage: OnboardingClientLanguageReadiness;
  autoCenter: OnboardingAutoCenterReadiness;
}
