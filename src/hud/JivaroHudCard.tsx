import { useState } from "react";
import type { HudProfile, Player, PlayerStats } from "../data/types";
import { STAT_LABELS, formatBb, formatChips } from "./statFormat";
import {
  BAND_UNKNOWN,
  STAT_BANDS,
  bandColor,
  bandFraction,
  formatBandValue,
} from "./statBands";
import styles from "./JivaroHudCard.module.css";

/**
 * The Jivaro visual model (the notes), built from the accepted mock
 * at the notes.
 *
 * It is a separate component rather than another branch inside
 * `PlayerHudCard`, because its DOM is genuinely different: the three existing
 * models share one skeleton (ring on the left, a column of text on the right)
 * and differ only in CSS, while this one is a segmented gauge with an info
 * card tucked *behind* it. Keeping it apart also means the three shipped
 * models' markup and stylesheet are untouched by this addition.
 *
 * Everything below the presentation is shared with them: the same stat pages,
 * the same `min_hands` gate, and the same classification colour resolved by
 * `classification::resolve_for_player` (so the automatic archetype is absent
 * in distributed builds and a manual override always renders —).
 */

/** Arc ticks per stat. 9 reads as a scale at the ring's real 80px size. */
const TICKS_PER_ARC = 9;
/** Bottom gap left for the hand count, in degrees — the reference's own. */
const RING_GAP_DEG = 42;

interface RingTick {
  angle: number;
  color: string;
}

/**
 * One arc group per stat on the active page, each sweeping its share of the
 * ring clockwise from the bottom gap. Lit ticks carry the value, unlit ticks
 * the remainder of that stat's scale.
 */
function ringTicks(statKeys: (keyof PlayerStats)[], stats: PlayerStats, gated: boolean): RingTick[] {
  const groups = Math.max(1, statKeys.length);
  const span = (360 - RING_GAP_DEG) / groups;
  const step = span / TICKS_PER_ARC;
  const ticks: RingTick[] = [];

  statKeys.forEach((key, groupIndex) => {
    const value = stats[key];
    const lit = gated ? 0 : Math.round(bandFraction(key, value) * TICKS_PER_ARC);
    const color = gated ? BAND_UNKNOWN : bandColor(key, value);
    const start = 180 + RING_GAP_DEG / 2 + groupIndex * span;
    for (let i = 0; i < TICKS_PER_ARC; i += 1) {
      ticks.push({
        angle: start + i * step + step / 2,
        // Unlit ticks are the same hue at low alpha, so an arc still reads as
        // one stat's scale rather than as two unrelated runs of colour.
        color: i < lit ? color : `color-mix(in srgb, ${color} 26%, #0b0e12)`,
      });
    }
  });

  return ticks;
}

interface JivaroHudCardProps {
  player: Player;
  profile: HudProfile;
  onOpenDetail?: (player: Player) => void;
  dragHandleProps?: React.HTMLAttributes<HTMLDivElement>;
  fixedWidth?: boolean;
  dotClusterRef?: (el: HTMLDivElement | null) => void;
  /**
   * Overlay usage only: hands back the content button's own DOM node so the
   * overlay can register it as an always-clickable hot zone with the native
   * hit test, same reason as `dotClusterRef`.
   */
  contentRef?: (el: HTMLButtonElement | null) => void;
  /**
   * Overlay only: mirrors the element so the info card sits outboard on the
   * table's right-hand seats, which is what the reference does. The preview
   * grid in HUD Profiles leaves it off.
   */
  mirrored?: boolean;
}

export function JivaroHudCard({
  player,
  profile,
  onOpenDetail,
  dragHandleProps,
  fixedWidth,
  dotClusterRef,
  contentRef,
  mirrored,
}: JivaroHudCardProps) {
  const [pageIndex, setPageIndex] = useState(0);
  const pages = profile.statPages;
  const activePage = pages[pageIndex] ?? pages[0];
  const statKeys = (activePage?.statKeys ?? []).filter((key) => key in STAT_BANDS);

  // The same gate the other models respect (default 25). Below it this model
  // shows dashes rather than the confidence-shaded values the other three use
  // — that is the accepted design's own below-threshold state.
  const gated = player.hands < profile.minHands;

  const classification = player.classification;
  // `resolve_for_player` already decides what may be shown. A manual
  // override always arrives with `isOverride`; the automatic archetype only
  // arrives at all in a build with the `auto-classification` feature, and
  // otherwise comes back as the neutral `unknown` result. So this renders
  // whatever the backend allowed, and nothing else.
  const tint =
    classification && (classification.isOverride || classification.classification !== "unknown")
      ? classification.color
      : null;

  const initials = player.name.slice(0, 2).toUpperCase();
  const ticks = ringTicks(statKeys, player.stats, gated);

  return (
    <div
      className={[
        styles.element,
        mirrored ? styles.mirrored : "",
        dragHandleProps ? styles.draggable : "",
        fixedWidth ? styles.fixedWidth : "",
      ]
        .filter(Boolean)
        .join(" ")}
      style={tint ? ({ ["--tint" as string]: tint }) : undefined}
      {...dragHandleProps}
    >
      <button
        type="button"
        ref={contentRef}
        className={styles.card}
        // Same guard as PlayerHudCard's content button: without it, the card
        // root's drag-handle pointerdown (dragHandleProps, overlay usage
        // only) captures this press first and the click never fires.
        onPointerDown={(e) => e.stopPropagation()}
        onClick={() => onOpenDetail?.(player)}
        aria-label={`Open detailed stats for ${player.name}`}
      >
        <div className={styles.header}>
          <span className={`${styles.name} ${tint ? styles.nameTinted : ""}`} title={player.name}>
            {player.name}
          </span>
          {pages.length > 1 && (
            // The four-dot pager is carried over unchanged from the shipped
            // models on purpose:  replaces it with click-anywhere-to-cycle
            // as a separate task, and until that lands this model has to page
            // the same way the other three do. It sits on the header row
            // rather than in the ring's bottom gap, which the hand count owns
            // in this design.
            <div
              ref={dotClusterRef}
              className={styles.pager}
              onPointerDown={(e) => e.stopPropagation()}
            >
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

        {player.snapshot ? (
          <div className={styles.stackRow}>
            <b>
              {formatChips(
                player.snapshot.stack,
                player.snapshot.currency,
                player.snapshot.format,
              )}
            </b>
            <i>{formatBb(player.snapshot.stackBb)}</i>
            <u>BB</u>
          </div>
        ) : (
          <div className={styles.stackRow}>
            <span className={styles.noSnapshot}>No recent hand</span>
          </div>
        )}

        <div className={styles.stats} key={activePage?.id}>
          {statKeys.map((key) => (
            <div key={key} className={styles.cell}>
              <span
                className={`${styles.value} tabular`}
                style={{ color: gated ? undefined : bandColor(key, player.stats[key]) }}
              >
                {gated ? "–" : formatBandValue(key, player.stats)}
              </span>
              <em className={styles.label}>{STAT_LABELS[key]}</em>
            </div>
          ))}
        </div>
      </button>

      <div className={styles.ring}>
        <svg viewBox="0 0 100 100" aria-hidden="true">
          <circle cx="50" cy="50" r="47.2" fill="none" stroke="#121417" strokeWidth="5.6" />
          <circle cx="50" cy="50" r="49.7" fill="none" stroke="#31363d" strokeWidth="0.8" />
          <circle cx="50" cy="50" r="44.3" fill="#0c0e12" />
          {ticks.map((tick, i) => (
            <rect
              key={i}
              x="47.85"
              y="6"
              width="4.3"
              height="8.2"
              rx="1.3"
              fill={tick.color}
              transform={`rotate(${tick.angle.toFixed(2)} 50 50)`}
            />
          ))}
          <circle cx="50" cy="50" r="35.4" fill="#131619" />
          <circle cx="50" cy="50" r="35.4" fill="none" stroke="#05070a" strokeWidth="1.4" />
        </svg>
        <span className={styles.initials}>{initials}</span>
        <span className={styles.hands}>
          {gated ? `${player.hands}/${profile.minHands}` : `${player.hands}h`}
        </span>
      </div>
    </div>
  );
}
