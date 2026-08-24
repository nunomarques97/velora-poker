import type { ComponentType, SVGProps } from "react";
import styles from "./Sidebar.module.css";
import {
  DashboardIcon,
  PlayersIcon,
  SessionsIcon,
  HudIcon,
  SettingsIcon,
} from "../icons";
import type { View } from "../../App";

interface NavEntry {
  id: View;
  label: string;
  icon: ComponentType<SVGProps<SVGSVGElement>>;
}

const NAV_ENTRIES: NavEntry[] = [
  { id: "dashboard", label: "Dashboard", icon: DashboardIcon },
  { id: "players", label: "Players", icon: PlayersIcon },
  { id: "sessions", label: "Sessions", icon: SessionsIcon },
  { id: "hud", label: "HUD Profiles", icon: HudIcon },
  { id: "settings", label: "Settings", icon: SettingsIcon },
];

interface SidebarProps {
  active: View;
  onNavigate: (view: View) => void;
}

export function Sidebar({ active, onNavigate }: SidebarProps) {
  return (
    <aside className={styles.sidebar}>
      <div className={styles.brand}>
        <div className={styles.mark}>V</div>
        <div className={styles.brandText}>
          <span className={styles.brandName}>Velora</span>
          <span className={styles.brandSub}>Poker HUD</span>
        </div>
      </div>

      <nav className={styles.nav}>
        {NAV_ENTRIES.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            type="button"
            className={`${styles.navItem} ${active === id ? styles.navItemActive : ""}`}
            onClick={() => onNavigate(id)}
          >
            <Icon className={styles.navIcon} />
            {label}
          </button>
        ))}
      </nav>

      <div className={styles.spacer} />

      <div className={styles.footer}>
        <span className={styles.footerLabel}>Velora v0.1.0 &middot; MVP</span>
      </div>
    </aside>
  );
}
