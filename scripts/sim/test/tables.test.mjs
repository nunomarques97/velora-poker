// Fake tables: the tables a generated folder describes, the titles their
// windows get, where they go on screen, and that the window program, this
// module and the HUD's seat layout agree on class and seat positions.
// Opens no window. Run with `npm run test:sim`.

import assert from "node:assert/strict";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { titlesFor } from "../fake-tables.mjs";
import { loadProfiles, runSession } from "../lib/session.mjs";
import {
  chipHoverPoints,
  FELT_CLICK_POINT,
  LOGGED_IN_MARKER,
  parseTableFile,
  planLayout,
  plannedCashTables,
  scanTables,
  seatPlates,
  TABLE_CLASS,
  TABLE_SIZES,
  tableTitle,
} from "../lib/tables.mjs";

const profiles = loadProfiles();
const HERO = profiles.hero.name;
const scratch = (label) => mkdtempSync(join(tmpdir(), `velora-tables-${label}-`));
const repoFile = (path) => readFileSync(new URL(`../../../${path}`, import.meta.url), "utf8");

/**
 * The table name table_track reads out of a title: a line-for-line port of
 * `extract_table_name` (src-tauri/src/table_track/mod.rs). The Rust test
 * sim_titles_tests.rs checks the same titles against the real function.
 */
function extractTableName(title) {
  const prefix = title.split(LOGGED_IN_MARKER)[0];
  const keyword = /Tournament\s+(\d+)\s+Table\s+(\d+)/.exec(prefix);
  if (keyword) return `${keyword[1]} ${keyword[2]}`;
  const parts = prefix.split(" - ");
  if (parts.length < 2) return null;
  return parts[parts.length - 2].trim() || null;
}

const sessions = {};
async function session(format, tables, hands) {
  const key = `${format}:${tables}:${hands}`;
  if (!sessions[key]) {
    const out = scratch(format);
    sessions[key] = { out, manifest: await runSession({ format, seed: "titles", out, tables, hands, pace: 0 }) };
  }
  return sessions[key];
}

test("every generated table gets a title table_track accepts and maps back to its hand-history name", async () => {
  for (const [format, tables, hands, maxSeats] of [
    ["cash", 4, 15, 6],
    ["zoom", 1, 30, 6],
    ["mtt", 3, 80, 9],
    ["spin", 2, 60, 3],
  ]) {
    const { out, manifest } = await session(format, tables, hands);
    const titles = titlesFor(out, HERO);
    assert.equal(titles.length, manifest.files.length, `${format}: one table per file`);
    assert.deepEqual(new Set(titles.map((t) => t.table)), new Set(manifest.tables.map((t) => t.name)), format);
    for (const t of titles) {
      assert.equal(t.kind, format);
      assert.equal(t.maxSeats, maxSeats);
      assert.ok(t.title.includes(LOGGED_IN_MARKER), t.title);
      assert.ok(t.title.endsWith(`${LOGGED_IN_MARKER}${HERO}`), t.title);
      assert.equal(extractTableName(t.title), t.table, t.title);
    }
  }
});

test("the cash tables planned before launch are exactly the ones the generator opens", async () => {
  for (const seed of ["e2e", "titles", "velora"]) {
    const out = scratch(`plan-${seed}`);
    const manifest = await runSession({ format: "cash", seed, out, tables: 5, hands: 2, pace: 0 });
    const planned = plannedCashTables({ seed, tables: 5, stakes: profiles.stakes.cash });
    assert.deepEqual(planned.map((t) => t.name), manifest.tables.map((t) => t.name), seed);
    const scanned = new Map(scanTables(out, HERO).map((t) => [t.name, t]));
    for (const table of planned) {
      // The title made from the plan equals the one made from the file.
      assert.equal(tableTitle(table, HERO), tableTitle(scanned.get(table.name), HERO));
    }
  }
});

test("scanning reads kind, size, blinds and hand count, and sees the hero's tournament end", async () => {
  const cash = await session("cash", 4, 15);
  for (const t of scanTables(cash.out, HERO)) {
    assert.equal(t.hands, 15);
    assert.equal(t.sb, profiles.stakes.cash.sb);
    assert.equal(t.bb, profiles.stakes.cash.bb);
    assert.equal(t.heroOut, false);
  }
  for (const format of ["mtt", "spin"]) {
    const { out, manifest } = await session(format, format === "mtt" ? 3 : 2, format === "mtt" ? 80 : 60);
    const scanned = new Map(scanTables(out, HERO).map((t) => [t.name, t]));
    const finished = manifest.tournaments.filter((t) => t.file);
    assert.ok(finished.some((t) => t.heroFinish !== "running"), `${format}: some tournament ended`);
    for (const record of finished) {
      const t = scanned.get(record.table);
      assert.equal(t.tournamentId, record.id);
      assert.equal(t.hands, record.hands);
      assert.equal(t.heroOut, record.heroFinish !== "running", `${format} ${record.table}: ${record.heroFinish}`);
      assert.deepEqual(t.buyIn, profiles.stakes[format].buyIn);
    }
  }
  assert.equal(parseTableFile("not a hand history", HERO), null);
});

test("titles carry the session timer and refuse an unknown kind or a missing hero", () => {
  const table = { kind: "cash", name: "Aegle IV", sb: 25, bb: 50 };
  assert.equal(
    tableTitle(table, HERO, 311),
    `Session: 05:11 - Aegle IV - No Limit Hold'em $0.25/$0.50 USD${LOGGED_IN_MARKER}${HERO}`,
  );
  assert.throws(() => tableTitle({ ...table, kind: "omaha" }, HERO), /unknown table kind/);
  assert.throws(() => tableTitle(table, ""), /hero/);
});

test("the layout keeps exact table sizes inside the work area", () => {
  const slots = ["min", "medium", "min", "medium", "min", "medium", "min", "medium"].map((size) => ({ size }));
  for (const area of [
    { x: 0, y: 0, width: 1920, height: 1040 },
    { x: 0, y: 0, width: 1024, height: 720 },
    { x: -1920, y: 0, width: 1920, height: 1040 },
  ]) {
    const placed = planLayout(slots, area);
    assert.equal(placed.length, 8);
    for (const p of placed) {
      assert.deepEqual({ width: p.rect.width, height: p.rect.height }, TABLE_SIZES[p.size]);
      assert.ok(p.rect.x >= area.x && p.rect.x + p.rect.width <= area.x + area.width, JSON.stringify(p.rect));
      assert.ok(p.rect.y >= area.y && p.rect.y + p.rect.height <= area.y + area.height, JSON.stringify(p.rect));
    }
  }
  // On a large screen nothing overlaps.
  const wide = planLayout(slots.slice(0, 4), { x: 0, y: 0, width: 2560, height: 1400 });
  for (const a of wide) {
    for (const b of wide) {
      if (a === b) continue;
      const apart =
        a.rect.x + a.rect.width <= b.rect.x || b.rect.x + b.rect.width <= a.rect.x ||
        a.rect.y + a.rect.height <= b.rect.y || b.rect.y + b.rect.height <= a.rect.y;
      assert.ok(apart, `${JSON.stringify(a.rect)} overlaps ${JSON.stringify(b.rect)}`);
    }
  }
});

test("hover points sit beside every villain's plate and the felt click point avoids plates, board and buttons", () => {
  for (const seats of [6, 9]) {
    const plates = seatPlates(seats);
    const hovers = chipHoverPoints(seats);
    assert.equal(hovers.length, seats - 1);
    for (const h of hovers) {
      assert.ok(h.x > 0 && h.x < 1 && h.y > 0 && h.y < 1);
      const plate = plates[h.slot];
      assert.ok(Math.abs(h.y - plate.y) > 0.04 && Math.abs(h.y - plate.y) < 0.1, `slot ${h.slot}`);
    }
    for (const p of plates) {
      const inPlate = Math.abs(FELT_CLICK_POINT.x - p.x) < 0.075 + 0.02 && Math.abs(FELT_CLICK_POINT.y - p.y) < 0.04 + 0.02;
      assert.ok(!inPlate, `felt point is on a ${seats}-max plate at ${p.x},${p.y}`);
    }
  }
  const board = { x: 0.33, y: 0.33, w: 0.34, h: 0.24 };
  const inBoard = (p) => p.x >= board.x && p.x <= board.x + board.w && p.y >= board.y && p.y <= board.y + board.h;
  assert.ok(!inBoard(FELT_CLICK_POINT));
  assert.ok(FELT_CLICK_POINT.y < 0.83, "above the action buttons");
});

test("the window program, this module and the HUD's seat layout agree on class and seat plates", () => {
  const ps1 = repoFile("scripts/sim/fake-tables.ps1");
  assert.equal(/public const string ClassName = "([^"]+)";/.exec(ps1)[1], TABLE_CLASS);
  assert.equal(TABLE_CLASS, "GLFW30");

  const csPlates = (name) =>
    [...new RegExp(`${name} = \\{([\\s\\S]*?)\\};`).exec(ps1)[1].matchAll(/\{([\d.]+)f,([\d.]+)f\}/g)].map((m) => ({
      x: Number(m[1]),
      y: Number(m[2]),
    }));
  assert.deepEqual(csPlates("Plates6"), seatPlates(6));
  assert.deepEqual(csPlates("Plates9"), seatPlates(9));

  const ts = repoFile("src/overlay/seatLayout.ts");
  const block = /const SEAT_PLATES[^=]*= \{([\s\S]*?)\n\};/.exec(ts)[1];
  for (const seats of [6, 9]) {
    const rows = new RegExp(`${seats}: \\[([\\s\\S]*?)\\]`).exec(block)[1];
    const tsPlates = [...rows.matchAll(/\{ x: ([\d.]+), y: ([\d.]+) \}/g)].map((m) => ({ x: Number(m[1]), y: Number(m[2]) }));
    assert.deepEqual(tsPlates, seatPlates(seats), `${seats}-max plates match seatLayout.ts`);
  }
  for (const [name, size] of Object.entries({ MIN_TABLE_SIZE: TABLE_SIZES.min, MEDIUM_TABLE_SIZE: TABLE_SIZES.medium })) {
    const m = new RegExp(`${name} = \\{ width: (\\d+), height: (\\d+) \\}`).exec(ts);
    assert.deepEqual({ width: Number(m[1]), height: Number(m[2]) }, size, name);
  }
});

test("the probe reports the monitors and whether the session is locked", { skip: process.platform !== "win32" }, async () => {
  const { probe } = await import("../fake-tables.mjs");
  const dir = scratch("probe");
  const probed = probe({ env: { ...process.env, TEMP: dir, TMP: dir } });
  assert.equal(probed.className, TABLE_CLASS);
  assert.ok(probed.monitors.length >= 1);
  assert.ok([true, false, null].includes(probed.locked), String(probed.locked));
  // A hidden window's non-ASCII title reads back whole: with an ANSI
  // DefWindowProc it read back as "S", and table_track tracked no table.
  assert.equal(probed.titleRoundTrip, true);
});

test("the window program answers commands and writes its status without opening a window", { skip: process.platform !== "win32" }, async () => {
  const { FakeTables } = await import("../fake-tables.mjs");
  const dir = scratch("program");
  const statusPath = join(dir, "status.json");
  const tables = await FakeTables.start({ statusPath, env: { ...process.env, TEMP: dir, TMP: dir } });
  try {
    const status = await tables.status();
    assert.equal(status.className, TABLE_CLASS);
    assert.deepEqual(status.tables, []);
    assert.deepEqual(tables.readStatus().tables, [], "status file written at start");
    // A title no real window has: the listing works and finds nothing.
    assert.deepEqual(await tables.list("velora-e2e-no-such-window-7f3a"), []);
    await assert.rejects(tables.close("t1"), /no open table t1/);
    await assert.rejects(tables.move("t9", { x: 0, y: 0, width: 483, height: 359 }), /no open table t9/);
    await assert.rejects(tables.send("format-disk"), /unknown op/);
    // A screen capture writes a PNG (the overlay needs SRCCOPY | CAPTUREBLT,
    // which CopyFromScreen refused as an enum value).
    const png = join(dir, "region.png");
    assert.deepEqual(await tables.shot(png, { x: 0, y: 0, width: 8, height: 6 }), { path: png, width: 8, height: 6 });
    assert.deepEqual([...readFileSync(png).subarray(0, 4)], [0x89, 0x50, 0x4e, 0x47]);
    await assert.rejects(tables.shot(join(dir, "empty.png"), { x: 0, y: 0, width: 0, height: 6 }), /empty capture region/);
    // Rendering a table's own window needs that table open (the live run
    // renders every open table with its HUD on top).
    await assert.rejects(tables.print("t1", join(dir, "t1.png")), /no open table t1/);
  } finally {
    await tables.quit();
  }
  assert.equal(tables.exited, true);
  await assert.rejects(tables.status(), /not running/);
});
