import type { Player } from "../../data/types";
import styles from "./HudTableMock.module.css";

interface SeatPosition {
  top: string;
  left: string;
}

const SEAT_POSITIONS: SeatPosition[] = [
  { top: "4%", left: "50%" },
  { top: "26%", left: "89%" },
  { top: "78%", left: "89%" },
  { top: "98%", left: "50%" },
  { top: "78%", left: "11%" },
  { top: "26%", left: "11%" },
];

interface HudTableMockProps {
  seats: Player[];
  onSelectPlayer: (player: Player) => void;
}

function formatHands(hands: number): string {
  if (hands >= 1000) {
    return `${(hands / 1000).toFixed(1)}k hands`;
  }
  return `${hands} hands`;
}

export function HudTableMock({ seats, onSelectPlayer }: HudTableMockProps) {
  return (
    <div className={styles.tableWrap}>
      <div className={styles.tableChrome}>
        <span className={styles.tableTitle}>
          <strong>NL50</strong> &middot; 6-max cash
        </span>
        <span className={styles.tableStakes}>Preview &middot; mock data</span>
      </div>

      <div className={styles.felt}>
        <span className={styles.feltLabel}>Velora HUD preview</span>
        {seats.map((player, index) => {
          const pos = SEAT_POSITIONS[index % SEAT_POSITIONS.length];
          return (
            <div
              key={player.id}
              className={styles.seat}
              style={{ top: pos.top, left: pos.left }}
            >
              <div className={styles.seatAvatar}>
                {player.name.slice(0, 2).toUpperCase()}
              </div>
              <button
                type="button"
                className={styles.hudTag}
                onClick={() => onSelectPlayer(player)}
              >
                <span className={styles.hudName}>{player.name}</span>
                <span className={`${styles.hudLine} tabular`}>
                  {Math.round(player.stats.vpip)} / {Math.round(player.stats.pfr)} /{" "}
                  {Math.round(player.stats.threeBet)}
                </span>
                <span className={styles.hudHands}>{formatHands(player.hands)}</span>
              </button>
            </div>
          );
        })}
      </div>

      <p className={styles.legend}>VPIP / PFR / 3-Bet &middot; click a player tag to open their profile</p>
    </div>
  );
}
