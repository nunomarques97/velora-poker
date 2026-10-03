import type { ChipTag, ClassificationResult, HudProfile, Player, PlayerStats } from "../data/types";
import { NO_OPPORTUNITY, STAT_LABELS } from "./statFormat";
import styles from "./PlayerHudCard.module.css";

/**
 * How many stats the compact chip shows: the first three of the profile's
 * first stat page (VPIP/PFR/3-bet by default). More would not fit one line on
 * a minimum-size table.
 */
const COMPACT_STAT_COUNT = 3;

/** Left edge of a chip with no archetype to show. */
const NEUTRAL_EDGE = "#4a5266";

/**
 * The archetype the chip may show, or `null`. An automatic archetype needs
 * the profile's `min_hands` sample, whatever the classifier itself decided,
 * so the HUD never labels a player on too few hands. A manual override is
 * the user's own colour and always shows, the same rule as the Players view
 * and the detail panel.
 */
function shownArchetype(
  classification: ClassificationResult | undefined,
  hands: number,
  minHands: number,
): ClassificationResult | null {
  if (!classification || !classification.available) return null;
  if (classification.isOverride) return classification;
  if (classification.classification === "unknown") return null;
  return hands >= minHands ? classification : null;
}

/** Whole numbers on the chip: "24", not "24.3%". AF keeps one decimal. */
function chipValue(key: keyof PlayerStats, stats: PlayerStats): string {
  const value = stats[key];
  if (value === null || value === undefined) return NO_OPPORTUNITY;
  return key === "aggressionFactor" ? value.toFixed(1) : String(Math.round(value));
}

/**
 * The tag in words, for the chip's accessible name: what kind of tag it is
 * and, for a read's tag, the read itself. The tag letters alone ("F3B")
 * mean nothing to a screen reader.
 */
function tagDescription(tag: ChipTag, player: Player): string {
  if (tag.source === "tilt") return `tag ${tag.text}, playing far looser than usual lately`;
  if (tag.source === "stack") return `tag ${tag.text}, effective stack`;
  const read = player.engine?.reads.find((r) => r.ruleId === tag.ruleId);
  return read ? `tag ${tag.text}, ${read.observation}` : `tag ${tag.text}`;
}

/** Pointer and focus handlers the overlay uses to open and close the hover read card. */
export type HoverHandlers = Pick<
  React.HTMLAttributes<HTMLButtonElement>,
  "onPointerEnter" | "onPointerLeave" | "onFocus" | "onBlur"
>;

interface PlayerHudCardProps {
  player: Player;
  profile: HudProfile;
  onOpenDetail?: (player: Player) => void;
  /**
   * Overlay only: the chip is also its own drag handle. The overlay defers
   * pointer capture until the pointer really moves, so a plain click still
   * reaches `onClick`.
   */
  dragHandleProps?: Pick<React.HTMLAttributes<HTMLButtonElement>, "onPointerDown">;
  /** Overlay only: the chip's node, registered as a clickable hot zone. */
  contentRef?: (el: HTMLButtonElement | null) => void;
  /** HUD Profiles preview only: one width per model, so the preview lines up in columns. */
  fixedWidth?: boolean;
  /** This player's detail panel is open. */
  expanded?: boolean;
  /** Overlay only: the chip is being dragged. */
  dragging?: boolean;
  /** Overlay only (`strategic-analysis` build): opens and closes the hover read card. */
  hoverHandlers?: HoverHandlers;
  /** Overlay only: the id of this chip's open hover read card. */
  describedBy?: string;
}

/**
 * One player's HUD chip. Two designs, picked by the profile:
 * - `compact` (the default, and what any unknown model renders as): one line
 *   with the first stat page's values and the hand count;
 * - `badge`: initials and the hand count, stats one click away.
 * Either way the chip is one button: click opens the detail panel, drag
 * moves it. Its size comes from the overlay (`--chip-font`, `--chip-w`,
 * `--chip-h`), which scales it with the table.
 *
 * `strategic-analysis` build only: when the opponent engine gave the player
 * a tag (`engine.tag`), it leads the chip in its own segment. With no
 * engine (the default build) or no tag the chip is exactly the one above.
 */
export function PlayerHudCard({
  player,
  profile,
  onOpenDetail,
  dragHandleProps,
  contentRef,
  fixedWidth,
  expanded,
  dragging,
  hoverHandlers,
  describedBy,
}: PlayerHudCardProps) {
  const model = profile.visualModel === "badge" ? "badge" : "compact";
  const archetype = shownArchetype(player.classification, player.hands, profile.minHands);
  const smallSample = player.hands < profile.minHands;
  const handsText = `${player.hands.toLocaleString()} hand${player.hands === 1 ? "" : "s"}`;

  const keys = (profile.statPages[0]?.statKeys ?? []).slice(0, COMPACT_STAT_COUNT);
  const values = keys.map((key) => chipValue(key, player.stats));
  const statsText = keys.map((key, i) => `${STAT_LABELS[key]} ${values[i]}`).join(", ");
  const tag = player.engine?.tag ?? null;

  // The colour is never the only signal: the archetype's name is in the
  // chip's accessible name and tooltip, and the detail panel spells it out.
  const description = [
    player.name,
    handsText,
    model === "compact" && statsText ? statsText : null,
    archetype ? archetype.label : null,
    smallSample ? `under ${profile.minHands} hands, small sample` : null,
    tag ? tagDescription(tag, player) : null,
  ]
    .filter(Boolean)
    .join(", ");

  // Badge only: the outline turns accent-coloured when opening this player
  // shows something written (a read or an archetype), so a glance says which
  // players are worth opening.
  const hasWrittenInfo = (player.descriptions?.length ?? 0) > 0 || archetype !== null;

  const className = [
    styles.chip,
    model === "badge" ? styles.badge : styles.compact,
    model === "badge" && hasWrittenInfo ? styles.hasInfo : "",
    smallSample ? styles.smallSample : "",
    dragHandleProps ? styles.draggable : "",
    dragging ? styles.dragging : "",
    expanded ? styles.expanded : "",
    fixedWidth ? styles.fixedWidth : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <button
      type="button"
      ref={contentRef}
      className={className}
      style={{ ["--edge" as string]: archetype?.color ?? NEUTRAL_EDGE }}
      onClick={() => onOpenDetail?.(player)}
      aria-label={`${description}. Open details`}
      aria-expanded={expanded === undefined ? undefined : expanded}
      aria-describedby={describedBy}
      // The hover read card replaces the native tooltip, which would
      // otherwise open on top of it.
      title={hoverHandlers && player.engine ? undefined : description}
      data-tag={tag?.text}
      {...hoverHandlers}
      {...dragHandleProps}
    >
      {tag && (
        <span className={`${styles.tag} ${styles[`tag-${tag.source}`] ?? ""}`} aria-hidden="true">
          {tag.text}
        </span>
      )}
      {model === "badge" ? (
        <span className={styles.initials}>{player.name.slice(0, 2).toUpperCase()}</span>
      ) : (
        <span className={styles.stats}>
          {values.map((value, i) => (
            <span key={keys[i]}>
              {i > 0 && <span className={styles.sep}>/</span>}
              {value}
            </span>
          ))}
        </span>
      )}
      <span className={styles.hands}>{player.hands}</span>
    </button>
  );
}
