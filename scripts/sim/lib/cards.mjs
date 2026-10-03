// Cards, a 7-card hand evaluator with PokerStars' showdown wording, a
// preflop hand ranking and a coarse postflop strength class.

export const RANKS = "23456789TJQKA";
export const SUITS = "cdhs";

const SINGULAR = ["Deuce", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten", "Jack", "Queen", "King", "Ace"];
const PLURAL = ["Deuces", "Threes", "Fours", "Fives", "Sixes", "Sevens", "Eights", "Nines", "Tens", "Jacks", "Queens", "Kings", "Aces"];

export const CATEGORY = {
  HIGH_CARD: 0,
  PAIR: 1,
  TWO_PAIR: 2,
  TRIPS: 3,
  STRAIGHT: 4,
  FLUSH: 5,
  FULL_HOUSE: 6,
  QUADS: 7,
  STRAIGHT_FLUSH: 8,
};

/** Rank value 2..14 of a card such as `Ah`. */
export function rankOf(card) {
  return RANKS.indexOf(card[0]) + 2;
}

export function suitOf(card) {
  return card[1];
}

export function newDeck() {
  const deck = [];
  for (const r of RANKS) for (const s of SUITS) deck.push(r + s);
  return deck;
}

const single = (v) => SINGULAR[v - 2];
const plural = (v) => PLURAL[v - 2];

/** Highest straight in a set of rank values (5 for the wheel), or 0. */
function straightHigh(values) {
  const set = new Set(values);
  if (set.has(14)) set.add(1);
  for (let high = 14; high >= 5; high--) {
    let ok = true;
    for (let v = high; v > high - 5; v--) {
      if (!set.has(v)) {
        ok = false;
        break;
      }
    }
    if (ok) return high;
  }
  return 0;
}

/**
 * Best five-card hand of 5 to 7 cards: `{ category, ranks, score, text }`.
 * `score` orders hands (higher wins, equal ties); `text` is the PokerStars
 * description ("a pair of Tens", "two pair, Kings and Sevens", ...).
 */
export function evaluate(cards) {
  const values = cards.map(rankOf);
  const bySuit = new Map();
  for (const c of cards) {
    const s = suitOf(c);
    if (!bySuit.has(s)) bySuit.set(s, []);
    bySuit.get(s).push(rankOf(c));
  }
  const counts = new Map();
  for (const v of values) counts.set(v, (counts.get(v) || 0) + 1);
  // Groups ordered by count, then rank.
  const groups = [...counts.entries()].sort((a, b) => b[1] - a[1] || b[0] - a[0]);
  const desc = [...values].sort((a, b) => b - a);

  let category;
  let ranks;
  const flushSuit = [...bySuit.entries()].find(([, vs]) => vs.length >= 5);
  const sfHigh = flushSuit ? straightHigh(flushSuit[1]) : 0;
  const stHigh = straightHigh(values);

  if (sfHigh) {
    category = CATEGORY.STRAIGHT_FLUSH;
    ranks = [sfHigh];
  } else if (groups[0][1] === 4) {
    category = CATEGORY.QUADS;
    const quad = groups[0][0];
    ranks = [quad, desc.find((v) => v !== quad)];
  } else if (groups[0][1] === 3 && groups.length > 1 && groups[1][1] >= 2) {
    category = CATEGORY.FULL_HOUSE;
    // A second set plays as the pair part.
    const trips = groups[0][0];
    const pair = Math.max(...groups.slice(1).filter(([, n]) => n >= 2).map(([v]) => v));
    ranks = [trips, pair];
  } else if (flushSuit) {
    category = CATEGORY.FLUSH;
    ranks = [...flushSuit[1]].sort((a, b) => b - a).slice(0, 5);
  } else if (stHigh) {
    category = CATEGORY.STRAIGHT;
    ranks = [stHigh];
  } else if (groups[0][1] === 3) {
    category = CATEGORY.TRIPS;
    const trips = groups[0][0];
    ranks = [trips, ...desc.filter((v) => v !== trips).slice(0, 2)];
  } else if (groups[0][1] === 2 && groups.length > 1 && groups[1][1] === 2) {
    category = CATEGORY.TWO_PAIR;
    const pairs = groups.filter(([, n]) => n === 2).map(([v]) => v).sort((a, b) => b - a);
    const [hi, lo] = pairs;
    ranks = [hi, lo, desc.find((v) => v !== hi && v !== lo)];
  } else if (groups[0][1] === 2) {
    category = CATEGORY.PAIR;
    const pair = groups[0][0];
    ranks = [pair, ...desc.filter((v) => v !== pair).slice(0, 3)];
  } else {
    category = CATEGORY.HIGH_CARD;
    ranks = desc.slice(0, 5);
  }

  let score = category;
  for (let i = 0; i < 5; i++) score = score * 16 + (ranks[i] || 0);
  return { category, ranks, score, text: describe(category, ranks) };
}

function describe(category, ranks) {
  switch (category) {
    case CATEGORY.HIGH_CARD:
      return `high card ${single(ranks[0])}`;
    case CATEGORY.PAIR:
      return `a pair of ${plural(ranks[0])}`;
    case CATEGORY.TWO_PAIR:
      return `two pair, ${plural(ranks[0])} and ${plural(ranks[1])}`;
    case CATEGORY.TRIPS:
      return `three of a kind, ${plural(ranks[0])}`;
    case CATEGORY.STRAIGHT:
      return `a straight, ${single(ranks[0] === 5 ? 14 : ranks[0] - 4)} to ${single(ranks[0])}`;
    case CATEGORY.FLUSH:
      return `a flush, ${single(ranks[0])} high`;
    case CATEGORY.FULL_HOUSE:
      return `a full house, ${plural(ranks[0])} full of ${plural(ranks[1])}`;
    case CATEGORY.QUADS:
      return `four of a kind, ${plural(ranks[0])}`;
    default:
      return ranks[0] === 14
        ? "a Royal Flush"
        : `a straight flush, ${single(ranks[0] === 5 ? 14 : ranks[0] - 4)} to ${single(ranks[0])}`;
  }
}

// ------------------------------------------------------------ preflop ranking

/** Chen-formula points of a starting hand (higher is stronger). */
function chen(hi, lo, suited) {
  const top = hi === 14 ? 10 : hi === 13 ? 8 : hi === 12 ? 7 : hi === 11 ? 6 : hi / 2;
  if (hi === lo) return Math.max(5, top * 2);
  let points = top + (suited ? 2 : 0);
  const gap = hi - lo - 1;
  points -= gap === 0 ? 0 : gap === 1 ? 1 : gap === 2 ? 2 : gap === 3 ? 4 : 5;
  if (gap <= 1 && hi < 12) points += 1;
  return points;
}

/** The 169 starting-hand classes, strongest first, with their combo share. */
function buildClasses() {
  const classes = [];
  for (let hi = 14; hi >= 2; hi--) {
    for (let lo = hi; lo >= 2; lo--) {
      if (hi === lo) classes.push({ key: `${hi}-${lo}-p`, hi, lo, combos: 6, points: chen(hi, lo, false) });
      else {
        classes.push({ key: `${hi}-${lo}-s`, hi, lo, combos: 4, points: chen(hi, lo, true) });
        classes.push({ key: `${hi}-${lo}-o`, hi, lo, combos: 12, points: chen(hi, lo, false) });
      }
    }
  }
  classes.sort((a, b) => b.points - a.points || b.hi - a.hi || b.lo - a.lo || a.combos - b.combos);
  let before = 0;
  for (const c of classes) {
    c.before = before / 1326;
    c.share = c.combos / 1326;
    before += c.combos;
  }
  return new Map(classes.map((c) => [c.key, c]));
}

const CLASSES = buildClasses();

export function classKey(c1, c2) {
  const [a, b] = [rankOf(c1), rankOf(c2)];
  const hi = Math.max(a, b);
  const lo = Math.min(a, b);
  if (hi === lo) return `${hi}-${lo}-p`;
  return `${hi}-${lo}-${suitOf(c1) === suitOf(c2) ? "s" : "o"}`;
}

/**
 * Where a starting hand sits in the ranking, as a share of all 1326 combos
 * (0 = best). `u` in [0, 1) places it uniformly inside its own class, so a
 * "top X%" range is entered with probability exactly X.
 */
export function preflopPercentile(c1, c2, u) {
  const c = CLASSES.get(classKey(c1, c2));
  return c.before + u * c.share;
}

// ----------------------------------------------------------- postflop strength

/**
 * Coarse made-hand class with the board: 3 strong (two pair or better that
 * uses a hole card), 2 good (top pair or an overpair), 1 marginal (a lower
 * pair or a flush/straight draw before the river), 0 nothing.
 */
export function postflopStrength(hole, board) {
  const mine = evaluate([...hole, ...board]);
  const onBoard = board.length >= 5 ? evaluate(board) : null;
  const boardCategory = onBoard ? onBoard.category : boardPairCategory(board);
  if (mine.category >= CATEGORY.TWO_PAIR && mine.category > boardCategory) return 3;
  const top = Math.max(...board.map(rankOf));
  const holeValues = hole.map(rankOf);
  if (mine.category === CATEGORY.PAIR || (mine.category === CATEGORY.TWO_PAIR && boardCategory === CATEGORY.PAIR)) {
    const pocket = holeValues[0] === holeValues[1];
    if (pocket && holeValues[0] > top) return 2;
    if (holeValues.includes(top) && board.some((c) => rankOf(c) === top)) return 2;
    if (pocket || holeValues.some((v) => board.some((c) => rankOf(c) === v))) return 1;
  }
  if (board.length < 5 && hasDraw(hole, board)) return 1;
  return 0;
}

function boardPairCategory(board) {
  const counts = new Map();
  for (const c of board) counts.set(rankOf(c), (counts.get(rankOf(c)) || 0) + 1);
  const max = Math.max(...counts.values());
  if (max >= 3) return CATEGORY.TRIPS;
  const pairs = [...counts.values()].filter((n) => n === 2).length;
  return pairs >= 2 ? CATEGORY.TWO_PAIR : pairs === 1 ? CATEGORY.PAIR : CATEGORY.HIGH_CARD;
}

function hasDraw(hole, board) {
  const all = [...hole, ...board];
  for (const s of SUITS) {
    const n = all.filter((c) => suitOf(c) === s).length;
    if (n === 4 && hole.some((c) => suitOf(c) === s)) return true;
  }
  const values = new Set(all.map(rankOf));
  if (values.has(14)) values.add(1);
  for (let low = 1; low <= 10; low++) {
    let run = 0;
    for (let v = low; v < low + 4; v++) if (values.has(v)) run++;
    const usesHole = hole.some((c) => {
      const v = rankOf(c);
      return (v >= low && v < low + 4) || (v === 14 && low === 1);
    });
    if (run === 4 && usesHole && low >= 2 && low <= 10) return true;
  }
  return false;
}
