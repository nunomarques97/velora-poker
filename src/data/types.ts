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

export interface Player {
  id: string;
  name: string;
  hands: number;
  stats: PlayerStats;
  note?: string;
}

export interface DashboardSummary {
  sessionsToday: number;
  handsPlayed: number;
  playersTracked: number;
  currentHudProfile: string;
}

export interface Session {
  id: string;
  date: string;
  stakes: string;
  tables: number;
  hands: number;
  duration: string;
  result: string;
  positive: boolean;
}

export interface HudProfile {
  id: string;
  name: string;
  description: string;
  active: boolean;
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
