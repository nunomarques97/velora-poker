// Unit tests for mergeDrawerUpdate (a drawer edit applied to the overlay's
// roster). Run with `npm run test:layout`.
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import type { Player } from "../data/types.js";
import { mergeDrawerUpdate } from "./rosterUpdate.js";

const stats = {
  vpip: 23, pfr: 19, threeBet: 8, foldToThreeBet: 55, cBet: 62, foldToCBet: 41,
  aggressionFactor: 2.4, wtsd: 27, wsd: 51,
} as unknown as Player["stats"];

describe("mergeDrawerUpdate", () => {
  it("keeps the roster's empty reads on the hero when a note edit returns the full payload", () => {
    // The roster drops the hero's reads; set_player_note answers with the
    // Players-list payload, which has them.
    const roster: Player = { id: "7", name: "Hero", hands: 30, stats, descriptions: [], engine: null, seat: 4, seatOffset: 0 };
    const engine = { tag: { text: "3B+" } } as unknown as Player["engine"];
    const updated: Player = {
      id: "7", name: "Hero", hands: 30, stats, note: "tilts", engine,
      descriptions: [{ ruleId: "pf.3bet.high_ip" }] as unknown as Player["descriptions"], seat: null, seatOffset: null,
    };
    const merged = mergeDrawerUpdate(roster, updated);
    assert.equal(merged.engine, null);
    assert.deepEqual(merged.descriptions, []);
    assert.equal(merged.note, "tilts");
    assert.equal(merged.seat, 4);
    assert.equal(merged.seatOffset, 0);
  });

  it("keeps a villain's table-context reads and seat, and takes the edit", () => {
    const tableEngine = { tag: { text: "ICM-" } } as unknown as Player["engine"];
    const roster: Player = { id: "9", name: "Vil", hands: 80, stats, engine: tableEngine, descriptions: [], seat: 2, seatOffset: 5 };
    const updated: Player = {
      id: "9", name: "Vil", hands: 80, stats, engine: { tag: { text: "LAG" } } as unknown as Player["engine"],
      classification: { classification: "maniac", label: "Maniac", color: "#e0524f", isOverride: true, available: true },
      seat: null, seatOffset: null,
    };
    const merged = mergeDrawerUpdate(roster, updated);
    assert.equal(merged.engine, tableEngine);
    assert.equal(merged.classification?.isOverride, true);
    assert.equal(merged.seat, 2);
    assert.equal(merged.seatOffset, 5);
  });
});
