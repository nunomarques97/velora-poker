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
const SEARCH_INPUT_ID = "villain-search";
const SEARCH_RESULT_ID = "villain-search-result";

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

/** The search box's text as matched: trimmed, case-insensitive. */
function searchNeedle(query: string): string {
  return query.trim().toLocaleLowerCase();
}

function nameMatches(name: string, needle: string): boolean {
  return needle !== "" && name.toLocaleLowerCase().includes(needle);
}

/** What the search found, across every open table. */
function searchResult(tables: SidePanelTable[], needle: string) {
  const players = new Set<string>();
  const tableIds = new Set<number>();
  for (const table of tables) {
    for (const villain of table.villains) {
      if (nameMatches(villain.name, needle)) {
        players.add(villain.playerId);
        tableIds.add(table.tableId);
      }
    }
  }
  return { players: players.size, tableIds };
}

/** The name with the matched part wrapped in <mark>. */
function HighlightedName({ name, needle }: { name: string; needle: string }) {
  const lower = name.toLocaleLowerCase();
  const at = needle === "" ? -1 : lower.indexOf(needle);
  if (at < 0) return <>{name}</>;
  // Lower-casing can change a string's length (e.g. "İ"); then the whole
  // name is marked rather than a slice in the wrong place.
  if (lower.length !== name.length) return <mark className={styles.match}>{name}</mark>;
  return (
    <>
      {name.slice(0, at)}
      <mark className={styles.match}>{name.slice(at, at + needle.length)}</mark>
      {name.slice(at + needle.length)}
    </>
  );
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
  // The villain search. Separate from the snapshot, so refreshes, table
  // changes and failed refreshes never touch it. While it filters, the
  // matching tables are open unless collapsed during this search
  // (`searchCollapsed`); `expanded` is left alone, so clearing the search
  // brings back exactly the rows the user had open.
  const [query, setQuery] = useState("");
  const [searchCollapsed, setSearchCollapsed] = useState<ReadonlySet<number>>(() => new Set());

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
    const unlistenHands = onHandsImported(() => load()).catch(() => undefined);
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

  const needle = searchNeedle(query);
  const searching = needle !== "";
  const result =
    state.status === "ready" && searching ? searchResult(state.snapshot.tables, needle) : null;
  // Announced once the typing pauses, and again only when what it finds
  // changes (a refresh can seat or unseat a match).
  const searchAnnouncement =
    state.status !== "ready"
      ? null
      : result === null
        ? ""
        : result.players === 0
          ? `No player matches "${query.trim()}".`
          : `${plural(result.players, "player matches", "players match")} "${query.trim()}" at ${plural(result.tableIds.size, "table", "tables")}.`;
  const lastSearchAnnouncement = useRef("");

  useEffect(() => {
    if (searchAnnouncement === null || searchAnnouncement === lastSearchAnnouncement.current) return;
    const timer = window.setTimeout(() => {
      const cleared = searchAnnouncement === "" && lastSearchAnnouncement.current !== "";
      lastSearchAnnouncement.current = searchAnnouncement;
      if (searchAnnouncement !== "") setAnnouncement(searchAnnouncement);
      else if (cleared && state.status === "ready") {
        setAnnouncement(`Search cleared. ${plural(state.snapshot.tables.length, "table", "tables")} open.`);
      }
    }, 400);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [searchAnnouncement]);

  function changeQuery(next: string) {
    if (searchNeedle(next) !== needle) setSearchCollapsed(new Set());
    setQuery(next);
  }

  function toggle(tableId: number) {
    if (searching) {
      setSearchCollapsed((prev) => {
        const next = new Set(prev);
        if (next.has(tableId)) next.delete(tableId);
        else next.add(tableId);
        return next;
      });
      return;
    }
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
        {state.status === "ready" && (
          <div className={styles.search} role="search">
            <label className={styles.searchLabel} htmlFor={SEARCH_INPUT_ID}>
              Find a player
            </label>
            <input
              id={SEARCH_INPUT_ID}
              className={styles.searchInput}
              type="search"
              placeholder="Screen name, at any table"
              autoComplete="off"
              spellCheck={false}
              aria-describedby={result !== null ? SEARCH_RESULT_ID : undefined}
              value={query}
              onChange={(e) => changeQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape" && query !== "") {
                  e.preventDefault();
                  changeQuery("");
                }
              }}
            />
            {result !== null && (
              <p id={SEARCH_RESULT_ID} className={styles.searchResult}>
                {result.players === 0
                  ? "No match · Esc clears"
                  : `${plural(result.players, "player", "players")} at ${result.tableIds.size} of ${plural(state.snapshot.tables.length, "table", "tables")} · Esc clears`}
              </p>
            )}
          </div>
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
          ) : result !== null && result.players === 0 ? (
            <div className={styles.stateBox} data-search-empty>
              <p className={styles.emptyTitle}>No player matches &ldquo;{query.trim()}&rdquo;</p>
              <p>
                Nobody at your {plural(state.snapshot.tables.length, "open table", "open tables")} has
                that in their screen name. Press Esc or clear the box to see every table again.
              </p>
            </div>
          ) : (
            <ul className={styles.tables}>
              {state.snapshot.tables
                .filter((table) => result === null || result.tableIds.has(table.tableId))
                .map((table) => (
                  <TableRow
                    key={table.tableId}
                    table={table}
                    allTables={state.snapshot.tables}
                    strategic={state.strategic}
                    needle={needle}
                    open={searching ? !searchCollapsed.has(table.tableId) : expanded.has(table.tableId)}
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
  needle,
  open,
  onToggle,
}: {
  table: SidePanelTable;
  allTables: SidePanelTable[];
  strategic: boolean;
  /** The active search, "" when none. */
  needle: string;
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
                needle={needle}
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
  needle,
  otherTables,
}: {
  villain: SidePanelVillain;
  strategic: boolean;
  needle: string;
  otherTables: string[];
}) {
  const read = villain.topRead;
  const match = nameMatches(villain.name, needle);
  return (
    <li
      className={`${styles.villain} ${match ? styles.villainMatch : ""}`}
      data-player-id={villain.playerId}
      data-match={match ? "" : undefined}
    >
      <div className={styles.villainHead}>
        {villain.tag !== null && (
          <span className={styles.tag}>
            <span className={styles.srOnly}>Tag </span>
            {villain.tag}
          </span>
        )}
        <span className={styles.villainName} title={villain.name}>
          <HighlightedName name={villain.name} needle={needle} />
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
