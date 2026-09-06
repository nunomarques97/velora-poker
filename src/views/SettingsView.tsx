import { useEffect, useState } from "react";
import type { AppSettings, HudProfile, ImportStatus } from "../data/types";
import type { TableDetectionStatus } from "../data/api";
import {
  DesktopAppRequiredError,
  getActiveHudProfile,
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
  const [trackingDebug, setTrackingDebug] = useState<TableDetectionStatus | null>(null);
  const [savingAutoCenter, setSavingAutoCenter] = useState(false);

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
            setTrackingDebug(status);
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
        <p>Configuration will become available as features are implemented.</p>
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
                  {appSettings?.overlayEnabled ? "Enabled" : "Disabled"}
                </span>
              </div>
            </div>
            <label className={styles.minHandsRow}>
              Minimum hands before automatic classification
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
              Switch HUD models, open the overlay, and manage per-player color overrides from the
              HUD Profiles page.
            </p>
          </>
        ) : (
          <div className={styles.stateBox}>Loading&hellip;</div>
        )}
      </div>

      <div className={styles.section}>
        <div className={styles.sectionTitle}>Table Detection</div>
        <div className={styles.statusGrid}>
          <div className={styles.statusCell}>
            <span className={styles.statusLabel}>PokerStars Table Window</span>
            <span className={styles.statusValue}>
              {tableDetected ? "Detected — overlay is following it" : "Not detected"}
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
        {trackingDebug && (
          <div className={styles.hudHint} style={{ marginTop: 8 }}>
            Window-following debug — hooks installed: {trackingDebug.hooksInstalled}/2, event
            callbacks: {trackingDebug.eventCallbacksTotal} total /{" "}
            {trackingDebug.eventCallbacksMatched} matched, poll ticks:{" "}
            {trackingDebug.pollTicks}, integrity level — Velora:{" "}
            {trackingDebug.appIntegrityLevel ?? "unknown"}, table:{" "}
            {trackingDebug.tableIntegrityLevel ?? "not tracked"}
          </div>
        )}
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
