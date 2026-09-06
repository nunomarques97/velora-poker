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
  { color: "#57b88b", label: "Recreational" },
  { color: "#a780e8", label: "Custom" },
];

export function PlayerProfileDrawer({ player, onClose, onPlayerUpdated }: PlayerProfileDrawerProps) {
  const { stats } = player;
  const [saving, setSaving] = useState(false);
  const classification = player.classification;

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
              style={classification ? { boxShadow: `0 0 0 2px ${classification.color}` } : undefined}
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

        {classification && (
          <div className={styles.classificationBar}>
            <span
              className={styles.classificationDot}
              style={{ backgroundColor: classification.color }}
            />
            <span className={styles.classificationText}>{classification.label}</span>
            {classification.isOverride && <span className={styles.overrideTag}>Manual</span>}
          </div>
        )}

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

function StatRow({ label, value }: { label: string; value: string }) {
  return (
    <div className={styles.statRow}>
      <span className={styles.statRowLabel}>{label}</span>
      <span className={`${styles.statRowValue} tabular`}>{value}</span>
    </div>
  );
}
