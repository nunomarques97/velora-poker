import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import type { DashboardSummary, HudProfile, Player } from "../data/types";
import {
  closeOverlay,
  getActiveHudProfile,
  getActiveTableMaxPlayers,
  getActiveTablePlayers,
  getAppSettings,
  getDashboardSummary,
  getHudPositions,
  getOverlayMode,
  getSeatTemplates,
  isOverlayDismissed,
  onHandsImported,
  onOverlayDismissedChanged,
  onOverlayModeChanged,
  onOverlayVisibilityChanged,
  onSeatTemplatesChanged,
  saveHudPosition,
  saveSeatTemplate,
  setOverlayHotZones,
  setOverlayMode,
  showOverlay,
  type OverlayHotZone,
  type OverlayMode,
} from "../data/api";
import { PlayerHudCard } from "../hud/PlayerHudCard";
import { PlayerProfileDrawer } from "../components/PlayerProfileDrawer/PlayerProfileDrawer";
import styles from "./OverlayApp.module.css";

/** Fractions (0..1) of the overlay window — see `HudPosition`/`SeatTemplate` docs (Phase E). */
interface PositionMap {
  [key: string]: { x: number; y: number };
}

const DEFAULT_COL_FRACTION = 0.16;
const DEFAULT_ROW_FRACTION = 0.1;
const DEFAULT_MARGIN = 0.02;
const DEFAULT_COLS_PER_ROW = 6;

/**
 * Grown around every hot zone before it is registered. The pagination
 * dots are 4px targets whose own clickable area (a CSS pseudo-element) already
 * extends 8px past the pill, so the registered zone has to reach that far too
 * or the outer half of every target would fall through to the table instead.
 * Still tiny: 60x24 pixels per card.
 */
const HOT_ZONE_PADDING = 8;

/**
 * Sub-pixel jitter (touchpads, some mice) must never start a drag or block a
 * click; real intent to drag reliably exceeds this within the first couple
 * of pixels. See `dragState`'s own doc comment for why this exists.
 */
const DRAG_THRESHOLD_PX = 4;

/**
 * URGENT build (live session, 2026-09-13): the user reported the overlay's
 * own Reposition/Hide buttons block visibility of the table during live play.
 * Set to `false` for this build to stop rendering them — no settings toggle
 * yet, just this one flag. Everything the buttons drove (`toggleMode`,
 * `handleHide`, the `mode`/`hidden` state, the main window's own Reposition
 * control, the global hide hotkey) is untouched, so flipping this back to
 * `true` is the entire reversal.
 */
const SHOW_REPOSITION_HIDE_BUTTONS = false;

/**
 * URGENT build (2026-09-13, multi-tabling live, same night): the corner
 * Reposition button added right after the flag above turned out unnecessary
 * once dragging worked directly off the badge's own hot zone — see the long
 * comment at this button's render site for why. Off for now; the button,
 * its ref, and its hot-zone registration all stay in place.
 */
const SHOW_REPOSITION_CORNER_BUTTON = false;

/**
 * Which table this overlay window belongs to.
 *
 * Every overlay is created with `overlay.html?table=<id>` as its URL, so the
 * answer is known before the first backend call and cannot change for the life
 * of the window — read once at module scope rather than in an effect. `null`
 * only for the hidden prototype window `tauri.conf.json` still declares (see
 * `overlay::manager`), which is never shown and renders nothing.
 */
const TABLE_ID: number | null = (() => {
  const raw = new URLSearchParams(window.location.search).get("table");
  if (raw === null) return null;
  const parsed = Number.parseInt(raw, 10);
  return Number.isInteger(parsed) && parsed > 0 ? parsed : null;
})();

/**
 * Same centred ellipse the backend seeds `seat_templates` with for every
 * table size that has no hand-measured layout of its own (,
 * `ellipse_seat_template` in `src-tauri/src/db/mod.rs:518-527`, read-only —
 * never edit that file from here). Constants, angle convention and clamp
 * copied verbatim so a card that has no calibrated template still opens on
 * the ring instead of drifting from whatever the backend would have derived:
 * offset 0 (hero) at the bottom-centre, subsequent offsets sweeping the same
 * direction (left side next, then up and around to the right).
 */
const ELLIPSE_CENTER = { x: 0.364, y: 0.382 };
const ELLIPSE_RADII = { x: 0.4, y: 0.282 };
const ELLIPSE_START_DEG = 91.5;
const ELLIPSE_MIN = 0.02;
const ELLIPSE_MAX = 0.92;

function ellipseSeatPosition(maxPlayersForSeat: number, seatOffset: number) {
  const stepDeg = 360 / maxPlayersForSeat;
  const theta = ((ELLIPSE_START_DEG + seatOffset * stepDeg) * Math.PI) / 180;
  const x = ELLIPSE_CENTER.x + ELLIPSE_RADII.x * Math.cos(theta);
  const y = ELLIPSE_CENTER.y + ELLIPSE_RADII.y * Math.sin(theta);
  return {
    x: Math.min(Math.max(x, ELLIPSE_MIN), ELLIPSE_MAX),
    y: Math.min(Math.max(y, ELLIPSE_MIN), ELLIPSE_MAX),
  };
}

/**
 * The position a card opens at when nothing else has claimed one yet
 * (, the notes a known issue / product-profile inacceptable #4).
 * Not the whole precedence chain by itself — see the comment at this
 * function's call sites for the full order — just the last two links:
 * derive a ring position from `seatOffset` + `maxPlayersForSeat` when both
 * are known, matching the backend's ellipse convention (); fall back to
 * the stacked grid only when one of them is `null` (no hero seat resolved
 * for this hand, or no table size read yet) — the one case a seat-based
 * layout genuinely cannot answer.
 */
function defaultPosition(
  index: number,
  seatOffset: number | null | undefined,
  maxPlayersForSeat: number | null | undefined,
) {
  if (seatOffset != null && maxPlayersForSeat != null && maxPlayersForSeat > 0) {
    return ellipseSeatPosition(maxPlayersForSeat, seatOffset);
  }
  const col = index % DEFAULT_COLS_PER_ROW;
  const row = Math.floor(index / DEFAULT_COLS_PER_ROW);
  return {
    x: DEFAULT_MARGIN + col * DEFAULT_COL_FRACTION,
    y: DEFAULT_MARGIN + row * DEFAULT_ROW_FRACTION,
  };
}

export function OverlayApp() {
  if (TABLE_ID === null) {
    return null;
  }
  return <TableOverlay tableId={TABLE_ID} />;
}

/** One tracked table's HUD. Every backend call it makes names its own table. */
function TableOverlay({ tableId }: { tableId: number }) {
  const [players, setPlayers] = useState<Player[]>([]);
  const [profile, setProfile] = useState<HudProfile | null>(null);
  const [positions, setPositions] = useState<PositionMap>({});
  const [autoCenterEnabled, setAutoCenterEnabled] = useState(false);
  const [maxPlayers, setMaxPlayers] = useState<number | null>(null);
  // the overlay opens table-interactive. It is not a state the user has
  // to reach by pressing something first — interacting with the table is what
  // happens essentially all the time an overlay is open.
  const [mode, setMode] = useState<OverlayMode>("normal");
  /** Card currently being dragged — raised above its neighbours while it moves. */
  const [draggingId, setDraggingId] = useState<string | null>(null);
  // whether this table's HUD content is collapsed to a "Show" pill.
  // Deliberately a *content* flag, not a window-visibility one — the window
  // itself never hides for this, which is what makes a button in the same
  // spot possible at all (a genuinely hidden window can render nothing).
  const [hidden, setHidden] = useState(false);
  // The player whose detail drawer is open, if any ( work unit 4 wiring).
  // A ref-registered hot zone can only ever cover the card/control-bar rects
  // it knows about, so while the drawer is open — with its own backdrop and
  // content covering the whole window — reportHotZones below switches to a
  // single full-window zone instead of the usual per-element ones.
  const [detailPlayer, setDetailPlayer] = useState<Player | null>(null);

  // `captured`/`pointerId`/`target`/`startClientX`/`startClientY` support the
  // click-vs-drag threshold below (bug found 2026-09-13, badge rollout): a
  // plain press is recorded here immediately, but pointer capture — the
  // thing that actually blocks a click from reaching the pressed element,
  // see `handlePointerMove` — is deferred until the pointer has genuinely
  // moved. Before that fix, capturing on every pointerdown (regardless of
  // movement) meant a player's on-table badge, now both the drag handle and
  // the click target on the same small element, could never be clicked: the
  // instant capture engaged, the native `click` this button relies on to
  // open the detail drawer never fired.
  const dragState = useRef<{
    playerId: string;
    pointerId: number;
    target: HTMLElement;
    offsetX: number;
    offsetY: number;
    startClientX: number;
    startClientY: number;
    captured: boolean;
  } | null>(null);
  // Mirrors the in-progress drag position outside React state so the
  // pointerup handler always reads the latest value synchronously, instead
  // of a `positions` state closure that can be stale relative to the final
  // native pointermove event.
  const liveDragPosition = useRef<{ x: number; y: number } | null>(null);

  // The pointermove/pointerup listeners below are subscribed once for the
  // component's lifetime (see the effect's own comment) so they can't close
  // over fresh `players`/`autoCenterEnabled`/`maxPlayers` state — these refs
  // give handlePointerUp a way to read the latest values anyway, to decide
  // whether a drag should persist as a seat-mapping template or a manual
  // per-player override (Phase E).
  const playersRef = useRef<Player[]>([]);
  const autoCenterRef = useRef(false);
  const maxPlayersRef = useRef<number | null>(null);
  useEffect(() => {
    playersRef.current = players;
  }, [players]);
  useEffect(() => {
    autoCenterRef.current = autoCenterEnabled;
  }, [autoCenterEnabled]);
  useEffect(() => {
    maxPlayersRef.current = maxPlayers;
  }, [maxPlayers]);

  //  hot zones: the two things that must stay clickable while every other
  // point on the overlay falls through to the table. Held as DOM refs rather
  // than derived from state because what the native hit test needs is where
  // these actually painted, not where we think they should be.
  const controlBarRef = useRef<HTMLDivElement | null>(null);
  const dotClusterRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const contentRefs = useRef<Map<string, HTMLElement>>(new Map());
  // URGENT build (2026-09-13, multi-tabling live): the user's only way to
  // unlock per-table dragging (the old center Reposition button) was removed
  // a build earlier tonight along with Hide. This is its own small ref/hot
  // zone, deliberately separate from `controlBarRef` — see `repositionCorner`
  // below for why it isn't just re-added to the old control bar.
  const repositionCornerRef = useRef<HTMLButtonElement | null>(null);

  const refresh = useCallback(() => {
    Promise.all([
      getActiveTablePlayers(tableId),
      getActiveHudProfile(),
      getHudPositions(),
      getActiveTableMaxPlayers(tableId),
      getAppSettings(),
    ])
      .then(async ([allPlayers, activeProfile, savedPositions, activeMaxPlayers, appSettings]) => {
        const manual: PositionMap = {};
        for (const pos of savedPositions) {
          manual[pos.playerId] = { x: pos.x, y: pos.y };
        }

        const autoCenter = appSettings.autoCenterEnabled;
        const seatMap: PositionMap = {};
        if (autoCenter && activeMaxPlayers != null) {
          const templates = await getSeatTemplates(activeMaxPlayers).catch(() => []);
          for (const t of templates) {
            seatMap[String(t.seatOffset)] = { x: t.x, y: t.y };
          }
        }

        setPlayers(allPlayers);
        setProfile(activeProfile);
        setAutoCenterEnabled(autoCenter);
        setMaxPlayers(activeMaxPlayers);

        setPositions((prev) => {
          const next: PositionMap = { ...prev };
          allPlayers.forEach((player, index) => {
            // keyed by the seat's offset from the hero, never its
            // absolute PokerStars seat number. "Auto-Center me" rotates the
            // display so the hero is the fixed screen anchor, so absolute
            // seat N is not a fixed screen slot — which is why the user's
            // own card kept landing in the wrong one.
            const offsetKey = player.seatOffset != null ? String(player.seatOffset) : null;
            const fromTemplate = autoCenter && offsetKey ? seatMap[offsetKey] : undefined;
            // Precedence (/; a user's own drag always wins in the end —
            // product-profile priority 8, reversibility — because a drag
            // immediately writes `manual`/a seat template and both outrank
            // this on the very next refresh):
            //   1. `fromTemplate`   — Auto-Center seat template (calibrated or backend-derived)
            //   2. `manual[...]`    — a position the user dragged and saved for this player
            //   3. `next[...]`      — whatever this card is already showing on screen
            //   4. ellipse (inside `defaultPosition`) — derived from seatOffset + maxPlayers,
            //      same convention as the backend's `ellipse_seat_template` ()
            //   5. grid (inside `defaultPosition`) — only when seatOffset or maxPlayers is null
            next[player.id] =
              fromTemplate ??
              manual[player.id] ??
              next[player.id] ??
              defaultPosition(index, player.seatOffset, activeMaxPlayers);
          });
          return next;
        });
      })
      .catch(() => undefined);
  }, [tableId]);

  useEffect(() => {
    refresh();
    const unlisten = onHandsImported(refresh).catch(() => undefined);
    // The overlay webview stays mounted while the window is hidden, so
    // being shown again produces no render of its own — this is the only
    // signal that the cards are back on screen and worth re-reading.
    const unlistenVisibility = onOverlayVisibilityChanged((change) => {
      if (change.tableId === tableId && change.visible) refresh();
    }).catch(() => undefined);
    // A seat template calibrated on *another* table of the same size applies
    // here too. Without this the shared layout only ever reached the
    // window it was dragged in, and every other table kept the old placement
    // until its next hand — which, with one overlay per table, is most of them.
    const unlistenTemplates = onSeatTemplatesChanged(() => refresh()).catch(() => undefined);
    return () => {
      unlisten.then((fn) => fn?.());
      unlistenVisibility.then((fn) => fn?.());
      unlistenTemplates.then((fn) => fn?.());
    };
  }, [refresh, tableId]);

  // The main window's HUD Profiles page has its own mode control, and this
  // control bar has no other way to see it being used. Rust broadcasts every
  // change after the native state actually moved, so mirroring it keeps this
  // label honest about what the next click will really do.
  useEffect(() => {
    getOverlayMode(tableId)
      .then(setMode)
      .catch(() => undefined);
    // Filtered by table id: the main window offers one Reposition control for
    // every overlay at once, but a mode change belonging to another table must
    // never rewrite this control bar's label.
    const unlisten = onOverlayModeChanged((change) => {
      if (change.tableId === tableId) setMode(change.mode);
    }).catch(() => undefined);
    return () => {
      unlisten.then((fn) => fn?.());
    };
  }, [tableId]);

  // this table's own collapsed/expanded state — read once on mount
  // (before any event has fired) and kept in sync afterwards, whichever of
  // the three triggers changed it: this window's own Hide/Show pill, the
  // main window's table list, or the global hotkey. A dedicated event
  // (`onOverlayDismissedChanged`), not `onOverlayVisibilityChanged` — that
  // one also fires for the window being minimized/restored, which must never
  // un-collapse a HUD the user dismissed on purpose.
  useEffect(() => {
    isOverlayDismissed(tableId)
      .then(setHidden)
      .catch(() => undefined);
    const unlisten = onOverlayDismissedChanged((change) => {
      if (change.tableId === tableId) setHidden(change.dismissed);
    }).catch(() => undefined);
    return () => {
      unlisten.then((fn) => fn?.());
    };
  }, [tableId]);

  /**
   * Re-registers the always-clickable rectangles with the native hit test.
   * Runs after every layout that could have moved one — cards being
   * repositioned, the roster changing, the tracked table window resizing —
   * because a stale rect would go on swallowing table clicks at a point where
   * nothing of Velora's is drawn any more.
   */
  const reportHotZones = useCallback(() => {
    // The drawer's own backdrop (`PlayerProfileDrawer.module.css`'s
    // `.backdrop`) covers the whole window and has to be clickable to close
    // it, but the native hit test only keeps registered rects clickable — so
    // while it's open, the entire window is the one hot zone instead of the
    // usual per-element ones.
    if (detailPlayer) {
      setOverlayHotZones(tableId, [
        { x: 0, y: 0, width: window.innerWidth, height: window.innerHeight },
      ]).catch(() => undefined);
      return;
    }

    const zones: OverlayHotZone[] = [];
    const add = (el: HTMLElement | null | undefined) => {
      if (!el) return;
      const rect = el.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      zones.push({
        x: rect.x - HOT_ZONE_PADDING,
        y: rect.y - HOT_ZONE_PADDING,
        width: rect.width + HOT_ZONE_PADDING * 2,
        height: rect.height + HOT_ZONE_PADDING * 2,
      });
    };

    add(controlBarRef.current);
    add(repositionCornerRef.current);
    for (const el of dotClusterRefs.current.values()) {
      add(el);
    }
    for (const el of contentRefs.current.values()) {
      add(el);
    }
    setOverlayHotZones(tableId, zones).catch(() => undefined);
  }, [tableId, detailPlayer]);

  useLayoutEffect(() => {
    // One frame later as well as immediately: the first pass catches the
    // common case, the rAF catches layout that only settles after fonts or
    // the card's own content have resolved.
    reportHotZones();
    const frame = requestAnimationFrame(reportHotZones);
    return () => cancelAnimationFrame(frame);
  }, [reportHotZones, players, profile, positions, mode, hidden, detailPlayer]);

  useEffect(() => {
    window.addEventListener("resize", reportHotZones);
    // Being shown again does not re-render, and `overlay::close` deliberately
    // drops every zone, so this is what puts them back.
    const unlistenVisibility = onOverlayVisibilityChanged((change) => {
      if (change.tableId === tableId && change.visible) reportHotZones();
    }).catch(() => undefined);
    return () => {
      window.removeEventListener("resize", reportHotZones);
      unlistenVisibility.then((fn) => fn?.());
    };
  }, [reportHotZones, tableId]);

  useEffect(() => {
    function handlePointerMove(e: PointerEvent) {
      const drag = dragState.current;
      if (!drag) return;

      if (!drag.captured) {
        const dx = e.clientX - drag.startClientX;
        const dy = e.clientY - drag.startClientY;
        if (Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;
        // Real movement, not jitter: commit to a drag now, not on
        // pointerdown. Capturing only from here is what leaves a plain tap
        // (no movement before pointerup) free to fire its native `click`
        // normally — see `dragState`'s own doc comment above.
        drag.captured = true;
        drag.target.setPointerCapture(drag.pointerId);
        setDraggingId(drag.playerId);
      }

      const pxX = e.clientX - drag.offsetX;
      const pxY = e.clientY - drag.offsetY;
      const next = { x: pxX / window.innerWidth, y: pxY / window.innerHeight };
      liveDragPosition.current = next;
      setPositions((prev) => ({ ...prev, [drag.playerId]: next }));
    }

    function endDrag(persist: boolean) {
      const drag = dragState.current;
      if (!drag) return;
      dragState.current = null;
      setDraggingId(null);
      const pos = liveDragPosition.current;
      liveDragPosition.current = null;
      if (!drag.captured || !persist || !pos) return;

      const player = playersRef.current.find((p) => p.id === drag.playerId);
      // saved against the hero-relative offset, the same key the lookup
      // in `refresh` reads back. A player with no resolvable offset (a hand
      // with no hero, or no recorded table size) falls through to a manual
      // per-player position rather than writing a template under a key that
      // would mean nothing on the next table.
      const seatOffset = player?.seatOffset ?? null;
      const maxP = maxPlayersRef.current;

      if (autoCenterRef.current && maxP != null && seatOffset != null) {
        saveSeatTemplate(maxP, seatOffset, pos.x, pos.y).catch(() => undefined);
      } else {
        saveHudPosition(drag.playerId, pos.x, pos.y).catch(() => undefined);
      }
    }

    const handlePointerUp = () => endDrag(true);
    // A cancelled pointer (OS gesture, the window losing the input capture)
    // never delivers a pointerup; without this the card would stay welded to
    // the cursor — the "feels locked" half of a known issue.
    const handlePointerCancel = () => endDrag(false);

    // Subscribed once for the component's lifetime — re-subscribing on
    // every position update (as a `[positions]` dependency would do) races
    // the native pointerup event against React's render/effect timing.
    // Pointer events rather than mouse events so the card can hold pointer
    // capture (set in the drag-handle's onPointerDown below): captured
    // pointermove/pointerup are delivered to the captured element and bubble
    // here even when the cursor leaves the card or the overlay window, where
    // a plain window-level mouseup used to be lost.
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
    window.addEventListener("pointercancel", handlePointerCancel);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
      window.removeEventListener("pointercancel", handlePointerCancel);
    };
  }, []);

  async function toggleMode() {
    const next: OverlayMode = mode === "reposition" ? "normal" : "reposition";
    await setOverlayMode(tableId, next);
    setMode(next);
  }

  // optimistic and local first — the button the user just clicked
  // has to react instantly, not wait on an IPC round trip — with the actual
  // backend call still made so the main window and the hotkey agree.
  function handleHide() {
    setHidden(true);
    closeOverlay(tableId).catch(() => undefined);
  }

  function handleShow() {
    setHidden(false);
    showOverlay(tableId).catch(() => undefined);
  }

  const repositioning = mode === "reposition";
  // The Jivaro model brings its own overlay chrome with it: the
  // control bar docks bottom-left with a wordmark, and a session strip runs
  // beside it. Both are part of the design the user accepted, so they
  // follow the model rather than being global.
  const jivaro = profile?.visualModel === "jivaro";

  return (
    <div className={styles.stage}>
      <div
        ref={controlBarRef}
        className={`${styles.controlBar} ${jivaro ? styles.controlBarDocked : ""} ${
          repositioning ? styles.controlBarActive : ""
        }`}
      >
        {/* Same buttons, same ref, same hot zone — only the dock and the
            wordmark are new, so nothing about the native hit test changes. */}
        {jivaro && <span className={styles.dockLogo}>VELORA</span>}
        {hidden && (
          // the whole point — a collapsed HUD leaves exactly this one
          // small pill behind, in the same spot the control bar always sits,
          // so restoring it never means leaving the overlay.
          <button type="button" className={styles.controlButton} onClick={handleShow}>
            Show
          </button>
        )}
        {/*
         * URGENT build (live session, 2026-09-13): the user reported the
         * Reposition/Hide buttons block visibility of the table during live
         * play. Gated off by SHOW_REPOSITION_HIDE_BUTTONS below rather than
         * deleted — `toggleMode`/`handleHide`/`mode`/`repositioning` are all
         * still intact and still reachable from the main window's HUD
         * Profiles page and the global hotkey (see the effects above this
         * render); only this button pair's own render is off. Reversible by
         * flipping that one constant back to true, no other change needed.
         */}
        {!hidden && SHOW_REPOSITION_HIDE_BUTTONS && (
          <>
            <button type="button" className={styles.controlButton} onClick={toggleMode}>
              {repositioning ? "Done" : "Reposition"}
            </button>
            <button type="button" className={styles.controlButton} onClick={handleHide}>
              Hide
            </button>
          </>
        )}
      </div>

      {/*
       * URGENT build (2026-09-13, multi-tabling live): added earlier tonight
       * as a corner-anchored, Reposition-only control, separate from the
       * center `controlBar`. Confirmed unnecessary once the drag-vs-click
       * threshold fix landed (`dragState` above) — the badge itself is
       * already a registered hot zone, and pointer capture, once a
       * real drag starts, keeps the gesture routed to this window
       * regardless of the cursor leaving that zone (Win32 mouse capture
       * takes priority over the native click-through hit test), so
       * reposition mode's "disable click-through for the whole window" is no
       * longer needed just to drag a card. Gated off rather than deleted —
       * same reversibility as SHOW_REPOSITION_HIDE_BUTTONS above — since the
       * ref/hot-zone registration and `toggleMode` are still intact for
       * whatever still uses them (the main window's own Reposition control).
       */}
      {SHOW_REPOSITION_CORNER_BUTTON && !jivaro && !hidden && (
        <button
          type="button"
          ref={repositionCornerRef}
          className={`${styles.repositionCorner} ${
            repositioning ? styles.repositionCornerActive : ""
          }`}
          onClick={toggleMode}
        >
          {repositioning ? "Done" : "Reposition"}
        </button>
      )}

      {jivaro && !hidden && <SessionStrip minHands={profile?.minHands ?? 0} />}

      {!hidden && players.length === 0 && (
        <div className={styles.loadingIndicator}>
          <span className={styles.loadingDots}>
            <span />
            <span />
            <span />
          </span>
          Waiting for hand
        </div>
      )}

      {!hidden &&
        profile &&
        players.map((player, index) => {
          const pos = positions[player.id] ?? defaultPosition(index, player.seatOffset, maxPlayers);
          return (
            <div
              key={player.id}
              className={styles.cardWrap}
              style={{
                left: `${pos.x * 100}%`,
                top: `${pos.y * 100}%`,
                // Cards overlap freely once the user positions them around a
                // table; the one being dragged must stay on top of, and keep
                // receiving events over, whatever it passes under.
                zIndex: draggingId === player.id ? 5 : undefined,
              }}
            >
              <PlayerHudCard
                player={player}
                profile={profile}
                onOpenDetail={setDetailPlayer}
                // Jivaro model only: a card on the table's right-hand side
                // puts its info card outboard, as the reference does. Read
                // from the card's own saved position, so it follows the seat
                // the user dragged it to rather than a seat number.
                mirrored={pos.x > 0.5}
                dotClusterRef={(el) => {
                  if (el) {
                    dotClusterRefs.current.set(player.id, el);
                  } else {
                    dotClusterRefs.current.delete(player.id);
                  }
                }}
                contentRef={(el) => {
                  if (el) {
                    contentRefs.current.set(player.id, el);
                  } else {
                    contentRefs.current.delete(player.id);
                  }
                }}
                dragHandleProps={{
                  onPointerDown: (e) => {
                    if (e.button !== 0) return;
                    // Bug found 2026-09-13 (badge rollout): this used to call
                    // `setPointerCapture` immediately, which reliably blocked
                    // the badge's own native `click` from ever firing, since
                    // the badge is now both the drag handle and the click
                    // target on the one small element (no separate drag-only
                    // area around it the way the old full card had). Capture
                    // — which is what routes the rest of a real drag gesture
                    // to this element even if the cursor outruns it or leaves
                    // the window — is now deferred to
                    // `handlePointerMove` above, once real movement crosses
                    // `DRAG_THRESHOLD_PX`; a plain tap never captures, so its
                    // click reaches the badge exactly as any other button's
                    // would.
                    const posPx = { x: pos.x * window.innerWidth, y: pos.y * window.innerHeight };
                    dragState.current = {
                      playerId: player.id,
                      pointerId: e.pointerId,
                      target: e.currentTarget,
                      offsetX: e.clientX - posPx.x,
                      offsetY: e.clientY - posPx.y,
                      startClientX: e.clientX,
                      startClientY: e.clientY,
                      captured: false,
                    };
                    liveDragPosition.current = null;
                  },
                }}
              />
            </div>
          );
        })}

      {detailPlayer && (
        <PlayerProfileDrawer
          player={detailPlayer}
          onClose={() => setDetailPlayer(null)}
          onPlayerUpdated={setDetailPlayer}
        />
      )}
    </div>
  );
}

/**
 * The session info strip that docks beside the control bar in the Jivaro
 * model. The reference's own strip shows tournament fields Velora
 * does not compute (entrants, places paid, pot odds), so this carries the
 * session data that actually exists — the same numbers the Dashboard reads.
 *
 * Purely informational: it registers no hot zone and takes no pointer events,
 * so every click on it falls through to the table underneath.
 */
function SessionStrip({ minHands }: { minHands: number }) {
  const [summary, setSummary] = useState<DashboardSummary | null>(null);

  useEffect(() => {
    const load = () => {
      getDashboardSummary()
        .then(setSummary)
        .catch(() => undefined);
    };
    load();
    const unlisten = onHandsImported(load).catch(() => undefined);
    return () => {
      unlisten.then((fn) => fn?.());
    };
  }, []);

  const today = summary?.sessionsToday ?? null;

  return (
    <div className={styles.sessionStrip}>
      <span className={styles.sessionTitle}>
        Session
        <em>Velora</em>
      </span>
      {today ? (
        <>
          <SessionCell label="Duration" value={formatDuration(today.totalDurationSecs)} />
          <SessionCell label="Hands" value={String(today.totalHands)} />
          <SessionCell label="Sessions today" value={String(today.sessionCount)} />
          <SessionCell label="Min hands" value={String(minHands)} />
          <span className={styles.sessionSpacer} />
          {today.netResultCash != null && (
            <SessionCell
              label="Today, cash"
              value={formatNet(today.netResultCash, today.currency)}
              positive={today.netResultCash >= 0}
            />
          )}
        </>
      ) : (
        <>
          <SessionCell label="Today" value="No session yet" />
          <SessionCell label="Min hands" value={String(minHands)} />
        </>
      )}
    </div>
  );
}

function SessionCell({
  label,
  value,
  positive,
}: {
  label: string;
  value: string;
  positive?: boolean;
}) {
  return (
    <span className={styles.sessionCell}>
      <b className={positive === undefined ? "" : positive ? styles.netUp : styles.netDown}>
        {value}
      </b>
      <span>{label}</span>
    </span>
  );
}

function formatDuration(totalSecs: number): string {
  const hours = Math.floor(totalSecs / 3600);
  const minutes = Math.round((totalSecs % 3600) / 60);
  return hours > 0 ? `${hours}h ${minutes}m` : `${minutes}m`;
}

function formatNet(net: number, currency: string | null): string {
  const symbol = currency === "EUR" ? "€" : currency === "GBP" ? "£" : "$";
  const sign = net >= 0 ? "+" : "-";
  return `${sign}${symbol}${Math.abs(net).toFixed(2)}`;
}
