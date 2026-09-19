import { useState } from "react";
import type { Player } from "../../data/types";
import { clearPlayerColorOverride, setPlayerColorOverride, setPlayerNote } from "../../data/api";
import styles from "./PlayerProfileDrawer.module.css";
import { CloseIcon } from "../icons";

interface PlayerProfileDrawerProps {
  player: Player;
  onClose: () => void;
  onPlayerUpdated?: (updated: Player) => void;
}

const OVERRIDE_COLORS: { color: string; label: string }[] = [
  { color: "#6f8fff", label: "Tight Aggressive" },
  { color: "#e0524f", label: "Maniac" },
  { color: "#e0954f", label: "Loose Aggressive" },
  { color: "#d9b44a", label: "Loose Passive" },
  { color: "#5b7a99", label: "Nitty / Rock" },
  { color: "#57b88b", label: "Recreational" },
  { color: "#a780e8", label: "Custom" },
];

const TIER_LABEL: Record<string, string> = {
  high: "High",
  medium: "Medium",
  low: "Low",
  insufficientData: "Insufficient data",
};

export function PlayerProfileDrawer({ player, onClose, onPlayerUpdated }: PlayerProfileDrawerProps) {
  const descriptions = player.descriptions ?? [];
  const tendencies = descriptions.filter((d) => d.category === "tendency");
  const exploits = descriptions.filter((d) => d.category === "exploit");
  const [saving, setSaving] = useState(false);
  const classification = player.classification;
  // Same gate as PlayersView and the HUD cards: an archetype may be
  // shown only when this build produced one (a manual override, or an
  // automatic match). `available: false` is the build having no classifier at
  // all; a genuine `unknown` is the classifier having nothing to say yet. The
  // three cases read differently below, and only the first paints a colour.
  const hasArchetype =
    !!classification &&
    classification.available &&
    (classification.isOverride || classification.classification !== "unknown");

  // Draft of the note being typed. Committed on blur — same auto-save shape as
  // the `minHands` input in HudProfilesView, no explicit Save button.
  const [noteDraft, setNoteDraft] = useState(player.note ?? "");
  const [noteSaving, setNoteSaving] = useState(false);
  // The drawer stays mounted when the selected player changes, so an
  // in-progress draft would otherwise leak onto the next player. Resetting
  // during render (rather than in an effect) avoids a frame showing the wrong
  // player's note.
  const [noteOwnerId, setNoteOwnerId] = useState(player.id);
  if (noteOwnerId !== player.id) {
    setNoteOwnerId(player.id);
    setNoteDraft(player.note ?? "");
  }

  async function handleNoteBlur() {
    const next = noteDraft.trim();
    if (next === (player.note ?? "")) return;
    setNoteSaving(true);
    try {
      const updated = await setPlayerNote(player.id, next);
      setNoteDraft(updated.note ?? "");
      onPlayerUpdated?.(updated);
    } catch {
      // Non-fatal — the draft stays in the box so nothing typed is lost.
    } finally {
      setNoteSaving(false);
    }
  }

  async function applyOverride(color: string, label: string) {
    setSaving(true);
    try {
      const updated = await setPlayerColorOverride(player.id, color, label);
      onPlayerUpdated?.(updated);
    } catch {
      // Non-fatal — the classification simply stays as-is if this fails.
    } finally {
      setSaving(false);
    }
  }

  async function resetToAutomatic() {
    setSaving(true);
    try {
      const updated = await clearPlayerColorOverride(player.id);
      onPlayerUpdated?.(updated);
    } catch {
      // Non-fatal.
    } finally {
      setSaving(false);
    }
  }

  return (
    <>
      <div className={styles.backdrop} onClick={onClose} />
      <aside className={styles.drawer}>
        <div className={styles.header}>
          <div className={styles.identity}>
            <div
              className={styles.avatar}
              style={
                hasArchetype && classification
                  ? { boxShadow: `0 0 0 2px ${classification.color}` }
                  : undefined
              }
            >
              {player.name.slice(0, 2).toUpperCase()}
            </div>
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

        <div className={styles.section}>
          <div className={styles.sectionTitle}>Profile</div>
          {!classification ? (
            <div className={styles.descriptionEmpty}>No classification data.</div>
          ) : !classification.available ? (
            <div className={styles.descriptionEmpty}>Classification unavailable in this build.</div>
          ) : !hasArchetype ? (
            /* Available, but nothing matched: below a rule's own hand minimum, or
               no rule covers these stats. Said in words instead of a grey
               "Unknown" badge, so it can never be read as this build's
               "unavailable" state — the two are different facts. */
            <div className={styles.descriptionEmpty}>
              No archetype yet — {player.hands.toLocaleString()} tracked hand
              {player.hands === 1 ? "" : "s"} is under every rule&apos;s own hand minimum, or no
              rule matches these stats. Stats, notes and the colour you set yourself are
              unaffected.
            </div>
          ) : (
            <div className={styles.classificationBar}>
              <span
                className={styles.classificationDot}
                style={{ backgroundColor: classification.color }}
              />
              <span className={styles.classificationText}>{classification.label}</span>
              {classification.isOverride && <span className={styles.overrideTag}>Manual</span>}
            </div>
          )}
        </div>

        <div className={styles.section}>
          <div className={styles.sectionTitle}>Tendencies</div>
          {tendencies.length > 0 ? (
            <ul className={styles.descriptionList}>
              {tendencies.map((r) => (
                <li key={r.ruleId} className={styles.descriptionItem}>
                  <span className={styles.descriptionText}>{r.conclusion}</span>
                  <span
                    className={`${styles.confidenceBadge} ${styles[`tier-${r.confidenceTier}`] ?? ""}`}
                  >
                    {TIER_LABEL[r.confidenceTier] ?? r.confidenceTier}
                  </span>
                </li>
              ))}
            </ul>
          ) : (
            <div className={styles.descriptionEmpty}>No tendencies identified yet.</div>
          )}
        </div>

        <div className={styles.section}>
          <div className={styles.sectionTitle}>Exploits</div>
          {exploits.length > 0 ? (
            <ul className={styles.descriptionList}>
              {exploits.map((r) => (
                <li key={r.ruleId} className={styles.descriptionItem}>
                  <span className={styles.descriptionText}>{r.conclusion}</span>
                  <span
                    className={`${styles.confidenceBadge} ${styles[`tier-${r.confidenceTier}`] ?? ""}`}
                  >
                    {TIER_LABEL[r.confidenceTier] ?? r.confidenceTier}
                  </span>
                </li>
              ))}
            </ul>
          ) : (
            <div className={styles.descriptionEmpty}>No exploits identified yet.</div>
          )}
        </div>

        <div className={styles.section}>
          <div className={styles.sectionTitle}>Notes</div>
          <textarea
            className={styles.noteInput}
            value={noteDraft}
            onChange={(e) => setNoteDraft(e.target.value)}
            onBlur={handleNoteBlur}
            disabled={noteSaving}
            rows={4}
            placeholder="Tells, tendencies, anything worth remembering about this player…"
            aria-label={`Notes about ${player.name}`}
          />
          <div className={styles.noteHint}>Saves automatically when you click away.</div>
        </div>

        <div className={styles.section}>
          <div className={styles.sectionTitle}>Player Color</div>
          <div className={styles.swatchRow}>
            {OVERRIDE_COLORS.map((c) => (
              <button
                key={c.color}
                type="button"
                className={styles.swatch}
                style={{ backgroundColor: c.color }}
                title={c.label}
                disabled={saving}
                onClick={() => applyOverride(c.color, c.label)}
              />
            ))}
          </div>
          {classification?.isOverride && (
            <button
              type="button"
              className={styles.resetButton}
              disabled={saving}
              onClick={resetToAutomatic}
            >
              Use Automatic
            </button>
          )}
        </div>
      </aside>
    </>
  );
}
