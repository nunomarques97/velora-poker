import { useState } from "react";
import type { Player, HudProfile } from "../data/types";
import { STAT_COLUMN_COLORS, STAT_LABELS, formatBb, formatChips, formatStatValue } from "./statFormat";
import styles from "./PlayerHudCard.module.css";

/**
 * Sample-size confidence shading (Phase E). Stat values fade toward
 * `MIN_STAT_OPACITY` as a player's hand count drops toward zero, reaching full
 * opacity at the active profile's `min_hands` — the same threshold that
 * already gates archetype classification, reused deliberately as the single
 * confidence anchor rather than computing each stat's own denominator (spec:
 * simplicity over precision for a purely cosmetic signal).
 *
 * Purely visual: it never hides a value and never gates classification.
 */
const MIN_STAT_OPACITY = 0.35;

function sampleConfidence(hands: number, minHands: number): number {
  if (minHands <= 0) return 1;
  const ratio = Math.min(1, Math.max(0, hands / minHands));
  return MIN_STAT_OPACITY + (1 - MIN_STAT_OPACITY) * ratio;
}

interface PlayerHudCardProps {
  player: Player;
  profile: HudProfile;
  onOpenDetail?: (player: Player) => void;
  /**
   * Overlay usage passes drag handlers down; main-app previews leave it off.
   * These land on the card root, so the whole card is the drag surface — it
   * used to be only the 64px avatar ring, which made repositioning fiddly
   * (Phase E polish, a known issue). Only the pagination dots opt out.
   */
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
  const statConfidence = sampleConfidence(player.hands, profile.minHands);

  return (
    <div
      className={`${styles.card} ${styles[profile.visualModel] ?? ""} ${
        dragHandleProps ? styles.draggable : ""
      }`}
      style={{ ["--player-color" as string]: color }}
      {...dragHandleProps}
    >
      <div className={styles.ring}>
        <div className={styles.ringSegments} />
        <div className={styles.avatar}>{initials}</div>
        <span className={styles.handsInRing}>{player.hands}</span>
        {pages.length > 1 && (
          <div className={styles.dots} onPointerDown={(e) => e.stopPropagation()}>
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
          <div
            className={styles.statRow}
            key={activePage.id}
            style={{ ["--stat-confidence" as string]: statConfidence }}
          >
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
