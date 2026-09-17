import { useEffect, useState } from "react";
import "./App.css";
import { Sidebar } from "./components/Sidebar/Sidebar";
import { PlayerProfileDrawer } from "./components/PlayerProfileDrawer/PlayerProfileDrawer";
import { IngestionStatusBar } from "./components/IngestionStatusBar/IngestionStatusBar";
import { DashboardView } from "./views/DashboardView";
import { PlayersView } from "./views/PlayersView";
import { SessionsView } from "./views/SessionsView";
import { HudProfilesView } from "./views/HudProfilesView";
import { SettingsView, INGESTION_SECTION_ID } from "./views/SettingsView";
import { OnboardingFlow } from "./onboarding/OnboardingFlow";
import type { Player } from "./data/types";
import { getAppSettings, getIngestionHealth, isTauriAvailable, onHandsImported } from "./data/api";

export type View = "dashboard" | "players" | "sessions" | "hud" | "settings";

// / how often the main window re-checks ingestion health for the
// persistent status line. Cheap (one read-only SQLite query) and matches the
// existing Table Detection poll cadence in SettingsView.
const INGESTION_POLL_MS = 5000;

function App() {
  const [view, setView] = useState<View>("dashboard");
  const [selectedPlayer, setSelectedPlayer] = useState<Player | null>(null);
  const [onboardingComplete, setOnboardingComplete] = useState<boolean | null>(null);
  const [handsRejected, setHandsRejected] = useState(0);

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

  useEffect(() => {
    if (!isTauriAvailable()) return;
    let cancelled = false;
    function poll() {
      getIngestionHealth()
        .then((health) => {
          if (!cancelled) setHandsRejected(health.handsRejected);
        })
        .catch(() => undefined);
    }
    poll();
    const interval = window.setInterval(poll, INGESTION_POLL_MS);
    const unlistenPromise = onHandsImported(poll);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  function goToIngestionSettings() {
    setView("settings");
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    // Double rAF: wait for SettingsView to actually mount before scrolling to
    // its Ingestion section, instead of racing the view switch.
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        document
          .getElementById(INGESTION_SECTION_ID)
          ?.scrollIntoView({ behavior: reduceMotion ? "auto" : "smooth", block: "start" });
      });
    });
  }

  if (onboardingComplete === null) {
    return null;
  }

  if (!onboardingComplete) {
    return <OnboardingFlow onComplete={() => setOnboardingComplete(true)} />;
  }

  return (
    <div className="app-root">
      {handsRejected > 0 && (
        <IngestionStatusBar handsRejected={handsRejected} onViewDetails={goToIngestionSettings} />
      )}

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
    </div>
  );
}

export default App;
