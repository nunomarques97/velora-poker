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
  IngestionHealth,
  OnboardingReadinessPayload,
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

export interface PlayersPage {
  players: Player[];
  /** Total players matching `search` (or the whole roster with no search), for a "showing N of TOTAL" / "load more" UI. */
  total: number;
}

/**
 * A bounded, most-played-first page of the roster. Use this instead of
 * `getPlayers()` in any view that renders player cards/rows — `getPlayers()`
 * computes full stats for every tracked player in one call, which is fine at
 * a couple hundred players and multi-second at the thousands a bulk import
 * can produce, and it holds the same connection lock the live overlay refresh
 * needs.
 */
export async function getPlayersPage(
  offset: number,
  limit: number,
  search?: string,
): Promise<PlayersPage> {
  assertTauriAvailable();
  return invoke<PlayersPage>("get_players_page", { offset, limit, search: search ?? null });
}

/**
 * Players seated in the most recently imported hand at one specific tracked
 * table — what that table's own overlay shows. `tableId` comes from the
 * overlay window's own URL; every overlay asks about its own table and
 * no other.
 */
export async function getActiveTablePlayers(tableId: number): Promise<Player[]> {
  assertTauriAvailable();
  return invoke<Player[]>("get_active_table_players", { tableId });
}

/** One tracked table's max-players count (2/6/9-max), or `null` before any hand is imported there. */
export async function getActiveTableMaxPlayers(tableId: number): Promise<number | null> {
  assertTauriAvailable();
  return invoke<number | null>("get_active_table_max_players", { tableId });
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
 * Saves this player's free-text note, returning the refreshed
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

/**
 * Read-only ingestion health: how many hands were read, rejected or
 * flagged with a warning, and why, recomputed from `hands`/`import_problems`
 * on every call. Polled by the main window's persistent status line and by
 * the Settings "Ingestion" section — never a hand dropped without a trace.
 */
export async function getIngestionHealth(): Promise<IngestionHealth> {
  assertTauriAvailable();
  return invoke<IngestionHealth>("get_ingestion_health");
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

/** One tracked PokerStars table window and whether its own HUD overlay is up. */
export interface TrackedTableStatus {
  id: number;
  /** Parsed from the table window's title; `null` when it didn't parse. */
  name: string | null;
  minimized: boolean;
  overlayVisible: boolean;
  /**
   * Dismissed by hand via the overlay's own "Hide" button and not shown
   * again since — distinct from `overlayVisible: false` while a window is
   * merely still starting up.
   */
  dismissed: boolean;
}

export interface OverlayStatus {
  /** Global kill switch. When off, no table gets a HUD however many are open. */
  enabled: boolean;
  tables: TrackedTableStatus[];
}

/**
 * Every tracked table plus whether its HUD is up. Overlays are automatic — one appears for each real table window and disappears with it — so
 * this replaces the old single `isOverlayOpen()` the "Open Overlay" button
 * used to read.
 */
export async function getOverlayStatus(): Promise<OverlayStatus> {
  assertTauriAvailable();
  return invoke<OverlayStatus>("get_overlay_status");
}

/**
 * The global HUD kill switch — a "not right now" for the rest of this
 * session only. The backend forces it back to enabled on every app startup,
 * so a past session's "off" can never silently suppress every HUD on a
 * future launch.
 */
export async function setOverlaysEnabled(enabled: boolean): Promise<void> {
  assertTauriAvailable();
  return invoke("set_overlays_enabled", { enabled });
}

/**
 * Collapses one table's HUD content from its own "Hide" button, leaving every
 * other table's HUD alone. Not the kill switch, and not a
 * window hide either: the overlay window stays up and positioned, only its
 * content collapses to a "Show" pill in the same spot, so bringing it back
 * never requires leaving the overlay. Also reversible from the main window's
 * table list or the global hotkey via `showOverlay`.
 */
export async function closeOverlay(tableId: number): Promise<void> {
  assertTauriAvailable();
  return invoke("close_overlay", { tableId });
}

/**
 * Restores one table's full HUD content after it was collapsed via
 * `closeOverlay` — the overlay's own "Show" pill, the HUD Profiles
 * page's per-table action, or the global hotkey. Leaves every other table
 * and the global switch untouched.
 */
export async function showOverlay(tableId: number): Promise<void> {
  assertTauriAvailable();
  return invoke("show_overlay", { tableId });
}

export async function isOverlayOpen(tableId: number): Promise<boolean> {
  assertTauriAvailable();
  return invoke<boolean>("is_overlay_open", { tableId });
}

/**
 * Whether this table's HUD content is currently collapsed — read once
 * on the overlay's own mount so it knows whether to paint the full HUD or
 * the "Show" pill from the very first render, before any event has fired.
 */
export async function isOverlayDismissed(tableId: number): Promise<boolean> {
  assertTauriAvailable();
  return invoke<boolean>("is_overlay_dismissed", { tableId });
}

/**
 * How an overlay window answers mouse input.
 *
 * - `normal` — table-interactive, and where every overlay opens. Clicks pass
 *   through to the table everywhere except two hot zones that stay clickable
 *   no matter what: that overlay's own control bar and each of its visible
 *   cards' pagination dots.
 * - `reposition` — the whole window captures pointer events so a card can be
 *   dragged from anywhere on it. Nothing on that table is clickable meanwhile.
 *
 * Per table: repositioning one table's cards leaves every other
 * table clickable.
 */
export type OverlayMode = "normal" | "reposition";

export async function setOverlayMode(tableId: number, mode: OverlayMode): Promise<void> {
  assertTauriAvailable();
  return invoke("set_overlay_mode", { tableId, mode });
}

/** Puts every overlay in one mode — the main window's single Reposition control. */
export async function setAllOverlayModes(mode: OverlayMode): Promise<void> {
  assertTauriAvailable();
  return invoke("set_all_overlay_modes", { mode });
}

/** One overlay's live mode, read from native hit-test state rather than any window's own flag. */
export async function getOverlayMode(tableId: number): Promise<OverlayMode> {
  assertTauriAvailable();
  return invoke<OverlayMode>("get_overlay_mode", { tableId });
}

/** `reposition` if any overlay is repositioning — what the main window's one control reads. */
export async function getAnyOverlayMode(): Promise<OverlayMode> {
  assertTauriAvailable();
  return invoke<OverlayMode>("get_any_overlay_mode");
}

/** One always-clickable rectangle, in CSS pixels relative to one overlay's viewport. */
export interface OverlayHotZone {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Registers the rectangles that stay clickable in `normal` mode on one
 * overlay. Reported by that overlay after every layout change, because a rect
 * that outlives the control it describes would keep swallowing table clicks at
 * a point where nothing of Velora's is drawn any more.
 */
export async function setOverlayHotZones(
  tableId: number,
  zones: OverlayHotZone[],
): Promise<void> {
  assertTauriAvailable();
  return invoke("set_overlay_hot_zones", { tableId, zones });
}

/**
 * Saves one player's manually-dragged card position.
 *
 * Deliberately not table-scoped, unlike its neighbours. A saved position
 * is keyed by player and expressed as a fraction of the overlay window, which
 * mirrors whichever table that player is sitting at — there is no
 * table-specific component to store. The backend clamps `x`/`y` into 0..1.
 */
export async function saveHudPosition(playerId: string, x: number, y: number): Promise<void> {
  assertTauriAvailable();
  return invoke("save_hud_position", { playerId, x, y });
}

export async function getHudPositions(): Promise<HudPosition[]> {
  assertTauriAvailable();
  return invoke<HudPosition[]>("get_hud_positions");
}

// ---------------------------------------------------------------------
// Seat-mapping templates
// ---------------------------------------------------------------------

export async function getSeatTemplates(maxPlayers: number): Promise<SeatTemplate[]> {
  assertTauriAvailable();
  return invoke<SeatTemplate[]>("get_seat_templates", { maxPlayers });
}

/** `seatOffset` is the seat's distance from the hero, not an absolute PokerStars seat number. */
export async function saveSeatTemplate(
  maxPlayers: number,
  seatOffset: number,
  x: number,
  y: number,
): Promise<void> {
  assertTauriAvailable();
  return invoke("save_seat_template", { maxPlayers, seatOffset, x, y });
}

export async function setAutoCenterEnabled(enabled: boolean): Promise<void> {
  assertTauriAvailable();
  return invoke("set_auto_center_enabled", { enabled });
}

/**
 * Onboarding readiness gate: read-only, no settings write, no import, no
 * watcher start — safe to call as often as the readiness step wants to
 * re-check. `path` defaults to the same auto-detection Settings uses when
 * omitted.
 */
export async function getOnboardingReadiness(path?: string | null): Promise<OnboardingReadinessPayload> {
  assertTauriAvailable();
  return invoke<OnboardingReadinessPayload>("get_onboarding_readiness", { path: path ?? null });
}

export interface TableDetectionStatus {
  detected: boolean;
  /**
   * How many real PokerStars table windows are open right now. Each one has
   * its own overlay, so this is simply how many HUDs are running.
   */
  tableWindowCount: number;
  /** Debug evidence for diagnosing window-following problems. */
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
// Build / version
// ---------------------------------------------------------------------

/**
 * Mirrors `commands::AppVersionPayload`. `features` is empty on a
 * distributed build (`auto-classification`/`strategic-analysis` compiled
 * out) — the only place a tester can tell the two builds apart without
 * asking anyone (Settings displays this).
 */
export interface AppVersion {
  version: string;
  features: string[];
}

export async function getAppVersion(): Promise<AppVersion> {
  assertTauriAvailable();
  return invoke<AppVersion>("get_app_version");
}

// ---------------------------------------------------------------------
// Classification rules (read-only)
// ---------------------------------------------------------------------

/**
 * One archetype rule exactly as the backend engine holds it. Thresholds are
 * percentages and any of them may be null, meaning "unconstrained on this
 * stat" — a rule with every threshold null (Recreational) matches on hand
 * count alone, which is what makes it the catch-all.
 */
export interface ClassificationRule {
  id: string;
  label: string;
  color: string;
  priority: number;
  minHands: number;
  vpipMin: number | null;
  vpipMax: number | null;
  pfrMin: number | null;
  pfrMax: number | null;
  threeBetMin: number | null;
  threeBetMax: number | null;
}

/** The archetype rules in the order the engine evaluates them (first match wins). */
export async function getClassificationRules(): Promise<ClassificationRule[]> {
  assertTauriAvailable();
  return invoke<ClassificationRule[]>("get_classification_rules");
}

// ---------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------

/**
 * Plain-text self-serve diagnostics dump — which table(s)/hand the overlay
 * currently considers active and where that came from, recent overlay
 * refresh/resync events, the last imported hands and their table
 * identifiers, and current watcher/import status. Meant to be copied and
 * pasted into a bug report when something "feels wrong" during play, without
 * the user needing to characterize the bug themselves.
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
 * Fires whenever one table's overlay window is shown or hidden, from whichever
 * window triggered it — including that overlay's own "Close" button. Emitted
 * by `overlay::manager` in Rust, so this is the single source of truth for
 * overlay visibility and no window has to keep a flag in sync by hand. Carries the table id: "the overlay
 * closed" is only half an answer once there is one per table.
 */
export async function onOverlayVisibilityChanged(
  callback: (change: { tableId: number; visible: boolean }) => void,
): Promise<UnlistenFn> {
  return listen<{ tableId: number; visible: boolean }>(
    "overlay-visibility-changed",
    (event) => callback(event.payload),
  );
}

/**
 * Fires whenever one table's HUD *content* is collapsed or restored —
 * `closeOverlay`/`showOverlay`, from any of their three triggers (the
 * overlay's own "Hide"/"Show" pill, the main window's table list, or the
 * global hotkey). Deliberately a separate event from
 * `onOverlayVisibilityChanged`: that one also fires for a window-level
 * change (minimize/restore, the kill switch) that has nothing to do with a
 * deliberate per-table dismissal, and conflating the two would un-collapse a
 * table's HUD just because its window was restored from being minimized.
 */
export async function onOverlayDismissedChanged(
  callback: (change: { tableId: number; dismissed: boolean }) => void,
): Promise<UnlistenFn> {
  return listen<{ tableId: number; dismissed: boolean }>(
    "overlay-dismissed-changed",
    (event) => callback(event.payload),
  );
}

/**
 * Fires in every window whenever the overlay's interaction mode changes,
 * whichever window triggered it. Emitted from `overlay::set_mode` in Rust
 * only after the native state actually changed, so this is the single source
 * of truth for the mode.
 *
 * The main window and the overlay each own a mode control, and before this
 * existed neither could see the other's toggle. That let a button's label say
 * the opposite of what it would do, which during live play left the overlay
 * capturing while the control offered to make it capture — swallowing clicks
 * intended for the table underneath (same bug class as a visibility flag kept in sync by hand).
 */
export async function onOverlayModeChanged(
  callback: (change: { tableId: number; mode: OverlayMode }) => void,
): Promise<UnlistenFn> {
  return listen<{ tableId: number; mode: OverlayMode }>(
    "overlay-mode-changed",
    (event) => callback(event.payload),
  );
}

/**
 * Fires whenever the set of tracked PokerStars table windows changes — a
 * table opened, closed, was renamed or was minimized — so the HUD page's table
 * list stays live without polling. Replaced an earlier bare count broadcast, which
 * existed only to power the "only one table gets a HUD" notice.
 */
export async function onTrackedTablesChanged(
  callback: (tables: TrackedTable[]) => void,
): Promise<UnlistenFn> {
  return listen<TrackedTable[]>("tracked-tables-changed", (event) => callback(event.payload));
}

/**
 * Fires whenever a seat template is calibrated by a drag, carrying the table
 * size it was saved for. A template is shared by every same-sized table,
 * and with one overlay per table the others have no other way to learn
 * that a card just moved.
 */
export async function onSeatTemplatesChanged(
  callback: (maxPlayers: number) => void,
): Promise<UnlistenFn> {
  return listen<number>("seat-templates-changed", (event) => callback(event.payload));
}

/** One table window as the tracker sees it, as broadcast by `onTrackedTablesChanged`. */
export interface TrackedTable {
  id: number;
  hwnd: number;
  name: string | null;
  rect: { x: number; y: number; width: number; height: number };
  minimized: boolean;
}
