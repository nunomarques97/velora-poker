import { useEffect, useState } from "react";
import { DesktopAppRequiredError, getSessions } from "../data/api";
import type { Session } from "../data/types";
import { formatCurrencyResult, formatDuration, formatSessionDate, formatSessionTime } from "../utils/format";
import styles from "./SessionsView.module.css";

type SessionsState =
  | { status: "loading" }
  | { status: "ready"; data: Session[] }
  | { status: "unavailable"; message: string }
  | { status: "error"; message: string };

function formatKind(session: Session): string {
  if (session.hasCash && session.hasTournament) return "Cash + Tournament";
  if (session.hasTournament) return "Tournament";
  return "Cash";
}

function formatResult(session: Session): { text: string; positive: boolean | null } {
  if (session.netResultCash === null || session.currency === null) {
    return { text: "—", positive: null };
  }
  return {
    text: formatCurrencyResult(session.netResultCash, session.currency),
    positive: session.netResultCash > 0,
  };
}

export function SessionsView() {
  const [state, setState] = useState<SessionsState>({ status: "loading" });

  useEffect(() => {
    getSessions()
      .then((data) => setState({ status: "ready", data }))
      .catch((err: unknown) => {
        if (err instanceof DesktopAppRequiredError) {
          setState({ status: "unavailable", message: err.message });
        } else {
          setState({ status: "error", message: String(err) });
        }
      });
  }, []);

  return (
    <div>
      <div className="view-header">
        <h1>Sessions</h1>
        <p>Recent tracked sessions across all tables.</p>
      </div>

      {state.status === "unavailable" && <p>{state.message}</p>}
      {state.status === "error" && <p>Failed to load sessions: {state.message}</p>}
      {state.status === "ready" && state.data.length === 0 && (
        <p>No sessions yet — play a few hands and they&apos;ll show up here.</p>
      )}

      {state.status === "ready" && state.data.length > 0 && (
        <div className={styles.list}>
          <div className={styles.headerRow}>
            <span>Date</span>
            <span>Start–End</span>
            <span>Format</span>
            <span>Tables</span>
            <span>Hands</span>
            <span>Duration</span>
            <span>Result</span>
          </div>
          {state.data.map((session) => {
            const result = formatResult(session);
            return (
              <div key={session.startAt} className={styles.row}>
                <span className={styles.date}>{formatSessionDate(session.startAt)}</span>
                <span className={`${styles.cell} tabular`}>
                  {/* Zero-width space after the dash: the only place this
                      range is allowed to wrap, now that each time is atomic. */}
                  {formatSessionTime(session.startAt)}
                  {"\u2013\u200b"}
                  {formatSessionTime(session.endAt)}
                </span>
                <span className={styles.cell}>{formatKind(session)}</span>
                <span className={`${styles.cell} tabular`}>{session.tableCount}</span>
                <span className={`${styles.cell} tabular`}>{session.handCount.toLocaleString()}</span>
                <span className={`${styles.cell} tabular`}>{formatDuration(session.durationSecs)}</span>
                <span
                  className={`${styles.result} tabular ${
                    result.positive === null ? "" : result.positive ? styles.positive : styles.negative
                  }`}
                >
                  {result.text}
                </span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
