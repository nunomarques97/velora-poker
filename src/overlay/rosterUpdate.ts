import type { Player } from "../data/types.js";

/**
 * A drawer edit (colour override, note) applied to the overlay's roster.
 * The edit commands answer with the Players-list payload: no seat, and reads
 * without this table's context, including reads about the hero, which the
 * overlay roster deliberately drops (mock-session report, item 3). An edit
 * changes neither, so the roster keeps its own seat and reads until the next
 * refresh; everything else comes from the edit.
 */
export function mergeDrawerUpdate(roster: Player, updated: Player): Player {
  return {
    ...updated,
    engine: roster.engine,
    descriptions: roster.descriptions,
    seat: roster.seat,
    seatOffset: roster.seatOffset,
  };
}
