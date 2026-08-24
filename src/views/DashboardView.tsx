import { StatCard } from "../components/StatCard/StatCard";
import { dashboardSummary } from "../data/mockData";
import styles from "./DashboardView.module.css";

export function DashboardView() {
  return (
    <div>
      <div className="view-header">
        <h1>Dashboard</h1>
        <p>Overview of today&apos;s tracked activity.</p>
      </div>

      <div className={styles.grid}>
        <StatCard label="Sessions Today" value={dashboardSummary.sessionsToday} />
        <StatCard
          label="Hands Played"
          value={dashboardSummary.handsPlayed.toLocaleString()}
        />
        <StatCard
          label="Players Tracked"
          value={dashboardSummary.playersTracked.toLocaleString()}
        />
        <StatCard
          label="Current HUD Profile"
          value={dashboardSummary.currentHudProfile}
        />
      </div>
    </div>
  );
}
