// Fake PokerStars table windows for the e2e driver: which tables a
// hand-history folder describes, the window title each one gets, and where
// the windows go on screen. Pure functions; the windows themselves are
// drawn by fake-tables.ps1.
//
// Titles follow the real client's shape, which table_track parses
// (src-tauri/src/table_track/mod.rs, `extract_table_name`):
//   cash  `Session: 05:11 - Aegle IV - No Limit Hold'em $0.25/$0.50 USD - Logged In as SimHero`
//   mtt   `Session: 00:14 - Progressive KO $11 [9-Max] - 75/150 - Tournament 4100000001 Table 7 - Logged In as SimHero`
// The cash and tournament shapes are confirmed real titles; the Zoom and
// Spin & Go ones are ASSUMED (same rules, different description).

import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { money, money2, tableName } from "./format.mjs";
import { hash32 } from "./rng.mjs";

/** The window class real table windows report (`POKERSTARS_TABLE_CLASS_CANDIDATES`). */
export const TABLE_CLASS = "GLFW30";
/** The marker every real table title carries (`LOGGED_IN_TITLE_MARKER`). */
export const LOGGED_IN_MARKER = " - Logged In as ";

/** PokerStars' minimum table size and the medium one the HUD is tuned at (seatLayout.ts). */
export const TABLE_SIZES = {
  min: { width: 483, height: 359 },
  medium: { width: 800, height: 570 },
};

/**
 * Seat plate centres as fractions of the table window, hero at slot 0
 * (bottom centre) and clockwise: a copy of SEAT_PLATES in
 * src/overlay/seatLayout.ts, so the mock felt draws its plates where the
 * HUD expects them and the default chips land beside them.
 */
const SEAT_PLATES = {
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
export const SEAT_PLATE = { w: 0.15, h: 0.08 };
export const BOARD_ZONE = { x: 0.33, y: 0.33, w: 0.34, h: 0.24 };
export const ACTIONS_ZONE = { x: 0.56, y: 0.83, w: 0.44, h: 0.17 };
/**
 * A felt point no HUD element covers by default: between the board and the
 * hero's plate, left of centre (the pot line), where clickability is
 * measured.
 */
export const FELT_CLICK_POINT = { x: 0.4, y: 0.64 };

/** Seat plate centres for a table of `maxSeats` (other sizes: the HUD's ring). */
export function seatPlates(maxSeats) {
  if (SEAT_PLATES[maxSeats]) return SEAT_PLATES[maxSeats];
  const n = Math.min(10, Math.max(2, Math.round(maxSeats)));
  const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
  return Array.from({ length: n }, (_, slot) => {
    const theta = ((90 + (slot * 360) / n) * Math.PI) / 180;
    return { x: clamp(0.5 + 0.4 * Math.cos(theta), 0.09, 0.91), y: clamp(0.47 + 0.3 * Math.sin(theta), 0.18, 0.79) };
  });
}

/**
 * Where to hover for a villain's chip, as fractions: the chip sits beside
 * its seat plate (above it for lower seats, below for upper ones). An
 * estimate (ASSUMED): the live run calibrates against its screenshots.
 */
export function chipHoverPoints(maxSeats) {
  return seatPlates(maxSeats)
    .slice(1)
    .map((p, i) => ({ slot: i + 1, x: p.x, y: p.y < 0.47 ? p.y + SEAT_PLATE.h * 0.9 : p.y - SEAT_PLATE.h * 0.9 }));
}

const pad = (n) => String(n).padStart(2, "0");
/** `05:11`: the session timer at the start of a real title. */
export function sessionClock(seconds) {
  const s = Math.max(0, Math.floor(seconds));
  return `${pad(Math.floor(s / 60) % 100)}:${pad(s % 60)}`;
}

/**
 * The window title of a table described by `scanTables`/`plannedCashTables`.
 * `kind` is cash, zoom, mtt or spin.
 */
export function tableTitle(table, hero, sessionSeconds = 0) {
  if (!hero) throw new Error("tableTitle needs the hero's screen name");
  const clock = `Session: ${sessionClock(sessionSeconds)}`;
  let middle;
  switch (table.kind) {
    case "cash":
      middle = `${table.name} - No Limit Hold'em ${money(table.sb)}/${money(table.bb)} USD`;
      break;
    case "zoom":
      middle = `${table.name} - Zoom No Limit Hold'em ${money(table.sb)}/${money(table.bb)}`;
      break;
    case "mtt":
    case "spin": {
      const [id, number] = table.name.split(" ");
      const total = (table.buyIn ?? []).reduce((a, b) => a + b, 0);
      const event = table.kind === "spin" ? `Spin & Go ${money2(total)}` : `Progressive KO ${money2(total)} [${table.maxSeats}-Max]`;
      middle = `${event} - ${table.sb}/${table.bb} - Tournament ${id} Table ${number}`;
      break;
    }
    default:
      throw new Error(`unknown table kind ${table.kind}`);
  }
  return `${clock} - ${middle}${LOGGED_IN_MARKER}${hero}`;
}

/** Cents from `$0.25` / `$10` / `$1.50`. */
const cents = (text) => Math.round(Number(text.replace(/[^0-9.]/g, "")) * 100);

/**
 * The table a hand-history file belongs to, from its hands: name (as the
 * importer stores `hands.table_name`), kind, size, blinds of the latest
 * hand, hand count, and whether the hero's tournament is over (he was
 * eliminated, finished, or someone won a Spin).
 */
export function parseTableFile(text, hero) {
  const headers = [...text.matchAll(/^(?:﻿)?PokerStars (Zoom )?Hand #\d+:\s+(.*)$/gm)];
  const tableLine = /^Table '([^']+)' (\d+)-max/m.exec(text);
  if (headers.length === 0 || !tableLine) return null;
  const last = headers[headers.length - 1];
  const zoom = Boolean(last[1]);
  const rest = last[2];
  const tournament = /^Tournament #(\d+), ([$\d.+]+) USD .*\((\d+)\/(\d+)\)/.exec(rest);
  const maxSeats = Number(tableLine[2]);
  const table = { name: tableLine[1], maxSeats, hands: headers.length, heroOut: false };
  if (tournament) {
    Object.assign(table, {
      kind: maxSeats === 3 ? "spin" : "mtt",
      tournamentId: tournament[1],
      buyIn: tournament[2].split("+").map(cents),
      sb: Number(tournament[3]),
      bb: Number(tournament[4]),
    });
    const name = hero.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    table.heroOut = new RegExp(
      `for eliminating ${name} and|^${name} finished the tournament|wins the tournament and receives`,
      "m",
    ).test(text);
  } else {
    const blinds = /\((\$[\d.]+)\/(\$[\d.]+)/.exec(rest);
    if (!blinds) return null;
    Object.assign(table, { kind: zoom ? "zoom" : "cash", sb: cents(blinds[1]), bb: cents(blinds[2]) });
  }
  return table;
}

/** Every table in a hand-history folder (and its per-player subfolders), by file name. */
export function scanTables(dir, hero) {
  const out = [];
  const visit = (folder, depth) => {
    let entries;
    try {
      entries = readdirSync(folder, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const path = join(folder, entry.name);
      if (entry.isDirectory()) {
        if (depth < 1) visit(path, depth + 1);
      } else if (entry.name.toLowerCase().endsWith(".txt")) {
        let text;
        try {
          text = readFileSync(path, "utf8");
        } catch {
          continue;
        }
        const table = parseTableFile(text, hero);
        if (table) out.push({ file: entry.name, path, ...table });
      }
    }
  };
  visit(dir, 0);
  return out.sort((a, b) => a.file.localeCompare(b.file));
}

/**
 * The cash tables a generator session with this seed will open, in order:
 * the names session.mjs gives them (`tableName(t, hash32(seed) % 20)`), so
 * their windows can be up before their first hand is written.
 */
export function plannedCashTables({ seed = "velora", tables, stakes }) {
  const offset = hash32(seed) % 20;
  return Array.from({ length: tables }, (_, t) => ({
    kind: "cash",
    name: tableName(t, offset),
    maxSeats: stakes.maxSeats,
    sb: stakes.sb,
    bb: stakes.bb,
  }));
}

/**
 * Window slots on the primary work area: `slots` is a list of
 * `{ kind, size }` (`size` a TABLE_SIZES key). Rows are filled left to
 * right; when the screen is full the next windows cascade from the top-left,
 * overlapping (as a real 8-table session does on one monitor).
 */
export function planLayout(slots, area, gap = 8) {
  let x = area.x;
  let y = area.y;
  let row = 0;
  let cascade = 0;
  return slots.map((slot, index) => {
    const { width, height } = TABLE_SIZES[slot.size] ?? slot.size;
    if (x + width > area.x + area.width) {
      x = area.x;
      y += row + gap;
      row = 0;
    }
    let rect;
    if (y + height > area.y + area.height) {
      const step = 40 * ++cascade;
      rect = {
        x: area.x + Math.min(step, Math.max(0, area.width - width)),
        y: area.y + Math.min(step, Math.max(0, area.height - height)),
        width,
        height,
      };
    } else {
      rect = { x, y, width, height };
      x += width + gap;
      row = Math.max(row, height);
    }
    return { index, ...slot, rect };
  });
}

/** Absolute screen point of a fraction of a window rect. */
export function pointIn(rect, fraction) {
  return { x: Math.round(rect.x + fraction.x * rect.width), y: Math.round(rect.y + fraction.y * rect.height) };
}
