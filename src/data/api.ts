import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  DetectedDir,
  DirValidation,
  HudPosition,
  HudProfile,
  ImportStatus,
  Player,
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
// Live events
// ---------------------------------------------------------------------

/** Fires whenever the watcher or a manual folder change imports new hands. */
export async function onHandsImported(callback: () => void): Promise<UnlistenFn> {
  return listen<number>("hands-imported", () => callback());
}
