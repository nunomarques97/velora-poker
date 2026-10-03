// Tournament formats of the generator: MTT 9-max progressive knockouts and
// hyper Spin & Go. Every hand is re-read with helpers/hand-check (chips,
// antes, bounties, eliminations), and the text is held to the shape of the
// real fixtures in src-tauri/tests/fixtures. Run with `npm run test:sim`.

import assert from "node:assert/strict";
import { mkdtempSync, readdirSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { roman, tournamentHeader } from "../lib/format.mjs";
import { loadProfiles, runSession } from "../lib/session.mjs";
import { checkHand, splitHands } from "./helpers/hand-check.mjs";

const profiles = loadProfiles();
const hero = profiles.hero.name;
const scratch = (label) => mkdtempSync(join(tmpdir(), `velora-sim-${label}-`));
const fixture = (name) => readFileSync(new URL(`../../../src-tauri/tests/fixtures/${name}`, import.meta.url), "utf8");
const BOM = [0xef, 0xbb, 0xbf];
const shortStacks = profiles.profiles.find((p) => p.id === "short").players;

/** The parser's tournament header and seat line (src-tauri/src/parser/pokerstars.rs). */
const PARSER_HEADER =
  /^PokerStars Hand #(\d+): (?:Zoom )?Tournament #(\d+), (.+?) - Level (\S+)\s*\(([\d,]+)\/([\d,]+)\) - (\d{4}\/\d{2}\/\d{2}) (\d{1,2}:\d{2}:\d{2})/;
const PARSER_SEAT = /^Seat (\d+): (.+) \([$€£]?([\d.]+) in chips(.*)$/;
const PARSER_BOUNTY = /^,\s*[$€£]?(\d{1,3}(?:,\d{3})+|\d+)(\.\d+)? bounty\)/;

/** Level, blinds and ante of a tournament hand, from its own text. */
function levelOf(block) {
  const m = /- Level ([IVXLC]+) \((\d+)\/(\d+)\) -/.exec(block);
  assert.ok(m, "a tournament header with its level");
  // The level's ante is the largest posted (a short stack antes all-in for less).
  const antes = [...block.matchAll(/^.+: posts the ante (\d+)/gm)].map((a) => Number(a[1]));
  return { level: m[1], sb: Number(m[2]), bb: Number(m[3]), ante: Math.max(0, ...antes) };
}

/** Reads every tournament file of a session back and checks every hand. */
function readSession(out, manifest) {
  const files = [];
  for (const table of manifest.tables) {
    const bytes = readFileSync(join(out, table.file));
    assert.deepEqual([...bytes.subarray(0, 3)], BOM, `${table.file} starts with a BOM`);
    assert.equal(bytes.indexOf(Buffer.from(BOM), 3), -1, `${table.file}: appended hands carry no second BOM`);
    const blocks = splitHands(bytes.toString("utf8"));
    assert.equal(blocks.length, table.hands, `${table.file}: hand count`);
    const hands = blocks.map((block) => {
      const level = levelOf(block);
      const hand = checkHand(block, { hero, bb: level.bb, rakeCapBb: 0, chips: true, ante: level.ante });
      return { block, level, ...hand };
    });
    files.push({ table, hands });
  }
  return files;
}

const mtt = (seed, extra = {}) => {
  const out = scratch(`mtt-${seed}`);
  return runSession({ format: "mtt", seed, out, tables: 3, hands: 300, pace: 0, ...extra }).then((manifest) => ({ out, manifest }));
};
const spin = (seed, extra = {}) => {
  const out = scratch(`spin-${seed}`);
  return runSession({ format: "spin", seed, out, tables: 3, hands: 200, pace: 0, ...extra }).then((manifest) => ({ out, manifest }));
};

test("headers, seat lines and bounties have the real fixtures' shape", () => {
  const header = tournamentHeader({
    handId: 262000000001,
    tournamentId: 4100000001,
    buyIn: [500, 500, 100],
    level: 6,
    sb: 75,
    bb: 150,
    date: new Date(2026, 8, 12, 0, 6, 23),
  });
  assert.equal(
    header,
    "PokerStars Hand #262000000001: Tournament #4100000001, $5.00+$5.00+$1.00 USD Hold'em No Limit - Level VI (75/150) - 2026/09/12 0:06:23 WET [2026/09/11 19:06:23 ET]",
  );
  const real = fixture("real_bounty_tournament.txt").replace(/^﻿/, "").split(/\r?\n/)[0];
  const realSpin = fixture("spin_three_max.txt").split(/\r?\n/)[0];
  for (const line of [header, real, realSpin]) assert.match(line, PARSER_HEADER);
  // Same layout as the real knockout: a three-part buy-in, then the game.
  assert.equal(PARSER_HEADER.exec(header)[3], "$5.00+$5.00+$1.00 USD Hold'em No Limit");
  assert.equal(PARSER_HEADER.exec(real)[3], "€13.50+€13.50+€3.00 EUR Hold'em No Limit");
  assert.deepEqual([1, 4, 9, 14, 17, 20].map(roman), ["I", "IV", "IX", "XIV", "XVII", "XX"]);
});

test("mtt: one file per tournament, named like the client, 9-max with antes and bounties on every seat", async () => {
  const { out, manifest } = await mtt("mtt-shape");
  assert.equal(manifest.format, "mtt");
  assert.deepEqual(readdirSync(out).sort(), [...manifest.files].sort());
  assert.equal(manifest.files.length, manifest.tournaments.length);
  assert.ok(manifest.tournaments.length > 3, "the hero busts and starts new tournaments");
  for (const { table, hands } of readSession(out, manifest)) {
    assert.match(table.file, new RegExp(`^HH20260912 T${table.tournamentId} No Limit Hold'em \\$5\\.00 \\+ \\$5\\.00 \\+ \\$1\\.00\\.txt$`));
    assert.match(table.name, new RegExp(`^${table.tournamentId} \\d+$`));
    let previousLevel = 0;
    for (const hand of hands) {
      const lines = hand.block.split("\r\n");
      assert.match(lines[0], PARSER_HEADER);
      assert.equal(PARSER_HEADER.exec(lines[0])[2], table.tournamentId);
      assert.equal(lines[1].replace(/ Seat #\d is the button$/, ""), `Table '${table.name}' 9-max`);
      // Levels only go up within a tournament.
      const number = ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X", "XI", "XII", "XIII", "XIV", "XV", "XVI", "XVII", "XVIII", "XIX", "XX"].indexOf(hand.level.level) + 1;
      assert.ok(number >= previousLevel, "the level never goes down");
      previousLevel = number;
      assert.ok(hand.level.ante > 0 && hand.level.ante < hand.level.bb / 4, "an ante at every level");
      // Every dealt-in player antes, before the blinds; every seat line carries a bounty.
      const antes = lines.filter((l) => / posts the ante \d+/.test(l)).length;
      assert.equal(antes, hand.seats.length);
      assert.ok(lines.findIndex((l) => / posts the ante /.test(l)) < lines.findIndex((l) => / posts (small|big) blind /.test(l)) || !lines.some((l) => / posts (small|big) blind /.test(l)));
      for (const line of lines.filter((l) => /^Seat \d: .+ in chips/.test(l) && !/ \((button|small blind|big blind)\)| folded | showed | collected /.test(l))) {
        const seat = PARSER_SEAT.exec(line);
        assert.ok(seat, line);
        const bounty = PARSER_BOUNTY.exec(seat[4]);
        assert.ok(bounty, `${line}: a bounty the parser reads`);
      }
      assert.ok(hand.seats.length >= 2 && hand.seats.length <= 9);
      assert.ok(hand.seats.every((s) => s.bounty >= 500), "a bounty starts at $5.00 and only grows");
      assert.equal(hand.rake, 0);
    }
  }
});

test("mtt: villains bust, are paid for, leave the table and are replaced; bounties grow", async () => {
  const { out, manifest } = await mtt("mtt-busts");
  let eliminations = 0;
  let replaced = 0;
  let heroBusts = 0;
  for (const { table, hands } of readSession(out, manifest)) {
    for (let i = 0; i < hands.length; i++) {
      const hand = hands[i];
      eliminations += hand.eliminations.length;
      // checkHand has already tied each elimination to a busted player, a
      // pot winner and a half/half split onto the winner's bounty.
      const next = hands[i + 1];
      if (!next) {
        if (hand.busted.includes(hero)) heroBusts++;
        continue;
      }
      assert.ok(!hand.busted.includes(hero), `${table.file}: the file stops when the hero busts`);
      const names = new Set(next.seats.map((s) => s.name));
      for (const name of hand.busted) assert.ok(!names.has(name), `${name} busted but is dealt the next hand`);
      // Survivors carry their chips and their bounty into the next hand.
      const won = new Map();
      for (const m of hand.block.matchAll(/^(.+) collected (\d+) from (?:main |side )?pot(?:-\d+)?$/gm)) {
        won.set(m[1], (won.get(m[1]) || 0) + Number(m[2]));
      }
      const grown = new Map(hand.seats.map((s) => [s.name, s.bounty]));
      for (const e of hand.eliminations) grown.set(e.by, e.to);
      for (const seat of next.seats) {
        const before = hand.seats.find((s) => s.name === seat.name);
        if (!before) {
          replaced++;
          continue;
        }
        assert.equal(seat.stack, before.stack - hand.put.get(seat.name) + (won.get(seat.name) || 0), `${seat.name}'s chips carry over`);
        assert.equal(seat.bounty, grown.get(seat.name), `${seat.name}'s bounty carries over`);
      }
    }
  }
  assert.ok(eliminations > 30, `knockouts are paid (${eliminations})`);
  assert.ok(replaced > 30, `busted seats are refilled (${replaced})`);
  assert.ok(heroBusts > 2, `the hero busts and his file ends (${heroBusts})`);
  const finishes = manifest.tournaments.map((t) => t.heroFinish);
  assert.ok(finishes.every((f) => f === "busted" || f === "running"));

  // The check itself catches a knockout paid wrong or not at all.
  const knockout = readSession(out, manifest)
    .flatMap((f) => f.hands)
    .find((h) => h.eliminations.length === 1);
  const { bb, ante } = knockout.level;
  const options = { hero, bb, rakeCapBb: 0, chips: true, ante };
  const line = /^.+ wins \$[\d.]+ for eliminating .+$/m;
  const tampered = (to) => knockout.block.replace(line, to);
  const e = knockout.eliminations[0];
  assert.throws(() => checkHand(tampered(""), options), /1 busted but 0 elimination/);
  assert.throws(
    () => checkHand(tampered(`${e.by} wins $9.99 for eliminating ${e.player} and their own bounty increases by $2.50 to $7.50`), options),
    /does not add up|wrong amount/,
  );
});

test("mtt: the short-stack profile sits down with a push/fold stack (15bb or less) and shoves", async () => {
  const { out, manifest } = await mtt("mtt-short");
  let hands = 0;
  let pushFold = 0;
  let arrivals = 0;
  let shoves = 0;
  for (const { hands: list } of readSession(out, manifest)) {
    for (const [i, hand] of list.entries()) {
      const me = hand.seats.find((s) => s.name === hero);
      for (const seat of hand.seats.filter((s) => shortStacks.includes(s.name))) {
        hands++;
        if (Math.min(seat.stack, me.stack) / hand.level.bb <= 15) pushFold++;
        if (new RegExp(`^${seat.name}: raises \\d+ to \\d+ and is all-in$`, "m").test(hand.block)) shoves++;
        // Sitting down (the file's first hand, or not seated the hand before): always short.
        if (i === 0 || !list[i - 1].seats.some((s) => s.name === seat.name)) {
          arrivals++;
          assert.ok(seat.stack / hand.level.bb <= 15, `${seat.name} sits down with ${seat.stack / hand.level.bb}bb`);
        }
      }
    }
  }
  assert.ok(hands > 200 && arrivals > 5, `short stacks are seated (${hands} hands, ${arrivals} arrivals)`);
  // Shoving wide, they double up or bust: a double takes them deeper for
  // a while (0.35 to 0.65 of their hands stay at push/fold depth over ten
  // seeds of this size).
  assert.ok(pushFold / hands > 0.3, `push/fold depth in ${pushFold} of ${hands} hands`);
  assert.ok(shoves > 30, `and they shove (${shoves})`);
});

test("spin: 3-max hyper with 500 chips, no antes or bounties, played until one player is left", async () => {
  const { out, manifest } = await spin("spin-shape");
  assert.equal(manifest.format, "spin");
  assert.ok(manifest.tournaments.length > 6);
  const totalBuyIn = profiles.stakes.spin.buyIn.reduce((a, b) => a + b, 0);
  let headsUp = 0;
  for (const { table, hands } of readSession(out, manifest)) {
    const record = manifest.tournaments.find((t) => t.id === table.tournamentId);
    assert.match(table.file, new RegExp(`^HH20260912 T${table.tournamentId} No Limit Hold'em \\$4\\.80 \\+ \\$0\\.20\\.txt$`));
    assert.equal(table.name, `${table.tournamentId} 1`);
    assert.equal(hands[0].seats.length, 3);
    assert.ok(hands[0].seats.every((s) => s.stack === 500), "everyone starts with 500 chips");
    assert.equal(hands[0].level.level, "I");
    const chips = hands[0].seats.reduce((sum, s) => sum + s.stack, 0);
    for (const hand of hands) {
      const lines = hand.block.split("\r\n");
      assert.match(lines[0], new RegExp(`^PokerStars Hand #\\d+: Tournament #${table.tournamentId}, \\$4\\.80\\+\\$0\\.20 USD Hold'em No Limit - Level [IVX]+ \\(\\d+/\\d+\\) - `));
      assert.equal(lines[1].replace(/ Seat #\d is the button$/, ""), `Table '${table.name}' 3-max`);
      assert.equal(hand.level.ante, 0, "no antes in a Spin");
      assert.ok(hand.seats.every((s) => s.bounty === null), "no bounty in a Spin");
      // Chips never leave a Spin: the stacks of the players still in add up.
      assert.equal(hand.seats.reduce((sum, s) => sum + s.stack, 0) <= chips, true);
      if (hand.seats.length === 2) headsUp++;
      for (const f of hand.finishes) {
        if (f.place === 1) assert.equal(f.prize, record.multiplier * totalBuyIn);
      }
    }
    const last = hands[hands.length - 1];
    if (record.heroFinish === "won") {
      const places = last.finishes.map((f) => f.place);
      assert.equal(new Set(places).size, places.length, "one player per place");
      assert.ok(places.includes(2));
      assert.equal(last.finishes.find((f) => f.place === 1).player, hero);
    } else if (record.heroFinish === "busted") {
      assert.ok(last.busted.includes(hero));
      assert.ok(last.finishes.some((f) => f.player === hero && f.place >= 2));
    }
    // Blinds rise fast: a hyper level lasts three simulated minutes.
    if (hands.length > 30) assert.ok(hands[hands.length - 1].level.level !== "I", `${table.file}: the blinds go up`);
  }
  assert.ok(headsUp > 50, `the last two play heads-up (${headsUp})`);
  const finishes = new Set(manifest.tournaments.map((t) => t.heroFinish));
  assert.ok(finishes.has("won") && finishes.has("busted"));
});

test("tournaments: the same seed writes the same bytes; pace waits between hands", async () => {
  for (const format of ["mtt", "spin"]) {
    const a = scratch(`${format}-a`);
    const b = scratch(`${format}-b`);
    await runSession({ format, seed: "same", out: a, tables: 2, hands: 40, pace: 0 });
    await runSession({ format, seed: "same", out: b, tables: 2, hands: 40, pace: 0 });
    const names = readdirSync(a).sort();
    assert.deepEqual(names, readdirSync(b).sort());
    for (const name of names) assert.ok(readFileSync(join(a, name)).equals(readFileSync(join(b, name))), `${format}: ${name} differs`);

    const waits = [];
    const manifest = await runSession({ format, seed: "paced", out: scratch(`${format}-pace`), tables: 2, hands: 10, pace: 1, sleep: async (ms) => waits.push(ms) });
    assert.equal(waits.length, manifest.hands - 1);
    assert.ok(waits.every((ms) => ms >= 0));
  }
});
