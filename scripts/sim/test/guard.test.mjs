// The output-folder guard: the simulator never writes into a PokerStars
// folder, the Velora app data folder, a drive root, or next to someone
// else's real hand histories. Run with `npm run test:sim`.

import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, parse } from "node:path";
import { test } from "node:test";
import { assertSafeOutputDir, forbiddenRoots } from "../lib/guard.mjs";
import { runSession } from "../lib/session.mjs";

const scratch = (label) => mkdtempSync(join(tmpdir(), `velora-guard-${label}-`));
const HERO = "SimHero";

test("PokerStars folders are refused, whatever the client's region or the path's case", () => {
  for (const dir of [
    "C:\\Program Files\\PokerStars\\HandHistory",
    "C:\\Program Files (x86)\\PokerStars.EU",
    join(scratch("ps"), "PokerStars.PT", "HandHistory", "Hero"),
    join(scratch("ps"), "pokerstars", "x"),
  ]) {
    assert.throws(() => assertSafeOutputDir(dir, { hero: HERO }), /PokerStars folder/, dir);
  }
});

test("the Velora app data folder is refused, from the environment and from the real profile", () => {
  const fakeAppData = scratch("appdata");
  const fakeHome = scratch("home");
  const env = { APPDATA: fakeAppData, LOCALAPPDATA: join(fakeHome, "AppData", "Local") };
  const roots = forbiddenRoots({ env, home: fakeHome });
  assert.ok(roots.includes(join(fakeAppData, "com.velora.poker")));
  assert.ok(roots.includes(join(fakeHome, "AppData", "Roaming", "com.velora.poker")));
  for (const dir of [
    join(fakeAppData, "com.velora.poker"),
    join(fakeAppData, "COM.VELORA.POKER", "hh"),
    join(fakeHome, "AppData", "Roaming", "com.velora.poker", "HandHistory"),
  ]) {
    assert.throws(() => assertSafeOutputDir(dir, { hero: HERO, env, home: fakeHome }), /Velora app data/, dir);
  }
  // An overridden APPDATA (the e2e driver's) does not hide the real one.
  const realRoots = forbiddenRoots({ env: { APPDATA: fakeAppData } });
  assert.ok(realRoots.some((r) => r.toLowerCase().endsWith(join("AppData", "Roaming", "com.velora.poker").toLowerCase())));
});

test("a drive root is refused", () => {
  const root = parse(tmpdir()).root;
  assert.throws(() => assertSafeOutputDir(root, { hero: HERO }), /drive root/);
});

test("a folder holding another screen name's hand histories is refused; the simulator's own are fine", () => {
  const theirs = scratch("theirs");
  mkdirSync(join(theirs, "RealPlayer"));
  writeFileSync(join(theirs, "RealPlayer", "HH20260101 Aegle.txt"), "PokerStars Hand #1:\r\nDealt to RealPlayer [Ah Kd]\r\n");
  assert.throws(() => assertSafeOutputDir(theirs, { hero: HERO }), /real hand histories/);

  const ours = scratch("ours");
  writeFileSync(join(ours, "HH20260912 Aegle II.txt"), "PokerStars Hand #1:\r\nDealt to SimHero [Ah Kd]\r\n");
  assert.equal(assertSafeOutputDir(ours, { hero: HERO }), ours);
});

test("a fresh temporary folder is accepted", () => {
  const dir = join(scratch("fresh"), "HandHistory", "SimHero");
  assert.equal(assertSafeOutputDir(dir, { hero: HERO }), dir);
});

test("runSession refuses an unsafe folder before creating anything", async () => {
  const base = scratch("session");
  const target = join(base, "PokerStars.PT", "HandHistory");
  await assert.rejects(runSession({ out: target, hands: 1, tables: 1 }), /PokerStars folder/);
  assert.equal(existsSync(join(base, "PokerStars.PT")), false);
  await assert.rejects(runSession({ hands: 1 }), /no output folder/);
});
