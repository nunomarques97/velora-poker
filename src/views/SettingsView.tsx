import { useEffect, useState } from "react";
import type { ImportStatus } from "../data/types";
import { DesktopAppRequiredError, getImportStatus, setHandHistoryDir } from "../data/api";
import styles from "./SettingsView.module.css";

const SETTINGS_ITEMS = [
  {
    label: "Table Detection",
    description: "Automatically detect open poker tables.",
  },
  {
    label: "HUD Appearance",
    description: "Theme, opacity, and stat display density.",
  },
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
      return status.startsWith("error:") ? status : status;
  }
}

export function SettingsView() {
  const [state, setState] = useState<StatusState>({ status: "loading" });
  const [pathInput, setPathInput] = useState("");
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

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
  }

  useEffect(() => {
    load();
  }, []);

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

  return (
    <div>
      <div className="view-header">
        <h1>Settings</h1>
        <p>Configuration will become available as features are implemented.</p>
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
              placeholder="C:\Users\you\AppData\Local\PokerStars\HandHistory\ScreenName"
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
