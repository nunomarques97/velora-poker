import styles from "./IngestionStatusBar.module.css";

interface IngestionStatusBarProps {
  /** Only ever rendered by the caller when this is greater than zero. */
  handsRejected: number;
  onViewDetails: () => void;
}

/**
 * Persistent warning at the top of the main window — never a toast
 * that disappears on its own. Being honest about errors (hands silently
 * dropped is the failure this guards against) means a nonzero
 * `handsRejected` stays visible, with a shortcut to
 * the Settings section that explains why, until the count is genuinely zero
 * again on the next poll.
 */
export function IngestionStatusBar({ handsRejected, onViewDetails }: IngestionStatusBarProps) {
  const label =
    handsRejected === 1
      ? "1 hand could not be read."
      : `${handsRejected.toLocaleString()} hands could not be read.`;

  return (
    <div className={styles.bar} role="status">
      <span className={styles.message}>
        <span className={styles.count}>{label}</span> Some data may be missing from your stats.
      </span>
      <button type="button" className={styles.action} onClick={onViewDetails}>
        View in Settings
      </button>
    </div>
  );
}
