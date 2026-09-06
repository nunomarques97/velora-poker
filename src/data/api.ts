import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  DashboardSummary,
  DetectedDir,
  DirValidation,
  HudPosition,
  HudProfile,
  ImportStatus,
  Player,
  SeatTemplate,
  Session,
} from "./types";

export class DesktopAppRequiredError extends Error {
  constructor() {
    super(
      "Real hand history data requires running inside the Velora desktop app window (npm run tauri dev), not a plain browser tab.",
    );
    this.name = "DesktopAppRequiredError";
  }
}

export function isTauriAvailable(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function assertTauriAvailable(): void {
  if (!isTauriAvailable()) {
    throw new DesktopAppRequiredError();
  }
}

// ---------------------------------------------------------------------
// Players
// ---------------------------------------------------------------------

export async function getPlayers(): Promise<Player[]> {
  assertTauriAvailable();
  return invoke<Player[]>("get_players");
}

/** Players seated in the most recently imported hand (the active table) — what the live overlay shows. */
export async function getActiveTablePlayers(): Promise<Player[]> {
  assertTauriAvailable();
  return invoke<Player[]>("get_active_table_players");
}

/** The current active table's max-players count (2/6/9-max), or `null` before any hand is imported. */
export async function getActiveTableMaxPlayers(): Promise<number | null> {
  assertTauriAvailable();
  return invoke<number | null>("get_active_table_max_players");
}

export async function setPlayerColorOverride(
  playerId: string,
  color: string,
  label?: string,
): Promise<Player> {
  assertTauriAvailable();
  return invoke<Player>("set_player_color_override", { playerId, color, label: label ?? null });
}

export async function clearPlayerColorOverride(playerId: string): Promise<Player> {
  assertTauriAvailable();
  return invoke<Player>("clear_player_color_override", { playerId });
}

/**
 * Saves this player's free-text note (Phase E), returning the refreshed
 * player. One note per player, overwritten on each save; a blank note clears
 * it. Whitespace is trimmed on the Rust side.
 */
export async function setPlayerNote(playerId: string, note: string): Promise<Player> {
  assertTauriAvailable();
  return invoke<Player>("set_player_note", { playerId, note });
}

// ---------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------

export async function getDashboardSummary(): Promise<DashboardSummary> {
  assertTauriAvailable();
  return invoke<DashboardSummary>("get_dashboard_summary");
}

// ---------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------

/** Every tracked session, most recent first. */
export async function getSessions(): Promise<Session[]> {
  assertTauriAvailable();
  return invoke<Session[]>("get_sessions");
}

// ---------------------------------------------------------------------
// Hand history import / PokerStars configuration
// ---------------------------------------------------------------------

export async function getImportStatus(): Promise<ImportStatus> {
  assertTauriAvailable();
  return invoke<ImportStatus>("get_import_status");
}

export async function setHandHistoryDir(path: string): Promise<ImportStatus> {
  assertTauriAvailable();
  return invoke<ImportStatus>("set_hand_history_dir", { path });
}

export async function detectPokerStarsDirs(): Promise<DetectedDir[]> {
  assertTauriAvailable();
  return invoke<DetectedDir[]>("detect_pokerstars_dirs");
}

export async function validateHandHistoryDir(path: string): Promise<DirValidation> {
  assertTauriAvailable();
  return invoke<DirValidation>("validate_hand_history_dir", { path });
}

export async function pickFolderDialog(): Promise<string | null> {
  assertTauriAvailable();
  return invoke<string | null>("pick_folder_dialog");
}

// ---------------------------------------------------------------------
// App settings / onboarding
// ---------------------------------------------------------------------

export async function getAppSettings(): Promise<AppSettings> {
  assertTauriAvailable();
  return invoke<AppSettings>("get_app_settings");
}

export async function completeOnboarding(pokerRoom: string): Promise<void> {
  assertTauriAvailable();
  return invoke("complete_onboarding", { pokerRoom });
}

export async function resetOnboarding(): Promise<void> {
  assertTauriAvailable();
  return invoke("reset_onboarding");
}

// ---------------------------------------------------------------------
// HUD profiles
// ---------------------------------------------------------------------

export async function getHudProfiles(): Promise<HudProfile[]> {
  assertTauriAvailable();
  return invoke<HudProfile[]>("get_hud_profiles");
}

export async function getActiveHudProfile(): Promise<HudProfile> {
  assertTauriAvailable();
  return invoke<HudProfile>("get_active_hud_profile");
}

export async function setActiveHudProfile(id: string): Promise<HudProfile> {
  assertTauriAvailable();
  return invoke<HudProfile>("set_active_hud_profile", { id });
}

export async function setHudProfileMinHands(id: string, minHands: number): Promise<HudProfile> {
  assertTauriAvailable();
  return invoke<HudProfile>("set_hud_profile_min_hands", { id, minHands });
}

// ---------------------------------------------------------------------
// Overlay window + HUD positions
// ---------------------------------------------------------------------

export async function openOverlay(): Promise<void> {
  assertTauriAvailable();
  return invoke("open_overlay");
}

export async function closeOverlay(): Promise<void> {
  assertTauriAvailable();
  return invoke("close_overlay");
}

export async function isOverlayOpen(): Promise<boolean> {
  assertTauriAvailable();
  return invoke<boolean>("is_overlay_open");
}

export async function setOverlayClickThrough(enabled: boolean): Promise<void> {
  assertTauriAvailable();
  return invoke("set_overlay_click_through", { enabled });
}

export async function saveHudPosition(playerId: string, x: number, y: number): Promise<void> {
  assertTauriAvailable();
  return invoke("save_hud_position", { playerId, x, y });
}

export async function getHudPositions(): Promise<HudPosition[]> {
  assertTauriAvailable();
  return invoke<HudPosition[]>("get_hud_positions");
}

// ---------------------------------------------------------------------
// Seat-mapping templates (Phase E)
// ---------------------------------------------------------------------

export async function getSeatTemplates(maxPlayers: number): Promise<SeatTemplate[]> {
  assertTauriAvailable();
  return invoke<SeatTemplate[]>("get_seat_templates", { maxPlayers });
}

export async function saveSeatTemplate(
  maxPlayers: number,
  seat: number,
  x: number,
  y: number,
): Promise<void> {
  assertTauriAvailable();
  return invoke("save_seat_template", { maxPlayers, seat, x, y });
}

export async function setAutoCenterEnabled(enabled: boolean): Promise<void> {
  assertTauriAvailable();
  return invoke("set_auto_center_enabled", { enabled });
}

export interface TableDetectionStatus {
  detected: boolean;
  /**  debug evidence — see the notes decision  for the window-following bug this was added to diagnose. */
  hooksInstalled: number;
  eventCallbacksTotal: number;
  eventCallbacksMatched: number;
  pollTicks: number;
  appIntegrityLevel: string | null;
  tableIntegrityLevel: string | null;
}

/** Whether the PokerStars table window is currently found and being tracked. */
export async function getTableDetectionStatus(): Promise<TableDetectionStatus> {
  assertTauriAvailable();
  return invoke<TableDetectionStatus>("get_table_detection_status");
}

// ---------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------

/**
 * Plain-text self-serve diagnostics dump — which table(s)/hand the overlay
 * currently considers active and where that came from, recent overlay
 * refresh/resync events, the last imported hands and their table
 * identifiers, and current watcher/import status. Meant to be copied and
 * pasted back to the maintainer when something "feels wrong" during play, without
 * the user needing to characterize the bug himself.
 */
export async function getDiagnosticsReport(): Promise<string> {
  assertTauriAvailable();
  return invoke<string>("get_diagnostics_report");
}

// ---------------------------------------------------------------------
// Live events
// ---------------------------------------------------------------------

/** Fires whenever the watcher or a manual folder change imports new hands. */
export async function onHandsImported(callback: () => void): Promise<UnlistenFn> {
  return listen<number>("hands-imported", () => callback());
}

/**
 * Fires whenever the overlay window is shown or hidden, from whichever window
 * triggered it — including the overlay's own "Close" button. Emitted by
 * `overlay::open`/`overlay::close` in Rust, so this is the single source of
 * truth for overlay visibility and no window has to keep a flag in sync by
 * hand (Phase E polish, a known issue).
 */
export async function onOverlayVisibilityChanged(
  callback: (open: boolean) => void,
): Promise<UnlistenFn> {
  return listen<boolean>("overlay-visibility-changed", (event) => callback(event.payload));
}
