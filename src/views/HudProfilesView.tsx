import type { Player } from "../data/types";
import { hudProfiles, hudTableSeats } from "../data/mockData";
import { HudTableMock } from "../components/HudTableMock/HudTableMock";
import styles from "./HudProfilesView.module.css";

interface HudProfilesViewProps {
  onSelectPlayer: (player: Player) => void;
}

export function HudProfilesView({ onSelectPlayer }: HudProfilesViewProps) {
  return (
    <div>
      <div className="view-header">
        <h1>HUD Profiles</h1>
        <p>Manage stat layouts and preview them on a live table.</p>
      </div>

      <div className={styles.layout}>
        <div className={styles.profileList}>
          {hudProfiles.map((profile) => (
            <div
              key={profile.id}
              className={`${styles.profileCard} ${
                profile.active ? styles.profileCardActive : ""
              }`}
            >
              <div className={styles.profileHead}>
                <span className={styles.profileName}>{profile.name}</span>
                {profile.active && <span className={styles.activeTag}>Active</span>}
              </div>
              <p className={styles.profileDescription}>{profile.description}</p>
            </div>
          ))}
        </div>

        <HudTableMock seats={hudTableSeats} onSelectPlayer={onSelectPlayer} />
      </div>
    </div>
  );
}
