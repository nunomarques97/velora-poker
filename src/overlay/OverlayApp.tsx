import { useCallback, useEffect, useRef, useState } from "react";
import type { HudProfile, Player } from "../data/types";
import {
  closeOverlay,
  getActiveHudProfile,
  getActiveTablePlayers,
  getHudPositions,
  onHandsImported,
  saveHudPosition,
  setOverlayClickThrough,
} from "../data/api";
import { PlayerHudCard } from "../hud/PlayerHudCard";
import styles from "./OverlayApp.module.css";

interface PositionMap {
  [playerId: string]: { x: number; y: number };
}

const DEFAULT_SPACING = 270;
const DEFAULT_ROW_HEIGHT = 100;

export function OverlayApp() {
  const [players, setPlayers] = useState<Player[]>([]);
  const [profile, setProfile] = useState<HudProfile | null>(null);
  const [positions, setPositions] = useState<PositionMap>({});
  const [locked, setLocked] = useState(false);
  const dragState = useRef<{ playerId: string; offsetX: number; offsetY: number } | null>(null);
  // Mirrors the in-progress drag position outside React state so the
  // mouseup handler always reads the latest value synchronously, instead
  // of a `positions` state closure that can be stale relative to the final
  // native mousemove event.
  const liveDragPosition = useRef<{ x: number; y: number } | null>(null);

  const refresh = useCallback(() => {
    Promise.all([getActiveTablePlayers(), getActiveHudProfile(), getHudPositions()])
      .then(([allPlayers, activeProfile, savedPositions]) => {
        setPlayers(allPlayers);
        setProfile(activeProfile);
        setPositions((prev) => {
          const next: PositionMap = { ...prev };
          for (const pos of savedPositions) {
            next[pos.playerId] = { x: pos.x, y: pos.y };
          }
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
    function handleMouseMove(e: MouseEvent) {
      const drag = dragState.current;
      if (!drag) return;
      const next = { x: e.clientX - drag.offsetX, y: e.clientY - drag.offsetY };
      liveDragPosition.current = next;
      setPositions((prev) => ({ ...prev, [drag.playerId]: next }));
    }

    function handleMouseUp() {
      const drag = dragState.current;
      if (!drag) return;
      dragState.current = null;
      const pos = liveDragPosition.current;
      liveDragPosition.current = null;
      if (pos) {
        saveHudPosition(drag.playerId, pos.x, pos.y).catch(() => undefined);
      }
    }

    // Subscribed once for the component's lifetime — re-subscribing on
    // every position update (as a `[positions]` dependency would do) races
    // the native mouseup event against React's render/effect timing.
    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
    return () => {
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
    };
  }, []);

  async function toggleLock() {
    const next = !locked;
    await setOverlayClickThrough(next);
    setLocked(next);
  }

  function defaultPosition(index: number) {
    const col = index % 6;
    const row = Math.floor(index / 6);
    return { x: 24 + col * DEFAULT_SPACING, y: 24 + row * DEFAULT_ROW_HEIGHT };
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
              style={{ left: pos.x, top: pos.y }}
            >
              <PlayerHudCard
                player={player}
                profile={profile}
                dragHandleProps={{
                  onMouseDown: (e) => {
                    dragState.current = {
                      playerId: player.id,
                      offsetX: e.clientX - pos.x,
                      offsetY: e.clientY - pos.y,
                    };
                  },
                }}
              />
            </div>
          );
        })}
    </div>
  );
}
