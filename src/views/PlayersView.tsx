import { useEffect, useState } from "react";
import type { Player } from "../data/types";
import { DesktopAppRequiredError, getPlayers, onHandsImported } from "../data/api";
import styles from "./PlayersView.module.css";

interface PlayersViewProps {
  onSelectPlayer: (player: Player) => void;
}

type LoadState =
  | { status: "loading" }
  | { status: "ready"; players: Player[] }
  | { status: "unavailable"; message: string }
  | { status: "error"; message: string };

export function PlayersView({ onSelectPlayer }: PlayersViewProps) {
  const [state, setState] = useState<LoadState>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;

    function load() {
      getPlayers()
        .then((players) => {
          if (!cancelled) setState({ status: "ready", players });
        })
        .catch((err: unknown) => {
          if (cancelled) return;
          if (err instanceof DesktopAppRequiredError) {
            setState({ status: "unavailable", message: err.message });
          } else {
            setState({ status: "error", message: String(err) });
          }
        });
    }

    load();
    const unlisten = onHandsImported(load).catch(() => undefined);

    return () => {
      cancelled = true;
      unlisten.then((fn) => fn?.());
    };
  }, []);

  return (
    <div>
      <div className="view-header">
        <h1>Players</h1>
        <p>
          {state.status === "ready"
            ? `${state.players.length} tracked players from imported hand histories.`
            : "Tracked players from imported hand histories."}
        </p>
      </div>

      {state.status === "loading" && <div className={styles.stateBox}>Loading players&hellip;</div>}

      {state.status === "unavailable" && <div className={styles.stateBox}>{state.message}</div>}

      {state.status === "error" && (
        <div className={styles.stateBox}>Failed to load players: {state.message}</div>
      )}

      {state.status === "ready" && state.players.length === 0 && (
        <div className={styles.stateBox}>
          No hands imported yet. Configure your PokerStars hand history folder in Settings to
          start tracking players.
        </div>
      )}

      {state.status === "ready" && state.players.length > 0 && (
        <div className={styles.list}>
          <div className={styles.headerRow}>
            <span>Player</span>
            <span>Hands</span>
            <span>VPIP</span>
            <span>PFR</span>
            <span>3-Bet</span>
            <span>Notes</span>
          </div>
          {state.players.map((player) => (
            <button
              key={player.id}
              type="button"
              className={styles.row}
              onClick={() => onSelectPlayer(player)}
            >
              <span className={styles.playerCell}>
                <span
                  className={styles.avatar}
                  style={
                    player.classification
                      ? { boxShadow: `0 0 0 2px ${player.classification.color}` }
                      : undefined
                  }
                >
                  {player.name.slice(0, 2).toUpperCase()}
                </span>
                <span>
                  <span className={styles.name}>{player.name}</span>
                  {player.classification && (
                    <span
                      className={styles.classificationChip}
                      style={{ color: player.classification.color }}
                    >
                      {player.classification.label}
                    </span>
                  )}
                </span>
              </span>
              <span className={`${styles.statValue} tabular`}>
                {player.hands.toLocaleString()}
              </span>
              <span className={`${styles.statValue} tabular`}>{player.stats.vpip}%</span>
              <span className={`${styles.statValue} tabular`}>{player.stats.pfr}%</span>
              <span className={`${styles.statValue} tabular`}>
                {player.stats.threeBet}%
              </span>
              <span className={styles.note}>{player.note ?? ""}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
