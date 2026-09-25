import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { HudProfile, Player, SeatFrame, SeatPosition } from "../data/types";
import {
  getActiveHudProfile,
  getActiveTableMaxPlayers,
  getActiveTablePlayers,
  getAppSettings,
  getSeatPositions,
  isOverlayDismissed,
  onHandsImported,
  onOverlayDismissedChanged,
  onOverlayVisibilityChanged,
  onSeatTemplatesChanged,
  saveSeatPosition,
  setOverlayHotZones,
  showOverlay,
  type OverlayHotZone,
} from "../data/api";
import { PlayerHudCard } from "../hud/PlayerHudCard";
import { PlayerProfileDrawer } from "../components/PlayerProfileDrawer/PlayerProfileDrawer";
import { chipMetrics, clampOverride, layoutSeats, seatSlot, type ChipModel, type Point } from "./seatLayout";
import styles from "./OverlayApp.module.css";

/**
 * Grown around every chip before it is registered as a hot zone, so a press
 * on the chip's very edge still lands on it instead of on the table. Small:
 * a 4px ring around a ~20px-high chip.
 */
const HOT_ZONE_PADDING = 4;

/**
 * Sub-pixel jitter (touchpads, some mice) must never start a drag or block a
 * click; real intent to drag reliably exceeds this within the first couple
 * of pixels.
 */
const DRAG_THRESHOLD_PX = 4;

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

export function OverlayApp() {
  if (TABLE_ID === null) {
    return null;
  }
  return <TableOverlay tableId={TABLE_ID} />;
}

/** Where one player's chip goes, as the render needs it. */
interface ChipSlot {
  player: Player;
  /** `null` for a spare slot: the chip shows, but has no seat to save a drag against. */
  seatKey: number | null;
  centre: Point;
}

/** A press on a chip that may turn into a drag. */
interface DragState {
  playerId: string;
  seatKey: number;
  maxPlayers: number;
  frame: SeatFrame;
  pointerId: number;
  target: HTMLElement;
  /** Pointer position minus the chip centre, in px, so the chip does not jump under the cursor. */
  offsetX: number;
  offsetY: number;
  startClientX: number;
  startClientY: number;
  /** Pointer capture is deferred until the press really moves, so a plain click still clicks. */
  captured: boolean;
}

function windowSize() {
  return { w: window.innerWidth, h: window.innerHeight };
}

/** One tracked table's HUD. Every backend call it makes names its own table. */
function TableOverlay({ tableId }: { tableId: number }) {
  const [players, setPlayers] = useState<Player[]>([]);
  const [profile, setProfile] = useState<HudProfile | null>(null);
  const [maxPlayers, setMaxPlayers] = useState<number | null>(null);
  // Which seat keys a drag is saved against: the hero's offset when the
  // user declared PokerStars' "Auto-Center me" (the hero is then always
  // drawn bottom centre), PokerStars' own seat number otherwise.
  const [frame, setFrame] = useState<SeatFrame>("absolute");
  /** Saved chip centres for this table's size and frame, shared with every table like it. */
  const [overrides, setOverrides] = useState<SeatPosition[]>([]);
  // Whether this table's HUD is collapsed to a "Show" pill (main window
  // table list or the global hotkey). A *content* flag: the window itself
  // stays, which is what lets the pill be clicked at all.
  const [hidden, setHidden] = useState(false);
  const [detailPlayer, setDetailPlayer] = useState<Player | null>(null);
  const [size, setSize] = useState(windowSize);
  /** The widest/tallest chip actually rendered, so the layout spaces real chips, not nominal ones. */
  const [measuredChip, setMeasuredChip] = useState<{ widthPx: number; heightPx: number } | null>(null);
  /** The chip being dragged and its live centre. Only the release saves it. */
  const [drag, setDrag] = useState<{ playerId: string; centre: Point } | null>(null);

  const dragState = useRef<DragState | null>(null);
  // Mirrors `drag` so the window-level pointerup handler reads the final
  // position synchronously, not a render behind the last pointermove.
  const liveCentre = useRef<Point | null>(null);
  // A drag ends in a native `click` on the chip it started on (pointer
  // capture keeps both ends on it); this swallows that one click so a drag
  // never also opens the detail panel.
  const suppressClick = useRef(false);
  // Every refresh takes a ticket; only the newest one may write state, so a
  // slow response can never overwrite a newer one (or a just-dropped chip).
  const refreshSeq = useRef(0);
  const maxPlayersRef = useRef<number | null>(null);
  maxPlayersRef.current = maxPlayers;

  // Hot zones: the only points of this window that take clicks. Held as DOM
  // refs because the native hit test needs where these actually painted.
  const chipRefs = useRef<Map<string, HTMLElement>>(new Map());
  const showPillRef = useRef<HTMLButtonElement | null>(null);
  const panelRef = useRef<HTMLElement | null>(null);

  const refresh = useCallback(() => {
    const seq = ++refreshSeq.current;
    Promise.all([
      getActiveTablePlayers(tableId),
      getActiveHudProfile(),
      getActiveTableMaxPlayers(tableId),
      getAppSettings(),
    ])
      .then(async ([allPlayers, activeProfile, activeMaxPlayers, appSettings]) => {
        const nextFrame: SeatFrame = appSettings.autoCenterEnabled ? "hero" : "absolute";
        // A failed read keeps the positions already on screen rather than
        // snapping every chip back to its default.
        const saved =
          activeMaxPlayers != null
            ? await getSeatPositions(activeMaxPlayers, nextFrame).catch(() => null)
            : [];
        if (seq !== refreshSeq.current) return;
        setPlayers(allPlayers);
        setProfile(activeProfile);
        setMaxPlayers(activeMaxPlayers);
        setFrame(nextFrame);
        if (saved) setOverrides(saved);
        setDetailPlayer((open) => (open ? allPlayers.find((p) => p.id === open.id) ?? open : open));
      })
      .catch(() => undefined);
  }, [tableId]);

  useEffect(() => {
    refresh();
    const unlisten = onHandsImported(refresh).catch(() => undefined);
    // The overlay webview stays mounted while the window is hidden, so being
    // shown again produces no render of its own.
    const unlistenVisibility = onOverlayVisibilityChanged((change) => {
      if (change.tableId === tableId && change.visible) refresh();
    }).catch(() => undefined);
    // A drag (or a reset) on any table of this size applies here too.
    const unlistenPositions = onSeatTemplatesChanged((changedSize) => {
      if (changedSize === null || changedSize === maxPlayersRef.current) refresh();
    }).catch(() => undefined);
    return () => {
      unlisten.then((fn) => fn?.());
      unlistenVisibility.then((fn) => fn?.());
      unlistenPositions.then((fn) => fn?.());
    };
  }, [refresh, tableId]);

  // Collapsed/expanded, whichever of the main window's table list or the
  // global hotkey changed it. A dedicated event, not visibility: a
  // minimize/restore must never un-collapse a HUD the user dismissed.
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

  useEffect(() => {
    const onResize = () => setSize(windowSize());
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  const model: ChipModel = profile?.visualModel === "badge" ? "badge" : "compact";
  const nominalChip = chipMetrics(size.w, size.h, model);

  // Every chip's centre. A player sits on a seat key when this table's size
  // is known and the key is a seat of it; the first player on a key keeps
  // it, anyone else (no seat, a duplicate) gets a collision-free spare slot.
  const slots = useMemo<ChipSlot[]>(() => {
    // Size unknown (no hand read yet): every chip takes a spare slot, laid
    // out around the more likely table picture.
    const effectiveMax = maxPlayers ?? (players.length > 6 ? 9 : 6);
    const seated = new Map<number, Player>();
    const spare: Player[] = [];
    for (const player of players) {
      const key = frame === "hero" ? player.seatOffset : player.seat;
      const valid = maxPlayers != null && key != null && seatSlot(effectiveMax, frame, key) !== null;
      if (valid && !seated.has(key)) seated.set(key, player);
      else spare.push(player);
    }
    const heroSeat = players.find((p) => p.seatOffset === 0)?.seat ?? null;
    const layout = layoutSeats({
      maxPlayers: effectiveMax,
      frame,
      windowW: size.w,
      windowH: size.h,
      seats: [...seated.keys()],
      overrides,
      model,
      chip: measuredChip ?? undefined,
      heroSeatKey: frame === "absolute" ? heroSeat : null,
      spareCount: spare.length,
    });
    const result: ChipSlot[] = [];
    for (const placement of layout.positions) {
      const player = seated.get(placement.seatKey as number);
      if (player) result.push({ player, seatKey: placement.seatKey, centre: placement });
    }
    spare.forEach((player, i) => {
      const placement = layout.spares[i];
      if (placement) result.push({ player, seatKey: null, centre: placement });
    });
    return result;
  }, [players, maxPlayers, frame, overrides, size, model, measuredChip]);

  // Measure the chips as painted (their text decides their width) and lay
  // out again with that size. Converges in one pass: moving a chip never
  // changes its size.
  useLayoutEffect(() => {
    let widthPx = 0;
    let heightPx = 0;
    for (const el of chipRefs.current.values()) {
      widthPx = Math.max(widthPx, el.offsetWidth);
      heightPx = Math.max(heightPx, el.offsetHeight);
    }
    const next = widthPx > 0 && heightPx > 0 ? { widthPx, heightPx } : null;
    setMeasuredChip((prev) =>
      prev === next ||
      (prev && next && Math.abs(prev.widthPx - next.widthPx) < 0.5 && Math.abs(prev.heightPx - next.heightPx) < 0.5)
        ? prev
        : next,
    );
  }, [slots, hidden, model, nominalChip.fontPx]);

  /**
   * Re-registers the clickable rectangles with the native hit test: every
   * chip (with a small padding), the Show pill and the open detail panel.
   * Nothing else, ever: every other point of the window passes clicks to
   * the table, with or without an open panel. Re-run after every layout that
   * could move one, because a stale rect would go on swallowing table clicks
   * where nothing of Velora's is drawn any more.
   */
  const reportHotZones = useCallback(() => {
    const zones: OverlayHotZone[] = [];
    const add = (el: HTMLElement | null | undefined, padding: number) => {
      if (!el) return;
      const rect = el.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;
      zones.push({
        x: rect.x - padding,
        y: rect.y - padding,
        width: rect.width + padding * 2,
        height: rect.height + padding * 2,
      });
    };
    for (const el of chipRefs.current.values()) add(el, HOT_ZONE_PADDING);
    add(showPillRef.current, HOT_ZONE_PADDING);
    add(panelRef.current, 0);
    setOverlayHotZones(tableId, zones).catch(() => undefined);
  }, [tableId]);

  useLayoutEffect(() => {
    // Now and one frame later: the rAF catches layout that settles only
    // after fonts load.
    reportHotZones();
    const frameId = requestAnimationFrame(reportHotZones);
    return () => cancelAnimationFrame(frameId);
  }, [reportHotZones, slots, drag, hidden, detailPlayer, size, measuredChip]);

  useEffect(() => {
    // `overlay::close` drops every zone, so being shown again puts them back.
    const unlisten = onOverlayVisibilityChanged((change) => {
      if (change.tableId === tableId && change.visible) reportHotZones();
    }).catch(() => undefined);
    return () => {
      unlisten.then((fn) => fn?.());
    };
  }, [reportHotZones, tableId]);

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  useEffect(() => {
    function handlePointerMove(e: PointerEvent) {
      const state = dragState.current;
      if (!state) return;
      if (!state.captured) {
        if (Math.hypot(e.clientX - state.startClientX, e.clientY - state.startClientY) < DRAG_THRESHOLD_PX) {
          return;
        }
        // Real movement: commit to a drag. Capture keeps the rest of the
        // gesture on this chip even when the cursor outruns it or leaves
        // the hot zone, so no window-wide "unlocked" state is ever needed.
        state.captured = true;
        try {
          state.target.setPointerCapture(state.pointerId);
        } catch {
          // The pointer is already gone; the drag still follows window events.
        }
      }
      const { w, h } = windowSize();
      const el = state.target;
      const centre = clampOverride(
        { x: (e.clientX - state.offsetX) / w, y: (e.clientY - state.offsetY) / h },
        { scale: 1, fontPx: 0, widthPx: el.offsetWidth, heightPx: el.offsetHeight },
        w,
        h,
      );
      liveCentre.current = centre;
      setDrag({ playerId: state.playerId, centre });
    }

    function endDrag(persist: boolean) {
      const state = dragState.current;
      if (!state) return;
      dragState.current = null;
      const centre = liveCentre.current;
      liveCentre.current = null;
      setDrag(null);
      if (!state.captured || !centre) return;
      // A cancelled pointer never clicks. Otherwise the release's click is
      // dispatched before this timeout runs, so the flag never outlives it
      // (a later keyboard Enter on the chip still opens the detail).
      if (!persist) return;
      suppressClick.current = true;
      setTimeout(() => {
        suppressClick.current = false;
      }, 0);
      // In-flight refreshes read the old positions; drop them. The backend's
      // `seat-templates-changed` brings the saved one back to every overlay
      // of this size, this one included.
      refreshSeq.current += 1;
      const saved: SeatPosition = { seatKey: state.seatKey, x: centre.x, y: centre.y };
      setOverrides((prev) => [...prev.filter((p) => p.seatKey !== state.seatKey), saved]);
      saveSeatPosition(state.maxPlayers, state.frame, state.seatKey, centre.x, centre.y).catch(() =>
        // Not saved: show what is really stored instead of a position that
        // would vanish on the next hand.
        refreshRef.current(),
      );
    }

    const handlePointerUp = () => endDrag(true);
    // A cancelled pointer (OS gesture, capture lost) never delivers a
    // pointerup: drop the drag where it started, and save nothing.
    const handlePointerCancel = () => endDrag(false);

    // Subscribed once: re-subscribing per render would race the native
    // pointerup against React's effect timing.
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
    window.addEventListener("pointercancel", handlePointerCancel);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
      window.removeEventListener("pointercancel", handlePointerCancel);
    };
  }, []);

  const detailRef = useRef<Player | null>(null);
  detailRef.current = detailPlayer;
  const closeDetail = useCallback(() => {
    const open = detailRef.current;
    setDetailPlayer(null);
    // Back to the chip that opened it, so the keyboard stays on the table's HUD.
    if (open) chipRefs.current.get(open.id)?.focus({ preventScroll: true });
  }, []);

  useEffect(() => {
    if (!detailPlayer) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeDetail();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [detailPlayer, closeDetail]);

  function toggleDetail(player: Player) {
    if (suppressClick.current) {
      suppressClick.current = false;
      return;
    }
    setDetailPlayer((open) => (open?.id === player.id ? null : player));
  }

  function handlePlayerUpdated(updated: Player) {
    setDetailPlayer(updated);
    setPlayers((prev) => prev.map((p) => (p.id === updated.id ? updated : p)));
  }

  function handleShow() {
    setHidden(false);
    showOverlay(tableId).catch(() => undefined);
  }

  const detailSlot = detailPlayer ? slots.find((s) => s.player.id === detailPlayer.id) : undefined;
  const chipVars = {
    ["--chip-font" as string]: `${nominalChip.fontPx}px`,
    ["--chip-w" as string]: `${nominalChip.widthPx}px`,
    ["--chip-h" as string]: `${Math.max(nominalChip.heightPx, nominalChip.fontPx + 6)}px`,
  };

  if (hidden) {
    return (
      <div className={styles.stage}>
        <button type="button" ref={showPillRef} className={styles.showPill} onClick={handleShow}>
          Show HUD
        </button>
      </div>
    );
  }

  return (
    <div className={styles.stage} style={chipVars}>
      {players.length === 0 && (
        <div className={styles.loadingIndicator} role="status">
          <span className={styles.loadingDots} aria-hidden="true">
            <span />
            <span />
            <span />
          </span>
          Waiting for hand
        </div>
      )}

      {profile &&
        slots.map(({ player, seatKey, centre }) => {
          const dragging = drag?.playerId === player.id;
          const at = dragging ? drag.centre : centre;
          const draggable = seatKey !== null && maxPlayers !== null;
          return (
            <div
              key={player.id}
              className={styles.chipWrap}
              data-hud-chip={player.id}
              data-seat-key={seatKey ?? ""}
              style={{
                left: `${at.x * 100}%`,
                top: `${at.y * 100}%`,
                // The chip being dragged stays above whatever it passes over.
                zIndex: dragging ? 5 : undefined,
              }}
            >
              <PlayerHudCard
                player={player}
                profile={profile}
                onOpenDetail={toggleDetail}
                expanded={detailPlayer?.id === player.id}
                dragging={dragging}
                contentRef={(el) => {
                  if (el) chipRefs.current.set(player.id, el);
                  else chipRefs.current.delete(player.id);
                }}
                dragHandleProps={
                  draggable
                    ? {
                        onPointerDown: (e) => {
                          if (e.button !== 0) return;
                          suppressClick.current = false;
                          const { w, h } = windowSize();
                          dragState.current = {
                            playerId: player.id,
                            seatKey,
                            maxPlayers,
                            frame,
                            pointerId: e.pointerId,
                            target: e.currentTarget,
                            offsetX: e.clientX - at.x * w,
                            offsetY: e.clientY - at.y * h,
                            startClientX: e.clientX,
                            startClientY: e.clientY,
                            captured: false,
                          };
                          liveCentre.current = null;
                        },
                      }
                    : undefined
                }
              />
            </div>
          );
        })}

      {detailPlayer && (
        <PlayerProfileDrawer
          variant="panel"
          // Docked away from the chip, so the chip stays visible and can be
          // clicked again to close.
          side={detailSlot && detailSlot.centre.x > 0.5 ? "left" : "right"}
          panelRef={(el) => {
            panelRef.current = el;
          }}
          player={detailPlayer}
          onClose={closeDetail}
          onPlayerUpdated={handlePlayerUpdated}
        />
      )}
    </div>
  );
}
