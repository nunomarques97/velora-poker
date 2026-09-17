import { useEffect, useState } from "react";
import type { AppSettings, HudProfile, ImportStatus } from "../data/types";
import type { ClassificationRule } from "../data/api";
import {
  DesktopAppRequiredError,
  getActiveHudProfile,
  getClassificationRules,
  getAppSettings,
  getDiagnosticsReport,
  getImportStatus,
  getTableDetectionStatus,
  resetOnboarding,
  setAutoCenterEnabled,
  setHandHistoryDir,
  setHudProfileMinHands,
} from "../data/api";
import styles from "./SettingsView.module.css";

const SETTINGS_ITEMS = [
  {
    label: "Account",
    description: "Manage your Velora account and subscription.",
  },
];

type StatusState =
  | { status: "loading" }
  | { status: "ready"; data: ImportStatus }
  | { status: "unavailable"; message: string }
  | { status: "error"; message: string };

function formatDate(iso: string | null): string {
  if (!iso) return "Never";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString();
}

function formatPct(value: number): string {
  return `${Number.isInteger(value) ? value : value.toFixed(1)}%`;
}

/**
 * Renders one stat's threshold pair in plain language, or null when the rule
 * places no constraint on that stat.
 */
function statClause(name: string, min: number | null, max: number | null): string | null {
  if (min !== null && max !== null) return `${name} ${formatPct(min)}–${formatPct(max)}`;
  if (min !== null) return `${name} ${formatPct(min)}+`;
  if (max !== null) return `${name} under ${formatPct(max)}`;
  return null;
}

/**
 * The rule's stat constraints as readable clauses, derived from the values the
 * backend actually returns rather than restated as copy — if a threshold moves
 * in the rule table, this text moves with it. An empty result means the rule
 * constrains nothing but hand count, i.e. it is a catch-all.
 */
function ruleClauses(rule: ClassificationRule): string[] {
  return [
    statClause("VPIP", rule.vpipMin, rule.vpipMax),
    statClause("PFR", rule.pfrMin, rule.pfrMax),
    statClause("3-Bet", rule.threeBetMin, rule.threeBetMax),
  ].filter((clause): clause is string => clause !== null);
}

function formatParserStatus(status: string): string {
  switch (status) {
    case "watching":
      return "Watching for new hands";
    case "not_configured":
      return "No folder configured";
    default:
      return status;
  }
}

export function SettingsView() {
  const [state, setState] = useState<StatusState>({ status: "loading" });
  const [pathInput, setPathInput] = useState("");
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const [appSettings, setAppSettings] = useState<AppSettings | null>(null);
  const [hudProfile, setHudProfile] = useState<HudProfile | null>(null);
  const [minHandsInput, setMinHandsInput] = useState("25");
  const [resetting, setResetting] = useState(false);
  const [tableDetected, setTableDetected] = useState(false);
  // how many real PokerStars tables are open. Each one has its own HUD,
  // so this reads as a count of running HUDs rather than 's disclaimer.
  const [tableWindowCount, setTableWindowCount] = useState(0);
  const [savingAutoCenter, setSavingAutoCenter] = useState(false);

  const [rules, setRules] = useState<ClassificationRule[] | null>(null);

  const [copyingDiagnostics, setCopyingDiagnostics] = useState(false);
  const [diagnosticsStatus, setDiagnosticsStatus] = useState<string | null>(null);
  const [diagnosticsText, setDiagnosticsText] = useState<string | null>(null);

  function load() {
    getImportStatus()
      .then((data) => {
        setState({ status: "ready", data });
        setPathInput(data.configuredDir ?? "");
      })
      .catch((err: unknown) => {
        if (err instanceof DesktopAppRequiredError) {
          setState({ status: "unavailable", message: err.message });
        } else {
          setState({ status: "error", message: String(err) });
        }
      });

    getAppSettings()
      .then(setAppSettings)
      .catch(() => undefined);

    getActiveHudProfile()
      .then((p) => {
        setHudProfile(p);
        setMinHandsInput(String(p.minHands));
      })
      .catch(() => undefined);

    getClassificationRules()
      .then(setRules)
      .catch(() => setRules([]));
  }

  useEffect(() => {
    load();
  }, []);

  useEffect(() => {
    let cancelled = false;
    function pollDetection() {
      getTableDetectionStatus()
        .then((status) => {
          if (!cancelled) {
            setTableDetected(status.detected);
            setTableWindowCount(status.tableWindowCount);
          }
        })
        .catch(() => undefined);
    }
    pollDetection();
    const interval = window.setInterval(pollDetection, 3000);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
    };
  }, []);

  async function handleAutoCenterToggle(checked: boolean) {
    setSavingAutoCenter(true);
    try {
      await setAutoCenterEnabled(checked);
      setAppSettings((prev) => (prev ? { ...prev, autoCenterEnabled: checked } : prev));
    } finally {
      setSavingAutoCenter(false);
    }
  }

  async function handleSave() {
    setSaving(true);
    setSaveError(null);
    try {
      const data = await setHandHistoryDir(pathInput.trim());
      setState({ status: "ready", data });
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  }

  async function handleMinHandsBlur() {
    if (!hudProfile) return;
    const parsed = Number.parseInt(minHandsInput, 10);
    const value = Number.isFinite(parsed) && parsed >= 0 ? parsed : hudProfile.minHands;
    setMinHandsInput(String(value));
    const updated = await setHudProfileMinHands(hudProfile.id, value);
    setHudProfile(updated);
  }

  async function handleCopyDiagnostics() {
    setCopyingDiagnostics(true);
    setDiagnosticsStatus(null);
    setDiagnosticsText(null);
    try {
      const report = await getDiagnosticsReport();
      try {
        await navigator.clipboard.writeText(report);
        setDiagnosticsStatus("Copied to clipboard — paste it wherever you're reporting the issue.");
      } catch {
        // Clipboard access can be blocked (focus/permissions); fall back to
        // showing the text so it can still be copied manually.
        setDiagnosticsText(report);
        setDiagnosticsStatus("Couldn't access the clipboard — select the text below and copy it.");
      }
    } catch (err) {
      setDiagnosticsStatus(err instanceof Error ? err.message : String(err));
    } finally {
      setCopyingDiagnostics(false);
    }
  }

  async function handleResetSetup() {
    setResetting(true);
    try {
      await resetOnboarding();
      window.location.reload();
    } finally {
      setResetting(false);
    }
  }

  return (
    <div>
      <div className="view-header">
        <h1>Settings</h1>
        <p>Your poker room, hand history folder, HUD threshold and table detection.</p>
      </div>

      <div className={styles.section}>
        <div className={styles.sectionTitle}>Poker Room</div>
        {appSettings ? (
          <div className={styles.roomRow}>
            <div>
              <div className={styles.roomName}>
                {appSettings.pokerRoom === "pokerstars" ? "PokerStars" : "Not configured"}
              </div>
              <div className={styles.roomHint}>Redo the onboarding flow to change rooms or HUD.</div>
            </div>
            <button
              type="button"
              className={styles.resetSetupButton}
              disabled={resetting}
              onClick={handleResetSetup}
            >
              {resetting ? "Resetting…" : "Redo Setup"}
            </button>
          </div>
        ) : (
          <div className={styles.stateBox}>Loading&hellip;</div>
        )}
      </div>

      <div className={styles.section}>
        <div className={styles.sectionTitle}>Hand History</div>

        {state.status === "loading" && <div className={styles.stateBox}>Loading&hellip;</div>}
        {state.status === "unavailable" && <div className={styles.stateBox}>{state.message}</div>}
        {state.status === "error" && (
          <div className={styles.stateBox}>Failed to load status: {state.message}</div>
        )}

        {(state.status === "ready" || state.status === "loading") && (
          <div className={styles.folderRow}>
            <input
              className={styles.folderInput}
              type="text"
              placeholder="C:\Users\you\AppData\Local\PokerStars\HandHistory"
              value={pathInput}
              onChange={(e) => setPathInput(e.target.value)}
            />
            <button
              type="button"
              className={styles.saveButton}
              onClick={handleSave}
              disabled={saving || pathInput.trim().length === 0}
            >
              {saving ? "Saving\u2026" : "Save"}
            </button>
          </div>
        )}
        {saveError && <div className={styles.errorText}>{saveError}</div>}

        {state.status === "ready" && (
          <div className={styles.statusGrid}>
            <div className={styles.statusCell}>
              <span className={styles.statusLabel}>Detected Folder</span>
              <span className={styles.statusValue}>
                {state.data.configuredDir ?? "Not configured"}
              </span>
            </div>
            <div className={styles.statusCell}>
              <span className={styles.statusLabel}>Hands Imported</span>
              <span className={`${styles.statusValue} tabular`}>
                {state.data.handsImported.toLocaleString()}
              </span>
            </div>
            <div className={styles.statusCell}>
              <span className={styles.statusLabel}>Last Import</span>
              <span className={styles.statusValue}>{formatDate(state.data.lastImportAt)}</span>
            </div>
            <div className={styles.statusCell}>
              <span className={styles.statusLabel}>Parser Status</span>
              <span className={styles.statusValue}>
                {formatParserStatus(state.data.parserStatus)}
              </span>
            </div>
          </div>
        )}
      </div>

      <div className={styles.section}>
        <div className={styles.sectionTitle}>HUD</div>
        {hudProfile ? (
          <>
            <div className={styles.statusGrid}>
              <div className={styles.statusCell}>
                <span className={styles.statusLabel}>Active Model</span>
                <span className={styles.statusValue}>{hudProfile.name}</span>
              </div>
              <div className={styles.statusCell}>
                <span className={styles.statusLabel}>Overlay</span>
                <span className={styles.statusValue}>
                  {appSettings?.overlayEnabled ? "On for every table" : "Off (kill switch)"}
                </span>
              </div>
            </div>
            <label className={styles.minHandsRow}>
              <span>
                Sample size for full-strength stat display
                {/* This field writes the active HUD profile's `min_hands`, and the
                    only thing that reads it is the HUD card's opacity fade. It used
                    to be labelled "minimum hands before automatic classification",
                    which is what the *rules* below do with their own minimum — two
                    unrelated numbers under one name. */}
                <div className={styles.hudHint} style={{ marginTop: 4 }}>
                  Stat values on a HUD card fade while a player has fewer hands than
                  this, and reach full strength at it. Purely visual — archetype labels
                  come from the rules below and their own hand minimums.
                </div>
              </span>
              <input
                className={styles.minHandsInput}
                type="number"
                min={0}
                value={minHandsInput}
                onChange={(e) => setMinHandsInput(e.target.value)}
                onBlur={handleMinHandsBlur}
              />
            </label>
            <p className={styles.hudHint}>
              Switch HUD models, turn HUDs on or off, and manage per-player color overrides from
              the HUD Profiles page. A HUD appears on every PokerStars table you open, by itself.
            </p>
          </>
        ) : (
          <div className={styles.stateBox}>Loading&hellip;</div>
        )}
      </div>

      <div className={styles.section}>
        <div className={styles.sectionTitle}>Player Classification</div>
        <p className={styles.ruleIntro}>
          Every tracked opponent is checked against these rules in order, top to bottom. The first
          one that matches decides the label and colour on their HUD card.
        </p>

        {rules === null && <div className={styles.stateBox}>Loading&hellip;</div>}
        {rules !== null && rules.length === 0 && (
          <div className={styles.stateBox}>Classification rules are unavailable.</div>
        )}

        {rules !== null && rules.length > 0 && (
          <>
            <div className={styles.ruleList}>
              {rules.map((rule) => {
                const clauses = ruleClauses(rule);
                return (
                  <div key={rule.id} className={styles.ruleRow}>
                    <span className={styles.ruleSwatch} style={{ background: rule.color }} />
                    <div className={styles.ruleBody}>
                      <div className={styles.ruleLabel}>{rule.label}</div>
                      {clauses.length > 0 ? (
                        <div className={styles.ruleCriteria}>
                          {clauses.length > 1
                            ? `All of: ${clauses.join(", ")}`
                            : clauses[0]}
                          , on at least {rule.minHands} tracked hands.
                        </div>
                      ) : (
                        <div className={styles.ruleDefault}>
                          No stat thresholds at all — this rule sets none, so it matches on hand
                          count alone and catches everyone the rules above did not. Anyone with at
                          least {rule.minHands} tracked hands lands here, including tight, passive
                          players who never reach the aggression floor above. It is a fallback
                          bucket, not a detection of recreational play.
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>

            <p className={styles.ruleNote}>
              Below a rule&apos;s hand minimum nothing matches at all, and the player stays Unknown
              with no archetype shown. A per-player colour override also wins outright: an
              overridden player shows that colour and its label instead of any rule&apos;s.
            </p>
            <p className={styles.ruleNote}>
              These hand minimums are not the &quot;Min hands&quot; field on the HUD Profiles page.
              That one is a display setting: it fades stat values on thin samples and changes no
              label. The minimums above are the only sample gate on classification.
            </p>
          </>
        )}
      </div>

      <div className={styles.section}>
        <div className={styles.sectionTitle}>Table Detection</div>
        <div className={styles.statusGrid}>
          <div className={styles.statusCell}>
            <span className={styles.statusLabel}>PokerStars Table Window</span>
            <span className={styles.statusValue}>
              {tableDetected ? "Detected — each table has its own HUD" : "Not detected"}
            </span>
          </div>
          <div className={styles.statusCell}>
            <span className={styles.statusLabel}>Open Tables</span>
            <span className={styles.statusValue}>
              {tableWindowCount === 0
                ? "None"
                : tableWindowCount === 1
                  ? "1 — tracked, with its own HUD"
                  : `${tableWindowCount} — each tracked, with its own HUD`}
            </span>
          </div>
        </div>
        <label className={styles.minHandsRow}>
          <span>
            PokerStars &quot;Auto-Center&quot; is enabled
            <div className={styles.hudHint} style={{ marginTop: 4 }}>
              Required for automatic seat mapping (cards placed on the right seat with no
              dragging). Without it, cards fall back to manual per-player placement — window
              tracking still applies either way.
            </div>
          </span>
          <input
            type="checkbox"
            checked={appSettings?.autoCenterEnabled ?? false}
            disabled={!appSettings || savingAutoCenter}
            onChange={(e) => handleAutoCenterToggle(e.target.checked)}
          />
        </label>
      </div>

      <div className={styles.section}>
        <div className={styles.sectionTitle}>Diagnostics</div>
        <p className={styles.hudHint} style={{ marginTop: 0 }}>
          If the HUD ever looks wrong during play (cards for the wrong table, stats that don&apos;t
          update, anything that &quot;feels off&quot;), click Copy Diagnostics and paste the result
          back — no need to describe what happened.
        </p>
        <div className={styles.diagnosticsRow}>
          <button
            type="button"
            className={styles.resetSetupButton}
            onClick={handleCopyDiagnostics}
            disabled={copyingDiagnostics}
          >
            {copyingDiagnostics ? "Gathering…" : "Copy Diagnostics"}
          </button>
          {diagnosticsStatus && <span className={styles.diagnosticsStatus}>{diagnosticsStatus}</span>}
        </div>
        {diagnosticsText && (
          <textarea
            className={styles.diagnosticsTextarea}
            readOnly
            value={diagnosticsText}
            onFocus={(e) => e.currentTarget.select()}
          />
        )}
      </div>

      <div className={styles.list}>
        {SETTINGS_ITEMS.map((item) => (
          <div key={item.label} className={styles.row}>
            <div>
              <div className={styles.label}>{item.label}</div>
              <div className={styles.description}>{item.description}</div>
            </div>
            <span className={styles.badge}>Coming soon</span>
          </div>
        ))}
      </div>
    </div>
  );
}
