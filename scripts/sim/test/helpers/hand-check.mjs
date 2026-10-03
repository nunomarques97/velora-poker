// Test helper: reads generated hand histories back as text and checks that
// every hand adds up the way a real PokerStars hand does. Independent of
// the generator's own bookkeeping, so a bug there cannot hide itself.

import { evaluate } from "../../lib/cards.mjs";

const BOM = new RegExp("^" + String.fromCharCode(0xfeff));
const CENTS = /\$(\d+(?:\.\d{2})?)/;

export function cents(text) {
  const m = CENTS.exec(text);
  if (!m) throw new Error(`no amount in ${JSON.stringify(text)}`);
  return Math.round(Number(m[1]) * 100);
}

/** Hands of one file, without the BOM and the blank separator lines. */
export function splitHands(text) {
  return text
    .replace(BOM, "")
    .split(/\r\n\r\n\r\n/)
    .map((block) => block.trim())
    .filter(Boolean);
}

/**
 * Checks one hand and returns its facts. Throws with the hand id on the
 * first inconsistency. `bb` and `rakeCapBb` are the table's stakes;
 * `chips` reads amounts as bare tournament chips (`ante` is then the
 * level's ante, and a knockout's elimination lines are checked against the
 * seat bounties).
 */
export function checkHand(block, { hero, bb, rakeCapBb, chips = false, ante = 0 }) {
  const lines = block.split(/\r\n/);
  const AMT = chips ? String.raw`(\d+)` : String.raw`(\$[\d.]+)`;
  const amount = chips ? (text) => Number(text) : cents;
  const re = (source) => new RegExp(source.replaceAll("AMT", AMT));
  const id = /Hand #(\d+):/.exec(lines[0])?.[1];
  const fail = (why) => {
    throw new Error(`hand ${id}: ${why}`);
  };
  if (!id) fail("no hand id in the header");
  const buttonSeat = Number(/Seat #(\d+) is the button/.exec(lines[1])?.[1]);

  const seats = [];
  let i = 2;
  for (; /^Seat \d+: /.test(lines[i]); i++) {
    const m = re(String.raw`^Seat (\d+): (.+) \(AMT in chips(?:, (\$[\d.]+) bounty)?\)$`).exec(lines[i]);
    if (!m) fail(`bad seat line ${lines[i]}`);
    seats.push({ seat: Number(m[1]), name: m[2], stack: amount(m[3]), bounty: m[4] ? cents(m[4]) : null });
  }
  const byName = new Map(seats.map((s) => [s.name, s]));
  if (!seats.some((s) => s.seat === buttonSeat)) fail("the button is not an occupied seat");

  // Blinds: the two occupied seats after the button (heads-up: the button
  // posts the small blind).
  const ring = seats.map((s) => s.seat).sort((a, b) => a - b);
  const after = (seat, k) => ring[(ring.indexOf(seat) + k) % ring.length];
  const sbSeat = ring.length === 2 ? buttonSeat : after(buttonSeat, 1);
  const bbSeat = after(sbSeat, 1);
  const seatOf = (name) => byName.get(name)?.seat;

  const put = new Map(seats.map((s) => [s.name, 0]));
  let street = new Map();
  let toCall = 0;
  let lastRaise = bb;
  let collected = 0;
  const winners = new Set();
  const folded = new Set();
  // Collections per pot index: 0 = "pot" or "main pot", 1 = "side pot" or
  // "side pot-1", N = "side pot-N".
  const byPot = new Map();
  const shown = [];
  let board = [];
  let total = null;
  let rake = null;
  let sawFlop = false;
  let dealtToHero = false;
  const eliminations = [];
  const finishes = [];

  const commit = (name, to) => {
    if (!byName.has(name)) fail(`unknown player ${name}`);
    const add = to - (street.get(name) || 0);
    if (add < 0) fail(`${name} takes chips back`);
    street.set(name, to);
    put.set(name, put.get(name) + add);
    if (put.get(name) > byName.get(name).stack) fail(`${name} puts in more than his stack`);
  };

  for (; i < lines.length; i++) {
    const line = lines[i];
    let m;
    if ((m = re(String.raw`^(.+): posts the ante AMT( and is all-in)?$`).exec(line))) {
      if (!byName.has(m[1])) fail(`unknown player ${m[1]}`);
      const paid = amount(m[2]);
      if (paid > ante || (paid < ante && !m[3])) fail(`${m[1]} posts an ante of ${paid}, not ${ante}`);
      put.set(m[1], put.get(m[1]) + paid);
      if (put.get(m[1]) > byName.get(m[1]).stack) fail(`${m[1]} antes more than his stack`);
    } else if ((m = re(String.raw`^(.+): posts small blind AMT( and is all-in)?$`).exec(line))) {
      if (seatOf(m[1]) !== sbSeat) fail(`small blind posted by ${m[1]} out of position`);
      commit(m[1], amount(m[2]));
      toCall = amount(m[2]);
    } else if ((m = re(String.raw`^(.+): posts big blind AMT`).exec(line))) {
      if (seatOf(m[1]) !== bbSeat) fail(`big blind posted by ${m[1]} out of position`);
      commit(m[1], amount(m[2]));
      // The bet to call is the largest blind posted (a short all-in big
      // blind may post less than the small blind).
      toCall = Math.max(toCall, amount(m[2]));
    } else if ((m = /^Dealt to (.+) \[(\w\w) (\w\w)\]$/.exec(line))) {
      if (m[1] !== hero) fail(`cards dealt to ${m[1]}, not the hero`);
      dealtToHero = true;
    } else if ((m = /^\*\*\* (FLOP|TURN|RIVER) \*\*\* (.+)$/.exec(line))) {
      board = [...m[2].matchAll(/\b([2-9TJQKA][cdhs])\b/g)].map((x) => x[1]);
      const expected = { FLOP: 3, TURN: 4, RIVER: 5 }[m[1]];
      if (board.length !== expected) fail(`${m[1]} shows ${board.length} board cards`);
      sawFlop = true;
      street = new Map();
      toCall = 0;
      lastRaise = bb;
    } else if ((m = re(String.raw`^(.+): calls AMT( and is all-in)?$`).exec(line))) {
      const to = (street.get(m[1]) || 0) + amount(m[2]);
      if (to > toCall) fail(`${m[1]} calls more than the bet`);
      if (to < toCall && !m[3]) fail(`${m[1]} calls short without being all-in`);
      commit(m[1], to);
    } else if ((m = re(String.raw`^(.+): bets AMT( and is all-in)?$`).exec(line))) {
      if (toCall !== 0) fail(`${m[1]} bets into a bet`);
      const bet = amount(m[2]);
      if (bet < bb && !m[3]) fail(`${m[1]} bets less than the big blind`);
      commit(m[1], bet);
      toCall = bet;
      lastRaise = bet;
    } else if ((m = re(String.raw`^(.+): raises AMT to AMT( and is all-in)?$`).exec(line))) {
      const by = amount(m[2]);
      const to = amount(m[3]);
      if (to - toCall !== by) fail(`${m[1]} raise size does not add up`);
      if (by < lastRaise && !m[4]) fail(`${m[1]} min-raise violated (${by} < ${lastRaise})`);
      commit(m[1], to);
      if (by >= lastRaise) lastRaise = by;
      toCall = to;
    } else if ((m = re(String.raw`^Uncalled bet \(AMT\) returned to (.+)$`).exec(line))) {
      const back = amount(m[1]);
      put.set(m[2], put.get(m[2]) - back);
      street.set(m[2], street.get(m[2]) - back);
    } else if ((m = /^(.+): shows \[(\w\w) (\w\w)\] \((.+)\)$/.exec(line))) {
      const hand = evaluate([m[2], m[3], ...board]);
      if (hand.text !== m[4]) fail(`${m[1]} shows "${m[4]}" but holds ${hand.text}`);
      shown.push({ name: m[1], score: hand.score });
    } else if ((m = /^(.+): folds$/.exec(line))) {
      folded.add(m[1]);
    } else if ((m = re(String.raw`^(.+) collected AMT from (pot|main pot|side pot(?:-(\d+))?)$`).exec(line))) {
      const won = amount(m[2]);
      const index = m[3].startsWith("side") ? Number(m[4] ?? 1) : 0;
      if (!byPot.has(index)) byPot.set(index, new Map());
      const pot = byPot.get(index);
      pot.set(m[1], (pot.get(m[1]) || 0) + won);
      collected += won;
      winners.add(m[1]);
    } else if ((m = /^(.+) wins (\$[\d.]+) for eliminating (.+) and their own bounty increases by (\$[\d.]+) to (\$[\d.]+)$/.exec(line))) {
      eliminations.push({ by: m[1], player: m[3], cash: cents(m[2]), head: cents(m[4]), to: cents(m[5]) });
    } else if ((m = /^(.+) finished the tournament in (\d+)(?:st|nd|rd|th) place$/.exec(line))) {
      finishes.push({ player: m[1], place: Number(m[2]) });
    } else if ((m = /^(.+) wins the tournament and receives (\$[\d.]+) - congratulations!$/.exec(line))) {
      finishes.push({ player: m[1], place: 1, prize: cents(m[2]) });
    } else if ((m = re(String.raw`^Total pot AMT.*\| Rake AMT$`).exec(line))) {
      total = amount(m[1]);
      rake = amount(m[2]);
    }
  }

  if (!dealtToHero) fail("no hole cards for the hero");
  if (total === null) fail("no summary line");
  const contributed = [...put.values()].reduce((a, b) => a + b, 0);
  if (contributed !== total) fail(`total pot ${total} but ${contributed} was put in`);
  if (collected + rake !== total) fail(`collected ${collected} + rake ${rake} != pot ${total}`);
  if (!sawFlop && rake !== 0) fail("rake taken without a flop");
  if (rake > Math.floor(total * 0.05) + 1 || rake > rakeCapBb * bb) fail(`rake ${rake} above 5% or the cap`);
  if (shown.length > 1) {
    const best = Math.max(...shown.map((s) => s.score));
    const top = shown.filter((s) => s.score === best);
    if (!top.some((s) => winners.has(s.name))) fail("the best shown hand won nothing");
  }
  if (!lines.includes("*** SUMMARY ***")) fail("no summary section");

  // Pots rebuilt from what each player put in: one pot per level a live
  // player is all-in for (or stopped at), each won only by the live players
  // who covered it. Nobody can win more than they matched from each player.
  const live = seats.map((s) => s.name).filter((name) => !folded.has(name));
  const levels = [...new Set(live.map((name) => put.get(name)))].sort((a, b) => a - b);
  const pots = [];
  let floor = 0;
  for (const level of levels) {
    let amount = 0;
    for (const value of put.values()) amount += Math.max(0, Math.min(value, level) - floor);
    if (amount > 0) pots.push({ amount, eligible: live.filter((name) => put.get(name) >= level) });
    floor = level;
  }
  const scoreOf = new Map(shown.map((s) => [s.name, s.score]));
  for (const name of winners) {
    if (folded.has(name)) fail(`${name} folded but collects`);
    const most = [...put.values()].reduce((sum, value) => sum + Math.min(value, put.get(name)), 0);
    const got = [...byPot.values()].reduce((sum, pot) => sum + (pot.get(name) || 0), 0);
    if (got > most) fail(`${name} collects ${got} but could win at most ${most}`);
  }
  for (const [index, takers] of byPot) {
    const pot = pots[index];
    if (!pot) fail(`collects from pot ${index} but only ${pots.length} pot(s) were built`);
    const got = [...takers.values()].reduce((a, b) => a + b, 0);
    if (got > pot.amount) fail(`pot ${index} pays ${got} but holds ${pot.amount}`);
    for (const name of takers.keys()) {
      if (!pot.eligible.includes(name)) fail(`${name} collects from pot ${index} without covering it`);
    }
    if (pot.eligible.length > 1) {
      const best = Math.max(...pot.eligible.map((name) => scoreOf.get(name) ?? -Infinity));
      for (const name of takers.keys()) {
        if (scoreOf.get(name) !== best) fail(`${name} collects from pot ${index} without the best hand in it`);
      }
    }
  }
  if (byPot.size !== pots.length) fail(`${pots.length} pot(s) built but ${byPot.size} paid out`);
  const soloSidePots = pots.filter((pot, index) => index > 0 && pot.eligible.length === 1).length;

  // Busted: nothing left once the pots are paid. In a knockout each one is
  // paid to a player who won chips from him: half his bounty in cash, half
  // onto the winner's own bounty.
  const wonBy = new Map();
  for (const pot of byPot.values()) for (const [name, got] of pot) wonBy.set(name, (wonBy.get(name) || 0) + got);
  const busted = seats.filter((s) => s.stack - put.get(s.name) + (wonBy.get(s.name) || 0) === 0).map((s) => s.name);
  if (seats.some((s) => s.bounty != null)) {
    const bounties = new Map(seats.map((s) => [s.name, s.bounty]));
    if (eliminations.length !== busted.length) fail(`${busted.length} busted but ${eliminations.length} elimination line(s)`);
    for (const e of eliminations) {
      if (!busted.includes(e.player)) fail(`${e.player} is eliminated with chips left`);
      if (!winners.has(e.by)) fail(`${e.by} eliminates ${e.player} without winning a pot`);
      if (e.cash + e.head !== bounties.get(e.player)) fail(`${e.player}'s bounty does not add up`);
      if (Math.abs(e.cash - e.head) > 1) fail(`${e.player}'s bounty is not split in half`);
      bounties.set(e.by, bounties.get(e.by) + e.head);
      if (bounties.get(e.by) !== e.to) fail(`${e.by}'s bounty grows to the wrong amount`);
    }
  } else if (eliminations.length > 0) fail("elimination lines without bounties");
  for (const f of finishes) {
    if (f.place > 1 && !busted.includes(f.player)) fail(`${f.player} finishes with chips left`);
  }
  return { id, seats, put, total, rake, shown: shown.length, sawFlop, pots: pots.length, soloSidePots, busted, eliminations, finishes };
}
