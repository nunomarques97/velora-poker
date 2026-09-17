import { useEffect, useState } from "react";
import type { HudProfile, Player } from "../data/types";
import {
  DesktopAppRequiredError,
  getActiveHudProfile,
  getAnyOverlayMode,
  getHudProfiles,
  getOverlayStatus,
  getPlayersPage,
  onHandsImported,
  onOverlayModeChanged,
  onOverlayVisibilityChanged,
  onTrackedTablesChanged,
  setActiveHudProfile,
  setAllOverlayModes,
  setHudProfileMinHands,
  setOverlaysEnabled,
  showOverlay,
  type OverlayMode,
  type TrackedTableStatus,
} from "../data/api";
import { PlayerHudCard } from "../hud/PlayerHudCard";
import styles from "./HudProfilesView.module.css";

interface HudProfilesViewProps {
  onSelectPlayer: (player: Player) => void;
}

// This grid is a *preview* of the HUD, not the HUD itself — the real overlay
// (table_track/manager) renders only the handful of seats at one table and
// is untouched by this. This page used to render one PlayerHudCard per row of
// `getPlayers()`'s full roster, with no limit — and `getPlayers()` computes
// full stats for every tracked player in one call, which froze this tab once
// a bulk import pushed the players table into the thousands. `getPlayersPage`
// only computes stats for the page requested, so this cap bounds the backend
// work too, not just what's rendered client-side.
const PREVIEW_GRID_MAX_PLAYERS = 60;

type LoadState =
  | { status: "loading" }
  | { status: "ready" }
  | { status: "unavailable"; message: string }
  | { status: "error"; message: string };

export function HudProfilesView({ onSelectPlayer }: HudProfilesViewProps) {
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [profiles, setProfiles] = useState<HudProfile[]>([]);
  const [activeProfile, setActiveProfileState] = useState<HudProfile | null>(null);
  const [players, setPlayers] = useState<Player[]>([]);
  const [totalPlayers, setTotalPlayers] = useState(0);
  // there is no "the overlay" any more. Every real table window gets its
  // own HUD automatically, so what this page shows is which tables are being
  // followed, plus one switch to turn the whole lot off.
  const [overlaysEnabled, setOverlaysEnabledState] = useState(true);
  const [tables, setTables] = useState<TrackedTableStatus[]>([]);
  // a secondary copy of the overlays' own control. Each overlay opens
  // table-interactive and its control bar is reachable by a real click in
  // every mode, so this is a convenience, never the only way back. It acts on
  // every overlay at once, since a card layout is calibrated across tables.
  const [overlayMode, setOverlayModeState] = useState<OverlayMode>("normal");
  const [minHandsInput, setMinHandsInput] = useState("25");

  function loadPreviewPlayers() {
    getPlayersPage(0, PREVIEW_GRID_MAX_PLAYERS)
      .then(({ players: page, total }) => {
        setPlayers(page);
        setTotalPlayers(total);
      })
      .catch(() => undefined);
  }

  function loadAll() {
    Promise.all([
      getHudProfiles(),
      getActiveHudProfile(),
      getPlayersPage(0, PREVIEW_GRID_MAX_PLAYERS),
      getOverlayStatus(),
      getAnyOverlayMode(),
    ])
      .then(([allProfiles, active, playersPage, overlayStatus, mode]) => {
        setProfiles(allProfiles);
        setActiveProfileState(active);
        setMinHandsInput(String(active.minHands));
        setPlayers(playersPage.players);
        setTotalPlayers(playersPage.total);
        setOverlaysEnabledState(overlayStatus.enabled);
        setTables(overlayStatus.tables);
        setOverlayModeState(mode);
        setState({ status: "ready" });
      })
      .catch((err: unknown) => {
        if (err instanceof DesktopAppRequiredError) {
          setState({ status: "unavailable", message: err.message });
        } else {
          setState({ status: "error", message: String(err) });
        }
      });
  }

  useEffect(() => {
    loadAll();
    const unlisten = onHandsImported(loadPreviewPlayers).catch(() => undefined);

    // Rust broadcasts every table appearing and disappearing, so this list is
    // live without polling — the same discipline as the overlay events below.
    const unlistenTables = onTrackedTablesChanged(() => {
      refreshOverlayStatus();
    }).catch(() => undefined);

    // Rust broadcasts every overlay show/hide, so this view stays correct even
    // when an overlay is closed from its own in-overlay "Close" button, which
    // this window has no other way to observe.
    const unlistenOverlay = onOverlayVisibilityChanged(() => {
      refreshOverlayStatus();
    }).catch(() => undefined);

    // Each overlay has its own mode control in its floating control bar, which
    // this window cannot otherwise observe. Without this the label here could
    // offer to do what the overlays are already doing, so the next click did
    // the opposite of what it advertised and the tables underneath stayed
    // dead. Re-read rather than taking the event's own mode: this one control
    // speaks for every overlay, and "any of them is repositioning" is the
    // state worth showing.
    const unlistenMode = onOverlayModeChanged(() => {
      getAnyOverlayMode().then(setOverlayModeState).catch(() => undefined);
    }).catch(() => undefined);

    return () => {
      unlisten.then((fn) => fn?.());
      unlistenTables.then((fn) => fn?.());
      unlistenOverlay.then((fn) => fn?.());
      unlistenMode.then((fn) => fn?.());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function handleSelectProfile(id: string) {
    const updated = await setActiveHudProfile(id);
    setActiveProfileState(updated);
    setMinHandsInput(String(updated.minHands));
    loadPreviewPlayers();
  }

  async function handleMinHandsBlur() {
    if (!activeProfile) return;
    const parsed = Number.parseInt(minHandsInput, 10);
    const value = Number.isFinite(parsed) && parsed >= 0 ? parsed : activeProfile.minHands;
    setMinHandsInput(String(value));
    const updated = await setHudProfileMinHands(activeProfile.id, value);
    setActiveProfileState(updated);
    loadPreviewPlayers();
  }

  /**
   * Re-reads both the tracked tables and the live reposition state from the
   * backend.
   *
   * The mode is re-read here rather than assumed, which is what closes known
   * issue #19. This view used to force its own flag back to `normal` whenever
   * the overlay closed, on the stated assumption that the OS resets
   * click-through for a hidden window — never verified, and untrue in general.
   * Now the backend forces `Normal` itself whenever an overlay is released or
   * handed to another table, and this asks it what the state actually is.
   */
  function refreshOverlayStatus() {
    getOverlayStatus()
      .then((status) => {
        setOverlaysEnabledState(status.enabled);
        setTables(status.tables);
      })
      .catch(() => undefined);
    getAnyOverlayMode()
      .then(setOverlayModeState)
      .catch(() => undefined);
  }

  async function toggleOverlaysEnabled() {
    const next = !overlaysEnabled;
    setOverlaysEnabledState(next);
    await setOverlaysEnabled(next);
    // The overlay thread creates or tears down the windows asynchronously;
    // read back what actually happened rather than assuming.
    window.setTimeout(refreshOverlayStatus, 300);
  }

  /** Per-table quick re-show for a table dismissed via the overlay's own "Hide". */
  async function handleShowTable(tableId: number) {
    await showOverlay(tableId);
    window.setTimeout(refreshOverlayStatus, 300);
  }

  async function toggleOverlayMode() {
    const next: OverlayMode = overlayMode === "reposition" ? "normal" : "reposition";
    await setAllOverlayModes(next);
    setOverlayModeState(next);
  }

  if (state.status === "loading") {
    return (
      <div>
        <ViewHeader />
        <div className={styles.stateBox}>Loading HUD&hellip;</div>
      </div>
    );
  }

  if (state.status === "unavailable") {
    return (
      <div>
        <ViewHeader />
        <div className={styles.stateBox}>{state.message}</div>
      </div>
    );
  }

  if (state.status === "error") {
    return (
      <div>
        <ViewHeader />
        <div className={styles.stateBox}>Failed to load HUD: {state.message}</div>
      </div>
    );
  }

  return (
    <div>
      <ViewHeader />

      <div className={styles.controls}>
        <div className={styles.profileSwitcher}>
          {profiles.map((p) => (
            <button
              key={p.id}
              type="button"
              className={`${styles.profileButton} ${
                activeProfile?.id === p.id ? styles.profileButtonActive : ""
              }`}
              onClick={() => handleSelectProfile(p.id)}
            >
              {p.name}
            </button>
          ))}
        </div>

        <div className={styles.rightControls}>
          <label className={styles.minHandsLabel}>
            Min hands
            <input
              className={styles.minHandsInput}
              type="number"
              min={0}
              value={minHandsInput}
              onChange={(e) => setMinHandsInput(e.target.value)}
              onBlur={handleMinHandsBlur}
            />
          </label>

          <button type="button" className={styles.overlayButton} onClick={toggleOverlaysEnabled}>
            {overlaysEnabled ? "Turn HUDs Off" : "Turn HUDs On"}
          </button>

          {overlaysEnabled && tables.length > 0 && (
            <button
              type="button"
              className={`${styles.lockButton} ${
                overlayMode === "reposition" ? styles.lockButtonActive : ""
              }`}
              onClick={toggleOverlayMode}
            >
              {overlayMode === "reposition" ? "Done Repositioning" : "Reposition Cards"}
            </button>
          )}
        </div>
      </div>

      <TrackedTables tables={tables} enabled={overlaysEnabled} onShowTable={handleShowTable} />

      {totalPlayers === 0 ? (
        <div className={styles.stateBox}>
          No hands imported yet. Configure your PokerStars hand history folder in Settings to
          populate the HUD with real player data.
        </div>
      ) : (
        activeProfile && (
          <PreviewGrid
            players={players}
            totalPlayers={totalPlayers}
            profile={activeProfile}
            onSelectPlayer={onSelectPlayer}
          />
        )
      )}

      <p className={styles.hint}>
        This grid previews the active HUD model with your real tracked players. Every PokerStars
        table you open gets its own overlay automatically, positioned over that table and showing
        only its players — nothing to click, and no limit on how many. Each overlay opens ready to
        play: clicks reach the table everywhere except its own control bar and each card&rsquo;s
        stat-page dots. Use Reposition to drag cards, then leave it again.
      </p>
    </div>
  );
}

/**
 * The live list of tables being followed. It exists because the honest
 * answer to "is my HUD working" used to be a single Open/Close button that
 * said nothing about which table it belonged to — and with two tables open,
 * one of them silently had no HUD at all.
 */
function TrackedTables({
  tables,
  enabled,
  onShowTable,
}: {
  tables: TrackedTableStatus[];
  enabled: boolean;
  onShowTable: (tableId: number) => void;
}) {
  if (!enabled) {
    return (
      <div className={styles.tableStrip}>
        HUD overlays are turned off. No table gets a HUD until you turn them back on.
      </div>
    );
  }

  if (tables.length === 0) {
    return (
      <div className={styles.tableStrip}>
        No PokerStars table window open. A HUD appears on each table by itself as you open them.
      </div>
    );
  }

  return (
    <div className={styles.tableStrip}>
      <span className={styles.tableStripLabel}>
        {tables.length === 1 ? "1 table tracked" : `${tables.length} tables tracked`}
      </span>
      {tables.map((table) => (
        <span key={table.id} className={styles.tableChip}>
          {table.name ?? "Unnamed table"}
          <span className={styles.tableChipState}>
            {table.minimized
              ? "minimized"
              : table.dismissed
                ? "hidden"
                : table.overlayVisible
                  ? "HUD on"
                  : "starting…"}
          </span>
          {/* the only way back for a table dismissed via
              the overlay's own "Hide" used to be the global switch (every
              table) or closing and reopening PokerStars' own window. */}
          {table.dismissed && !table.minimized && (
            <button
              type="button"
              className={styles.tableChipShowButton}
              onClick={() => onShowTable(table.id)}
            >
              Show
            </button>
          )}
        </span>
      ))}
    </div>
  );
}

/**
 * The HUD-model preview grid. Only meeting the active profile's `minHands`
 * matters for the real overlay; this additionally caps the *preview* to the
 * top `PREVIEW_GRID_MAX_PLAYERS` by hand count, since the point here is a
 * representative sample, not every tracked player (see comment above the
 * constant — this is what previously froze the tab at 6000+ players).
 */
function PreviewGrid({
  players,
  totalPlayers,
  profile,
  onSelectPlayer,
}: {
  players: Player[];
  totalPlayers: number;
  profile: HudProfile;
  onSelectPlayer: (player: Player) => void;
}) {
  // `players` already came back from the backend as the top
  // `PREVIEW_GRID_MAX_PLAYERS` by hand count (`get_players_page`, sorted
  // hands DESC) — no client-side sort/slice needed here any more, just the
  // minHands cutoff `get_players_page` doesn't know about.
  const eligible = players.filter((p) => p.hands >= profile.minHands);
  const truncatedCount = totalPlayers - eligible.length;

  if (eligible.length === 0) {
    return (
      <div className={styles.stateBox}>
        No tracked player has reached the {profile.minHands}-hand minimum for this profile yet.
      </div>
    );
  }

  return (
    <>
      <div className={styles.grid}>
        {eligible.map((player) => (
          <PlayerHudCard
            key={player.id}
            player={player}
            profile={profile}
            onOpenDetail={onSelectPlayer}
            fixedWidth
          />
        ))}
      </div>
      {truncatedCount > 0 && (
        <p className={styles.hint}>
          Showing the {eligible.length} most-played of {totalPlayers.toLocaleString()} tracked
          players ({truncatedCount.toLocaleString()} more not shown here). This is a preview only
          — every real table's overlay always shows all of its own seats.
        </p>
      )}
    </>
  );
}

function ViewHeader() {
  return (
    <div className="view-header">
      <h1>HUD Profiles</h1>
      <p>Choose a HUD model and preview it against your real tracked players.</p>
    </div>
  );
}
