/**
 * Wraps a refresh so that it runs at most once at a time. A trigger that
 * arrives while a run is in flight is not dropped: all of them collapse into
 * one more run after the current one settles, so the last state is always
 * read. A `run` that returns nothing (not a promise) settles immediately; one
 * that has not settled after `staleMs` no longer holds the next run back.
 *
 * Why: the watcher emits `hands-imported` once per changed file, which during
 * a session is once per hand at every table. Every window answered each event
 * with its own database-bound refresh and kept only the newest result, so a
 * burst queued requests faster than they could be served and the newest
 * refresh waited behind all the stale ones (D107: with eight tables the HUD
 * and the side panel stopped refreshing for good).
 */
export function singleFlight(run: () => unknown, staleMs = 15_000): () => void {
  let running = false;
  let again = false;
  // Which run is current: a run that outlives `staleMs` stops holding the
  // next one back (a refresh whose call never returns must not stop the HUD
  // refreshing for good), and its late settle is then ignored. Callers keep
  // only their newest response, so the overlap is harmless.
  let generation = 0;
  const start = () => {
    const mine = ++generation;
    running = true;
    again = false;
    const done = () => {
      if (mine !== generation || !running) return;
      clearTimeout(timer);
      running = false;
      if (again) start();
    };
    const timer = setTimeout(done, staleMs);
    Promise.resolve()
      .then(run)
      .catch(() => undefined)
      .finally(done);
  };
  return () => {
    if (running) again = true;
    else start();
  };
}
