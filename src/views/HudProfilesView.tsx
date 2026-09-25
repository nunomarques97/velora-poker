import { useEffect, useRef, useState } from "react";
import type { HudProfile, Player } from "../data/types";
import {
  DesktopAppRequiredError,
  getActiveHudProfile,
  getHudProfiles,
  getOverlayStatus,
  getPlayersPage,
  onHandsImported,
  onOverlayVisibilityChanged,
  onTrackedTablesChanged,
  resetSeatPositions,
  setActiveHudProfile,
  setHudProfileMinHands,
  setOverlaysEnabled,
  showOverlay,
  type TrackedTableStatus,
} from "../data/api";
import { players as SAMPLE_PLAYERS } from "../data/mockData";
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

/** The default profile, and the one this page recommends. */
const RECOMMENDED_PROFILE_ID = "compact";

/** One line per model, so the choice says what it changes on the table. */
const MODEL_DESCRIPTIONS: Record<HudProfile["visualModel"], string> = {
  compact: "One line per player: VPIP / PFR / 3-bet and hands. Readable on the smallest table.",
  badge: "Initials and hands only, the smallest footprint. Click a badge for the stats.",
};

/**
 * The fixed player each profile card draws its sample chip with, so the
 * choice is visible before a single hand is imported. Illustration only: the
 * live preview below uses real tracked players.
 */
const SAMPLE_PLAYER: Player = SAMPLE_PLAYERS[0];

type LoadState =
  | { status: "loading" }
  | { status: "ready" }
  | { status: "unavailable"; message: string }
  | { status: "error"; message: string };

/** The one line under the controls that says what the last action did. */
type Notice = { kind: "ok" | "error"; text: string } | null;

function describeError(err: unknown): string {
  if (err instanceof Error) return err.message;
  return String(err);
}

export function HudProfilesView({ onSelectPlayer }: HudProfilesViewProps) {
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [profiles, setProfiles] = useState<HudProfile[]>([]);
  const [activeProfile, setActiveProfileState] = useState<HudProfile | null>(null);
  const [players, setPlayers] = useState<Player[]>([]);
  const [totalPlayers, setTotalPlayers] = useState(0);
  // There is no single "the overlay". Every real table window gets its
  // own HUD automatically, so what this page shows is which tables are being
  // followed, plus one switch to turn the whole lot off.
  const [overlaysEnabled, setOverlaysEnabledState] = useState(true);
  const [tables, setTables] = useState<TrackedTableStatus[]>([]);
  const [minHandsInput, setMinHandsInput] = useState("25");
  const [resetting, setResetting] = useState(false);
  const [notice, setNotice] = useState<Notice>(null);

  // Set in the effect (not at render) so StrictMode's setup/cleanup/setup
  // leaves it true, and a response landing after the view is left is dropped.
  const mounted = useRef(false);
  // Only the latest profile click may write the active profile: two quick
  // clicks can answer out of order.
  const profileRequest = useRef(0);
  // A second press before React re-renders must not start a second reset.
  const resetPending = useRef(false);

  function loadPreviewPlayers() {
    getPlayersPage(0, PREVIEW_GRID_MAX_PLAYERS)
      .then(({ players: page, total }) => {
        if (!mounted.current) return;
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
    ])
      .then(([allProfiles, active, playersPage, overlayStatus]) => {
        if (!mounted.current) return;
        setProfiles(allProfiles);
        setActiveProfileState(active);
        setMinHandsInput(String(active.minHands));
        setPlayers(playersPage.players);
        setTotalPlayers(playersPage.total);
        setOverlaysEnabledState(overlayStatus.enabled);
        setTables(overlayStatus.tables);
        setState({ status: "ready" });
      })
      .catch((err: unknown) => {
        if (!mounted.current) return;
        if (err instanceof DesktopAppRequiredError) {
          setState({ status: "unavailable", message: err.message });
        } else {
          setState({ status: "error", message: String(err) });
        }
      });
  }

  useEffect(() => {
    mounted.current = true;
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

    return () => {
      mounted.current = false;
      unlisten.then((fn) => fn?.());
      unlistenTables.then((fn) => fn?.());
      unlistenOverlay.then((fn) => fn?.());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function handleSelectProfile(id: string) {
    const request = ++profileRequest.current;
    try {
      const updated = await setActiveHudProfile(id);
      if (!mounted.current || request !== profileRequest.current) return;
      setActiveProfileState(updated);
      setMinHandsInput(String(updated.minHands));
      setNotice(null);
      loadPreviewPlayers();
    } catch (err) {
      if (!mounted.current || request !== profileRequest.current) return;
      setNotice({ kind: "error", text: `Couldn't switch the HUD model: ${describeError(err)}` });
    }
  }

  async function handleMinHandsBlur() {
    if (!activeProfile) return;
    const parsed = Number.parseInt(minHandsInput, 10);
    const value = Number.isFinite(parsed) && parsed >= 0 ? parsed : activeProfile.minHands;
    setMinHandsInput(String(value));
    if (value === activeProfile.minHands) return;
    try {
      const updated = await setHudProfileMinHands(activeProfile.id, value);
      if (!mounted.current) return;
      setActiveProfileState(updated);
      loadPreviewPlayers();
    } catch (err) {
      if (!mounted.current) return;
      setMinHandsInput(String(activeProfile.minHands));
      setNotice({ kind: "error", text: `Couldn't save Min hands: ${describeError(err)}` });
    }
  }

  /** Re-reads the tracked tables from the backend. */
  function refreshOverlayStatus() {
    getOverlayStatus()
      .then((status) => {
        if (!mounted.current) return;
        setOverlaysEnabledState(status.enabled);
        setTables(status.tables);
      })
      .catch(() => undefined);
  }

  async function toggleOverlaysEnabled() {
    const next = !overlaysEnabled;
    setOverlaysEnabledState(next);
    try {
      await setOverlaysEnabled(next);
    } catch (err) {
      if (mounted.current) {
        setNotice({ kind: "error", text: `Couldn't turn HUDs ${next ? "on" : "off"}: ${describeError(err)}` });
      }
    }
    // The overlay thread creates or tears down the windows asynchronously;
    // read back what actually happened rather than assuming. On a failure this
    // also puts the button back to the real state.
    window.setTimeout(refreshOverlayStatus, 300);
  }

  /** Per-table quick re-show for a table dismissed via the overlay's own "Hide". */
  async function handleShowTable(tableId: number) {
    await showOverlay(tableId);
    window.setTimeout(refreshOverlayStatus, 300);
  }

  /**
   * Forgets every dragged chip position, for every table size. Each open
   * overlay hears `seat-templates-changed` and snaps back to the default
   * layout by itself.
   */
  async function handleResetSeatLayout() {
    if (resetPending.current) return;
    resetPending.current = true;
    setResetting(true);
    setNotice(null);
    try {
      await resetSeatPositions(null);
      if (!mounted.current) return;
      setNotice({
        kind: "ok",
        text: "Seat layout reset. Every table size is back to the default layout.",
      });
    } catch (err) {
      if (!mounted.current) return;
      setNotice({ kind: "error", text: `Couldn't reset the seat layout: ${describeError(err)}` });
    } finally {
      resetPending.current = false;
      if (mounted.current) setResetting(false);
    }
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

      <div className={styles.profileCards} role="group" aria-label="HUD model">
        {profiles.map((p) => {
          const active = activeProfile?.id === p.id;
          const recommended = p.id === RECOMMENDED_PROFILE_ID;
          return (
            <div key={p.id} className={`${styles.profileCard} ${active ? styles.profileCardActive : ""}`}>
              <button
                type="button"
                className={styles.profileChoice}
                aria-pressed={active}
                onClick={() => handleSelectProfile(p.id)}
              >
                <span className={styles.profileHead}>
                  <span className={styles.profileName}>{p.name}</span>
                  {recommended && <span className={styles.recommended}>Recommended</span>}
                  {active && <span className={styles.inUse}>In use</span>}
                </span>
                <span className={styles.profileDescription}>
                  {MODEL_DESCRIPTIONS[p.visualModel] ?? MODEL_DESCRIPTIONS.compact}
                </span>
              </button>
              {/* Illustration only: inert, so it is neither clickable nor
                  focusable, and a click on it falls through to the card. */}
              <div className={styles.profileSample} inert>
                <PlayerHudCard player={SAMPLE_PLAYER} profile={p} />
              </div>
            </div>
          );
        })}
      </div>

      <div className={styles.controls}>
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
      </div>

      <div className={styles.layoutPanel}>
        <div className={styles.layoutText}>
          <div className={styles.layoutTitle}>Seat layout</div>
          <p className={styles.layoutHint}>
            Drag any chip directly on the table to move it. There is no lock and no edit mode: the
            table stays clickable the whole time. A position you set applies to that seat on every
            table of the same size, so one drag covers all your 6-max (or 9-max) tables.
          </p>
        </div>
        <button
          type="button"
          className={styles.resetButton}
          onClick={handleResetSeatLayout}
          // Not `disabled`: a disabled button drops keyboard focus, and the
          // user who pressed Space here should still be on it afterwards.
          aria-disabled={resetting}
        >
          {resetting ? "Resetting…" : "Reset seat layout"}
        </button>
      </div>

      <p
        className={`${styles.notice} ${notice?.kind === "error" ? styles.noticeError : ""}`}
        role="status"
        aria-live="polite"
      >
        {notice?.text ?? ""}
      </p>

      <TrackedTables tables={tables} enabled={overlaysEnabled} onShowTable={handleShowTable} />

      <h2 className={styles.sectionTitle}>Live preview</h2>
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
        only its players — nothing to click, and no limit on how many. Clicks reach the table
        everywhere except the HUD chips themselves.
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
          {/* The only way back for a table dismissed via
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
      <p>Pick a HUD model. Every PokerStars table you open gets it automatically.</p>
    </div>
  );
}
