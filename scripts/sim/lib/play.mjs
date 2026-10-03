// Plays one No Limit Hold'em hand among seated players and renders it as
// PokerStars hand-history text. Decisions come from each player's profile
// behaviour (see profiles.json); everything else follows the rules: blinds,
// betting order, minimum raises, all-ins, uncalled bets, side pots, rake,
// real showdown hand ranks and the summary section.

import { evaluate, newDeck, postflopStrength, preflopPercentile } from "./cards.mjs";
import { money } from "./format.mjs";

const STREET_FOLD = ["before Flop", "on the Flop", "on the Turn", "on the River"];
/** Starting guess of the share of good-or-better hands in a postflop spot. */
const STRONG_SHARE = [0, 0.35, 0.45, 0.5];
/** Weight of that guess, in observed decisions. */
const SHARE_PRIOR_WEIGHT = 10;

/** Position labels for an n-handed ring, index 0 = button (as `parser::position`). */
export function positionLabels(n) {
  if (n === 2) return ["BTN", "BB"];
  const middle = n - 3;
  const lateCount = middle === 0 ? 0 : middle <= 2 ? 1 : middle === 3 ? 2 : 3;
  const slots = new Array(middle).fill("");
  ["CO", "HJ", "LJ"].slice(0, lateCount).forEach((name, i) => {
    slots[middle - 1 - i] = name;
  });
  let early = 0;
  for (let i = 0; i < middle; i++) {
    if (!slots[i]) slots[i] = early++ === 0 ? "UTG" : `UTG+${early - 1}`;
  }
  return ["BTN", "SB", "BB", ...slots];
}

/** The profile's opening group of a position label (as the engine's `rfi_key`). */
export function positionGroup(label) {
  if (label.startsWith("UTG")) return "EP";
  if (label === "LJ" || label === "HJ") return "MP";
  return label;
}

/**
 * Plays one hand.
 *
 * `seats`: `[{ seat, name, stack, behaviour, memory }]` (stacks in cents, every
 * stack at least one big blind; `memory` is the player's own object kept
 * across hands, see `spotShare`). `table`: `{ name, maxSeats, sb, bb,
 * rakeCapBb }`. `header`: the first line. Returns the text lines and, per
 * player, the net result in cents and whether he put money in voluntarily.
 */
export function playHand({ table, seats, button, header, heroName, rng }) {
  const { sb, bb } = table;
  const deck = rng.shuffle(newDeck());
  const ring = [...seats].sort((a, b) => a.seat - b.seat);
  const at = ring.findIndex((s) => s.seat === button);
  if (at < 0) throw new Error(`button seat ${button} is empty`);
  const clockwise = ring.slice(at).concat(ring.slice(0, at));
  const labels = positionLabels(clockwise.length);
  const maxStack = Math.max(...ring.map((s) => s.stack));

  const players = clockwise.map((s, i) => {
    const cards = [deck.pop(), deck.pop()];
    const others = Math.max(...ring.filter((o) => o !== s).map((o) => o.stack));
    return {
      seat: s.seat,
      name: s.name,
      stack: s.stack,
      b: s.behaviour,
      memory: s.memory ?? {},
      label: labels[i],
      group: positionGroup(labels[i]),
      cards,
      pct: preflopPercentile(cards[0], cards[1], rng.next()),
      effBb: Math.min(s.stack, others) / bb,
      invested: 0,
      street: 0,
      folded: false,
      foldedAt: null,
      allIn: false,
      voluntary: false,
      limped: false,
      openThr: 0,
      threeBetThr: 0,
      canRaise: true,
      betLastStreet: false,
    };
  });
  void maxStack;
  const board = [deck.pop(), deck.pop(), deck.pop(), deck.pop(), deck.pop()];
  const heads = players.length === 2;
  const btn = players[0];
  const sbP = heads ? players[0] : players[1];
  const bbP = heads ? players[1] : players[2];
  const preflopOrder = heads ? [players[0], players[1]] : [...players.slice(3), players[0], players[1], players[2]];
  const postflopOrder = heads ? [players[1], players[0]] : [...players.slice(1), players[0]];

  const lines = [header];
  lines.push(`Table '${table.name}' ${table.maxSeats}-max Seat #${button} is the button`);
  for (const s of ring) lines.push(`Seat ${s.seat}: ${s.name} (${money(s.stack)} in chips)`);

  const left = (p) => p.stack - p.invested;
  const st = { currentBet: 0, lastInc: bb, raises: [], limpers: [], callers: [], street: 0, raisesThisStreet: 0 };
  const put = (p, amount) => {
    p.invested += amount;
    p.street += amount;
    if (left(p) === 0) p.allIn = true;
  };
  const allInText = (p) => (p.allIn ? " and is all-in" : "");

  // Blinds.
  for (const [p, amount, kind] of [
    [sbP, sb, "small"],
    [bbP, bb, "big"],
  ]) {
    put(p, Math.min(amount, p.stack));
    lines.push(`${p.name}: posts ${kind} blind ${money(p.street)}${allInText(p)}`);
  }
  st.currentBet = bb;
  lines.push("*** HOLE CARDS ***");
  const hero = players.find((p) => p.name === heroName);
  if (hero) lines.push(`Dealt to ${hero.name} [${hero.cards.join(" ")}]`);

  // ------------------------------------------------------------ actions

  const act = (p, decision) => {
    const facing = st.currentBet - p.street;
    if (decision.type === "raise" && (!p.canRaise || left(p) <= facing)) decision = { type: "call" };
    if (decision.type === "call" && facing === 0) decision = { type: "check" };
    if (decision.type === "check" && facing > 0) decision = { type: "fold" };
    const before = p.street;
    switch (decision.type) {
      case "fold":
        p.folded = true;
        p.foldedAt = st.street;
        lines.push(`${p.name}: folds`);
        return;
      case "check":
        lines.push(`${p.name}: checks`);
        return;
      case "call": {
        const amount = Math.min(facing, left(p));
        put(p, amount);
        if (st.street === 0) {
          p.voluntary = true;
          if (st.raises.length === 0 && !p.limped && p !== bbP) {
            p.limped = true;
            st.limpers.push(p);
          } else if (st.raises.length > 0) st.callers.push(p);
        }
        lines.push(`${p.name}: calls ${money(amount)}${allInText(p)}`);
        return;
      }
      default: {
        // Bet or raise to a street total, at least a full raise, at most all-in.
        const maxTo = p.street + left(p);
        let to = Math.round(decision.to);
        if (st.currentBet === 0) to = Math.max(to, bb);
        else to = Math.max(to, st.currentBet + st.lastInc);
        if (decision.allIn || to >= maxTo * 0.85) to = maxTo;
        to = Math.min(to, maxTo);
        const increment = to - st.currentBet;
        put(p, to - before);
        const full = increment >= st.lastInc;
        if (full) st.lastInc = increment;
        const verb = st.currentBet === 0 ? `bets ${money(to)}` : `raises ${money(increment)} to ${money(to)}`;
        lines.push(`${p.name}: ${verb}${allInText(p)}`);
        if (st.street === 0) {
          st.raises.push({ p, to, allIn: p.allIn, firstIn: st.raises.length === 0 && st.limpers.length === 0 });
          st.callers = [];
          p.voluntary = true;
        }
        st.currentBet = to;
        st.raisesThisStreet += 1;
        st.aggressor = p;
        return { reopen: full };
      }
    }
  };

  const live = () => players.filter((p) => !p.folded);
  const canAct = () => players.filter((p) => !p.folded && !p.allIn);

  const bettingRound = (order, decide) => {
    let pending = order.filter((p) => !p.folded && !p.allIn);
    const acted = new Set();
    while (pending.length > 0) {
      const p = pending.shift();
      if (p.folded || p.allIn) continue;
      if (live().length === 1) break;
      const facing = st.currentBet - p.street;
      const others = canAct().filter((o) => o !== p);
      if (others.length === 0 && facing <= 0) continue;
      const decision = decide(p, facing);
      if (others.length === 0 && decision.type === "raise") decision.type = facing > 0 ? "call" : "check";
      const result = act(p, decision);
      acted.add(p);
      if (result) {
        const i = order.indexOf(p);
        const after = order.slice(i + 1).concat(order.slice(0, i));
        pending = after.filter((o) => !o.folded && !o.allIn);
        if (!result.reopen) {
          for (const o of pending) if (acted.has(o)) o.canRaise = false;
        }
      }
    }
    // Return the part of the top bet nobody matched.
    const top = Math.max(...players.map((p) => p.street));
    const leaders = players.filter((p) => p.street === top);
    if (leaders.length === 1 && top > 0) {
      const second = Math.max(0, ...players.filter((p) => p !== leaders[0]).map((p) => p.street));
      const back = top - second;
      if (back > 0) {
        const p = leaders[0];
        p.invested -= back;
        p.street -= back;
        p.allIn = false;
        lines.push(`Uncalled bet (${money(back)}) returned to ${p.name}`);
      }
    }
  };

  const potNow = () => players.reduce((sum, p) => sum + p.invested, 0);

  // ------------------------------------------------------------ preflop

  bettingRound(preflopOrder, (p, facing) => decidePreflop(p, facing, st, { bb, rng, bbP, preflopOrder }));
  const preflopAggressor = st.raises.length ? st.raises[st.raises.length - 1].p : null;
  let lastAggressor = preflopAggressor;

  // ------------------------------------------------------------ postflop

  const boardText = [
    null,
    `*** FLOP *** [${board.slice(0, 3).join(" ")}]`,
    `*** TURN *** [${board.slice(0, 3).join(" ")}] [${board[3]}]`,
    `*** RIVER *** [${board.slice(0, 4).join(" ")}] [${board[4]}]`,
  ];
  let lastStreet = 0;
  let riverAggressor = null;
  for (let street = 1; street <= 3 && live().length > 1; street++) {
    for (const p of players) {
      p.street = 0;
      p.canRaise = true;
    }
    Object.assign(st, { currentBet: 0, lastInc: bb, street, raisesThisStreet: 0, aggressor: null });
    lines.push(boardText[street]);
    lastStreet = street;
    if (canAct().length < 2) continue;
    const shown = board.slice(0, street + 2);
    for (const p of live()) p.strength = postflopStrength(p.cards, shown);
    const prev = lastAggressor;
    const order = postflopOrder;
    bettingRound(order, (p, facing) => {
      const prevIndex = prev && !prev.folded ? order.indexOf(prev) : -1;
      return decidePostflop(p, facing, st, {
        street,
        pot: potNow(),
        rng,
        isPrevAggressor: p === prev,
        prevStillToAct: prevIndex > order.indexOf(p) && st.raisesThisStreet === 0,
        bettorIsPrevAggressor: st.aggressor === prev,
      });
    });
    for (const p of players) p.betLastStreet = st.aggressor === p;
    lastAggressor = st.aggressor || null;
    if (street === 3) riverAggressor = st.aggressor || null;
  }

  // ------------------------------------------------------------ showdown and pots

  const contenders = live();
  const showdown = contenders.length > 1;
  if (showdown) {
    for (let street = lastStreet + 1; street <= 3; street++) lines.push(boardText[street]);
    lastStreet = 3;
  }
  const pots = buildPots(players);
  const total = pots.reduce((sum, pot) => sum + pot.amount, 0);
  const rake = lastStreet >= 1 ? Math.min(Math.floor(total * 0.05), Math.round((table.rakeCapBb ?? 6) * bb)) : 0;
  takeRake(pots, rake);

  const collected = new Map();
  const hands = new Map();
  if (showdown) {
    lines.push("*** SHOW DOWN ***");
    for (const p of contenders) hands.set(p, evaluate([...p.cards, ...board]));
    const first = riverAggressor && !riverAggressor.folded ? riverAggressor : null;
    const order = first ? [first, ...postflopOrder.filter((p) => p !== first)] : postflopOrder;
    for (const p of order) {
      if (!p.folded) lines.push(`${p.name}: shows [${p.cards.join(" ")}] (${hands.get(p).text})`);
    }
  }
  const potName = (i) => (pots.length === 1 ? "pot" : i === 0 ? "main pot" : pots.length === 2 ? "side pot" : `side pot-${i}`);
  // Side pots are awarded first, as the client writes them.
  for (let i = pots.length - 1; i >= 0; i--) {
    const pot = pots[i];
    let winners = pot.eligible;
    if (showdown && winners.length > 1) {
      const best = Math.max(...winners.map((p) => hands.get(p).score));
      winners = winners.filter((p) => hands.get(p).score === best);
    }
    winners = postflopOrder.filter((p) => winners.includes(p));
    const share = Math.floor(pot.amount / winners.length);
    let odd = pot.amount - share * winners.length;
    for (const w of winners) {
      const amount = share + (odd > 0 ? 1 : 0);
      if (odd > 0) odd -= 1;
      collected.set(w, (collected.get(w) || 0) + amount);
      lines.push(`${w.name} collected ${money(amount)} from ${potName(i)}`);
    }
  }
  if (!showdown) {
    const [winner] = contenders;
    lines.push(`${winner.name}: doesn't show hand`);
  }

  lines.push("*** SUMMARY ***");
  const after = total - rake;
  if (pots.length === 1) lines.push(`Total pot ${money(total)} | Rake ${money(rake)}`);
  else {
    const parts = pots.map((pot, i) => {
      const name = i === 0 ? "Main pot" : pots.length === 2 ? "Side pot" : `Side pot-${i}`;
      return `${name} ${money(pot.amount)}.`;
    });
    lines.push(`Total pot ${money(total)} ${parts.join(" ")} | Rake ${money(rake)}`);
  }
  void after;
  if (lastStreet >= 1) lines.push(`Board [${board.slice(0, lastStreet + 2).join(" ")}]`);
  for (const p of [...players].sort((a, b) => a.seat - b.seat)) {
    let tags = "";
    if (p === btn) tags += " (button)";
    if (p === sbP) tags += " (small blind)";
    if (p === bbP) tags += " (big blind)";
    let text;
    if (p.folded) {
      const blind = p === sbP || p === bbP;
      text = `folded ${STREET_FOLD[p.foldedAt]}${p.foldedAt === 0 && !p.voluntary && !blind ? " (didn't bet)" : ""}`;
    } else if (showdown) {
      const won = collected.get(p);
      text = won
        ? `showed [${p.cards.join(" ")}] and won (${money(won)}) with ${hands.get(p).text}`
        : `showed [${p.cards.join(" ")}] and lost with ${hands.get(p).text}`;
    } else {
      text = `collected (${money(collected.get(p) || 0)})`;
    }
    lines.push(`Seat ${p.seat}: ${p.name}${tags} ${text}`);
  }

  const results = new Map();
  for (const p of players) {
    results.set(p.name, {
      net: (collected.get(p) || 0) - p.invested,
      vpip: p.voluntary,
      showdown: showdown && !p.folded,
    });
  }
  return { lines, results, rake, total };
}

/** Main pot first, then side pots, from the non-folded players' all-in levels. */
function buildPots(players) {
  const live = players.filter((p) => !p.folded);
  const levels = [...new Set(live.map((p) => p.invested))].sort((a, b) => a - b);
  const pots = [];
  let floor = 0;
  for (const level of levels) {
    const amount = players.reduce((sum, p) => sum + Math.max(0, Math.min(p.invested, level) - floor), 0);
    const eligible = live.filter((p) => p.invested >= level);
    // A level only one live player reached is still its own side pot: it
    // holds folded players' chips the shorter all-ins never covered.
    if (amount > 0) pots.push({ amount, eligible });
    floor = level;
  }
  return pots;
}

/** Rake comes out of the main pot first. */
function takeRake(pots, rake) {
  let rest = rake;
  for (const pot of pots) {
    const cut = Math.min(rest, pot.amount);
    pot.amount -= cut;
    rest -= cut;
  }
}

// ------------------------------------------------------------ decisions

const raise = (to, allIn = false) => ({ type: "raise", to, allIn });
const CALL = { type: "call" };
const FOLD = { type: "fold" };
const CHECK = { type: "check" };

function decidePreflop(p, facing, st, { bb, rng, bbP }) {
  const b = p.b;
  const pct = p.pct;
  const raises = st.raises;
  const last = raises[raises.length - 1];

  // Our open shoved over: answered like any 3-bet (the profile's fold to 3-bet).
  if (raises.length === 2 && raises[0].p === p && last.allIn) {
    return pct < p.openThr * (1 - b.foldTo3bet) ? CALL : FOLD;
  }
  // An all-in nobody has called yet.
  if (last && last.allIn && last.p !== p && st.callers.length === 0) {
    const shoveBb = (last.to) / bb;
    const thr = shoveBb <= 25 ? Math.min(0.6, b.callShove * 1.6) : b.callShove;
    return pct < thr ? CALL : FOLD;
  }

  if (raises.length === 0) {
    if (st.limpers.length === 0) {
      if (p === bbP) return CHECK;
      const open = b.open[p.group] ?? 0;
      if (p.effBb <= 15) {
        p.openThr = open;
        return pct < open ? raise(0, true) : FOLD;
      }
      if (pct < open) {
        p.openThr = open;
        return raise((b.openSize ?? 2.5) * bb);
      }
      const limp = typeof b.limp === "number" ? b.limp : b.limp?.[p.group] ?? 0;
      if (pct < open + limp) return CALL;
      return FOLD;
    }
    // Limpers, no raise.
    if (p.limped) return CHECK;
    if (pct < b.iso) {
      p.openThr = b.iso;
      return raise((4 + st.limpers.length) * bb);
    }
    if (p === bbP) return CHECK;
    if (pct < b.iso + b.overlimp) return CALL;
    return FOLD;
  }

  // We limped and somebody raised.
  if (p.limped && raises.length === 1) {
    const open = b.open[p.group] ?? 0;
    const limp = typeof b.limp === "number" ? b.limp : b.limp?.[p.group] ?? 0;
    const u = limp > 0 ? Math.min(0.999, Math.max(0, (pct - open) / limp)) : 0.5;
    if (pct < open || u < b.limpRaise) return raise(last.to * 3);
    if (u > 1 - b.limpFold) return FOLD;
    return CALL;
  }

  if (raises.length === 1) {
    if (p.voluntary) return pct < 0.1 ? CALL : FOLD;
    if (p.effBb <= 15) return pct < b.callShove * 1.5 ? raise(0, true) : FOLD;
    if (p.effBb <= 25 && b.reshove > 0) {
      if (pct < b.reshove) return raise(0, true);
      return pct < b.reshove + b.flat * 0.5 ? CALL : FOLD;
    }
    const squeeze = st.callers.length > 0;
    const tb = squeeze ? b.squeeze ?? b.threeBet : b.threeBet;
    let flat = p === bbP ? b.bbDefend : p.label === "SB" ? b.sbFlat ?? b.flat : b.flat;
    if (squeeze) flat *= 0.7;
    if (pct < tb) {
      p.threeBetThr = tb;
      const inPosition = p.label === "BTN" || (p.label !== "SB" && p.label !== "BB");
      return raise(last.to * (inPosition ? 3 : 3.6) + st.callers.length * last.to);
    }
    if (pct < tb + flat) return CALL;
    return FOLD;
  }

  if (raises.length === 2) {
    if (raises[0].p === p) {
      const cont = p.openThr * (1 - b.foldTo3bet);
      if (pct < cont * b.fourBet) return raise(last.to * 2.3);
      if (pct < cont) return CALL;
      return FOLD;
    }
    if (pct < 0.025) return raise(last.to * 2.3);
    return pct < 0.045 ? CALL : FOLD;
  }

  if (raises.length === 3 && raises[1].p === p) {
    const cont = p.threeBetThr * (1 - b.foldTo4bet);
    if (pct < cont * 0.5) return raise(0, true);
    return pct < cont ? CALL : FOLD;
  }
  void rng;
  return pct < 0.02 ? raise(0, true) : pct < 0.03 ? CALL : FOLD;
}

/**
 * The share of good-or-better hands this player has held in this kind of
 * spot so far (a running estimate started from `STRONG_SHARE`), updated
 * with the current hand. Mixing on it makes a profile's frequency come out
 * at its target whatever ranges actually reach the spot, while good hands
 * are still the ones that bet and continue.
 */
function spotShare(p, key, street, strong) {
  const m = (p.memory[key] ??= { strong: STRONG_SHARE[street] * SHARE_PRIOR_WEIGHT, n: SHARE_PRIOR_WEIGHT });
  const q = Math.min(0.95, Math.max(0.05, m.strong / m.n));
  m.strong += strong ? 1 : 0;
  m.n += 1;
  return q;
}

/** Bets with frequency `f`, good hands first. */
function mixBet(rng, f, strong, q) {
  const p = strong ? Math.min(1, f / q) : Math.max(0, (f - q) / (1 - q));
  return rng.chance(p);
}

/** Folds with frequency `f`, weak hands first. */
function mixFold(rng, f, strong, q) {
  const p = strong ? Math.max(0, (f - (1 - q)) / q) : Math.min(1, f / (1 - q));
  return rng.chance(p);
}

function decidePostflop(p, facing, st, ctx) {
  const b = p.b;
  const { street, pot, rng } = ctx;
  const s = p.strength;
  const strong = s >= 2;
  const size = (frac) => Math.max(1, Math.round(pot * frac));

  if (facing === 0) {
    let f;
    if (ctx.isPrevAggressor) {
      f = street === 1 ? b.cbet : p.betLastStreet ? (street === 2 ? b.barrel : b.riverBarrel) : b.stab;
    } else if (ctx.prevStillToAct) f = b.donk;
    else f = b.stab;
    const kind = ctx.isPrevAggressor ? (p.betLastStreet || street === 1 ? "barrel" : "stab") : ctx.prevStillToAct ? "donk" : "stab";
    const q = spotShare(p, `${kind}${street}`, street, strong);
    return mixBet(rng, f, strong, q) ? raise(size(b.betSize)) : CHECK;
  }

  if (st.raisesThisStreet >= 2) {
    if (s === 3) return rng.chance(0.35) ? raise(0, true) : CALL;
    if (s === 2) return rng.chance(b.callRaise ?? 0.5) ? CALL : FOLD;
    return rng.chance(0.08) ? CALL : FOLD;
  }
  const f =
    street === 1 ? (ctx.bettorIsPrevAggressor ? b.foldToCbet : b.foldToBet ?? b.foldToCbet) : street === 2 ? b.foldToTurn : b.foldToRiver;
  if (s === 3 && rng.chance(Math.min(1, b.raiseVsBet / 0.12))) return raise(st.currentBet * 3 + Math.round(pot * 0.2));
  const q = spotShare(p, `face${street}${street === 1 && ctx.bettorIsPrevAggressor ? "c" : ""}`, street, strong);
  if (mixFold(rng, f, strong, q)) return FOLD;
  return CALL;
}
