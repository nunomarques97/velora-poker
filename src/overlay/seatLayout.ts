/**
 * Pure seat-layout engine for the overlay HUD: where each player's chip goes
 * on a PokerStars table window, for every table size from 2 to 10 seats.
 *
 * No React, no Tauri, no DOM: every function here is a plain calculation on
 * numbers, so it runs unchanged in the overlay and in `node:test`
 * (`npm run test:layout`).
 *
 * Coordinates: every position and rectangle is a *fraction* (0..1) of the
 * overlay window, which covers the table window. PokerStars scales the whole
 * table picture with its window, so a fraction of the window stays over the
 * same piece of felt at any size. Chip sizes are the exception: they are in
 * CSS pixels (text must stay readable), which is why placement and collision
 * checks are done in pixels for one concrete window size and only the result
 * is turned back into fractions.
 *
 * Frames (`SeatFrame`), the same keys `seat_positions` stores:
 * - 'hero': the seat key is the seat's offset from the hero (0 = hero). With
 *   PokerStars' "Auto-Center me" on, the hero is always drawn at the bottom
 *   centre, so an offset is a fixed screen slot. Offsets run clockwise on
 *   screen: bottom, left, top, right.
 * - 'absolute': the seat key is PokerStars' own seat number (1..maxPlayers),
 *   used when Auto-Center is off and seat N is always drawn in the same slot.
 *
 * ASSUMED geometry. Nothing below was measured on a live client except the
 * 6-max rows it starts from:
 * - The 6-max seat plates come from the six hand-calibrated 6-max rows in
 *   `src-tauri/src/db/mod.rs` (`BUILTIN_SEAT_TEMPLATES`): those are the
 *   top-left corners of a 168x76 px card on an ~800x570 table, so each was
 *   moved by half a card (+0.105, +0.067) to get a centre, then mirrored
 *   around the window's vertical centre line (the measured rows lean ~0.03
 *   to the left) and nudged onto the seat plate the card was dragged next to.
 * - Every other size, the plate and hole-card sizes, the board band, the
 *   bet-chip ring and the action-button block are derived from that 6-max
 *   picture of a PokerStars table, not measured.
 * - PokerStars' minimum table window is about 483x359 px (third-party
 *   figure; newer clients may enforce a larger minimum) and its default is
 *   about 800x570 px.
 * A user's drag always overrides a default, so a wrong assumption costs one
 * drag per seat and table size, never per table.
 */

export type SeatFrame = "hero" | "absolute";

/** The chip designs the layout knows the size of. */
export type ChipModel = "compact" | "badge";

/** A point as fractions (0..1) of the overlay window. */
export interface Point {
  x: number;
  y: number;
}

/** A rectangle as fractions of the overlay window: left, top, width, height. */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** A saved chip centre, the shape `get_seat_positions` returns. */
export interface SeatOverride {
  seatKey: number;
  x: number;
  y: number;
}

/** The centre of one seat's name/stack plate, by seat key. */
export interface SeatAnchor {
  seatKey: number;
  /** Screen slot, clockwise from the bottom-centre slot (0). */
  slot: number;
  x: number;
  y: number;
}

export type ProtectedZoneKind = "board" | "bet" | "holeCards" | "actions";

/** Part of the table the HUD must never cover. */
export interface ProtectedZone {
  kind: ProtectedZoneKind;
  rect: Rect;
  /** The seat a 'bet' or 'holeCards' zone belongs to. */
  seatKey?: number;
}

export interface ChipMetrics {
  /** 1 at the medium table size, never below the readable-text floor. */
  scale: number;
  fontPx: number;
  widthPx: number;
  heightPx: number;
}

export interface ChipPlacement {
  /** `null` for a spare slot (a player with no resolvable seat). */
  seatKey: number | null;
  /** Chip centre, as fractions of the window. */
  x: number;
  y: number;
  /** True when the position is the user's saved override. */
  overridden: boolean;
}

export interface LayoutInput {
  maxPlayers: number;
  frame: SeatFrame;
  windowW: number;
  windowH: number;
  /** Seat keys to return; every seat of the table when omitted. */
  seats?: readonly number[];
  /** Saved chip centres. Kept exactly, only clamped into the window. */
  overrides?: readonly SeatOverride[];
  /** Chip design to size for; 'compact' when omitted. */
  model?: ChipModel;
  /** Measured chip size in CSS px, used instead of the model's nominal size. */
  chip?: { widthPx: number; heightPx: number };
  /** 'absolute' frame only: the hero's seat number, to protect their hole cards. */
  heroSeatKey?: number | null;
  /** Extra collision-free slots for players without a seat key. */
  spareCount?: number;
}

export interface SeatLayout {
  chip: ChipMetrics;
  /** One entry per requested seat, in the order `seats` gave them. */
  positions: ChipPlacement[];
  spares: ChipPlacement[];
}

/** PokerStars' minimum table window, in px. ASSUMED (third-party figure). */
export const MIN_TABLE_SIZE = { width: 483, height: 359 } as const;
/** PokerStars' default table window, in px. ASSUMED. */
export const MEDIUM_TABLE_SIZE = { width: 800, height: 570 } as const;

export const MIN_MAX_PLAYERS = 2;
export const MAX_MAX_PLAYERS = 10;

/** Chip text size at the medium table size, and the readable floor. */
export const BASE_FONT_PX = 12;
export const MIN_FONT_PX = 10;

/**
 * Nominal chip size at scale 1 (the medium table). 'compact' fits one line
 * of three stats plus a hand count ("24/18/7 132") in tabular digits;
 * 'badge' fits two initials plus a hand count.
 */
const CHIP_BASE_PX: Record<ChipModel, { width: number; height: number }> = {
  compact: { width: 84, height: 20 },
  badge: { width: 56, height: 20 },
};

/** Centre of the felt. ASSUMED, from the 6-max plates below. */
export const TABLE_CENTRE: Point = { x: 0.5, y: 0.47 };

/** Seat plate (name + stack box). ASSUMED: ~120x46 px on an 800x570 table. */
export const SEAT_PLATE = { w: 0.15, h: 0.08 } as const;

/**
 * A seat's hole cards, drawn above its plate and slightly overlapping it.
 * ASSUMED: ~96x74 px on an 800x570 table.
 */
export const HOLE_CARDS = { w: 0.12, h: 0.13, overlap: 0.02 } as const;

/**
 * Bet chips sit on the line from a seat to the table centre, this far along
 * it, in a box this size. ASSUMED.
 */
export const BET_RING = { t: 0.42, w: 0.09, h: 0.06 } as const;

/** Community cards and the pot text above them. ASSUMED. */
export const BOARD_ZONE: Rect = { x: 0.33, y: 0.33, w: 0.34, h: 0.24 };

/** Fold/Check/Bet buttons and the bet slider, bottom right. ASSUMED. */
export const ACTIONS_ZONE: Rect = { x: 0.56, y: 0.83, w: 0.44, h: 0.17 };

/**
 * Seat plate centres in the hero frame, by offset (slot). 6-max derives from
 * the measured rows (see the module comment); 9-max follows the same
 * PokerStars picture with a bottom-centre hero, a pair on each lower side,
 * a pair on each upper side and a pair at the top. Both ASSUMED.
 */
const SEAT_PLATES: Partial<Record<number, readonly Point[]>> = {
  6: [
    { x: 0.5, y: 0.79 },
    { x: 0.12, y: 0.58 },
    { x: 0.12, y: 0.28 },
    { x: 0.5, y: 0.19 },
    { x: 0.88, y: 0.28 },
    { x: 0.88, y: 0.58 },
  ],
  9: [
    { x: 0.5, y: 0.79 },
    { x: 0.25, y: 0.75 },
    { x: 0.09, y: 0.57 },
    { x: 0.09, y: 0.33 },
    { x: 0.3, y: 0.18 },
    { x: 0.7, y: 0.18 },
    { x: 0.91, y: 0.33 },
    { x: 0.91, y: 0.57 },
    { x: 0.75, y: 0.75 },
  ],
};

/**
 * Every other size: evenly spaced on an ellipse through the 6-max top and
 * bottom plates, hero at the bottom, clockwise. ASSUMED.
 */
const RING_RADII = { x: 0.4, y: 0.3 } as const;
const RING_LIMITS = { minX: 0.09, maxX: 0.91, minY: 0.18, maxY: 0.79 } as const;

/** Gap between a chip and the plate it belongs to, and between chips, in px. */
const PLATE_GAP_PX = 2;
const CHIP_GAP_PX = 2;

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function validMaxPlayers(maxPlayers: number): number {
  if (!Number.isFinite(maxPlayers)) return 6;
  return clamp(Math.round(maxPlayers), MIN_MAX_PLAYERS, MAX_MAX_PLAYERS);
}

/** The seat keys a table of this size has in this frame. */
export function seatKeys(maxPlayers: number, frame: SeatFrame): number[] {
  const n = validMaxPlayers(maxPlayers);
  return Array.from({ length: n }, (_, i) => (frame === "hero" ? i : i + 1));
}

/**
 * The slot PokerStars' seat 1 is drawn in with Auto-Center off: the first
 * slot clockwise after the top of the table. ASSUMED.
 */
function absoluteSeatOneSlot(n: number): number {
  return (Math.floor(n / 2) + 1) % n;
}

/** Screen slot of a seat key, or `null` if the key is not a seat of this table. */
export function seatSlot(maxPlayers: number, frame: SeatFrame, seatKey: number): number | null {
  const n = validMaxPlayers(maxPlayers);
  if (!Number.isInteger(seatKey)) return null;
  if (frame === "hero") return seatKey >= 0 && seatKey < n ? seatKey : null;
  if (seatKey < 1 || seatKey > n) return null;
  return (seatKey - 1 + absoluteSeatOneSlot(n)) % n;
}

function slotPlate(n: number, slot: number): Point {
  const table = SEAT_PLATES[n];
  if (table) return table[slot];
  const theta = ((90 + (slot * 360) / n) * Math.PI) / 180;
  return {
    x: clamp(TABLE_CENTRE.x + RING_RADII.x * Math.cos(theta), RING_LIMITS.minX, RING_LIMITS.maxX),
    y: clamp(TABLE_CENTRE.y + RING_RADII.y * Math.sin(theta), RING_LIMITS.minY, RING_LIMITS.maxY),
  };
}

/** Default seat plate centres (the chips' anchors), one per seat key. */
export function defaultSeatAnchors(maxPlayers: number, frame: SeatFrame): SeatAnchor[] {
  const n = validMaxPlayers(maxPlayers);
  return seatKeys(n, frame).map((seatKey) => {
    const slot = seatSlot(n, frame, seatKey) as number;
    const plate = slotPlate(n, slot);
    return { seatKey, slot, x: plate.x, y: plate.y };
  });
}

function centredRect(centre: Point, w: number, h: number): Rect {
  return { x: centre.x - w / 2, y: centre.y - h / 2, w, h };
}

/** A seat's plate rectangle, as fractions. */
export function seatPlateRect(anchor: Point): Rect {
  return centredRect(anchor, SEAT_PLATE.w, SEAT_PLATE.h);
}

/** Where a seat's hole cards are drawn. ASSUMED, see `HOLE_CARDS`. */
export function holeCardsRect(anchor: Point): Rect {
  const top = anchor.y - SEAT_PLATE.h / 2 + HOLE_CARDS.overlap - HOLE_CARDS.h;
  return { x: anchor.x - HOLE_CARDS.w / 2, y: top, w: HOLE_CARDS.w, h: HOLE_CARDS.h };
}

function betRect(anchor: Point): Rect {
  const centre = {
    x: anchor.x + (TABLE_CENTRE.x - anchor.x) * BET_RING.t,
    y: anchor.y + (TABLE_CENTRE.y - anchor.y) * BET_RING.t,
  };
  return centredRect(centre, BET_RING.w, BET_RING.h);
}

/**
 * The parts of the table the HUD must not cover: the board band, every
 * seat's bet chips, the hero's hole cards and the action buttons. The hero
 * is seat key 0 in the 'hero' frame; in the 'absolute' frame their hole
 * cards are protected only when `heroSeatKey` is known.
 */
export function protectedZones(
  maxPlayers: number,
  frame: SeatFrame,
  heroSeatKey?: number | null,
): ProtectedZone[] {
  const anchors = defaultSeatAnchors(maxPlayers, frame);
  const hero = frame === "hero" ? 0 : heroSeatKey ?? null;
  const zones: ProtectedZone[] = [{ kind: "board", rect: BOARD_ZONE }];
  for (const anchor of anchors) {
    zones.push({ kind: "bet", rect: betRect(anchor), seatKey: anchor.seatKey });
  }
  const heroAnchor = anchors.find((a) => a.seatKey === hero);
  if (heroAnchor) {
    zones.push({ kind: "holeCards", rect: holeCardsRect(heroAnchor), seatKey: heroAnchor.seatKey });
  }
  zones.push({ kind: "actions", rect: ACTIONS_ZONE });
  return zones;
}

function usableSize(windowW: number, windowH: number): { w: number; h: number } {
  const ok = (v: number) => Number.isFinite(v) && v > 0;
  return ok(windowW) && ok(windowH)
    ? { w: windowW, h: windowH }
    : { w: MEDIUM_TABLE_SIZE.width, h: MEDIUM_TABLE_SIZE.height };
}

/**
 * Chip scale for a window: proportional to the table below the medium size,
 * floored so the text never drops under `MIN_FONT_PX`, and capped at 1 so a
 * big table does not grow big chips.
 */
export function chipScale(windowW: number, windowH: number): number {
  const { w, h } = usableSize(windowW, windowH);
  const proportional = Math.min(w / MEDIUM_TABLE_SIZE.width, h / MEDIUM_TABLE_SIZE.height);
  return clamp(proportional, MIN_FONT_PX / BASE_FONT_PX, 1);
}

/** Chip font and box size, in CSS px, for a window and chip design. */
export function chipMetrics(windowW: number, windowH: number, model: ChipModel = "compact"): ChipMetrics {
  const scale = chipScale(windowW, windowH);
  const base = CHIP_BASE_PX[model] ?? CHIP_BASE_PX.compact;
  return {
    scale,
    fontPx: Math.max(MIN_FONT_PX, BASE_FONT_PX * scale),
    widthPx: base.width * scale,
    heightPx: base.height * scale,
  };
}

/** A rectangle in window pixels. */
interface Box {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

function toBox(rect: Rect, w: number, h: number): Box {
  return { x0: rect.x * w, y0: rect.y * h, x1: (rect.x + rect.w) * w, y1: (rect.y + rect.h) * h };
}

function overlapArea(a: Box, b: Box): number {
  const dx = Math.min(a.x1, b.x1) - Math.max(a.x0, b.x0);
  const dy = Math.min(a.y1, b.y1) - Math.max(a.y0, b.y0);
  return dx > 0 && dy > 0 ? dx * dy : 0;
}

function grow(box: Box, by: number): Box {
  return { x0: box.x0 - by, y0: box.y0 - by, x1: box.x1 + by, y1: box.y1 + by };
}

/** Whether two fraction rectangles overlap with a positive area. */
export function rectsOverlap(a: Rect, b: Rect): boolean {
  return (
    Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x) > 1e-9 &&
    Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y) > 1e-9
  );
}

/** The rectangle a chip centred on `centre` covers, as fractions. */
export function chipRect(centre: Point, chip: ChipMetrics, windowW: number, windowH: number): Rect {
  const { w, h } = usableSize(windowW, windowH);
  return centredRect(centre, chip.widthPx / w, chip.heightPx / h);
}

/**
 * Chebyshev rings of grid offsets around a point, each ring sorted by
 * distance and then a fixed angular order, so the search is deterministic.
 */
function ringOffsets(r: number, cache: Map<number, Array<[number, number]>>): Array<[number, number]> {
  const cached = cache.get(r);
  if (cached) return cached;
  const ring: Array<[number, number]> = [];
  if (r === 0) {
    ring.push([0, 0]);
  } else {
    for (let i = -r; i <= r; i++) {
      ring.push([i, -r], [i, r]);
    }
    for (let j = -r + 1; j <= r - 1; j++) {
      ring.push([-r, j], [r, j]);
    }
  }
  const angle = ([i, j]: [number, number]) => {
    const a = Math.atan2(j, i);
    return a < 0 ? a + 2 * Math.PI : a;
  };
  ring.sort((a, b) => a[0] ** 2 + a[1] ** 2 - (b[0] ** 2 + b[1] ** 2) || angle(a) - angle(b));
  cache.set(r, ring);
  return ring;
}

interface SearchContext {
  w: number;
  h: number;
  cw: number;
  ch: number;
  step: number;
  hard: Box[];
  soft: Box[];
  rings: Map<number, Array<[number, number]>>;
}

interface Candidate {
  cx: number;
  cy: number;
}

function boxAt(ctx: SearchContext, cx: number, cy: number): Box {
  return { x0: cx - ctx.cw / 2, y0: cy - ctx.ch / 2, x1: cx + ctx.cw / 2, y1: cy + ctx.ch / 2 };
}

function inBounds(ctx: SearchContext, cx: number, cy: number): boolean {
  const eps = 1e-9;
  return (
    cx - ctx.cw / 2 >= -eps &&
    cy - ctx.ch / 2 >= -eps &&
    cx + ctx.cw / 2 <= ctx.w + eps &&
    cy + ctx.ch / 2 <= ctx.h + eps
  );
}

function clampCentre(ctx: SearchContext, cx: number, cy: number): Candidate {
  const halfW = Math.min(ctx.cw / 2, ctx.w / 2);
  const halfH = Math.min(ctx.ch / 2, ctx.h / 2);
  return { cx: clamp(cx, halfW, ctx.w - halfW), cy: clamp(cy, halfH, ctx.h - halfH) };
}

function hits(box: Box, obstacles: Box[]): boolean {
  return obstacles.some((o) => overlapArea(box, o) > 0);
}

/**
 * The feasible grid point nearest to `start`, or `null`. `useSoft` also
 * avoids seat plates; `outward` (a unit vector) keeps the chip on the side
 * of its plate facing away from the table centre.
 */
function nearestFree(
  ctx: SearchContext,
  start: Candidate,
  useSoft: boolean,
  outward: { plate: Candidate; dx: number; dy: number } | null,
): Candidate | null {
  const maxRing = Math.ceil(Math.max(ctx.w, ctx.h) / ctx.step);
  let best: Candidate | null = null;
  let bestD2 = Infinity;
  for (let r = 0; r <= maxRing; r++) {
    if ((r * ctx.step) ** 2 > bestD2) break;
    for (const [i, j] of ringOffsets(r, ctx.rings)) {
      const d2 = (i * ctx.step) ** 2 + (j * ctx.step) ** 2;
      if (d2 >= bestD2) continue;
      const cx = start.cx + i * ctx.step;
      const cy = start.cy + j * ctx.step;
      if (!inBounds(ctx, cx, cy)) continue;
      if (outward && (cx - outward.plate.cx) * outward.dx + (cy - outward.plate.cy) * outward.dy <= 0) continue;
      const box = boxAt(ctx, cx, cy);
      if (hits(box, ctx.hard) || (useSoft && hits(box, ctx.soft))) continue;
      best = { cx, cy };
      bestD2 = d2;
    }
  }
  return best;
}

/** Last resort when nothing is free: the in-bounds point overlapping least. */
function leastOverlap(ctx: SearchContext, start: Candidate): Candidate {
  let best = clampCentre(ctx, start.cx, start.cy);
  let bestCost = Infinity;
  let bestD2 = Infinity;
  const nx = Math.max(0, Math.floor((ctx.w - ctx.cw) / ctx.step));
  const ny = Math.max(0, Math.floor((ctx.h - ctx.ch) / ctx.step));
  for (let a = 0; a <= nx; a++) {
    for (let b = 0; b <= ny; b++) {
      const { cx, cy } = clampCentre(ctx, ctx.cw / 2 + a * ctx.step, ctx.ch / 2 + b * ctx.step);
      const box = boxAt(ctx, cx, cy);
      const cost = ctx.hard.reduce((sum, o) => sum + overlapArea(box, o), 0);
      const d2 = (cx - start.cx) ** 2 + (cy - start.cy) ** 2;
      if (cost < bestCost || (cost === bestCost && d2 < bestD2)) {
        best = { cx, cy };
        bestCost = cost;
        bestD2 = d2;
      }
    }
  }
  return best;
}

function place(
  ctx: SearchContext,
  start: Candidate,
  outward: { plate: Candidate; dx: number; dy: number } | null,
): Candidate {
  return (
    nearestFree(ctx, start, true, outward) ??
    nearestFree(ctx, start, false, outward) ??
    nearestFree(ctx, start, false, null) ??
    leastOverlap(ctx, start)
  );
}

/**
 * Where a chip first tries to go: just outside its plate, on the side facing
 * most directly away from the table centre among the sides where the chip
 * fits in the window.
 */
function outerAnchor(
  ctx: SearchContext,
  plate: Candidate,
): { start: Candidate; outward: { plate: Candidate; dx: number; dy: number } } {
  const pw = SEAT_PLATE.w * ctx.w;
  const ph = SEAT_PLATE.h * ctx.h;
  let dx = plate.cx - TABLE_CENTRE.x * ctx.w;
  let dy = plate.cy - TABLE_CENTRE.y * ctx.h;
  const len = Math.hypot(dx, dy);
  if (len < 1e-9) {
    dx = 0;
    dy = 1;
  } else {
    dx /= len;
    dy /= len;
  }
  const sides: Array<{ vx: number; vy: number; at: Candidate }> = [
    { vx: 0, vy: -1, at: { cx: plate.cx, cy: plate.cy - ph / 2 - PLATE_GAP_PX - ctx.ch / 2 } },
    { vx: 0, vy: 1, at: { cx: plate.cx, cy: plate.cy + ph / 2 + PLATE_GAP_PX + ctx.ch / 2 } },
    { vx: -1, vy: 0, at: { cx: plate.cx - pw / 2 - PLATE_GAP_PX - ctx.cw / 2, cy: plate.cy } },
    { vx: 1, vy: 0, at: { cx: plate.cx + pw / 2 + PLATE_GAP_PX + ctx.cw / 2, cy: plate.cy } },
  ];
  const ranked = sides
    .map((side) => ({ ...side, dot: side.vx * dx + side.vy * dy }))
    .sort((a, b) => b.dot - a.dot);
  const fitting = ranked.find((side) => side.dot > 1e-9 && inBounds(ctx, side.at.cx, side.at.cy));
  const chosen = fitting ?? ranked[0];
  return {
    start: clampCentre(ctx, chosen.at.cx, chosen.at.cy),
    outward: { plate, dx, dy },
  };
}

/**
 * Clamps a saved chip centre (fractions) so the whole chip is inside the
 * window. `NaN` goes to the middle, like the backend's `clamp_centre`.
 */
export function clampOverride(point: Point, chip: ChipMetrics, windowW: number, windowH: number): Point {
  const { w, h } = usableSize(windowW, windowH);
  const halfW = Math.min(chip.widthPx / w / 2, 0.5);
  const halfH = Math.min(chip.heightPx / h / 2, 0.5);
  const fx = Number.isFinite(point.x) ? point.x : 0.5;
  const fy = Number.isFinite(point.y) ? point.y : 0.5;
  return { x: clamp(fx, halfW, 1 - halfW), y: clamp(fy, halfH, 1 - halfH) };
}

/**
 * Lays out every chip of one table: saved overrides exactly where they were
 * put (only clamped into the window), and every other seat at its default,
 * just outside its plate on the side away from the table centre, nudged
 * until it overlaps neither another chip nor a protected zone and stays in
 * the window. Defaults are computed for every seat of the table, occupied
 * or not, so a seat's chip never jumps when another player sits down.
 * Deterministic: the same input always gives the same output.
 */
export function layoutSeats(input: LayoutInput): SeatLayout {
  const n = validMaxPlayers(input.maxPlayers);
  const frame: SeatFrame = input.frame === "absolute" ? "absolute" : "hero";
  const { w, h } = usableSize(input.windowW, input.windowH);
  const nominal = chipMetrics(w, h, input.model ?? "compact");
  const chip: ChipMetrics =
    input.chip && input.chip.widthPx > 0 && input.chip.heightPx > 0
      ? { ...nominal, widthPx: input.chip.widthPx, heightPx: input.chip.heightPx }
      : nominal;

  const anchors = defaultSeatAnchors(n, frame);
  const keys = new Set(anchors.map((a) => a.seatKey));

  const overrides = new Map<number, Point>();
  for (const saved of input.overrides ?? []) {
    if (keys.has(saved.seatKey)) {
      overrides.set(saved.seatKey, clampOverride(saved, chip, w, h));
    }
  }

  const ctx: SearchContext = {
    w,
    h,
    cw: Math.min(chip.widthPx, w),
    ch: Math.min(chip.heightPx, h),
    step: Math.max(1, Math.round(Math.min(chip.widthPx, chip.heightPx) / 5)),
    hard: protectedZones(n, frame, input.heroSeatKey).map((zone) => toBox(zone.rect, w, h)),
    // Plates and the opponents' hole cards are avoided when there is room;
    // the hero's hole cards are a protected (hard) zone already.
    soft: anchors.flatMap((anchor) => [
      toBox(seatPlateRect(anchor), w, h),
      toBox(holeCardsRect(anchor), w, h),
    ]),
    rings: new Map(),
  };

  const chipBox = (p: Point) => grow(boxAt(ctx, p.x * w, p.y * h), CHIP_GAP_PX / 2);
  for (const p of overrides.values()) {
    ctx.hard.push(chipBox(p));
  }

  const placed = new Map<number, ChipPlacement>();
  for (const anchor of anchors) {
    const saved = overrides.get(anchor.seatKey);
    if (saved) {
      placed.set(anchor.seatKey, { seatKey: anchor.seatKey, x: saved.x, y: saved.y, overridden: true });
    }
  }
  // The gap between chips is kept by growing every placed chip's box by half
  // of it and searching with a chip grown by the other half.
  const searchCtx: SearchContext = { ...ctx, cw: ctx.cw + CHIP_GAP_PX, ch: ctx.ch + CHIP_GAP_PX };
  const settle = (found: Candidate): Point => {
    // The search box is the chip plus its gap; nudge the chip itself back
    // inside the window if the extra gap pushed it past an edge.
    const c = clampCentre(ctx, found.cx, found.cy);
    return { x: c.cx / w, y: c.cy / h };
  };

  for (const anchor of anchors) {
    if (placed.has(anchor.seatKey)) continue;
    const plate = { cx: anchor.x * w, cy: anchor.y * h };
    const { start, outward } = outerAnchor(ctx, plate);
    const p = settle(place(searchCtx, clampCentre(searchCtx, start.cx, start.cy), outward));
    placed.set(anchor.seatKey, { seatKey: anchor.seatKey, x: p.x, y: p.y, overridden: false });
    searchCtx.hard.push(chipBox(p));
  }

  const spares: ChipPlacement[] = [];
  const spareCount = Math.max(0, Math.floor(input.spareCount ?? 0));
  for (let i = 0; i < spareCount; i++) {
    const start = clampCentre(searchCtx, 0, 0);
    const p = settle(place(searchCtx, start, null));
    spares.push({ seatKey: null, x: p.x, y: p.y, overridden: false });
    searchCtx.hard.push(chipBox(p));
  }

  const requested = input.seats ?? anchors.map((a) => a.seatKey);
  const positions = requested
    .map((key) => placed.get(key))
    .filter((p): p is ChipPlacement => p !== undefined);
  return { chip, positions, spares };
}
