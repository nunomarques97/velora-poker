import { useState } from "react";
import type { Player, HudProfile } from "../data/types";
import { STAT_COLUMN_COLORS, STAT_LABELS, formatBb, formatChips, formatStatValue } from "./statFormat";
import styles from "./PlayerHudCard.module.css";

interface PlayerHudCardProps {
  player: Player;
  profile: HudProfile;
  onOpenDetail?: (player: Player) => void;
  /** Overlay usage passes a drag handle down; main-app previews leave it off. */
  dragHandleProps?: React.HTMLAttributes<HTMLDivElement>;
}

export function PlayerHudCard({ player, profile, onOpenDetail, dragHandleProps }: PlayerHudCardProps) {
  const [pageIndex, setPageIndex] = useState(0);
  const pages = profile.statPages.length > 0 ? profile.statPages : [];
  const activePage = pages[pageIndex] ?? pages[0];
  const classification = player.classification;
  const color = classification?.color ?? "#6b7480";
  const label = classification?.label ?? "Unknown";
  const initials = player.name.slice(0, 2).toUpperCase();

  return (
    <div
      className={`${styles.card} ${styles[profile.visualModel] ?? ""}`}
      style={{ ["--player-color" as string]: color }}
    >
      <div className={styles.dragHandle} {...dragHandleProps}>
        <div className={styles.ring}>
          <div className={styles.ringSegments} />
          <div className={styles.avatar}>{initials}</div>
          <span className={styles.handsInRing}>{player.hands}</span>
          {pages.length > 1 && (
            <div className={styles.dots} onMouseDown={(e) => e.stopPropagation()}>
              {pages.map((page, idx) => (
                <button
                  key={page.id}
                  type="button"
                  className={`${styles.dot} ${idx === pageIndex ? styles.dotActive : ""}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    setPageIndex(idx);
                  }}
                  aria-label={`Show ${page.label} stats`}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      <button
        type="button"
        className={styles.content}
        onClick={() => onOpenDetail?.(player)}
        aria-label={`Open detailed stats for ${player.name}`}
      >
        <div className={styles.identityRow}>
          <span className={styles.name} title={player.name}>
            {player.name}
          </span>
          <span className={styles.classificationLabel}>{label}</span>
        </div>

        {player.snapshot ? (
          <div className={styles.stackRow}>
            <span className={styles.chips}>
              {formatChips(player.snapshot.stack, player.snapshot.currency, player.snapshot.format)}
            </span>
            <span className={styles.bbValue}>{formatBb(player.snapshot.stackBb)}</span>
            <span className={styles.bbLabel}>BB</span>
          </div>
        ) : (
          <div className={styles.stackRow}>
            <span className={styles.noSnapshot}>No recent hand</span>
          </div>
        )}

        {activePage && (
          <div className={styles.statRow} key={activePage.id}>
            {activePage.statKeys.map((key, i) => (
              <div key={key} className={styles.statCell}>
                <span
                  className={`${styles.statValue} tabular`}
                  style={{ color: STAT_COLUMN_COLORS[i % STAT_COLUMN_COLORS.length] }}
                >
                  {formatStatValue(key, player.stats)}
                </span>
                <span className={styles.statLabel}>{STAT_LABELS[key]}</span>
              </div>
            ))}
          </div>
        )}
      </button>
    </div>
  );
}
