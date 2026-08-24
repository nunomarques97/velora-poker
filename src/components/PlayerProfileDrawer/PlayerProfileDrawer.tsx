import type { Player } from "../../data/types";
import styles from "./PlayerProfileDrawer.module.css";
import { CloseIcon } from "../icons";

interface PlayerProfileDrawerProps {
  player: Player;
  onClose: () => void;
}

export function PlayerProfileDrawer({ player, onClose }: PlayerProfileDrawerProps) {
  const { stats } = player;

  return (
    <>
      <div className={styles.backdrop} onClick={onClose} />
      <aside className={styles.drawer}>
        <div className={styles.header}>
          <div className={styles.identity}>
            <div className={styles.avatar}>{player.name.slice(0, 2).toUpperCase()}</div>
            <div>
              <div className={styles.name}>{player.name}</div>
              <div className={styles.handCount}>
                {player.hands.toLocaleString()} hands tracked
              </div>
            </div>
          </div>
          <button type="button" className={styles.closeButton} onClick={onClose}>
            <CloseIcon className={styles.closeIcon} />
          </button>
        </div>

        <div className={styles.headline}>
          <div className={styles.headlineCell}>
            <div className={`${styles.headlineValue} tabular`}>{stats.vpip}%</div>
            <div className={styles.headlineLabel}>VPIP</div>
          </div>
          <div className={styles.headlineCell}>
            <div className={`${styles.headlineValue} tabular`}>{stats.pfr}%</div>
            <div className={styles.headlineLabel}>PFR</div>
          </div>
          <div className={styles.headlineCell}>
            <div className={`${styles.headlineValue} tabular`}>{stats.threeBet}%</div>
            <div className={styles.headlineLabel}>3-Bet</div>
          </div>
        </div>

        {player.note && <p className={styles.note}>{player.note}</p>}

        <div className={styles.section}>
          <div className={styles.sectionTitle}>Preflop</div>
          <StatRow label="Fold to 3-Bet" value={`${stats.foldToThreeBet}%`} />
        </div>

        <div className={styles.section}>
          <div className={styles.sectionTitle}>Postflop</div>
          <StatRow label="C-Bet" value={`${stats.cBet}%`} />
          <StatRow label="Fold to C-Bet" value={`${stats.foldToCBet}%`} />
          <StatRow label="Aggression Factor" value={stats.aggressionFactor.toFixed(1)} />
        </div>

        <div className={styles.section}>
          <div className={styles.sectionTitle}>Showdown</div>
          <StatRow label="WTSD" value={`${stats.wtsd}%`} />
          <StatRow label="W$SD" value={`${stats.wsd}%`} />
        </div>
      </aside>
    </>
  );
}

function StatRow({ label, value }: { label: string; value: string }) {
  return (
    <div className={styles.statRow}>
      <span className={styles.statRowLabel}>{label}</span>
      <span className={`${styles.statRowValue} tabular`}>{value}</span>
    </div>
  );
}
