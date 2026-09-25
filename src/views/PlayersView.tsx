import { useEffect, useState } from "react";
import type { ClassificationResult, Player } from "../data/types";
import { DesktopAppRequiredError, getPlayersPage, onHandsImported } from "../data/api";
import { NO_OPPORTUNITY } from "../hud/statFormat";
import styles from "./PlayersView.module.css";

/** `null` means the stat's denominator was zero (no opportunity yet) —
 * rendered as an em dash rather than a fabricated 0%. */
function formatPct(value: number | null): string {
  return value === null ? NO_OPPORTUNITY : `${value}%`;
}

/**
 * The archetype this row is allowed to show, or `null` when there is none to
 * show. Same gate as `PlayerHudCard`'s player colour, for
 * the same two reasons:
 * - `available: false` — this build has no automatic classifier and the player
 *   has no manual override, so the backend's label is the build-level sentence
 *   "Classification unavailable in this build"; repeating it on thousands of
 *   rows says nothing about any player. Settings and the profile drawer say it
 *   once, where it belongs.
 * - a genuine `unknown` — available, but below a rule's own hand minimum or no
 *   rule matched. A grey "Unknown" chip is a badge pretending to be a result;
 *   the hands column already shows the sample, and the drawer explains it.
 * A manual override always arrives with `isOverride` and always shows.
 */
function shownClassification(
  classification: ClassificationResult | undefined,
): ClassificationResult | null {
  if (!classification || !classification.available) return null;
  if (!classification.isOverride && classification.classification === "unknown") return null;
  return classification;
}

interface PlayersViewProps {
  onSelectPlayer: (player: Player) => void;
}

// A bulk import can push `players` into the thousands (see
// `getPlayersPage` doc comment); `get_players_page` only computes the
// expensive per-player stats for one page's worth, so this is a request-size
// knob, not a client-side truncation of already-fetched data.
const PAGE_SIZE = 100;

type LoadState =
  | { status: "loading" }
  | { status: "ready"; players: Player[]; total: number; search: string }
  | { status: "unavailable"; message: string }
  | { status: "error"; message: string };

export function PlayersView({ onSelectPlayer }: PlayersViewProps) {
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [searchInput, setSearchInput] = useState("");

  function load(search: string) {
    getPlayersPage(0, PAGE_SIZE, search || undefined)
      .then(({ players, total }) => {
        setState({ status: "ready", players, total, search });
      })
      .catch((err: unknown) => {
        if (err instanceof DesktopAppRequiredError) {
          setState({ status: "unavailable", message: err.message });
        } else {
          setState({ status: "error", message: String(err) });
        }
      });
  }

  function loadMore() {
    if (state.status !== "ready") return;
    const { players, search } = state;
    getPlayersPage(players.length, PAGE_SIZE, search || undefined)
      .then(({ players: nextPlayers, total }) => {
        setState({ status: "ready", players: [...players, ...nextPlayers], total, search });
      })
      .catch(() => undefined);
  }

  useEffect(() => {
    load("");
    const unlisten = onHandsImported(() => load(searchInput)).catch(() => undefined);

    return () => {
      unlisten.then((fn) => fn?.());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function handleSearchChange(value: string) {
    setSearchInput(value);
    load(value);
  }

  const remaining = state.status === "ready" ? state.total - state.players.length : 0;
  const canLoadMore = remaining > 0;

  return (
    <div>
      <div className="view-header">
        <h1>Players</h1>
        <p>
          {state.status === "ready"
            ? `${state.total.toLocaleString()} tracked player${state.total === 1 ? "" : "s"} from imported hand histories${
                state.total > state.players.length
                  ? ` (showing ${state.players.length.toLocaleString()})`
                  : ""
              }.`
            : "Tracked players from imported hand histories."}
        </p>
      </div>

      {state.status !== "unavailable" && state.status !== "error" && (
        <input
          type="text"
          className={styles.searchInput}
          placeholder="Search players by name…"
          value={searchInput}
          onChange={(e) => handleSearchChange(e.target.value)}
        />
      )}

      {state.status === "loading" && <div className={styles.stateBox}>Loading players&hellip;</div>}

      {state.status === "unavailable" && <div className={styles.stateBox}>{state.message}</div>}

      {state.status === "error" && (
        <div className={styles.stateBox}>Failed to load players: {state.message}</div>
      )}

      {state.status === "ready" && state.total === 0 && state.search === "" && (
        <div className={styles.stateBox}>
          No hands imported yet. Configure your PokerStars hand history folder in Settings to
          start tracking players.
        </div>
      )}

      {state.status === "ready" && state.total === 0 && state.search !== "" && (
        <div className={styles.stateBox}>No tracked player matches &ldquo;{state.search}&rdquo;.</div>
      )}

      {state.status === "ready" && state.players.length > 0 && (
        <div className={styles.list}>
          <div className={styles.headerRow}>
            <span>Player</span>
            <span>Hands</span>
            <span>VPIP</span>
            <span>PFR</span>
            <span>3-Bet</span>
            <span>Notes</span>
          </div>
          {state.players.map((player) => {
            const shown = shownClassification(player.classification);
            return (
            <button
              key={player.id}
              type="button"
              className={styles.row}
              onClick={() => onSelectPlayer(player)}
            >
              <span className={styles.playerCell}>
                <span
                  className={styles.avatar}
                  style={shown ? { boxShadow: `0 0 0 2px ${shown.color}` } : undefined}
                >
                  {player.name.slice(0, 2).toUpperCase()}
                </span>
                <span className={styles.playerIdentity}>
                  <span className={styles.name} title={player.name}>
                    {player.name}
                  </span>
                  {shown && (
                    <span className={styles.classificationChip} style={{ color: shown.color }}>
                      {shown.label}
                    </span>
                  )}
                </span>
              </span>
              <span className={`${styles.statValue} tabular`}>
                {player.hands.toLocaleString()}
              </span>
              <span className={`${styles.statValue} tabular`}>{formatPct(player.stats.vpip)}</span>
              <span className={`${styles.statValue} tabular`}>{formatPct(player.stats.pfr)}</span>
              <span className={`${styles.statValue} tabular`}>
                {formatPct(player.stats.threeBet)}
              </span>
              <span className={styles.note}>{player.note ?? ""}</span>
            </button>
            );
          })}
        </div>
      )}

      {canLoadMore && (
        <button type="button" className={styles.loadMoreButton} onClick={loadMore}>
          Load more ({remaining.toLocaleString()} remaining)
        </button>
      )}
    </div>
  );
}
