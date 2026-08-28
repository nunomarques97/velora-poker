import { useEffect, useState } from "react";
import { StatCard } from "../components/StatCard/StatCard";
import { DesktopAppRequiredError, getDashboardSummary } from "../data/api";
import type { DashboardSummary } from "../data/types";
import { formatCurrencyResult, formatDuration } from "../utils/format";
import styles from "./DashboardView.module.css";

type SummaryState =
  | { status: "loading" }
  | { status: "ready"; data: DashboardSummary }
  | { status: "unavailable"; message: string }
  | { status: "error"; message: string };

export function DashboardView() {
  const [state, setState] = useState<SummaryState>({ status: "loading" });

  useEffect(() => {
    getDashboardSummary()
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
        <h1>Dashboard</h1>
        <p>Overview of today&apos;s tracked activity.</p>
      </div>

      {state.status === "unavailable" && <p>{state.message}</p>}
      {state.status === "error" && <p>Failed to load dashboard data: {state.message}</p>}

      <div className={styles.grid}>
        <StatCard
          label="Hands Played"
          value={
            state.status === "ready" ? state.data.handsPlayed.toLocaleString() : "–"
          }
        />
        <StatCard
          label="Players Tracked"
          value={
            state.status === "ready" ? state.data.playersTracked.toLocaleString() : "–"
          }
        />
        <StatCard
          label="Current HUD Profile"
          value={state.status === "ready" ? state.data.currentHudProfile : "–"}
        />
        {state.status === "ready" && state.data.sessionsToday && (
          <StatCard
            label="Sessions Today"
            value={state.data.sessionsToday.sessionCount.toLocaleString()}
            sub={
              <>
                {formatDuration(state.data.sessionsToday.totalDurationSecs)} ·{" "}
                {state.data.sessionsToday.totalHands.toLocaleString()} hands
                {state.data.sessionsToday.netResultCash !== null &&
                  state.data.sessionsToday.currency !== null &&
                  ` · ${formatCurrencyResult(
                    state.data.sessionsToday.netResultCash,
                    state.data.sessionsToday.currency,
                  )}`}
              </>
            }
          />
        )}
      </div>
    </div>
  );
}
