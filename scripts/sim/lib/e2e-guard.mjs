// The e2e driver's guards. The driver launches the real app against a
// simulated session, so everything it points the app at must live inside a
// temporary root of its own: never the Sponsor's real Velora database or
// settings (%APPDATA%\com.velora.poker), never a real PokerStars folder.
// Real folders come from the real user profile as well as the original
// environment, and paths are also checked through links and junctions, so
// neither an overridden APPDATA nor a junction can hide them.

import { existsSync, readdirSync } from "node:fs";
import { homedir } from "node:os";
import { isAbsolute, join, parse, resolve } from "node:path";
import { assertSafeOutputDir, forbiddenRoots, isInside, realAncestor, VELORA_IDENTIFIER } from "./guard.mjs";

/** Written into every root the driver creates; a non-empty root without it is refused. */
export const ROOT_MARKER = ".velora-e2e-root";

/** True when any folder on the path (lexical or real) is a PokerStars folder. */
export function isPokerStarsPath(path) {
  return [resolve(path), realAncestor(path)].some((p) => p.split(/[\\/]+/).some((part) => /^pokerstars/i.test(part)));
}

function isDriveRoot(path) {
  const p = resolve(path);
  return p.replace(/[\\/]+$/, "").toLowerCase() === parse(p).root.replace(/[\\/]+$/, "").toLowerCase();
}

/** Both the lexical path and the path through links/junctions. */
const forms = (path) => [resolve(path), realAncestor(path)];

/** Every form of `path` is inside some form of `dir` (short names, junctions). */
function within(path, dir) {
  return forms(path).every((p) => forms(dir).some((d) => isInside(p, d)));
}

const same = (a, b) => forms(a).some((x) => forms(b).some((y) => x.toLowerCase() === y.toLowerCase()));

/** The layout of a driver root: everything the app and the simulator use. */
export function e2ePaths(root, hero = "SimHero") {
  const r = resolve(root);
  const appData = join(r, "AppData", "Roaming");
  const localAppData = join(r, "AppData", "Local");
  const dataDir = join(appData, VELORA_IDENTIFIER);
  return {
    root: r,
    marker: join(r, ROOT_MARKER),
    appData,
    localAppData,
    webview: join(r, "WebView2"),
    dataDir,
    db: join(dataDir, "velora.db"),
    // The app watches `handHistoryRoot`; the generator writes into the
    // hero's subfolder, as the client does.
    handHistoryRoot: join(r, "HandHistory"),
    handHistory: join(r, "HandHistory", hero),
    logs: join(r, "logs"),
    tmp: join(r, "tmp"),
    status: join(r, "fake-tables.json"),
    summary: join(r, "e2e-summary.json"),
  };
}

/** Real Velora data folders (real profile and original environment). */
export function realVeloraRoots({ env = process.env, home = homedir() } = {}) {
  return forbiddenRoots({ env, home });
}

function assertNotReal(label, path, real) {
  for (const p of forms(path)) {
    for (const root of real) {
      if (isInside(p, root)) throw new Error(`refusing: ${label} ${p} is the real Velora data folder ${root}`);
    }
  }
  if (isPokerStarsPath(path)) throw new Error(`refusing: ${label} ${path} is under a PokerStars folder`);
}

/**
 * Throws unless every path the session uses is safe: the root is not a
 * drive root, not a real data folder and holds none; every data and
 * hand-history path is inside the root (also through junctions) and not a
 * real Velora or PokerStars folder; the screenshot folder is not one either.
 */
export function assertSafeE2ePaths(paths, { shots, hero, env = process.env, home = homedir() } = {}) {
  const real = realVeloraRoots({ env, home });
  const { root } = paths;
  if (!root || !isAbsolute(root)) throw new Error("refusing: the e2e root must be an absolute path");
  if (isDriveRoot(root)) throw new Error(`refusing: the e2e root ${root} is a drive root`);
  assertNotReal("e2e root", root, real);
  for (const p of forms(root)) {
    for (const realRoot of real) {
      if (isInside(realRoot, p)) throw new Error(`refusing: the e2e root ${p} contains the real Velora data folder ${realRoot}`);
    }
  }
  for (const [label, path] of Object.entries(paths)) {
    if (label === "root") continue;
    assertNotReal(label, path, real);
    if (!within(path, root) || same(path, root)) {
      throw new Error(`refusing: ${label} ${path} is not inside the e2e root ${root}`);
    }
  }
  assertSafeOutputDir(paths.handHistory, { hero, env, home });
  if (shots) {
    if (!isAbsolute(shots)) throw new Error("refusing: the screenshot folder must be an absolute path");
    if (isDriveRoot(shots)) throw new Error(`refusing: the screenshot folder ${shots} is a drive root`);
    assertNotReal("screenshot folder", shots, real);
  }
  return paths;
}

/** A root may be reused only if it is empty or one the driver created. */
export function assertUsableRoot(root) {
  if (!existsSync(root)) return;
  const entries = readdirSync(root);
  if (entries.length > 0 && !entries.includes(ROOT_MARKER)) {
    throw new Error(`refusing: ${root} is not empty and was not created by the e2e driver`);
  }
}

/**
 * The environment the app, the generator and the window program run with:
 * APPDATA (the database path in lib.rs), LOCALAPPDATA (folder auto-detect,
 * WebView2 data) and WEBVIEW2_USER_DATA_FOLDER point inside the root, as do
 * TEMP/TMP. Checked: the app's database folder lands exactly on
 * `paths.dataDir`.
 */
export function appEnvironment(paths, base = process.env) {
  const env = { ...base };
  for (const key of Object.keys(env)) {
    if (/^(APPDATA|LOCALAPPDATA|WEBVIEW2_USER_DATA_FOLDER|TEMP|TMP)$/i.test(key)) delete env[key];
  }
  Object.assign(env, {
    APPDATA: paths.appData,
    LOCALAPPDATA: paths.localAppData,
    WEBVIEW2_USER_DATA_FOLDER: paths.webview,
    TEMP: paths.tmp,
    TMP: paths.tmp,
  });
  if (resolve(join(env.APPDATA, VELORA_IDENTIFIER)) !== resolve(paths.dataDir)) {
    throw new Error("refusing: the app's database folder would not be the e2e one");
  }
  return env;
}

/**
 * Every file the driver writes goes through `allow(path)`: inside the root
 * or the screenshot folder (also through junctions), or it throws.
 */
export function createWriteScope({ root, shots }) {
  const allowed = [root, shots].filter(Boolean).map((p) => resolve(p));
  return {
    allowed,
    allow(path) {
      const target = resolve(path);
      if (!allowed.some((dir) => within(target, dir))) throw new Error(`refusing to write outside the e2e root and screenshot folder: ${target}`);
      return target;
    },
  };
}
