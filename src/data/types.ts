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

/** One stat that fed a `RuleResult`. */
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
  /** What the opponent does. Never carries advice. */
  observation: string;
  /** What to do about it, addressed to the reader. Never carries the read. */
  advice: string;
  confidencePct: number | null;
  confidenceTier: ConfidenceTier;
  evidence: Evidence[];
}

// ---------------------------------------------------------------------
// Opponent engine (`docs/specs/opponent-engine.md`, section 13). Present
// only in the `strategic-analysis` build; `Player.engine` is `null` in the
// default build.
// ---------------------------------------------------------------------

/** Payload family of an engine read. */
export type EngineFamily =
  | "preflop"
  | "stack"
  | "context"
  | "stage"
  | "bounty"
  | "postflop"
  | "h2h"
  | "showdown"
  | "recency";

/** An `Evidence` item with the successes and the shrunk value (percentages). */
export interface EngineEvidence extends Evidence {
  hits: number;
  /** Shrunk percentage; `null` for counts that are not shrunk (showdown tallies, context facts). */
  shrunk: number | null;
}

/** What a read's sample counts: whole hands, or the times its spot came up. */
export type SampleUnit = "hands" | "opportunities";

/** The sample a read's confidence rests on (its weakest basis stat). */
export interface ReadSample {
  count: number;
  unit: SampleUnit;
}

/** One engine read: the `RuleResult` contract plus the engine's fields. */
export interface EngineRead extends RuleResult {
  evidence: EngineEvidence[];
  /** `null` for facts of the latest hand (stack depth, bounty), which have no sample. */
  sample: ReadSample | null;
  /** Catalogue row of the spec (e.g. `P06`). */
  scenarioId: string;
  family: EngineFamily;
  /** The read's own chip tag (`F3B`, `12bb`), or `null`. */
  tag: string | null;
  /** `deviation × confidence × multiplier`; reads are ranked by it. */
  score: number;
}

/** Why the chip shows its tag: tilt first, then a short tournament stack, then the top read. */
export type ChipTagSource = "tilt" | "stack" | "read";

export interface ChipTag {
  /** 2–4 characters (`F3B`, `ST-`, `TILT`) or a stack (`9bb`, `25bb`). */
  text: string;
  ruleId: string;
  source: ChipTagSource;
}

export type EngineFormat = "mtt" | "cash" | "zoom" | "spin";
export type StackBucket = "push_fold" | "reshove" | "mid" | "standard" | "deep";
export type TournamentStage = "early" | "middle" | "late";

export interface BountyContext {
  amount: number;
  /** ISO code (`EUR`, `USD`, `GBP`) when recognised. */
  currency: string | null;
  /** Current bounty / initial bounty; `null` when the buy-in has no bounty component. */
  ratio: number | null;
  /** `null` when the hero was not seated. */
  heroCovers: boolean | null;
}

export interface SeatRelation {
  /** Occupied seats counted clockwise from the hero (1 = directly on the hero's left). */
  distance: number;
  side: "left" | "right";
  actsAfterHero: boolean;
  directLeft: boolean;
  directRight: boolean;
}

/** Between-hands context, read from the latest completed hand at the table. */
export interface EngineContext {
  /** PokerStars' hand number the context was read from. */
  sourceHandId: string;
  variant: string | null;
  format: EngineFormat;
  effectiveStackBb: number | null;
  stackBucket: StackBucket | null;
  /** Tournaments only. */
  stage: TournamentStage | null;
  level: number | null;
  avgStackBb: number | null;
  bounty: BountyContext | null;
  /** `null` when the hero was not seated. */
  seat: SeatRelation | null;
}

export type HeadToHeadKey =
  | "three_bet_vs_hero_open"
  | "fold_to_hero_3bet"
  | "fold_to_hero_cbet"
  | "steal_vs_hero"
  | "defend_vs_hero_steal";

export interface HeadToHeadStat {
  key: HeadToHeadKey;
  hits: number;
  opportunities: number;
  rawPct: number | null;
  shrunkPct: number;
}

export interface HeadToHead {
  /** Hands the villain and the hero were both dealt into. */
  hands: number;
  /** Only stats with at least 8 opportunities. */
  stats: HeadToHeadStat[];
}

export type SizeBucket = "small" | "medium" | "large" | "overbet" | "allin";

export interface ShowdownLineStep {
  street: "preflop" | "flop" | "turn" | "river";
  action: "fold" | "check" | "call" | "bet" | "raise";
  isAllIn: boolean;
  /** Bets and raises only. */
  sizeBucket: SizeBucket | null;
  potFraction: number | null;
}

export interface ShowdownRecord {
  /** PokerStars' hand number. */
  handId: string;
  playedAt: string | null;
  /** `"Jd 9d"`. */
  cards: string;
  /** `"Ts 8h 2c 7d Ks"`. */
  board: string;
  category: string;
  line: ShowdownLineStep[];
  result: "won" | "lost" | "split";
  lastAggression: {
    street: "flop" | "turn" | "river";
    sizeBucket: SizeBucket;
    potFraction: number;
    class: "value" | "bluff" | "neither";
  } | null;
}

export interface SizingTell {
  bucket: SizeBucket;
  value: number;
  bluff: number;
  neither: number;
  n: number;
}

export interface RecentForm {
  window: number;
  windowHits: number;
  baselineOpportunities: number;
  windowVpipPct: number;
  baselineVpipPct: number;
  z: number;
  flag: "looser" | "tighter" | "tilt" | null;
  afterBigLoss: boolean;
}

export interface AutoNote {
  id: number;
  /** PokerStars' hand number. */
  handId: string;
  kind: string;
  text: string;
  createdAt: string;
  /** Always `"auto"`: keeps auto-notes apart from the editable manual note. */
  source: "auto";
}

/** `Player.engine` — the opponent engine's payload (contract version 1). */
export interface EnginePayload {
  version: number;
  tag: ChipTag | null;
  /** The first two of `reads`. */
  topReads: EngineRead[];
  /** Every eligible read, ranked by score. */
  reads: EngineRead[];
  /** `null` outside a table scope (Players list). */
  context: EngineContext | null;
  headToHead: HeadToHead | null;
  /** Newest first, at most 10. */
  showdowns: ShowdownRecord[];
  sizingTells: SizingTell[];
  recentForm: RecentForm | null;
  autoNotes: AutoNote[];
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
   * EXPLOITS/CONFIDENCE sections: the opponent engine's reads in ranking
   * order (`engine.reads` as plain `RuleResult`s). Empty when the
   * `strategic-analysis` build flag is off, or when no read cleared its
   * sample — each section renders its own empty state for either case, never
   * a blank or fabricated line. Independent of `classification` below (see
   * `ClassificationResult.available`).
   */
  descriptions?: RuleResult[];
  /**
   * The opponent engine's payload: chip tag, ranked reads, context,
   * head-to-head, showdowns. `null` in the default build, where the chip
   * renders exactly as before.
   */
  engine?: EnginePayload | null;
  /** Free-text note the user wrote about this player, or `null` when none. Not shown on the HUD overlay. */
  note?: string | null;
  classification?: ClassificationResult;
  snapshot?: PlayerSnapshot | null;
  /** This player's absolute PokerStars seat at the currently active table. `null`/absent outside that context (e.g. the Players view). */
  seat?: number | null;
  /**
   * The same seat rotated so the hero sits at offset 0 — the seat key of the
   * `hero` frame, because PokerStars' "Auto-Center me" makes the hero, not
   * any absolute seat number, the fixed screen anchor. `null` when the active
   * hand has no recorded hero or table size.
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

/**
 * The two HUD designs the overlay renders. `compact` is one line of stats per
 * player; `badge` is initials plus a hand count, with the stats one click
 * away. The backend migrated every retired model to `compact`; the overlay
 * also renders anything unexpected as `compact`.
 */
export type HudVisualModel = "compact" | "badge";

export interface HudProfile {
  id: string;
  name: string;
  visualModel: HudVisualModel;
  statPages: StatPage[];
  minHands: number;
  isBuiltin: boolean;
}

/**
 * How a saved chip position is keyed. `hero`: the seat's offset from the hero
 * (PokerStars' "Auto-Center me" on, so the hero is always bottom centre).
 * `absolute`: PokerStars' own seat number (Auto-Center off).
 */
export type SeatFrame = "hero" | "absolute";

/**
 * A chip centre the user dragged, shared by every table of one size in one
 * frame. `x`/`y` are fractions (0..1) of the overlay window, which tracks the
 * PokerStars table window, so the position follows the table as it moves or
 * resizes. Seats without one use the overlay's computed default.
 */
export interface SeatPosition {
  /** Seat offset from the hero in the `hero` frame, PokerStars seat number in `absolute`. */
  seatKey: number;
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

/** The side-panel global shortcut (`get_panel_shortcut`/`set_panel_shortcut`). */
export interface PanelShortcut {
  /** The chosen shortcut, canonical ("Ctrl+Alt+P"). */
  shortcut: string;
  /** Whether that shortcut is registered with Windows right now. */
  registered: boolean;
  defaultShortcut: string;
  /** The fixed HUD toggle, which the panel shortcut may not take. */
  hudShortcut: string;
  /** Why the shortcut chosen before this launch is not working, if it isn't. */
  error: string | null;
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

// ---------------------------------------------------------------------
// Side panel (`get_side_panel_snapshot`, spec section 13)
// ---------------------------------------------------------------------

export type TableQualityLabel = "soft" | "average" | "tough";

/** Mirrors `engine::TableQuality`: 0 (tough) to 100 (soft), 50 neutral. */
export interface TableQuality {
  score: number;
  label: TableQualityLabel;
  /** The villains' hands in total: the sample behind the score. */
  basisHands: number;
}

/** Mirrors `commands::SidePanelRead`: a villain's top-ranked read. */
export interface SidePanelRead {
  ruleId: string;
  observation: string;
  advice: string;
  confidencePct: number | null;
}

/** Mirrors `commands::SidePanelVillain`. */
export interface SidePanelVillain {
  playerId: string;
  name: string;
  seat: number | null;
  /** Hands dealt into, all-time. */
  hands: number;
  /** Chip tag text; `null` in the default build or when there is no tag. */
  tag: string | null;
  /** `null` in the default build or when no read cleared its sample. */
  topRead: SidePanelRead | null;
  /** Other tracked tables where this villain is seated, ascending. Empty when none. */
  otherTableIds: number[];
}

/** Mirrors `commands::SidePanelTable`: one tracked table. */
export interface SidePanelTable {
  tableId: number;
  /** Parsed from the window title; `null` when the title did not parse. */
  tableName: string | null;
  /** `null` before any hand is imported at this table. */
  maxPlayers: number | null;
  /** Players dealt into the table's latest hand, the hero included. */
  playerCount: number;
  /** `null` in the default build and below the spec's sample. */
  quality: TableQuality | null;
  /** Non-hero players of the latest hand, most hands first. */
  villains: SidePanelVillain[];
}

/** Mirrors `commands::SidePanelSnapshot`, the `get_side_panel_snapshot` response. */
export interface SidePanelSnapshot {
  generatedAt: string;
  tables: SidePanelTable[];
}
