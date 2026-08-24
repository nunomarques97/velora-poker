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
    label: "Hand History Import",
    description: "Watch folders for new hand history files.",
  },
  {
    label: "Account",
    description: "Manage your Velora account and subscription.",
  },
];

export function SettingsView() {
  return (
    <div>
      <div className="view-header">
        <h1>Settings</h1>
        <p>Configuration will become available as features are implemented.</p>
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
