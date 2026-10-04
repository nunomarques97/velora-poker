// Generator unit tests: determinism, file naming, appending, pacing, the
// command line, and that every generated hand adds up (helpers/hand-check).
// Run with `npm run test:sim`.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, readdirSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { parseArgs } from "../generate.mjs";
import { evaluate } from "../lib/cards.mjs";
import { loadProfiles, runSession } from "../lib/session.mjs";
import { cents, checkHand, splitHands } from "./helpers/hand-check.mjs";

const profiles = loadProfiles();
const scratch = (label) => mkdtempSync(join(tmpdir(), `velora-sim-${label}-`));
const BOM = [0xef, 0xbb, 0xbf];

function readAll(dir) {
  return Object.fromEntries(readdirSync(dir).map((name) => [name, readFileSync(join(dir, name))]));
}

test("the same seed writes byte-identical files; another seed does not", async () => {
  const a = scratch("a");
  const b = scratch("b");
  const c = scratch("c");
  const options = { format: "cash", tables: 3, hands: 25, pace: 0 };
  await runSession({ ...options, seed: "same", out: a });
  await runSession({ ...options, seed: "same", out: b });
  await runSession({ ...options, seed: "other", out: c });
  const [fa, fb, fc] = [readAll(a), readAll(b), readAll(c)];
  assert.deepEqual(Object.keys(fa).sort(), Object.keys(fb).sort());
  for (const name of Object.keys(fa)) assert.ok(fa[name].equals(fb[name]), `${name} differs`);
  const joined = (files) => Buffer.concat(Object.values(files)).toString("utf8");
  assert.notEqual(joined(fa), joined(fc));
});

test("cash: one file per table, named like the client, BOM once, hands appended", async () => {
  const out = scratch("cash");
  const manifest = await runSession({ format: "cash", seed: "names", out, tables: 4, hands: 12, pace: 0 });
  const files = readdirSync(out).sort();
  assert.equal(files.length, 4);
  assert.deepEqual(files, [...manifest.files].sort());
  const names = new Set();
  for (const table of manifest.tables) {
    assert.match(table.file, /^HH20260912 [A-Z][a-z]+ [IVX]+ - \$0\.25-\$0\.50 - USD No Limit Hold'em\.txt$/);
    assert.ok(table.file.includes(` ${table.name} - `), `${table.file} carries its table name`);
    names.add(table.name);
    const bytes = readFileSync(join(out, table.file));
    assert.deepEqual([...bytes.subarray(0, 3)], BOM, "a new file starts with a BOM");
    assert.equal(bytes.indexOf(Buffer.from(BOM), 3), -1, "appended hands carry no second BOM");
    const text = bytes.toString("utf8");
    const hands = splitHands(text);
    assert.equal(hands.length, 12);
    for (const hand of hands) {
      assert.match(hand, new RegExp(`^PokerStars Hand #\\d+:  Hold'em No Limit \\(\\$0\\.25/\\$0\\.50 USD\\) - `));
      assert.match(hand, new RegExp(`^Table '${table.name}' 6-max Seat #\\d is the button$`, "m"));
    }
    assert.ok(text.endsWith("\r\n\r\n\r\n"), "hands end with the client's blank lines");
  }
  assert.equal(names.size, 4, "table names are unique");
});

test("zoom: one pool file, Zoom headers without a currency code, a new table every hand", async () => {
  const out = scratch("zoom");
  const manifest = await runSession({ format: "zoom", seed: "zoom", out, hands: 30, pace: 0 });
  assert.equal(manifest.files.length, 1);
  assert.match(manifest.files[0], /^HH20260912 [A-Z][a-z]+ - \$0\.05-\$0\.10 - USD No Limit Hold'em\.txt$/);
  const hands = splitHands(readFileSync(join(out, manifest.files[0]), "utf8"));
  assert.equal(hands.length, 30);
  for (const hand of hands) assert.match(hand, /^PokerStars Zoom Hand #\d+:  Hold'em No Limit \(\$0\.05\/\$0\.10\) - /);
  const lineups = new Set(hands.map((h) => [...h.matchAll(/^Seat \d: (\S+) \(/gm)].map((m) => m[1]).join(",")));
  assert.ok(lineups.size > 20, "Zoom reseats the hero with new opponents");
});

test("every generated hand is internally consistent (cash and Zoom)", async () => {
  let soloSidePots = 0;
  for (const format of ["cash", "zoom"]) {
    const out = scratch(`consistency-${format}`);
    const manifest = await runSession({ format, seed: `consistency-${format}-b`, out, tables: 3, hands: 300, pace: 0 });
    const stakes = profiles.stakes[format];
    const ids = new Set();
    let showdowns = 0;
    let rakes = 0;
    for (const file of manifest.files) {
      const hands = splitHands(readFileSync(join(out, file), "utf8"));
      let lastId = 0;
      for (const block of hands) {
        const hand = checkHand(block, { hero: profiles.hero.name, bb: stakes.bb, rakeCapBb: stakes.rakeCapBb });
        assert.ok(!ids.has(hand.id), `hand id ${hand.id} repeats`);
        ids.add(hand.id);
        assert.ok(Number(hand.id) > lastId, "hand ids grow within a file");
        lastId = Number(hand.id);
        assert.equal(hand.seats.length, stakes.maxSeats);
        if (hand.shown > 1) showdowns++;
        if (hand.rake > 0) rakes++;
        soloSidePots += hand.soloSidePots;
      }
    }
    assert.equal(ids.size, manifest.hands);
    assert.ok(showdowns > 20, `${format}: showdowns with real hand ranks (${showdowns})`);
    assert.ok(rakes > 50, `${format}: raked pots (${rakes})`);
  }
  // A side pot only one live player covered (folded chips above a short
  // all-in) occurs in these seeds, so the check above has exercised it. It
  // is rare (about three seeds in five at this size); the seeds moved to
  // `-b` when the read profiles' river raises fell to .02 (spec section 18).
  assert.ok(soloSidePots > 0, "a side pot with a single eligible player was generated and checked");
});

test("a side pot only one player covered is not paid to the short all-in", () => {
  // X is all-in for $5 preflop, A and the hero call; A bets the flop and the
  // hero calls; A bets the turn and the hero folds. The $10 the hero and A
  // put in above X's $5 is a side pot only A can win.
  const hero = profiles.hero.name;
  const { bb, rakeCapBb } = profiles.stakes.cash;
  const board = ["2h", "5c", "9s", "Td", "3c"];
  const rank = (cards) => evaluate([...cards, ...board]).text;
  const hand = (payout) =>
    [
      "PokerStars Hand #900000000001:  Hold'em No Limit ($0.25/$0.50 USD) - 2026/09/12 20:00:00 ET",
      "Table 'Tamper' 6-max Seat #1 is the button",
      "Seat 1: ShortX ($5 in chips)",
      "Seat 2: BettorA ($50 in chips)",
      `Seat 3: ${hero} ($50 in chips)`,
      "BettorA: posts small blind $0.25",
      `${hero}: posts big blind $0.50`,
      "*** HOLE CARDS ***",
      `Dealt to ${hero} [7s 2c]`,
      "ShortX: raises $4.50 to $5 and is all-in",
      "BettorA: calls $4.75",
      `${hero}: calls $4.50`,
      "*** FLOP *** [2h 5c 9s]",
      "BettorA: bets $5",
      `${hero}: calls $5`,
      "*** TURN *** [2h 5c 9s] [Td]",
      "BettorA: bets $10",
      `${hero}: folds`,
      "Uncalled bet ($10) returned to BettorA",
      "*** RIVER *** [2h 5c 9s Td] [3c]",
      "*** SHOW DOWN ***",
      `BettorA: shows [Kc Kd] (${rank(["Kc", "Kd"])})`,
      `ShortX: shows [Ah Ad] (${rank(["Ah", "Ad"])})`,
      ...payout,
      "*** SUMMARY ***",
    ].join("\r\n");
  const options = { hero, bb, rakeCapBb };
  const split = checkHand(
    hand([
      "BettorA collected $10 from side pot",
      "ShortX collected $13.75 from main pot",
      "Total pot $25 Main pot $13.75. Side pot $10. | Rake $1.25",
    ]),
    options,
  );
  assert.equal(split.pots, 2);
  assert.equal(split.soloSidePots, 1);
  // The old merged payout: the $5 all-in collects the whole $25 pot.
  assert.throws(
    () => checkHand(hand(["ShortX collected $23.75 from pot", "Total pot $25 | Rake $1.25"]), options),
    /ShortX collects 2375 but could win at most 1500/,
  );
  // Paying the side pot to the player who did not cover it.
  assert.throws(
    () =>
      checkHand(
        hand([
          "ShortX collected $10 from side pot",
          "ShortX collected $13.75 from main pot",
          "Total pot $25 Main pot $13.75. Side pot $10. | Rake $1.25",
        ]),
        options,
      ),
    /could win at most|without covering it/,
  );
});

test("the consistency check itself catches a broken pot, rake, hand rank or blind", async () => {
  const out = scratch("tamper");
  const manifest = await runSession({ format: "cash", seed: "tamper", out, tables: 1, hands: 120, pace: 0 });
  const { bb, rakeCapBb } = profiles.stakes.cash;
  const options = { hero: profiles.hero.name, bb, rakeCapBb };
  const hands = splitHands(readFileSync(join(out, manifest.files[0]), "utf8"));
  const raked = hands.find((h) => /\| Rake \$0\.\d\d$/m.test(h) && / collected \$/.test(h));
  const shown = hands.find((h) => /: shows \[\w\w \w\w\] \(/.test(h));
  assert.ok(raked && shown, "the sample has a raked pot and a showdown");
  checkHand(raked, options);
  checkHand(shown, options);
  const tamper = (block, from, to) => {
    const changed = block.replace(from, to);
    assert.notEqual(changed, block, `${from} not found`);
    return changed;
  };
  assert.throws(() => checkHand(tamper(raked, /Total pot \$([\d.]+)/, (_, v) => `Total pot $${(Number(v) + 1).toFixed(2)}`), options), /total pot/);
  assert.throws(() => checkHand(tamper(raked, /\| Rake \$0\.(\d\d)$/m, "| Rake $9.$1"), options), /collected|rake/);
  assert.throws(() => checkHand(tamper(shown, /(: shows \[\w\w \w\w\] \()[^)]+\)/, "$1a royal flush)"), options), /holds/);
  assert.throws(() => checkHand(tamper(raked, /^(.+): posts small blind/m, "Nobody: posts small blind"), options), /small blind|unknown/);
  assert.throws(() => checkHand(tamper(raked, /^Dealt to \S+/m, "Dealt to Someone"), options), /hero/);
});

test("cash stacks carry over from hand to hand at a table", async () => {
  const out = scratch("stacks");
  const manifest = await runSession({ format: "cash", seed: "stacks", out, tables: 1, hands: 150, pace: 0 });
  const { bb, rakeCapBb } = profiles.stakes.cash;
  const hands = splitHands(readFileSync(join(out, manifest.files[0]), "utf8"));
  let previous = null;
  let carried = 0;
  for (const block of hands) {
    const hand = checkHand(block, { hero: profiles.hero.name, bb, rakeCapBb });
    const won = new Map();
    for (const m of block.matchAll(/^(.+) collected \$([\d.]+) from (?:main |side )?pot$/gm)) {
      won.set(m[1], (won.get(m[1]) || 0) + Math.round(Number(m[2]) * 100));
    }
    if (previous) {
      for (const seat of hand.seats) {
        const before = previous.get(seat.name);
        // A rebuy (or a short stack re-sitting) only ever tops up.
        if (before === seat.stack) carried++;
        else assert.ok(seat.stack > before, `${seat.name}'s stack shrank between hands`);
      }
    }
    previous = new Map(hand.seats.map((s) => [s.name, s.stack - hand.put.get(s.name) + (won.get(s.name) || 0)]));
  }
  assert.ok(carried > hands.length * 4, "most stacks carry over unchanged");
});

test("pace 0 never waits", async () => {
  const waits = [];
  await runSession({ seed: "fast", out: scratch("pace0"), tables: 2, hands: 10, pace: 0, sleep: async (ms) => waits.push(ms) });
  assert.deepEqual(waits, []);
});

test("pace spreads hands at about pace seconds per hand per table, writing each hand before waiting", async () => {
  const out = scratch("pace");
  const waits = [];
  const written = [];
  const count = () =>
    readdirSync(out).reduce((n, f) => n + splitHands(readFileSync(join(out, f), "utf8")).length, 0);
  const pace = 2;
  const hands = 20;
  const manifest = await runSession({
    seed: "paced",
    out,
    tables: 3,
    hands,
    pace,
    sleep: async (ms) => {
      waits.push(ms);
      written.push(count());
    },
  });
  // One wait after each hand but the last; the files grow hand by hand.
  assert.equal(waits.length, manifest.hands - 1);
  assert.deepEqual(written, written.map((_, i) => i + 1));
  assert.ok(waits.every((ms) => ms >= 0));
  // Simulated hands are 0.75-1.25 of the spacing apart at each table: the
  // session lasts about `hands` x `pace` seconds of real time.
  const total = waits.reduce((a, b) => a + b, 0) / 1000;
  assert.ok(total > (hands - 1) * pace * 0.75 - pace, `session too short: ${total}s`);
  assert.ok(total < hands * pace * 1.25 + pace, `session too long: ${total}s`);
});

test("live clock stamps hands with the wall clock as they are written", async () => {
  const out = scratch("live");
  let clock = new Date(2026, 9, 3, 21, 30, 0).getTime();
  const manifest = await runSession({
    seed: "live",
    out,
    tables: 1,
    hands: 3,
    pace: 1,
    clock: "live",
    now: () => clock,
    sleep: async (ms) => {
      clock += ms;
    },
  });
  const text = readFileSync(join(out, manifest.files[0]), "utf8");
  assert.match(manifest.files[0], /^HH20261003 /);
  const stamps = [...text.matchAll(/ - (2026\/10\/03 21:\d\d:\d\d) WET/g)].map((m) => m[1]);
  assert.equal(stamps.length, 3);
  assert.equal(stamps[0], "2026/10/03 21:30:00");
  assert.ok(stamps[2] > stamps[0]);
});

test("the command line: required --out, typed options, unknown flags refused", () => {
  assert.throws(() => parseArgs([]), /--out/);
  assert.throws(() => parseArgs(["--out", "x", "--tables", "two"]), /number/);
  assert.throws(() => parseArgs(["--out", "x", "--hands", "1.5"]), /number/);
  assert.throws(() => parseArgs(["--out", "x", "--bogus", "1"]), /unknown option/);
  assert.throws(() => parseArgs(["--out", "x", "--clock", "wall"]), /--clock/);
  assert.throws(() => parseArgs(["--out", "x", "--start", "not a date"]), /--start/);
  assert.throws(() => parseArgs(["--out", "x", "--late-share", "half"]), /number/);
  assert.throws(() => parseArgs(["--out", "x", "--profiles", " , "]), /at least one id/);
  const options = parseArgs(["--out", "x", "--format", "zoom", "--pace", "0.5", "--manifest"]);
  assert.deepEqual(options, { out: "x", format: "zoom", pace: 0.5, manifest: true });
  const mtt = parseArgs(["--out", "x", "--format", "mtt", "--late-share", "0.8", "--profiles", "icm, hunter"]);
  assert.deepEqual(mtt, { out: "x", format: "mtt", lateShare: 0.8, profileIds: ["icm", "hunter"] });
});

test("bad session options are refused before anything is written", async () => {
  const out = join(scratch("refuse"), "never");
  await assert.rejects(runSession({ out, format: "omaha" }), /unknown format/);
  await assert.rejects(runSession({ out, pace: -1 }), /pace/);
  await assert.rejects(runSession({ out, hands: 0 }), /hands/);
  await assert.rejects(runSession({ out, tables: 99 }), /tables/);
  await assert.rejects(runSession({ out, profileIds: ["fish", "whale"] }), /unknown profile whale/);
  await assert.rejects(runSession({ out, format: "cash", lateShare: 0.5 }), /mtt only/);
  await assert.rejects(runSession({ out, format: "mtt", lateShare: 2 }), /late share/);
  assert.equal(existsSync(out), false);
});

// ------------------------------------------------------------ behaviours of the new reads

/** The profiles that existed before the river, fit-or-fold and tournament behaviours. */
const BASELINE_PROFILES = ["fish", "nit", "tag", "lag", "short", "tilter"];
/**
 * sha256 over the sorted file names and bytes of one session per format
 * (seed `baseline-identity`, the six baseline profiles only), recorded with
 * the generator of commit 507cabf, before the new behaviours existed.
 */
const BASELINE_DIGESTS = {
  cash: ["84aad4ad287af9a7fd66c396af8fd9e53975ad86e194b78f8807bbbd379cb869", { tables: 3, hands: 150 }],
  zoom: ["6025603866d110162b46847a851305f8209a37c1f44c07175aa034001e32d792", { hands: 400 }],
  mtt: ["24a36d67a58ede4718ca972428f34c7e9af185ca731b22bd746a453b3acceb75", { tables: 2, hands: 300 }],
  spin: ["813cc7f04f0e98b8472e7d47c4052e7f2d87a0bcbbc5352c519a255b83203c38", { tables: 2, hands: 300 }],
};

function digest(dir) {
  const hash = createHash("sha256");
  for (const name of readdirSync(dir).sort()) {
    hash.update(name);
    hash.update(readFileSync(join(dir, name)));
  }
  return hash.digest("hex");
}

test("the new behaviours default off: the existing profiles write byte-identical hands for their seed", async () => {
  const baseline = { ...profiles, profiles: profiles.profiles.filter((p) => BASELINE_PROFILES.includes(p.id)) };
  assert.equal(baseline.profiles.length, BASELINE_PROFILES.length);
  for (const [format, [expected, options]] of Object.entries(BASELINE_DIGESTS)) {
    const out = scratch(`identity-${format}`);
    await runSession({ format, seed: "baseline-identity", out, pace: 0, profiles: baseline, ...options });
    assert.equal(digest(out), expected, `${format}: the baseline profiles' hands changed`);
  }
  // `--profiles` (the Rust convergence tests' baseline corpus) seats the
  // same villains in the same order, so it writes the same bytes.
  const viaIds = scratch("identity-ids");
  await runSession({ format: "cash", seed: "baseline-identity", out: viaIds, pace: 0, profileIds: BASELINE_PROFILES, ...BASELINE_DIGESTS.cash[1] });
  assert.equal(digest(viaIds), BASELINE_DIGESTS.cash[0], "profileIds: the baseline profiles' hands changed");
  // None of them sets a new behaviour, so the defaults are what kept them.
  const NEW_KEYS = ["riverSizing", "foldToRiverRaise", "fitOrFold", "callShoveCovering"];
  for (const p of baseline.profiles) {
    const behaviours = [p.behaviour, p.tournament?.behaviour, p.tilt?.behaviour].filter(Boolean);
    for (const key of NEW_KEYS) assert.ok(behaviours.every((b) => !(key in b)), `${p.id} sets ${key}`);
    assert.equal(p.tournament?.late, undefined, `${p.id} sets tournament.late`);
  }
});

/**
 * Street by street, one cash hand: the pot when the street was dealt and
 * its actions in order (`{ name, verb, amount, allIn }`, amounts in cents;
 * a raise's amount is its total).
 */
function streetsOf(block) {
  const streets = [{ pot: 0, actions: [] }];
  let pot = 0;
  let put = new Map();
  for (const line of block.split(/\r?\n/)) {
    const marker = /^\*\*\* (FLOP|TURN|RIVER|SHOW DOWN|SUMMARY) \*\*\*/.exec(line);
    if (marker) {
      if (marker[1] === "SHOW DOWN" || marker[1] === "SUMMARY") break;
      put = new Map();
      streets.push({ pot, actions: [] });
      continue;
    }
    const post = /^(.+): posts (?:small blind|big blind) (\S+)/.exec(line);
    if (post) {
      pot += cents(post[2]);
      put.set(post[1], (put.get(post[1]) || 0) + cents(post[2]));
      continue;
    }
    const back = /^Uncalled bet \((\S+)\) returned to (.+)$/.exec(line);
    if (back) {
      pot -= cents(back[1]);
      continue;
    }
    const m = /^(.+): (folds|checks|calls|bets|raises)(?: (\S+))?(?: to (\S+))?( and is all-in)?$/.exec(line);
    if (!m) continue;
    const [, name, verb, first, to, allIn] = m;
    const amount = verb === "raises" ? cents(to) : verb === "calls" || verb === "bets" ? cents(first) : 0;
    const added = verb === "raises" ? amount - (put.get(name) || 0) : amount;
    put.set(name, (put.get(name) || 0) + added);
    pot += added;
    streets[streets.length - 1].actions.push({ name, verb, amount, allIn: Boolean(allIn) });
  }
  return streets;
}

/**
 * One cash session shared by the behaviour tests below: the new profiles
 * with the fish and the LAG as callers and bettors, so the rare river
 * spots come up often enough to count.
 */
const BEHAVIOUR_ROSTER = ["overbettor", "blocker", "fitfold", "icm", "fish", "lag"];
let behaviourSession;
function behaviourHands() {
  behaviourSession ??= (async () => {
    const out = scratch("behaviours");
    const roster = { ...profiles, profiles: profiles.profiles.filter((p) => BEHAVIOUR_ROSTER.includes(p.id)) };
    const manifest = await runSession({ format: "cash", seed: "behaviours", out, tables: 6, hands: 1500, pace: 0, profiles: roster });
    return manifest.files.flatMap((file) => splitHands(readFileSync(join(out, file), "utf8")));
  })();
  return behaviourSession;
}
const playersOf = (id) => profiles.profiles.find((p) => p.id === id).players;

/**
 * The river bets of a profile's players (the first bet of a river is
 * always the bettor's first unopened river action): their size shares in
 * the engine's buckets (section 9: small under 40% of the pot, an overbet
 * from 110%, an all-in whatever its size), and how they answered a raise.
 */
async function riverBets(id) {
  const names = playersOf(id);
  const bets = [];
  const raised = { n: 0, folds: 0 };
  for (const block of await behaviourHands()) {
    const river = streetsOf(block)[3];
    const at = river ? river.actions.findIndex((a) => a.verb === "bets") : -1;
    if (at < 0 || !names.includes(river.actions[at].name)) continue;
    const bet = river.actions[at];
    bets.push({ fraction: bet.amount / river.pot, allIn: bet.allIn });
    const raise = river.actions.findIndex((a, i) => i > at && a.verb === "raises");
    const answer = river.actions.findIndex((a, i) => i > raise && a.name === bet.name);
    if (raise > at && answer > raise) {
      raised.n += 1;
      raised.folds += river.actions[answer].verb === "folds" ? 1 : 0;
    }
  }
  const share = (test) => bets.filter(test).length / bets.length;
  return {
    n: bets.length,
    polar: share((b) => b.allIn || b.fraction >= 1.1),
    small: share((b) => !b.allIn && b.fraction < 0.4),
    raised,
  };
}

test("riverSizing: the overbettor sizes his river bets polar, the blocker small; the same reg base does neither", async () => {
  const overbettor = await riverBets("overbettor");
  const blocker = await riverBets("blocker");
  const reg = await riverBets("icm"); // The shared reg base, without river sizing.
  for (const [id, r] of Object.entries({ overbettor, blocker, reg })) assert.ok(r.n > 60, `${id}: ${r.n} river bets`);
  // profiles.json: 60% polar and 5% small; 85% small and none polar.
  assert.ok(Math.abs(overbettor.polar - 0.6) < 0.1, `overbettor polar share ${overbettor.polar.toFixed(2)}`);
  assert.ok(overbettor.small < 0.12, `overbettor small share ${overbettor.small.toFixed(2)}`);
  assert.ok(Math.abs(blocker.small - 0.85) < 0.1, `blocker small share ${blocker.small.toFixed(2)}`);
  assert.ok(blocker.polar < 0.06, `blocker polar share ${blocker.polar.toFixed(2)}`);
  assert.ok(reg.polar < 0.1 && reg.small < 0.1, `reg shares ${reg.polar.toFixed(2)} / ${reg.small.toFixed(2)}`);
});

test("foldToRiverRaise: the blocker folds most of his raised river bets, the reg base far fewer", async () => {
  const blocker = (await riverBets("blocker")).raised;
  const reg = (await riverBets("overbettor")).raised;
  assert.ok(blocker.n >= 15, `raised river bets: ${blocker.n}`);
  // Target .85, weak hands first.
  assert.ok(blocker.folds / blocker.n > 0.7, `blocker folds ${blocker.folds} of ${blocker.n}`);
  assert.ok(reg.n === 0 || reg.folds / reg.n < 0.6, `reg folds ${reg.folds} of ${reg.n}`);
});

test("fitOrFold: the fit-or-fold reg folds far more flops to a bet than the same reg base", async () => {
  const foldRate = async (id) => {
    const names = playersOf(id);
    let n = 0;
    let folds = 0;
    for (const block of await behaviourHands()) {
      const flop = streetsOf(block)[1];
      const bet = flop ? flop.actions.findIndex((a) => a.verb === "bets") : -1;
      if (bet < 0) continue;
      // His first answer to the flop bet, before any raise.
      for (const a of flop.actions.slice(bet + 1)) {
        if (a.verb === "raises" && !names.includes(a.name)) break;
        if (!names.includes(a.name)) continue;
        n += 1;
        folds += a.verb === "folds" ? 1 : 0;
        break;
      }
    }
    return { n, rate: folds / n };
  };
  const fitfold = await foldRate("fitfold");
  const reg = await foldRate("icm");
  assert.ok(fitfold.n > 100 && reg.n > 100, `samples ${fitfold.n} / ${reg.n}`);
  assert.ok(fitfold.rate > 0.58, `fit-or-fold folds ${fitfold.rate.toFixed(2)} of flop bets`);
  assert.ok(fitfold.rate > reg.rate + 0.12, `fit-or-fold ${fitfold.rate.toFixed(2)} vs reg ${reg.rate.toFixed(2)}`);
});
