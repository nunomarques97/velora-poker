import { useState } from "react";
import type { Player, HudProfile } from "../data/types";
import {
  STAT_COLUMN_COLORS,
  STAT_LABELS,
  bbDepthTier,
  formatBb,
  formatChips,
  formatStatValue,
} from "./statFormat";
import { JivaroHudCard } from "./JivaroHudCard";
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

/**
 * On-table badge only (user request, live, 2026-09-13): the badge's ring
 * color reads stack depth instead of classification, since there's no
 * stack-row text left on a badge to carry that read. Same hex values as
 * `.bbShort`/`.bbMedium`/`.bbDeep` in PlayerHudCard.module.css, but NOT the
 * same boundary as `bbDepthTier` (statFormat.ts, `< 20`/`< 35`) — the user
 * asked for this specific badge to treat exactly 20bb as red ("lower or
 * equal to 20"), one bb different from the existing stack-row tiering. Not a
 * fix to `bbDepthTier` itself; that stat-row convention is unchanged.
 */
function badgeStackColor(stackBb: number | undefined): string {
  if (stackBb == null) return "#6b7480";
  if (stackBb <= 20) return "#ff5c5c";
  if (stackBb < 35) return "#4fd1ff";
  return "#4ade80";
}

/**
 * user toggle (live, 2026-09-13): the badge above was built for MTT
 * multi-tabling; single-table sessions want the normal full Velora card
 * back. Not a persisted setting — the user swaps this by session type and
 * asks for a rebuild each time (same pattern as SHOW_REPOSITION_* in
 * OverlayApp.tsx), so a flip here plus one rebuild is all switching back to
 * badge mode for the next MTT session takes.
 */
const SHOW_COMPACT_BADGE = false;

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
  /**
   * HUD Profiles preview only: pins the card to one width per visual model so
   * the preview's wrapping row falls into even columns instead of being sized
   * by each player's name and chip count. The overlay leaves this off — cards
   * there are positioned freely and stay content-sized.
   */
  fixedWidth?: boolean;
  /**
   * Overlay usage only: hands back the pagination-dot cluster's DOM node so
   * the overlay can register it as an always-clickable hot zone with the
   * native hit test. Without it the dots would be dead in the overlay's
   * normal, table-interactive mode, since every other point on the window
   * falls through to the table underneath.
   */
  dotClusterRef?: (el: HTMLDivElement | null) => void;
  /**
   * Overlay usage only: hands back the content button's own DOM node so the
   * overlay can register it as an always-clickable hot zone with the native
   * hit test, same reason as `dotClusterRef` — without it, clicking a
   * card to open its detail drawer falls through to the table underneath.
   */
  contentRef?: (el: HTMLButtonElement | null) => void;
  /**
   * Jivaro model only: mirrors the element so its info card sits outboard on
   * the table's right-hand seats. The three original models ignore it — their
   * layout is the same on both sides.
   */
  mirrored?: boolean;
}

export function PlayerHudCard({
  player,
  profile,
  onOpenDetail,
  dragHandleProps,
  fixedWidth,
  dotClusterRef,
  contentRef,
  mirrored,
}: PlayerHudCardProps) {
  const [pageIndex, setPageIndex] = useState(0);

  // The Jivaro model is a different shape, not a restyle of the
  // three below: a segmented gauge with an info card behind it. It renders
  // from its own component so that this one and its stylesheet stay exactly
  // as the three shipped models left them.
  if (profile.visualModel === "jivaro") {
    return (
      <JivaroHudCard
        player={player}
        profile={profile}
        onOpenDetail={onOpenDetail}
        dragHandleProps={dragHandleProps}
        fixedWidth={fixedWidth}
        dotClusterRef={dotClusterRef}
        contentRef={contentRef}
        mirrored={mirrored}
      />
    );
  }

  const classification = player.classification;
  const color = classification?.color ?? "#6b7480";
  const initials = player.name.slice(0, 2).toUpperCase();

  // URGENT build (2026-09-13, multi-tabling live): the user's own "for
  // now" call — the on-table card shrinks to just this badge, full stats
  // moving behind a click into the existing PlayerProfileDrawer. Not a
  // persisted design decision (the notes), and deliberately not a new
  // prop/setting: `dragHandleProps` is already overlay-only (see its own doc
  // comment above — main-app previews, e.g. the HUD Profiles page, never
  // pass it), so reusing its presence here means that preview keeps showing
  // full cards, unaffected, exactly as configuring stat pages there needs.
  const isOnTableCard = SHOW_COMPACT_BADGE && Boolean(dragHandleProps);

  if (isOnTableCard) {
    return (
      <div
        className={`${styles.badgeCard} ${styles[profile.visualModel] ?? ""} ${styles.draggable}`}
        // user request (live, same build, color twist): two-tone —
        // outer ring reads stack depth (badgeStackColor, the same
        // red/blue/green tiers `.bbShort`/`.bbMedium`/`.bbDeep` use for the
        // full card's stack row), center circle reads classification/profile
        // type (the same `color` the full card's ring used before this
        // badge existed).
        style={{
          ["--bb-color" as string]: badgeStackColor(player.snapshot?.stackBb),
          ["--profile-color" as string]: color,
        }}
        {...dragHandleProps}
      >
        <button
          type="button"
          ref={contentRef}
          className={styles.badge}
          // Bug fix (2026-09-13): no `stopPropagation` here, unlike the full
          // card's content button below — the badge IS the whole drag
          // surface (there's no separate ring/padding area outside it to
          // grab, the way the full card has), so the wrapper's
          // `dragHandleProps.onPointerDown` must see this press to record it.
          // That's now safe to let through without swallowing this button's
          // own click: `OverlayApp.tsx` defers pointer capture until real
          // movement crosses a threshold, so a plain tap never captures and
          // this native `click` still fires normally.
          onClick={() => onOpenDetail?.(player)}
          aria-label={`Open detailed stats for ${player.name}`}
          title={player.name}
        >
          <span className={styles.badgeInner}>
            <span className={styles.badgeName}>{initials}</span>
            <span className={styles.badgeHands}>{player.hands}</span>
          </span>
        </button>
      </div>
    );
  }

  const pages = profile.statPages.length > 0 ? profile.statPages : [];
  const activePage = pages[pageIndex] ?? pages[0];
  // / `resolve_for_player` already decided what may be shown (same
  // contract JivaroHudCard's `tint` reads). A manual override always carries
  // `isOverride`; the automatic archetype only ever arrives when this build
  // has `auto-classification` compiled in — otherwise it comes back as the
  // neutral, build-level "unavailable" result, and repeating that full
  // sentence next to every player's name would be noise, not information.
  // A genuine "Unknown" (available, no rule matched, or below its own
  // min_hands) still renders, same as before.
  const label =
    classification && (classification.isOverride || classification.classification !== "unknown")
      ? classification.label
      : classification?.available
        ? classification.label
        : null;
  const statConfidence = sampleConfidence(player.hands, profile.minHands);

  return (
    <div
      className={`${styles.card} ${styles[profile.visualModel] ?? ""} ${
        dragHandleProps ? styles.draggable : ""
      } ${fixedWidth ? styles.fixedWidth : ""}`}
      style={{ ["--player-color" as string]: color }}
      {...dragHandleProps}
    >
      <div className={styles.ring}>
        <div className={styles.ringSegments} />
        <div className={styles.avatar}>{initials}</div>
        <span className={styles.handsInRing}>{player.hands}</span>
        {pages.length > 1 && (
          <div
            ref={dotClusterRef}
            className={styles.dots}
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

      <button
        type="button"
        ref={contentRef}
        className={styles.content}
        // Stops the card root's drag-handle pointerdown (dragHandleProps,
        // overlay usage only) from capturing this press and swallowing the
        // click before it fires — the same guard the pagination dots use
        // above, now needed here too now that this button is interactive.
        onPointerDown={(e) => e.stopPropagation()}
        onClick={() => onOpenDetail?.(player)}
        aria-label={`Open detailed stats for ${player.name}`}
      >
        <div className={styles.identityRow}>
          <span className={styles.name} title={player.name}>
            {player.name}
          </span>
          {label && <span className={styles.classificationLabel}>{label}</span>}
        </div>

        {player.snapshot ? (
          <div className={styles.stackRow}>
            <span className={styles.chips}>
              {formatChips(player.snapshot.stack, player.snapshot.currency, player.snapshot.format)}
            </span>
            <span className={`${styles.bbValue} ${styles[bbDepthTier(player.snapshot.stackBb)]}`}>
              {formatBb(player.snapshot.stackBb)}
            </span>
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
