import { useEffect, useRef, useState } from "react";
import type { SidePanelSnapshot, SidePanelTable, SidePanelVillain, TableQuality } from "../data/types";
import {
  DesktopAppRequiredError,
  getAppVersion,
  getSidePanelSnapshot,
  onHandsImported,
  onTrackedTablesChanged,
} from "../data/api";
import styles from "./SidePanelApp.module.css";

/**
 * The side panel: a normal window meant for the second monitor, one row per
 * open PokerStars table. Between hands only: everything here comes from
 * completed hand histories (`get_side_panel_snapshot`).
 *
 * The default build has no engine, so quality, tags and reads are `null` in
 * the snapshot and shown as unavailable (D102); tables, villains, hand counts
 * and multi-table flags are always there.
 */

const STRATEGIC_FEATURE = "strategic-analysis";

type PanelState =
  | { status: "loading" }
  | {
      status: "ready";
      snapshot: SidePanelSnapshot;
      strategic: boolean;
      /** The last refresh failed; `snapshot` is the previous one. */
      refreshError: string | null;
    }
  | { status: "unavailable"; message: string }
  | { status: "error"; message: string };

const QUALITY_LABEL: Record<TableQuality["label"], string> = {
  soft: "Soft",
  average: "Average",
  tough: "Tough",
};

function describeError(err: unknown): string {
  if (err instanceof Error) return err.message;
  return String(err);
}

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

function tableName(table: Pick<SidePanelTable, "tableId" | "tableName">): string {
  return table.tableName ?? `Unnamed table #${table.tableId}`;
}

function updatedAt(iso: string): string {
  const at = new Date(iso);
  return Number.isNaN(at.getTime()) ? "" : at.toLocaleTimeString();
}

export function SidePanelApp() {
  const [state, setState] = useState<PanelState>({ status: "loading" });
  // Keyed by tableId, so a refresh (new hand, tables opening or closing)
  // keeps every still-open table exactly as the user left it.
  const [expanded, setExpanded] = useState<ReadonlySet<number>>(() => new Set());
  const [retrying, setRetrying] = useState(false);
  const [announcement, setAnnouncement] = useState("");

  // Set in the effect (not at render) so StrictMode's setup/cleanup/setup
  // leaves it true, and a response landing after unmount is dropped.
  const mounted = useRef(false);
  // Only the latest request may write: a new hand and a table opening can
  // fire two refreshes whose answers come back in either order.
  const request = useRef(0);
  // Which build this is never changes while the app runs; asked once.
  const strategic = useRef<boolean | null>(null);
  const lastTableCount = useRef<number | null>(null);
  // The last answer was a failure (error state or out-of-date list).
  const failing = useRef(false);
  // A successful retry removes the button that had focus.
  const focusAfterRetry = useRef(false);
  const retryPending = useRef(false);
  const headingRef = useRef<HTMLHeadingElement>(null);

  async function load(): Promise<void> {
    const id = ++request.current;
    try {
      const [isStrategic, snapshot] = await Promise.all([
        strategic.current !== null
          ? Promise.resolve(strategic.current)
          : getAppVersion().then((v) => v.features.includes(STRATEGIC_FEATURE)),
        getSidePanelSnapshot(),
      ]);
      strategic.current = isStrategic;
      if (!mounted.current || id !== request.current) return;

      const ids = new Set(snapshot.tables.map((t) => t.tableId));
      setExpanded((prev) => {
        const kept = [...prev].filter((tableId) => ids.has(tableId));
        return kept.length === prev.size ? prev : new Set(kept);
      });
      // Announce what changed, not every refresh: with 12 tables a new hand
      // lands every few seconds.
      const count = snapshot.tables.length;
      const recovered = failing.current;
      if (recovered || count !== lastTableCount.current) {
        setAnnouncement(
          count === 0
            ? "No PokerStars tables open."
            : `${plural(count, "table", "tables")} open${recovered ? ", updated" : ""}.`,
        );
      }
      lastTableCount.current = count;
      failing.current = false;
      setState({ status: "ready", snapshot, strategic: isStrategic, refreshError: null });
    } catch (err) {
      if (!mounted.current || id !== request.current) return;
      if (err instanceof DesktopAppRequiredError) {
        setState({ status: "unavailable", message: err.message });
        return;
      }
      failing.current = true;
      const message = describeError(err);
      // A failed refresh keeps the last good list, marked as out of date,
      // instead of blanking the panel mid-session.
      setState((prev) =>
        prev.status === "ready" ? { ...prev, refreshError: message } : { status: "error", message },
      );
    }
  }

  async function retry() {
    // A second press before React re-renders must not start a second load.
    if (retryPending.current) return;
    retryPending.current = true;
    focusAfterRetry.current = true;
    setRetrying(true);
    try {
      await load();
    } finally {
      retryPending.current = false;
      if (mounted.current) setRetrying(false);
    }
  }

  useEffect(() => {
    mounted.current = true;
    load();
    const unlistenHands = onHandsImported(() => {
      load();
    }).catch(() => undefined);
    const unlistenTables = onTrackedTablesChanged(() => {
      load();
    }).catch(() => undefined);
    return () => {
      mounted.current = false;
      unlistenHands.then((fn) => fn?.());
      unlistenTables.then((fn) => fn?.());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!focusAfterRetry.current) return;
    const recovered = state.status === "ready" && state.refreshError === null;
    if (!recovered) {
      if (state.status !== "loading") focusAfterRetry.current = false;
      return;
    }
    focusAfterRetry.current = false;
    const active = document.activeElement;
    if (active === null || active === document.body) headingRef.current?.focus();
  }, [state]);

  function toggle(tableId: number) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(tableId)) next.delete(tableId);
      else next.add(tableId);
      return next;
    });
  }

  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <h1 className={styles.title} ref={headingRef} tabIndex={-1}>
          Tables
        </h1>
        {state.status === "ready" && (
          <p className={styles.subtitle}>
            {plural(state.snapshot.tables.length, "table", "tables")} open
            {updatedAt(state.snapshot.generatedAt) && ` · updated ${updatedAt(state.snapshot.generatedAt)}`}
          </p>
        )}
      </header>

      <p className={styles.srOnly} role="status" aria-live="polite">
        {announcement}
      </p>

      {state.status === "loading" && <div className={styles.stateBox}>Loading tables&hellip;</div>}

      {state.status === "unavailable" && <div className={styles.stateBox}>{state.message}</div>}

      {state.status === "error" && (
        <div className={styles.stateBox} role="alert">
          <p>Failed to load tables: {state.message}</p>
          <button
            type="button"
            className={styles.retryButton}
            onClick={retry}
            // Not `disabled`: a disabled button drops keyboard focus.
            aria-disabled={retrying}
          >
            {retrying ? "Retrying…" : "Retry"}
          </button>
        </div>
      )}

      {state.status === "ready" && (
        <>
          {state.refreshError !== null && (
            <div className={styles.refreshError} role="alert">
              <span>
                Couldn't refresh the tables: {state.refreshError}. Showing the last update.
              </span>
              <button type="button" className={styles.retryButton} onClick={retry} aria-disabled={retrying}>
                {retrying ? "Retrying…" : "Retry"}
              </button>
            </div>
          )}

          {state.snapshot.tables.length === 0 ? (
            <div className={styles.stateBox}>
              <p className={styles.emptyTitle}>No PokerStars tables open</p>
              <p>
                Open a table in PokerStars. Each table appears here by itself, with its players, as
                soon as Velora finds its window.
              </p>
            </div>
          ) : (
            <ul className={styles.tables}>
              {state.snapshot.tables.map((table) => (
                <TableRow
                  key={table.tableId}
                  table={table}
                  allTables={state.snapshot.tables}
                  strategic={state.strategic}
                  open={expanded.has(table.tableId)}
                  onToggle={() => toggle(table.tableId)}
                />
              ))}
            </ul>
          )}
        </>
      )}
    </div>
  );
}

function TableRow({
  table,
  allTables,
  strategic,
  open,
  onToggle,
}: {
  table: SidePanelTable;
  allTables: SidePanelTable[];
  strategic: boolean;
  open: boolean;
  onToggle: () => void;
}) {
  const listId = `table-${table.tableId}-villains`;
  const shared = table.villains.filter((v) => v.otherTableIds.length > 0).length;
  const nameOf = (tableId: number) => {
    const other = allTables.find((t) => t.tableId === tableId);
    return other ? tableName(other) : `table #${tableId}`;
  };

  return (
    <li className={styles.table} data-table-id={table.tableId}>
      <h2 className={styles.tableHeading}>
        <button
          type="button"
          className={styles.expandButton}
          aria-expanded={open}
          aria-controls={listId}
          onClick={onToggle}
        >
          <span className={styles.chevron} aria-hidden="true" />
          <span className={styles.tableName} title={tableName(table)}>
            {tableName(table)}
          </span>
          <span className={styles.tableMeta}>
            {table.maxPlayers !== null ? `${table.maxPlayers}-max` : "Size unknown"}
            {" · "}
            {table.playerCount === 0 ? "no hands yet" : plural(table.playerCount, "player", "players")}
          </span>
          <Quality quality={table.quality} strategic={strategic} />
          {shared > 0 && (
            <span className={styles.sharedCount}>
              {plural(shared, "player", "players")} also at other tables
            </span>
          )}
        </button>
      </h2>

      <div id={listId} className={styles.villains} hidden={!open}>
        {!strategic && <p className={styles.unavailable}>Tags and reads unavailable in this build.</p>}
        {table.villains.length === 0 ? (
          <p className={styles.noVillains}>
            {table.playerCount === 0
              ? "No hands imported at this table yet. Its players appear after the first hand ends."
              : "No opponents in this table's latest hand."}
          </p>
        ) : (
          <ul className={styles.villainList}>
            {table.villains.map((villain) => (
              <VillainRow
                key={villain.playerId}
                villain={villain}
                strategic={strategic}
                otherTables={villain.otherTableIds.map(nameOf)}
              />
            ))}
          </ul>
        )}
      </div>
    </li>
  );
}

function Quality({ quality, strategic }: { quality: TableQuality | null; strategic: boolean }) {
  if (!strategic) {
    return <span className={styles.qualityMuted}>Quality unavailable in this build</span>;
  }
  if (quality === null) {
    return <span className={styles.qualityMuted}>Quality: not enough hands yet</span>;
  }
  return (
    <span
      className={`${styles.quality} ${styles[`quality-${quality.label}`] ?? ""}`}
      title={`Table quality ${quality.score} of 100, from ${quality.basisHands} opponent hands`}
    >
      <span className={styles.srOnly}>Quality </span>
      <span className={styles.qualityScore}>{quality.score}</span>
      <span>{QUALITY_LABEL[quality.label] ?? quality.label}</span>
    </span>
  );
}

function VillainRow({
  villain,
  strategic,
  otherTables,
}: {
  villain: SidePanelVillain;
  strategic: boolean;
  otherTables: string[];
}) {
  const read = villain.topRead;
  return (
    <li className={styles.villain} data-player-id={villain.playerId}>
      <div className={styles.villainHead}>
        {villain.tag !== null && (
          <span className={styles.tag}>
            <span className={styles.srOnly}>Tag </span>
            {villain.tag}
          </span>
        )}
        <span className={styles.villainName} title={villain.name}>
          {villain.name}
        </span>
        <span className={styles.villainHands}>
          {plural(villain.hands, "hand", "hands")}
          {villain.seat !== null && ` · seat ${villain.seat}`}
        </span>
      </div>
      {read !== null ? (
        <div className={styles.read}>
          <p className={styles.observation}>{read.observation}</p>
          {read.advice && <p className={styles.advice}>{read.advice}</p>}
          {read.confidencePct !== null && (
            <p className={styles.confidence}>Confidence {Math.round(read.confidencePct)}%</p>
          )}
        </div>
      ) : (
        strategic && <p className={styles.noRead}>No read yet: not enough hands in any spot.</p>
      )}
      {otherTables.length > 0 && (
        <p className={styles.flag} data-multi-table>
          <span className={styles.flagMark} aria-hidden="true">
            ⚑
          </span>{" "}
          Also at {otherTables.join(", ")}
        </p>
      )}
    </li>
  );
}
