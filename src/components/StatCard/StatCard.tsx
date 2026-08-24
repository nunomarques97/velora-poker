import type { ReactNode } from "react";
import styles from "./StatCard.module.css";

interface StatCardProps {
  label: string;
  value: ReactNode;
  sub?: ReactNode;
}

export function StatCard({ label, value, sub }: StatCardProps) {
  return (
    <div className={styles.card}>
      <span className={styles.label}>{label}</span>
      <span className={`${styles.value} tabular`}>{value}</span>
      {sub && <span className={styles.sub}>{sub}</span>}
    </div>
  );
}
