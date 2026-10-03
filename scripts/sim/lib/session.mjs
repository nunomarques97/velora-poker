// A simulated session: cash tables (one file per table, like the client) or
// a Zoom pool (one file per pool), villains seated from the shared profiles,
// hands written as they finish at a configurable pace. Tournament formats
// (MTT 9-max with bounties, Spin & Go) are played by tournament.mjs.

import { appendFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { assertSafeOutputDir } from "./guard.mjs";
import { cashFileName, cashHeader, tableName, zoomPoolName } from "./format.mjs";
import { hash32, Rng } from "./rng.mjs";
import { playHand } from "./play.mjs";
import { trackTilt } from "./tilt.mjs";
import { runTournaments, TOURNAMENT_FORMATS } from "./tournament.mjs";

export const PROFILES_PATH = new URL("../profiles.json", import.meta.url);
export const FORMATS = ["cash", "zoom", ...TOURNAMENT_FORMATS];
/** Default session start (local time), so a seed alone fixes the text. */
export const DEFAULT_START = () => new Date(2026, 8, 12, 20, 0, 0);
/** Mean simulated seconds between two hands at one table. */
const HAND_SECONDS = { cash: 50, zoom: 25 };
export const EOL = "\r\n";
export const BOM = String.fromCharCode(0xfeff);

export function loadProfiles(path = PROFILES_PATH) {
  return JSON.parse(readFileSync(path, "utf8"));
}

export const defaultSleep = (ms) => new Promise((done) => setTimeout(done, ms));

/**
 * Runs one session and returns its manifest.
 *
 * Options: `format` (`cash` | `zoom` | `mtt` | `spin`), `seed`, `out`
 * (folder), `tables` (cash tables, or tournaments played at once), `hands`
 * (per cash table or tournament seat; in total for Zoom), `pace` (real
 * seconds per simulated hand at one table; 0 writes as fast as possible),
 * `start` (Date), `clock` (`sim`: timestamps from simulated time; `live`:
 * the wall clock when each hand is written), `profiles` (parsed JSON),
 * `sleep(ms)` and `now()` (injectable for tests).
 */
export async function runSession(options) {
  const {
    format = "cash",
    seed = "velora",
    out,
    tables: tableCount = 6,
    hands = 200,
    pace = 0,
    clock = "sim",
    profiles = loadProfiles(),
    sleep = defaultSleep,
    now = () => Date.now(),
  } = options;
  if (!FORMATS.includes(format)) throw new Error(`unknown format ${format} (expected ${FORMATS.join(", ")})`);
  if (!(pace >= 0)) throw new Error("pace must be a number of seconds, 0 or more");
  if (!Number.isInteger(hands) || hands < 1) throw new Error("hands must be a positive integer");
  if (!Number.isInteger(tableCount) || tableCount < 1 || tableCount > 24) throw new Error("tables must be 1 to 24");
  const heroName = profiles.hero.name;
  const dir = assertSafeOutputDir(out, { hero: heroName });
  mkdirSync(dir, { recursive: true });

  const start = options.start ?? (clock === "live" ? new Date(now()) : DEFAULT_START());
  if (TOURNAMENT_FORMATS.includes(format)) {
    return runTournaments({ format, seed, dir, tableCount, hands, pace, clock, start, profiles, sleep, now });
  }
  const rng = new Rng(`${seed}:${format}`);
  const stakes = profiles.stakes[format];
  const roster = profiles.profiles.flatMap((profile) => profile.players.map((name) => ({ name, profile })));
  const hero = { name: heroName, profile: { id: "hero", ...profiles.hero } };
  const byName = new Map([...roster, hero].map((p) => [p.name, p]));
  const tilt = new Map(roster.map((p) => [p.name, { left: 0, cooldown: 0 }]));
  const memory = new Map([...roster, hero].map((p) => [p.name, {}]));
  const episodes = [];
  const counts = new Map();
  let handId = 250_000_000_000 + (hash32(`${seed}:${format}`) % 4_000_000) * 1000;

  const buyIn = (who) => Math.round((who.profile.buyInBb ?? 100) * stakes.bb);
  const behaviourOf = (who) => {
    const state = tilt.get(who.name);
    const base = who.profile.behaviour;
    return state && state.left > 0 ? { ...base, ...who.profile.tilt.behaviour } : base;
  };
  /** Rebuys below half a buy-in; a hit-and-run player re-sits after doubling. */
  const topUp = (who, stack) => {
    const full = buyIn(who);
    const leave = who.profile.leaveAboveBb;
    if (stack < full / 2 || stack < stakes.bb) return full;
    if (leave && stack > leave * stakes.bb) return full;
    return stack;
  };

  // Tables: cash seats hero plus five villains each; Zoom is one pool.
  const tables = [];
  const tableOffset = hash32(seed) % 20;
  if (format === "cash") {
    const order = rng.fork("seating").shuffle([...roster]);
    let cursor = 0;
    for (let t = 0; t < tableCount; t++) {
      const seatRng = rng.fork(`table-${t}`);
      const seatNumbers = seatRng.shuffle([1, 2, 3, 4, 5, 6]).slice(0, stakes.maxSeats);
      const sitting = [hero];
      while (sitting.length < stakes.maxSeats) {
        const next = order[cursor++ % order.length];
        if (!sitting.includes(next)) sitting.push(next);
      }
      const seats = sitting.map((who, i) => ({ seat: seatNumbers[i], who, stack: buyIn(who) }));
      tables.push({
        name: tableName(t, tableOffset),
        seats,
        button: seatRng.pick(seats).seat,
        next: seatRng.next() * HAND_SECONDS.cash,
        played: 0,
        target: hands,
        file: null,
      });
    }
  } else {
    tables.push({ name: zoomPoolName(tableOffset), seats: null, next: 0, played: 0, target: hands, file: null });
  }
  const zoomStacks = new Map([...roster, hero].map((who) => [who.name, null]));

  const manifest = {
    format,
    seed: String(seed),
    hero: heroName,
    out: dir,
    files: [],
    tables: [],
    hands: 0,
    players: {},
    tiltEpisodes: episodes,
  };

  for (;;) {
    const open = tables.filter((t) => t.played < t.target);
    if (open.length === 0) break;
    const table = open.reduce((a, b) => (b.next < a.next ? b : a));
    const simTime = table.next;
    const date = clock === "live" ? new Date(now()) : new Date(start.getTime() + Math.round(simTime * 1000));
    handId += 1 + rng.int(30);

    let seats;
    let button;
    if (format === "cash") {
      for (const s of table.seats) s.stack = topUp(s.who, s.stack);
      // The button moves one occupied seat clockwise each hand.
      const ring = table.seats.map((s) => s.seat).sort((a, b) => a - b);
      if (table.played > 0) table.button = ring[(ring.indexOf(table.button) + 1) % ring.length];
      seats = table.seats;
      button = table.button;
    } else {
      const pool = rng.shuffle(roster.slice()).slice(0, stakes.maxSeats - 1);
      const numbers = rng.shuffle([1, 2, 3, 4, 5, 6]).slice(0, stakes.maxSeats);
      seats = [hero, ...pool].map((who, i) => {
        const stack = topUp(who, zoomStacks.get(who.name) ?? buyIn(who));
        return { seat: numbers[i], who, stack };
      });
      button = rng.pick(seats).seat;
    }

    const header = cashHeader({ handId, zoom: format === "zoom", sb: stakes.sb, bb: stakes.bb, date });
    const tilted = seats.filter((s) => tilt.get(s.who.name)?.left > 0).map((s) => s.who.name);
    const hand = playHand({
      table: { name: table.name, maxSeats: stakes.maxSeats, sb: stakes.sb, bb: stakes.bb, rakeCapBb: stakes.rakeCapBb },
      seats: seats.map((s) => ({ seat: s.seat, name: s.who.name, stack: s.stack, behaviour: behaviourOf(s.who), memory: memory.get(s.who.name) })),
      button,
      header,
      heroName,
      rng,
    });

    for (const s of seats) {
      const result = hand.results.get(s.who.name);
      s.stack += result.net;
      if (format === "zoom") zoomStacks.set(s.who.name, s.stack);
      counts.set(s.who.name, (counts.get(s.who.name) || 0) + 1);
      trackTilt(s.who, result.net, handId, tilt.get(s.who.name), tilted, episodes, stakes.bb, counts.get(s.who.name));
    }

    if (!table.file) {
      table.file = cashFileName({ date, tableName: table.name, sb: stakes.sb, bb: stakes.bb });
      manifest.files.push(table.file);
    }
    const path = join(dir, table.file);
    const text = hand.lines.join(EOL) + EOL + EOL + EOL;
    // Like the client: a new file starts with a BOM, later hands are appended.
    appendFileSync(path, existsSync(path) ? text : BOM + text, "utf8");
    table.played += 1;
    manifest.hands += 1;
    const spacing = HAND_SECONDS[format];
    table.next += spacing * (0.75 + rng.next() * 0.5);

    if (pace > 0) {
      const pending = tables.filter((t) => t.played < t.target);
      if (pending.length > 0) {
        const nextTime = Math.min(...pending.map((t) => t.next));
        const wait = Math.max(0, ((nextTime - simTime) / spacing) * pace * 1000);
        await sleep(Math.round(wait));
      }
    }
  }

  manifest.tables = tables.map((t) => ({
    name: t.name,
    file: t.file,
    hands: t.played,
    seats: t.seats ? t.seats.map((s) => ({ seat: s.seat, name: s.who.name, profile: s.who.profile.id })) : null,
  }));
  for (const [name, n] of counts) manifest.players[name] = { profile: byName.get(name).profile.id, hands: n };
  return manifest;
}
