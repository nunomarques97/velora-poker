import { useEffect, useState } from "react";
import "./App.css";
import { Sidebar } from "./components/Sidebar/Sidebar";
import { PlayerProfileDrawer } from "./components/PlayerProfileDrawer/PlayerProfileDrawer";
import { DashboardView } from "./views/DashboardView";
import { PlayersView } from "./views/PlayersView";
import { SessionsView } from "./views/SessionsView";
import { HudProfilesView } from "./views/HudProfilesView";
import { SettingsView } from "./views/SettingsView";
import { OnboardingFlow } from "./onboarding/OnboardingFlow";
import type { Player } from "./data/types";
import { getAppSettings, isTauriAvailable } from "./data/api";

export type View = "dashboard" | "players" | "sessions" | "hud" | "settings";

function App() {
  const [view, setView] = useState<View>("dashboard");
  const [selectedPlayer, setSelectedPlayer] = useState<Player | null>(null);
  const [onboardingComplete, setOnboardingComplete] = useState<boolean | null>(null);

  useEffect(() => {
    if (!isTauriAvailable()) {
      // Plain browser preview: skip onboarding, real settings aren't reachable anyway.
      setOnboardingComplete(true);
      return;
    }
    getAppSettings()
      .then((s) => setOnboardingComplete(s.onboardingComplete))
      .catch(() => setOnboardingComplete(true));
  }, []);

  if (onboardingComplete === null) {
    return null;
  }

  if (!onboardingComplete) {
    return <OnboardingFlow onComplete={() => setOnboardingComplete(true)} />;
  }

  return (
    <div className="app-shell">
      <Sidebar active={view} onNavigate={setView} />

      <main className="app-main">
        <div className="app-main-inner">
          {view === "dashboard" && <DashboardView />}
          {view === "players" && <PlayersView onSelectPlayer={setSelectedPlayer} />}
          {view === "sessions" && <SessionsView />}
          {view === "hud" && <HudProfilesView onSelectPlayer={setSelectedPlayer} />}
          {view === "settings" && <SettingsView />}
        </div>
      </main>

      {selectedPlayer && (
        <PlayerProfileDrawer
          player={selectedPlayer}
          onClose={() => setSelectedPlayer(null)}
          onPlayerUpdated={setSelectedPlayer}
        />
      )}
    </div>
  );
}

export default App;
