import { sessions } from "../data/mockData";
import styles from "./SessionsView.module.css";

export function SessionsView() {
  return (
    <div>
      <div className="view-header">
        <h1>Sessions</h1>
        <p>Recent tracked sessions across all tables.</p>
      </div>

      <div className={styles.list}>
        <div className={styles.headerRow}>
          <span>Date</span>
          <span>Stakes</span>
          <span>Tables</span>
          <span>Hands</span>
          <span>Duration</span>
          <span>Result</span>
        </div>
        {sessions.map((session) => (
          <div key={session.id} className={styles.row}>
            <span className={styles.date}>{session.date}</span>
            <span className={`${styles.cell} tabular`}>{session.stakes}</span>
            <span className={`${styles.cell} tabular`}>{session.tables}</span>
            <span className={`${styles.cell} tabular`}>
              {session.hands.toLocaleString()}
            </span>
            <span className={`${styles.cell} tabular`}>{session.duration}</span>
            <span
              className={`${styles.result} tabular ${
                session.positive ? styles.positive : styles.negative
              }`}
            >
              {session.result}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
