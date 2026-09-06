import { useEffect, useState } from "react";
import type { HudProfile, Player } from "../data/types";
import {
  DesktopAppRequiredError,
  closeOverlay,
  getActiveHudProfile,
  getHudProfiles,
  getPlayers,
  isOverlayOpen,
  onHandsImported,
  onOverlayVisibilityChanged,
  openOverlay,
  setActiveHudProfile,
  setHudProfileMinHands,
  setOverlayClickThrough,
} from "../data/api";
import { PlayerHudCard } from "../hud/PlayerHudCard";
import styles from "./HudProfilesView.module.css";

interface HudProfilesViewProps {
  onSelectPlayer: (player: Player) => void;
}

type LoadState =
  | { status: "loading" }
  | { status: "ready" }
  | { status: "unavailable"; message: string }
  | { status: "error"; message: string };

export function HudProfilesView({ onSelectPlayer }: HudProfilesViewProps) {
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [profiles, setProfiles] = useState<HudProfile[]>([]);
  const [activeProfile, setActiveProfileState] = useState<HudProfile | null>(null);
  const [players, setPlayers] = useState<Player[]>([]);
  const [overlayOpen, setOverlayOpenState] = useState(false);
  const [locked, setLocked] = useState(false);
  const [minHandsInput, setMinHandsInput] = useState("25");

  function loadAll() {
    Promise.all([getHudProfiles(), getActiveHudProfile(), getPlayers(), isOverlayOpen()])
      .then(([allProfiles, active, allPlayers, overlayIsOpen]) => {
        setProfiles(allProfiles);
        setActiveProfileState(active);
        setMinHandsInput(String(active.minHands));
        setPlayers(allPlayers);
        setOverlayOpenState(overlayIsOpen);
        setState({ status: "ready" });
      })
      .catch((err: unknown) => {
        if (err instanceof DesktopAppRequiredError) {
          setState({ status: "unavailable", message: err.message });
        } else {
          setState({ status: "error", message: String(err) });
        }
      });
  }

  useEffect(() => {
    loadAll();
    const unlisten = onHandsImported(() => {
      getPlayers()
        .then(setPlayers)
        .catch(() => undefined);
    }).catch(() => undefined);

    // Rust broadcasts every overlay show/hide, so this view stays correct even
    // when the overlay is closed from its own in-overlay "Close" button, which
    // this window has no other way to observe.
    const unlistenOverlay = onOverlayVisibilityChanged((open) => {
      setOverlayOpenState(open);
      // Click-through is reset by the OS window going away; a re-opened
      // overlay always starts unlocked.
      if (!open) setLocked(false);
    }).catch(() => undefined);

    return () => {
      unlisten.then((fn) => fn?.());
      unlistenOverlay.then((fn) => fn?.());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function handleSelectProfile(id: string) {
    const updated = await setActiveHudProfile(id);
    setActiveProfileState(updated);
    setMinHandsInput(String(updated.minHands));
    getPlayers().then(setPlayers).catch(() => undefined);
  }

  async function handleMinHandsBlur() {
    if (!activeProfile) return;
    const parsed = Number.parseInt(minHandsInput, 10);
    const value = Number.isFinite(parsed) && parsed >= 0 ? parsed : activeProfile.minHands;
    setMinHandsInput(String(value));
    const updated = await setHudProfileMinHands(activeProfile.id, value);
    setActiveProfileState(updated);
    getPlayers().then(setPlayers).catch(() => undefined);
  }

  async function toggleOverlay() {
    // Re-check live state rather than trusting the rendered flag: cheap, and
    // it keeps the click correct even if this window mounted after an overlay
    // visibility event it never heard. `overlayOpen` itself is seeded once by
    // `loadAll` and thereafter written only by the `onOverlayVisibilityChanged`
    // subscription, so the label can't drift from what a click will really do.
    if (await isOverlayOpen()) {
      await closeOverlay();
    } else {
      await openOverlay();
    }
  }

  async function toggleLock() {
    const next = !locked;
    await setOverlayClickThrough(next);
    setLocked(next);
  }

  if (state.status === "loading") {
    return (
      <div>
        <ViewHeader />
        <div className={styles.stateBox}>Loading HUD&hellip;</div>
      </div>
    );
  }

  if (state.status === "unavailable") {
    return (
      <div>
        <ViewHeader />
        <div className={styles.stateBox}>{state.message}</div>
      </div>
    );
  }

  if (state.status === "error") {
    return (
      <div>
        <ViewHeader />
        <div className={styles.stateBox}>Failed to load HUD: {state.message}</div>
      </div>
    );
  }

  return (
    <div>
      <ViewHeader />

      <div className={styles.controls}>
        <div className={styles.profileSwitcher}>
          {profiles.map((p) => (
            <button
              key={p.id}
              type="button"
              className={`${styles.profileButton} ${
                activeProfile?.id === p.id ? styles.profileButtonActive : ""
              }`}
              onClick={() => handleSelectProfile(p.id)}
            >
              {p.name}
            </button>
          ))}
        </div>

        <div className={styles.rightControls}>
          <label className={styles.minHandsLabel}>
            Min hands
            <input
              className={styles.minHandsInput}
              type="number"
              min={0}
              value={minHandsInput}
              onChange={(e) => setMinHandsInput(e.target.value)}
              onBlur={handleMinHandsBlur}
            />
          </label>

          <button type="button" className={styles.overlayButton} onClick={toggleOverlay}>
            {overlayOpen ? "Close Overlay" : "Open Overlay"}
          </button>

          {overlayOpen && (
            <button
              type="button"
              className={`${styles.lockButton} ${locked ? styles.lockButtonActive : ""}`}
              onClick={toggleLock}
            >
              {locked ? "Unlock Positions" : "Lock (Click-through)"}
            </button>
          )}
        </div>
      </div>

      {players.length === 0 ? (
        <div className={styles.stateBox}>
          No hands imported yet. Configure your PokerStars hand history folder in Settings to
          populate the HUD with real player data.
        </div>
      ) : (
        activeProfile && (
          <div className={styles.grid}>
            {players.map((player) => (
              <PlayerHudCard
                key={player.id}
                player={player}
                profile={activeProfile}
                onOpenDetail={onSelectPlayer}
              />
            ))}
          </div>
        )
      )}

      <p className={styles.hint}>
        This grid previews the active HUD model with your real tracked players. The native overlay
        window shows the same cards, positioned over the table — open it with the button above.
      </p>
    </div>
  );
}

function ViewHeader() {
  return (
    <div className="view-header">
      <h1>HUD Profiles</h1>
      <p>Choose a HUD model and preview it against your real tracked players.</p>
    </div>
  );
}
