// The e2e driver's guards: it refuses to run when any data or hand-history
// path is the real Velora data folder or a PokerStars folder (also through
// a junction), and never writes outside its root and screenshot folder.
// Uses fake profiles under the system temp; opens nothing. Run with
// `npm run test:sim`.

import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, parse } from "node:path";
import { test } from "node:test";
import {
  appEnvironment,
  assertSafeE2ePaths,
  assertUsableRoot,
  createWriteScope,
  e2ePaths,
  ROOT_MARKER,
} from "../lib/e2e-guard.mjs";

const HERO = "SimHero";
const scratch = (label) => mkdtempSync(join(tmpdir(), `velora-e2e-guard-${label}-`));

/** A fake user profile whose real Velora folder exists, and its environment. */
function fakeProfile() {
  const home = scratch("home");
  const appData = join(home, "AppData", "Roaming");
  const real = join(appData, "com.velora.poker");
  mkdirSync(real, { recursive: true });
  writeFileSync(join(real, "velora.db"), "the Sponsor's real database");
  return { home, env: { APPDATA: appData, LOCALAPPDATA: join(home, "AppData", "Local") }, real };
}

const guard = (paths, profile, shots) => assertSafeE2ePaths(paths, { hero: HERO, shots, env: profile.env, home: profile.home });

test("a fresh temporary root is accepted and every path the app uses is inside it", () => {
  const profile = fakeProfile();
  const root = scratch("root");
  const paths = e2ePaths(root, HERO);
  assert.equal(guard(paths, profile, join(root, "shots")), paths);
  for (const [key, value] of Object.entries(paths)) {
    if (key !== "root") assert.ok(value.startsWith(root + "\\") || value.startsWith(root + "/"), key);
  }
  assert.ok(paths.db.endsWith(join("AppData", "Roaming", "com.velora.poker", "velora.db")));
  assert.ok(paths.handHistory.startsWith(paths.handHistoryRoot));
});

test("the real Velora data folder is refused as root, as data folder, and as a root's child", () => {
  const profile = fakeProfile();
  // Root inside the real data folder.
  assert.throws(() => guard(e2ePaths(join(profile.real, "e2e"), HERO), profile), /real Velora data folder/);
  // Root that contains it (root = the profile itself: its AppData would be the real one).
  assert.throws(() => guard(e2ePaths(profile.home, HERO), profile), /real Velora data folder/);
  // A data path redirected at it.
  const root = scratch("redirect");
  for (const key of ["dataDir", "db", "appData", "handHistory", "handHistoryRoot", "webview"]) {
    const paths = { ...e2ePaths(root, HERO), [key]: join(profile.real, key === "db" ? "velora.db" : "") };
    assert.throws(() => guard(paths, profile), /real Velora data folder|not inside the e2e root/, key);
  }
});

test("the real profile's Velora folder is refused even when APPDATA is overridden", () => {
  const realProfileFolder = join(homedir(), "AppData", "Roaming", "com.velora.poker", "e2e-root");
  const env = { APPDATA: scratch("override"), LOCALAPPDATA: scratch("override-local") };
  assert.throws(() => assertSafeE2ePaths(e2ePaths(realProfileFolder, HERO), { hero: HERO, env }), /real Velora data folder/);
});

test("PokerStars folders are refused for the root, the hand histories and the screenshots", () => {
  const profile = fakeProfile();
  const base = scratch("stars");
  assert.throws(() => guard(e2ePaths(join(base, "PokerStars.EU", "e2e"), HERO), profile), /PokerStars folder/);
  const root = scratch("stars-root");
  const paths = e2ePaths(root, HERO);
  assert.throws(
    () => guard({ ...paths, handHistory: join(root, "PokerStars", "HandHistory", HERO) }, profile),
    /PokerStars folder/,
  );
  assert.throws(() => guard(paths, profile, join(base, "pokerstars", "shots")), /PokerStars folder/);
  assert.throws(() => guard(paths, profile, join(profile.real, "shots")), /real Velora data folder/);
});

test("a junction inside the root that leads to a real folder is refused", () => {
  const profile = fakeProfile();
  const root = scratch("junction");
  const link = join(root, "AppData");
  symlinkSync(join(profile.home, "AppData"), link, "junction");
  // Through the junction, AppData\Roaming is the real profile's and dataDir the
  // real data folder: refused, whichever of the two is checked first.
  assert.throws(() => guard(e2ePaths(root, HERO), profile), /real Velora data folder|not inside the e2e root/);
  const { appData, localAppData, ...rest } = e2ePaths(root, HERO);
  assert.throws(() => guard(rest, profile), /real Velora data folder/);

  const starsRoot = scratch("junction-stars");
  const stars = join(scratch("stars-target"), "PokerStars.PT");
  mkdirSync(join(stars, "HandHistory"), { recursive: true });
  symlinkSync(join(stars, "HandHistory"), join(starsRoot, "HandHistory"), "junction");
  assert.throws(() => guard(e2ePaths(starsRoot, HERO), profile), /PokerStars folder/);
});

test("a path outside the root, a relative root and a drive root are refused", () => {
  const profile = fakeProfile();
  const root = scratch("outside");
  const elsewhere = scratch("elsewhere");
  assert.throws(() => guard({ ...e2ePaths(root, HERO), logs: elsewhere }, profile), /not inside the e2e root/);
  assert.throws(() => guard({ ...e2ePaths(root, HERO), tmp: join(root, "..", "tmp") }, profile), /not inside the e2e root/);
  assert.throws(() => guard({ ...e2ePaths(root, HERO), webview: root }, profile), /not inside the e2e root/);
  assert.throws(() => guard({ ...e2ePaths(root, HERO), root: "relative\\root" }, profile), /absolute/);
  assert.throws(() => guard(e2ePaths(parse(tmpdir()).root, HERO), profile), /drive root/);
  assert.throws(() => guard(e2ePaths(root, HERO), profile, parse(tmpdir()).root), /drive root/);
});

test("a non-empty root is reused only when the driver created it", () => {
  const empty = scratch("empty");
  assert.doesNotThrow(() => assertUsableRoot(empty));
  assert.doesNotThrow(() => assertUsableRoot(join(empty, "missing")));
  const theirs = scratch("theirs");
  writeFileSync(join(theirs, "notes.txt"), "someone's files");
  assert.throws(() => assertUsableRoot(theirs), /not created by the e2e driver/);
  writeFileSync(join(theirs, ROOT_MARKER), "");
  assert.doesNotThrow(() => assertUsableRoot(theirs));
});

test("the app environment points APPDATA, LOCALAPPDATA, WebView2 and TEMP inside the root", () => {
  const paths = e2ePaths(scratch("env"), HERO);
  const base = { Path: "C:\\Windows", Temp: "D:\\Profile\\Temp", AppData: "D:\\Profile\\AppData\\Roaming", KEEP: "1" };
  const env = appEnvironment(paths, base);
  assert.equal(env.APPDATA, paths.appData);
  assert.equal(env.LOCALAPPDATA, paths.localAppData);
  assert.equal(env.WEBVIEW2_USER_DATA_FOLDER, paths.webview);
  assert.equal(env.TEMP, paths.tmp);
  assert.equal(env.TMP, paths.tmp);
  // Differently-cased originals are dropped, not left to shadow the override.
  assert.equal(env.Temp, undefined);
  assert.equal(env.AppData, undefined);
  assert.equal(env.KEEP, "1");
  assert.throws(() => appEnvironment({ ...paths, dataDir: join(paths.root, "elsewhere") }, base), /database folder/);
});

test("writes are allowed only inside the root and the screenshot folder", () => {
  const root = scratch("scope");
  const shots = scratch("scope-shots");
  const scope = createWriteScope({ root, shots });
  assert.equal(scope.allow(join(root, "logs", "app.log")), join(root, "logs", "app.log"));
  assert.equal(scope.allow(join(shots, "r01-desktop.png")), join(shots, "r01-desktop.png"));
  assert.throws(() => scope.allow(join(root, "..", "escape.txt")), /outside/);
  assert.throws(() => scope.allow(join(homedir(), "AppData", "Roaming", "com.velora.poker", "velora.db")), /outside/);
  const outside = scratch("scope-outside");
  symlinkSync(outside, join(root, "link"), "junction");
  assert.throws(() => scope.allow(join(root, "link", "file.png")), /outside/);
  const noShots = createWriteScope({ root });
  assert.throws(() => noShots.allow(join(shots, "x.png")), /outside/);
});
