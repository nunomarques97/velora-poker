import type { CSSProperties, Ref } from "react";
import type { EngineRead } from "../data/types";
import styles from "./HoverReadCard.module.css";

const TIER_LABEL: Record<string, string> = {
  high: "High",
  medium: "Medium",
  low: "Low",
  insufficientData: "Low data",
};

/** "High 81%", or the tier alone when the read carries no percentage. */
export function confidenceText(read: Pick<EngineRead, "confidencePct" | "confidenceTier">): string {
  const tier = TIER_LABEL[read.confidenceTier] ?? read.confidenceTier;
  return read.confidencePct === null ? tier : `${tier} ${Math.round(read.confidencePct)}%`;
}

interface HoverReadCardProps {
  /** Referenced by the chip's `aria-describedby` while the card is open. */
  id: string;
  playerName: string;
  /** The reads to show, best first: `engine.topReads`, or only its first when two do not fit. */
  reads: EngineRead[];
  /** Position and size from the overlay (`placeHoverCard`); hidden until placed. */
  style: CSSProperties;
  cardRef?: Ref<HTMLDivElement>;
}

/**
 * The peek next to a chip (`strategic-analysis` build only): the player's
 * name, then his top reads, each as what they do, what to do about it, and
 * how sure the engine is. The name is there because the card is not always
 * beside its chip: it never covers any seat's cards, so near the top seats
 * it may sit across the table. Purely informational: it takes no pointer input and is never a
 * hot zone, so it can never take a click from the table; the drawer, one
 * click on the chip, has every read.
 */
export function HoverReadCard({ id, playerName, reads, style, cardRef }: HoverReadCardProps) {
  return (
    <div id={id} ref={cardRef} role="tooltip" className={styles.card} style={style} data-hover-card="">
      {reads.length === 0 ? (
        <p className={styles.empty}>No read on {playerName} has enough hands yet.</p>
      ) : (
        <>
          <p className={styles.name}>{playerName}</p>
          <ol className={styles.reads}>
            {reads.map((read) => (
              <li key={read.ruleId} className={styles.read}>
                <div className={styles.observationRow}>
                  <span className={styles.observation}>{read.observation}</span>
                  <span className={`${styles.confidence} ${styles[`tier-${read.confidenceTier}`] ?? ""}`}>
                    {confidenceText(read)}
                  </span>
                </div>
                <p className={styles.advice}>{read.advice}</p>
              </li>
            ))}
          </ol>
        </>
      )}
    </div>
  );
}
