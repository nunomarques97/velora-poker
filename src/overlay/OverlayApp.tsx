import { useCallback, useEffect, useRef, useState } from "react";
import type { HudProfile, Player } from "../data/types";
import {
  closeOverlay,
  getActiveHudProfile,
  getActiveTableMaxPlayers,
  getActiveTablePlayers,
  getAppSettings,
  getHudPositions,
  getSeatTemplates,
  onHandsImported,
  saveHudPosition,
  saveSeatTemplate,
  setOverlayClickThrough,
} from "../data/api";
import { PlayerHudCard } from "../hud/PlayerHudCard";
import styles from "./OverlayApp.module.css";

/** Fractions (0..1) of the overlay window — see `HudPosition`/`SeatTemplate` docs (Phase E). */
interface PositionMap {
  [key: string]: { x: number; y: number };
}

const DEFAULT_COL_FRACTION = 0.16;
const DEFAULT_ROW_FRACTION = 0.1;
const DEFAULT_MARGIN = 0.02;
const DEFAULT_COLS_PER_ROW = 6;

function defaultPosition(index: number) {
  const col = index % DEFAULT_COLS_PER_ROW;
  const row = Math.floor(index / DEFAULT_COLS_PER_ROW);
  return {
    x: DEFAULT_MARGIN + col * DEFAULT_COL_FRACTION,
    y: DEFAULT_MARGIN + row * DEFAULT_ROW_FRACTION,
  };
}

export function OverlayApp() {
  const [players, setPlayers] = useState<Player[]>([]);
  const [profile, setProfile] = useState<HudProfile | null>(null);
  const [positions, setPositions] = useState<PositionMap>({});
  const [autoCenterEnabled, setAutoCenterEnabled] = useState(false);
  const [maxPlayers, setMaxPlayers] = useState<number | null>(null);
  const [locked, setLocked] = useState(false);
  /** Card currently being dragged — raised above its neighbours while it moves. */
  const [draggingId, setDraggingId] = useState<string | null>(null);

  const dragState = useRef<{ playerId: string; offsetX: number; offsetY: number } | null>(null);
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

  const refresh = useCallback(() => {
    Promise.all([
      getActiveTablePlayers(),
      getActiveHudProfile(),
      getHudPositions(),
      getActiveTableMaxPlayers(),
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
            seatMap[String(t.seat)] = { x: t.x, y: t.y };
          }
        }

        setPlayers(allPlayers);
        setProfile(activeProfile);
        setAutoCenterEnabled(autoCenter);
        setMaxPlayers(activeMaxPlayers);

        setPositions((prev) => {
          const next: PositionMap = { ...prev };
          allPlayers.forEach((player, index) => {
            const seatKey = player.seat != null ? String(player.seat) : null;
            const fromTemplate = autoCenter && seatKey ? seatMap[seatKey] : undefined;
            next[player.id] = fromTemplate ?? manual[player.id] ?? next[player.id] ?? defaultPosition(index);
          });
          return next;
        });
      })
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    refresh();
    const unlisten = onHandsImported(refresh).catch(() => undefined);
    return () => {
      unlisten.then((fn) => fn?.());
    };
  }, [refresh]);

  useEffect(() => {
    function handlePointerMove(e: PointerEvent) {
      const drag = dragState.current;
      if (!drag) return;
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
      if (!persist || !pos) return;

      const player = playersRef.current.find((p) => p.id === drag.playerId);
      const seat = player?.seat ?? null;
      const maxP = maxPlayersRef.current;

      if (autoCenterRef.current && maxP != null && seat != null) {
        saveSeatTemplate(maxP, seat, pos.x, pos.y).catch(() => undefined);
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

  async function toggleLock() {
    const next = !locked;
    await setOverlayClickThrough(next);
    setLocked(next);
  }

  return (
    <div className={styles.stage}>
      <div className={styles.controlBar}>
        <button type="button" className={styles.controlButton} onClick={toggleLock}>
          {locked ? "Unlock" : "Lock"}
        </button>
        <button type="button" className={styles.controlButton} onClick={() => closeOverlay()}>
          Close
        </button>
      </div>

      {profile &&
        players.map((player, index) => {
          const pos = positions[player.id] ?? defaultPosition(index);
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
                dragHandleProps={{
                  onPointerDown: (e) => {
                    if (e.button !== 0) return;
                    // Capture routes the rest of this gesture to the card even
                    // if the cursor outruns it or leaves the window — see the
                    // window listeners above.
                    e.currentTarget.setPointerCapture(e.pointerId);
                    const posPx = { x: pos.x * window.innerWidth, y: pos.y * window.innerHeight };
                    dragState.current = {
                      playerId: player.id,
                      offsetX: e.clientX - posPx.x,
                      offsetY: e.clientY - posPx.y,
                    };
                    liveDragPosition.current = null;
                    setDraggingId(player.id);
                  },
                }}
              />
            </div>
          );
        })}
    </div>
  );
}
