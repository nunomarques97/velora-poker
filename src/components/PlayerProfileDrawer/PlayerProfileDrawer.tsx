import { useState } from "react";
import type {
  EnginePayload,
  EngineRead,
  HeadToHeadKey,
  Player,
  RuleResult,
  ShowdownLineStep,
  ShowdownRecord,
  SizeBucket,
} from "../../data/types";
import { confidenceText } from "../../hud/HoverReadCard";
import { clearPlayerColorOverride, setPlayerColorOverride, setPlayerNote } from "../../data/api";
import styles from "./PlayerProfileDrawer.module.css";
import { CloseIcon } from "../icons";

interface PlayerProfileDrawerProps {
  player: Player;
  onClose: () => void;
  onPlayerUpdated?: (updated: Player) => void;
  /**
   * `drawer` (default, main window): full-height sheet over a dimming
   * backdrop. `panel` (overlay): a compact floating panel with no backdrop,
   * so the rest of the table stays visible and click-through; it closes only
   * through its own close button (or the overlay's Esc / chip toggle).
   */
  variant?: "drawer" | "panel";
  /** `panel` only: which window edge the panel docks to. */
  side?: "left" | "right";
  /** `panel` only: the panel's node, registered by the overlay as a hot zone. */
  panelRef?: (el: HTMLElement | null) => void;
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

const CATEGORY_LABEL: Record<string, string> = {
  tendency: "Tendency",
  exploit: "Exploit",
};

const TIER_LABEL: Record<string, string> = {
  high: "High",
  medium: "Medium",
  low: "Low",
  insufficientData: "Insufficient data",
};

/**
 * One rule result: what the opponent does, then what to do about it, each
 * under its own label. They used to be a single sentence with the advice
 * tacked on after a dash — except seven rules used a full stop instead, so
 * there was no reliable way to tell where the read ended and the advice
 * began. The colour carries the same split: observations in the normal text
 * colour, advice in the accent.
 */
function ReadItem({ result, index, engineRead }: { result: RuleResult; index: number; engineRead?: EngineRead }) {
  return (
    <li className={styles.readItem}>
      <span className={styles.readIndex}>{index}</span>
      <div className={styles.readBody}>
        <div className={styles.readLabel}>
          {CATEGORY_LABEL[result.category] ?? result.category}
          <span className={styles.readLabelSep}>·</span>
          <span className={`${styles[`tier-${result.confidenceTier}`] ?? ""} ${styles.readTier}`}>
            {engineRead ? confidenceText(engineRead) : TIER_LABEL[result.confidenceTier] ?? result.confidenceTier}
          </span>
          {engineRead?.tag && <span className={styles.readTag}>{engineRead.tag}</span>}
        </div>
        <div className={styles.readObservation}>{result.observation}</div>
        <div className={styles.readLabel}>Advice</div>
        <div className={styles.readAdvice}>{result.advice}</div>
      </div>
    </li>
  );
}

const HEAD_TO_HEAD_LABEL: Record<HeadToHeadKey, string> = {
  three_bet_vs_hero_open: "3-bets your opens",
  fold_to_hero_3bet: "Folds to your 3-bets",
  fold_to_hero_cbet: "Folds to your c-bets",
  steal_vs_hero: "Steals into your blinds",
  defend_vs_hero_steal: "Defends against your steals",
};

const SIZE_LABEL: Record<SizeBucket, string> = {
  small: "Small (under 40% pot)",
  medium: "Medium (40-75% pot)",
  large: "Large (75-110% pot)",
  overbet: "Overbet (110% pot or more)",
  allin: "All-in",
};

const STREET_INITIAL: Record<ShowdownLineStep["street"], string> = {
  preflop: "P",
  flop: "F",
  turn: "T",
  river: "R",
};

/** A percentage with its sample, never a fabricated 0: "36% (4/11)" or "– (0/0)". */
function pctWithSample(pct: number | null, hits: number, n: number): string {
  return `${pct === null ? "–" : `${Math.round(pct)}%`} (${hits}/${n})`;
}

/** One step of a shown-down line: "F bet 33% pot", "R bet 1.4× pot", "T check". */
function lineStep(step: ShowdownLineStep): string {
  const size =
    step.isAllIn || step.sizeBucket === "allin"
      ? " all-in"
      : step.potFraction !== null
        ? step.potFraction >= 1
          ? ` ${step.potFraction.toFixed(1)}× pot`
          : ` ${Math.round(step.potFraction * 100)}% pot`
        : "";
  return `${STREET_INITIAL[step.street]} ${step.action}${size}`;
}

function formatDate(iso: string | null): string {
  if (!iso) return "";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString();
}

const RESULT_LABEL: Record<ShowdownRecord["result"], string> = { won: "Won", lost: "Lost", split: "Split" };

/**
 * What the opponent engine knows beyond the ranked reads
 * (`strategic-analysis` build only): head-to-head against the hero, recent
 * form, showdowns with the line taken, sizing tells, and auto-notes. Every
 * number carries its sample; an empty block says why it is empty.
 */
function EngineSections({ engine }: { engine: EnginePayload }) {
  const h2h = engine.headToHead;
  const form = engine.recentForm;
  return (
    <>
      <div className={styles.section}>
        <div className={styles.sectionTitle}>Against you</div>
        {!h2h ? (
          <div className={styles.descriptionEmpty}>No hands against you yet.</div>
        ) : h2h.stats.length === 0 ? (
          <div className={styles.descriptionEmpty}>
            {h2h.hands.toLocaleString()} hand{h2h.hands === 1 ? "" : "s"} together; no spot against you has 8
            chances yet.
          </div>
        ) : (
          <>
            <div className={styles.engineMeta}>{h2h.hands.toLocaleString()} hands together</div>
            <dl className={styles.factList}>
              {h2h.stats.map((stat) => (
                <div key={stat.key} className={styles.factRow}>
                  <dt>{HEAD_TO_HEAD_LABEL[stat.key] ?? stat.key}</dt>
                  <dd>
                    {pctWithSample(stat.rawPct, stat.hits, stat.opportunities)}
                    <span className={styles.factAside}> · adjusted {Math.round(stat.shrunkPct)}%</span>
                  </dd>
                </div>
              ))}
            </dl>
          </>
        )}
      </div>

      <div className={styles.section}>
        <div className={styles.sectionTitle}>Recent form</div>
        {!form ? (
          <div className={styles.descriptionEmpty}>Not enough recent hands to compare with the usual game.</div>
        ) : (
          <>
            {form.flag && (
              <div className={`${styles.formFlag} ${form.flag === "tilt" ? styles.formTilt : ""}`}>
                {form.flag === "tilt" ? "Tilt" : form.flag === "looser" ? "Looser than usual" : "Tighter than usual"}
                {form.afterBigLoss && " · after a big loss"}
              </div>
            )}
            <dl className={styles.factList}>
              <div className={styles.factRow}>
                <dt>VPIP, last {form.window} hands</dt>
                <dd>
                  {Math.round(form.windowVpipPct)}% ({form.windowHits}/{form.window})
                </dd>
              </div>
              <div className={styles.factRow}>
                <dt>VPIP before that</dt>
                <dd>
                  {Math.round(form.baselineVpipPct)}% ({form.baselineOpportunities} hands)
                </dd>
              </div>
            </dl>
          </>
        )}
      </div>

      <div className={styles.section}>
        <div className={styles.sectionTitle}>Showdowns</div>
        {engine.showdowns.length === 0 ? (
          <div className={styles.descriptionEmpty}>No shown hands yet.</div>
        ) : (
          <ul className={styles.showdownList}>
            {engine.showdowns.map((sd) => (
              <li key={sd.handId} className={styles.showdownItem}>
                <div className={styles.showdownHead}>
                  <span className={styles.showdownCards}>{sd.cards}</span>
                  <span className={styles.showdownCategory}>{sd.category.replace(/_/g, " ")}</span>
                  <span className={styles.showdownResult}>{RESULT_LABEL[sd.result] ?? sd.result}</span>
                </div>
                {sd.board && <div className={styles.showdownBoard}>Board {sd.board}</div>}
                {sd.line.length > 0 && (
                  <div className={styles.showdownLine}>{sd.line.map(lineStep).join(" · ")}</div>
                )}
                {sd.lastAggression && sd.lastAggression.class !== "neither" && (
                  <div className={styles.showdownClass}>
                    {sd.lastAggression.class === "value" ? "Value" : "Bluff"}: {sd.lastAggression.street}{" "}
                    {sd.lastAggression.sizeBucket}
                  </div>
                )}
                <div className={styles.engineMeta}>
                  Hand #{sd.handId}
                  {sd.playedAt ? ` · ${formatDate(sd.playedAt)}` : ""}
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className={styles.section}>
        <div className={styles.sectionTitle}>Sizing tells</div>
        {engine.sizingTells.length === 0 ? (
          <div className={styles.descriptionEmpty}>No bet size shown down often enough yet.</div>
        ) : (
          <dl className={styles.factList}>
            {engine.sizingTells.map((tell) => (
              <div key={tell.bucket} className={styles.factRow}>
                <dt>{SIZE_LABEL[tell.bucket] ?? tell.bucket}</dt>
                <dd>
                  {tell.value} value · {tell.bluff} bluff
                  {tell.neither > 0 ? ` · ${tell.neither} other` : ""} (n={tell.n})
                </dd>
              </div>
            ))}
          </dl>
        )}
      </div>

      <div className={styles.section}>
        <div className={styles.sectionTitle}>Auto-notes</div>
        {engine.autoNotes.length === 0 ? (
          <div className={styles.descriptionEmpty}>No notable hand recorded yet.</div>
        ) : (
          <ul className={styles.autoNoteList}>
            {engine.autoNotes.map((note) => (
              <li key={note.id} className={styles.autoNoteItem}>
                <span className={styles.autoTag}>Auto</span>
                <div className={styles.autoNoteBody}>
                  <div className={styles.autoNoteText}>{note.text}</div>
                  <div className={styles.engineMeta}>
                    Hand #{note.handId}
                    {note.createdAt ? ` · ${formatDate(note.createdAt)}` : ""}
                  </div>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </>
  );
}

export function PlayerProfileDrawer({
  player,
  onClose,
  onPlayerUpdated,
  variant = "drawer",
  side = "right",
  panelRef,
}: PlayerProfileDrawerProps) {
  const panel = variant === "panel";
  const descriptions = player.descriptions ?? [];
  const tendencies = descriptions.filter((d) => d.category === "tendency");
  const exploits = descriptions.filter((d) => d.category === "exploit");
  const engine = player.engine ?? null;
  const engineRead = (ruleId: string) => engine?.reads.find((r) => r.ruleId === ruleId);
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
      {!panel && <div className={styles.backdrop} onClick={onClose} />}
      <aside
        ref={panel ? panelRef : undefined}
        className={
          panel ? `${styles.panel} ${side === "left" ? styles.panelLeft : styles.panelRight}` : styles.drawer
        }
        aria-label={`${player.name} details`}
      >
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
          <button type="button" className={styles.closeButton} onClick={onClose} aria-label="Close details">
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
              {tendencies.map((r, i) => (
                <ReadItem key={r.ruleId} result={r} index={i + 1} engineRead={engineRead(r.ruleId)} />
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
              {exploits.map((r, i) => (
                <ReadItem key={r.ruleId} result={r} index={i + 1} engineRead={engineRead(r.ruleId)} />
              ))}
            </ul>
          ) : (
            <div className={styles.descriptionEmpty}>No exploits identified yet.</div>
          )}
        </div>

        {engine && <EngineSections engine={engine} />}

        <div className={styles.section}>
          <div className={styles.sectionTitle}>{engine ? "Your notes" : "Notes"}</div>
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
