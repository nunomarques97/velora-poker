import type { Player } from "../data/types";
import { players } from "../data/mockData";
import styles from "./PlayersView.module.css";

interface PlayersViewProps {
  onSelectPlayer: (player: Player) => void;
}

export function PlayersView({ onSelectPlayer }: PlayersViewProps) {
  return (
    <div>
      <div className="view-header">
        <h1>Players</h1>
        <p>{players.length} tracked players &middot; click a player to view details.</p>
      </div>

      <div className={styles.list}>
        <div className={styles.headerRow}>
          <span>Player</span>
          <span>Hands</span>
          <span>VPIP</span>
          <span>PFR</span>
          <span>3-Bet</span>
          <span>Notes</span>
        </div>
        {players.map((player) => (
          <button
            key={player.id}
            type="button"
            className={styles.row}
            onClick={() => onSelectPlayer(player)}
          >
            <span className={styles.playerCell}>
              <span className={styles.avatar}>
                {player.name.slice(0, 2).toUpperCase()}
              </span>
              <span className={styles.name}>{player.name}</span>
            </span>
            <span className={`${styles.statValue} tabular`}>
              {player.hands.toLocaleString()}
            </span>
            <span className={`${styles.statValue} tabular`}>{player.stats.vpip}%</span>
            <span className={`${styles.statValue} tabular`}>{player.stats.pfr}%</span>
            <span className={`${styles.statValue} tabular`}>
              {player.stats.threeBet}%
            </span>
            <span className={styles.note}>{player.note}</span>
          </button>
        ))}
      </div>
    </div>
  );
}
