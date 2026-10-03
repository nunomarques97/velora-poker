// Simulated tournaments: MTT 9-max progressive knockouts and hyper Spin &
// Go (3-max), with the same villain profiles as the cash games. Each
// tournament is one file, named like the client's; the hero plays
// `tableCount` of them at once and starts a new one when his ends (he busts,
// or wins a Spin). Blind levels rise with simulated time, villains bust and
// are replaced at the MTT table, bounties are paid and grow.

import { appendFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { money2, ordinal, tournamentFileName, tournamentHeader } from "./format.mjs";
import { playHand } from "./play.mjs";
import { hash32, Rng } from "./rng.mjs";
import { trackTilt } from "./tilt.mjs";

export const TOURNAMENT_FORMATS = ["mtt", "spin"];
const EOL = "\r\n";
const BOM = String.fromCharCode(0xfeff);
/** Simulated seconds between two tournaments at one of the hero's slots. */
const BETWEEN_TOURNAMENTS = 60;
/** Hands before a villain knocked out of a table can be seated there again. */
const REENTRY_HANDS = 20;

/** `[sb, bb, ante]` of a structure's level index (the last one repeats). */
export function levelAt(structure, index) {
  const [sb, bb, ante = 0] = structure.levels[Math.min(index, structure.levels.length - 1)];
  return { sb, bb, ante, number: index + 1 };
}

/**
 * Plays the hero's tournaments and returns the session manifest. `tables`
 * in the manifest lists one entry per tournament (its file and table), as
 * the cash manifest lists one per cash table.
 */
export async function runTournaments({ format, seed, dir, tableCount, hands, pace, clock, start, profiles, sleep, now }) {
  const structure = profiles.stakes[format];
  const rng = new Rng(`${seed}:${format}`);
  const heroName = profiles.hero.name;
  const roster = profiles.profiles.flatMap((profile) => profile.players.map((name) => ({ name, profile })));
  const hero = { name: heroName, profile: { id: "hero", ...profiles.hero } };
  const byName = new Map([...roster, hero].map((p) => [p.name, p]));
  const tilt = new Map(roster.map((p) => [p.name, { left: 0, cooldown: 0 }]));
  const memory = new Map([...roster, hero].map((p) => [p.name, {}]));
  const episodes = [];
  const counts = new Map();
  const tournaments = [];
  let handId = 262_000_000_000 + (hash32(`${seed}:${format}`) % 4_000_000) * 1000;
  let tournamentId = (format === "spin" ? 3_950_000_000 : 4_100_000_000) + (hash32(`${seed}:${format}:id`) % 40_000_000);

  /** His tournament behaviour (the profile's `tournament.behaviour` over his own), or his tilt. */
  const behaviourOf = (who) => {
    const state = tilt.get(who.name);
    const base = { ...who.profile.behaviour, ...who.profile.tournament?.behaviour };
    return state && state.left > 0 ? { ...base, ...who.profile.tilt.behaviour } : base;
  };

  /** Chips of a villain sitting down at level `index` mid-tournament. */
  const arrivingStack = (who, index) => {
    const { bb } = levelAt(structure, index);
    const range = who.profile.tournament?.stackBb;
    if (range) return Math.round(bb * (range[0] + rng.next() * (range[1] - range[0])));
    if (index === 0) return structure.startingStack;
    const avgBb = structure.avgStackBb[Math.min(index, structure.avgStackBb.length - 1)];
    return Math.max(2 * bb, Math.round(avgBb * bb * (0.35 + rng.next() * 1.3)));
  };
  /** A bounty that has grown with knockouts already made, in cents. */
  const arrivingBounty = (index) => {
    const initial = structure.buyIn[1];
    if (index === 0 || !rng.chance(0.35)) return initial;
    return Math.round(initial * (1 + 0.5 * (1 + rng.int(4))));
  };

  const newTournament = (slot, time) => {
    tournamentId += 1 + rng.int(4000);
    const max = structure.maxSeats;
    // MTT: the hero late-registers into a running tournament; a Spin
    // starts with its three players.
    const index = format === "mtt" ? rng.int(structure.lateRegLevels) : 0;
    const t0 = time - index * structure.levelSeconds - (index > 0 ? rng.next() * structure.levelSeconds : 0);
    const numbers = Array.from({ length: max }, (_, i) => i + 1);
    const pool = rng.shuffle(roster.slice()).slice(0, max - 1);
    const seats = rng.shuffle(numbers.slice()).map((seat, i) => {
      const who = i === 0 ? hero : pool[i - 1];
      const stack = who === hero || format === "spin" ? structure.startingStack : arrivingStack(who, index);
      return { seat, who, stack, bounty: structure.knockout ? (who === hero ? structure.buyIn[1] : arrivingBounty(index)) : null };
    });
    const multiplier = format === "spin" ? weighted(rng, structure.multipliers) : null;
    const record = {
      id: String(tournamentId),
      format,
      table: format === "mtt" ? `${tournamentId} ${1 + rng.int(structure.tables)}` : `${tournamentId} 1`,
      file: null,
      buyIn: structure.buyIn,
      startLevel: index + 1,
      hands: 0,
      heroFinish: null,
      multiplier,
      eliminations: [],
    };
    tournaments.push(record);
    Object.assign(slot, { record, t0, seats, button: rng.pick(seats).seat, first: true, busted: new Map() });
  };

  const slots = [];
  for (let t = 0; t < tableCount; t++) {
    const slot = { next: rng.next() * structure.handSeconds, played: 0, target: hands };
    newTournament(slot, slot.next);
    slots.push(slot);
  }

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
    tournaments,
  };

  for (;;) {
    const open = slots.filter((s) => s.played < s.target);
    if (open.length === 0) break;
    const slot = open.reduce((a, b) => (b.next < a.next ? b : a));
    const simTime = slot.next;
    const date = clock === "live" ? new Date(now()) : new Date(start.getTime() + Math.round(simTime * 1000));
    const record = slot.record;
    const index = Math.max(0, Math.floor((simTime - slot.t0) / structure.levelSeconds));
    const level = levelAt(structure, index);

    // MTT table balancing: empty seats are mostly refilled before the
    // hand, never with somebody already at this table or knocked out of it
    // in the last REENTRY_HANDS hands (a re-entry sits down elsewhere).
    if (format === "mtt") {
      const taken = new Set(slot.seats.map((s) => s.who.name));
      const recent = (who) => slot.played - (slot.busted.get(who.name) ?? -Infinity) < REENTRY_HANDS;
      const free = rng.shuffle(roster.filter((who) => !taken.has(who.name) && !recent(who)));
      for (let seat = 1; seat <= structure.maxSeats; seat++) {
        if (slot.seats.some((s) => s.seat === seat) || free.length === 0) continue;
        if (slot.seats.length > 2 && !rng.chance(structure.refill)) continue;
        const who = free.pop();
        slot.seats.push({ seat, who, stack: arrivingStack(who, index), bounty: arrivingBounty(index) });
      }
    }
    // The button moves to the next occupied seat clockwise.
    const ring = slot.seats.map((s) => s.seat).sort((a, b) => a - b);
    if (!slot.first) slot.button = ring.find((n) => n > slot.button) ?? ring[0];
    slot.first = false;

    handId += 1 + rng.int(30);
    const header = tournamentHeader({
      handId,
      tournamentId: record.id,
      buyIn: structure.buyIn,
      level: level.number,
      sb: level.sb,
      bb: level.bb,
      date,
    });
    const seats = slot.seats;
    const bounties = new Map(seats.map((s) => [s.who.name, s.bounty]));
    const tilted = seats.filter((s) => tilt.get(s.who.name)?.left > 0).map((s) => s.who.name);
    const remaining = seats.length;
    const tail = (busted) => {
      const lines = [];
      for (const [i, out] of busted.entries()) {
        if (structure.knockout && out.by) {
          const bounty = bounties.get(out.name);
          const cash = Math.floor(bounty / 2);
          const head = bounty - cash;
          bounties.set(out.by, bounties.get(out.by) + head);
          lines.push(
            `${out.by} wins ${money2(cash)} for eliminating ${out.name} and their own bounty increases by ${money2(head)} to ${money2(
              bounties.get(out.by),
            )}`,
          );
          record.eliminations.push({ handId: String(handId), player: out.name, by: out.by, cash, head });
        } else record.eliminations.push({ handId: String(handId), player: out.name, by: out.by });
        if (format === "spin") lines.push(`${out.name} finished the tournament in ${ordinal(remaining - i)} place`);
      }
      if (format === "spin" && remaining - busted.length === 1) {
        const winner = seats.find((s) => !busted.some((b) => b.name === s.who.name)).who.name;
        const prize = record.multiplier * structure.buyIn.reduce((a, b) => a + b, 0);
        lines.push(`${winner} wins the tournament and receives ${money2(prize)} - congratulations!`);
      }
      return lines;
    };
    const hand = playHand({
      table: {
        name: record.table,
        maxSeats: structure.maxSeats,
        sb: level.sb,
        bb: level.bb,
        ante: level.ante,
        chips: true,
        rakeCapBb: 0,
      },
      seats: seats.map((s) => ({
        seat: s.seat,
        name: s.who.name,
        stack: s.stack,
        bounty: s.bounty,
        behaviour: behaviourOf(s.who),
        memory: memory.get(s.who.name),
      })),
      button: slot.button,
      header,
      heroName,
      rng,
      tail,
    });

    for (const s of seats) {
      const result = hand.results.get(s.who.name);
      s.stack += result.net;
      s.bounty = bounties.get(s.who.name);
      counts.set(s.who.name, (counts.get(s.who.name) || 0) + 1);
      trackTilt(s.who, result.net, handId, tilt.get(s.who.name), tilted, episodes, level.bb, counts.get(s.who.name));
    }
    for (const s of seats) if (s.stack <= 0) slot.busted.set(s.who.name, slot.played);
    slot.seats = seats.filter((s) => s.stack > 0);

    if (!record.file) {
      record.file = tournamentFileName({ date, tournamentId: record.id, buyIn: structure.buyIn });
      manifest.files.push(record.file);
    }
    const path = join(dir, record.file);
    const text = hand.lines.join(EOL) + EOL + EOL + EOL;
    appendFileSync(path, existsSync(path) ? text : BOM + text, "utf8");
    record.hands += 1;
    slot.played += 1;
    manifest.hands += 1;
    const spacing = structure.handSeconds;
    slot.next += spacing * (0.75 + rng.next() * 0.5);

    // The hero's tournament ends when he busts or, in a Spin, wins it.
    const heroOut = !slot.seats.some((s) => s.who === hero);
    if (heroOut || (format === "spin" && slot.seats.length === 1)) {
      record.heroFinish = heroOut ? "busted" : "won";
      slot.next += BETWEEN_TOURNAMENTS;
      if (slot.played < slot.target) newTournament(slot, slot.next);
    }

    if (pace > 0) {
      const pending = slots.filter((s) => s.played < s.target);
      if (pending.length > 0) {
        const nextTime = Math.min(...pending.map((s) => s.next));
        const wait = Math.max(0, ((nextTime - simTime) / spacing) * pace * 1000);
        await sleep(Math.round(wait));
      }
    }
  }

  for (const record of tournaments) record.heroFinish ??= "running";
  manifest.tables = tournaments
    .filter((t) => t.file)
    .map((t) => ({ name: t.table, file: t.file, hands: t.hands, tournamentId: t.id }));
  for (const [name, n] of counts) manifest.players[name] = { profile: byName.get(name).profile.id, hands: n };
  return manifest;
}

/** Picks a value from `[[value, weight], ...]`. */
function weighted(rng, pairs) {
  const total = pairs.reduce((sum, [, w]) => sum + w, 0);
  let u = rng.next() * total;
  for (const [value, w] of pairs) {
    u -= w;
    if (u < 0) return value;
  }
  return pairs[pairs.length - 1][0];
}
