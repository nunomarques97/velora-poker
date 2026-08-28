export interface PlayerStats {
  vpip: number;
  pfr: number;
  threeBet: number;
  foldToThreeBet: number;
  cBet: number;
  foldToCBet: number;
  aggressionFactor: number;
  wtsd: number;
  wsd: number;
}

export type PlayerClassificationKind =
  | "unknown"
  | "loosePassive"
  | "looseAggressive"
  | "tightAggressive"
  | "maniac"
  | "recreational";

export interface ClassificationResult {
  classification: PlayerClassificationKind;
  label: string;
  color: string;
  isOverride: boolean;
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
  note?: string;
  classification?: ClassificationResult;
  snapshot?: PlayerSnapshot | null;
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

export type HudVisualModel = "velora_hud" | "velora_classic" | "minimal";

export interface HudProfile {
  id: string;
  name: string;
  visualModel: HudVisualModel;
  statPages: StatPage[];
  minHands: number;
  isBuiltin: boolean;
}

export interface HudPosition {
  playerId: string;
  x: number;
  y: number;
}

export interface AppSettings {
  onboardingComplete: boolean;
  pokerRoom: string | null;
  overlayEnabled: boolean;
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
