import { useState } from "react";
import "./App.css";
import { Sidebar } from "./components/Sidebar/Sidebar";
import { PlayerProfileDrawer } from "./components/PlayerProfileDrawer/PlayerProfileDrawer";
import { DashboardView } from "./views/DashboardView";
import { PlayersView } from "./views/PlayersView";
import { SessionsView } from "./views/SessionsView";
import { HudProfilesView } from "./views/HudProfilesView";
import { SettingsView } from "./views/SettingsView";
import type { Player } from "./data/types";

export type View = "dashboard" | "players" | "sessions" | "hud" | "settings";

function App() {
  const [view, setView] = useState<View>("dashboard");
  const [selectedPlayer, setSelectedPlayer] = useState<Player | null>(null);

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
        />
      )}
    </div>
  );
}

export default App;
