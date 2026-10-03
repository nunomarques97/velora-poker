// The output-folder guard. The simulator writes fake hand histories, so it
// must never write where they could be mistaken for real ones: a PokerStars
// install or data folder, or the Velora app's own data folder (the database
// lives in %APPDATA%\com.velora.poker). Paths come from the real user
// profile as well as the environment, so an overridden APPDATA (the e2e
// driver's) cannot hide the real folder.

import { existsSync, readdirSync, readFileSync, realpathSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, parse, resolve, sep } from "node:path";

export const VELORA_IDENTIFIER = "com.velora.poker";

const norm = (p) => resolve(p).replace(/[\\/]+$/, "").toLowerCase();

function isInside(child, parent) {
  const c = norm(child);
  const p = norm(parent);
  return c === p || c.startsWith(p + sep) || c.startsWith(p + "/");
}

/** The deepest existing ancestor, resolved through links and junctions. */
function realAncestor(target) {
  let current = resolve(target);
  const rest = [];
  while (!existsSync(current)) {
    const parent = dirname(current);
    if (parent === current) break;
    rest.unshift(parse(current).base);
    current = parent;
  }
  let real = current;
  try {
    real = realpathSync.native(current);
  } catch {
    // Keep the lexical path.
  }
  return join(real, ...rest);
}

/** Folders the simulator must never write into, real and environment-derived. */
export function forbiddenRoots({ env = process.env, home = homedir() } = {}) {
  const roots = new Set();
  const add = (p) => p && roots.add(resolve(p));
  for (const roaming of [env.APPDATA, join(home, "AppData", "Roaming")]) {
    add(roaming && join(roaming, VELORA_IDENTIFIER));
  }
  for (const local of [env.LOCALAPPDATA, join(home, "AppData", "Local")]) {
    add(local && join(local, VELORA_IDENTIFIER));
  }
  return [...roots];
}

/**
 * Throws when `dir` is not a safe place for generated hand histories:
 * inside the Velora app data folder, on a path with a PokerStars folder
 * (`PokerStars`, `PokerStars.PT`, `PokerStars.EU`, ...) such as the client's
 * install or its HandHistory folder, at a drive root, or in a folder that
 * already holds hand histories of another screen name than `hero`.
 */
export function assertSafeOutputDir(dir, { hero, env = process.env, home = homedir() } = {}) {
  if (!dir) throw new Error("no output folder given");
  const lexical = resolve(dir);
  const real = realAncestor(lexical);
  for (const candidate of [lexical, real]) {
    if (parse(candidate).root.toLowerCase() === (candidate + sep).toLowerCase() || norm(candidate) === norm(parse(candidate).root)) {
      throw new Error(`refusing to write into a drive root: ${candidate}`);
    }
    const parts = candidate.split(/[\\/]+/);
    const stars = parts.find((part) => /^pokerstars/i.test(part));
    if (stars) {
      throw new Error(`refusing to write under a PokerStars folder (${stars}): ${candidate}`);
    }
    for (const root of forbiddenRoots({ env, home })) {
      if (isInside(candidate, root)) {
        throw new Error(`refusing to write into the Velora app data folder ${root}: ${candidate}`);
      }
    }
  }
  if (hero && existsSync(lexical)) {
    const stranger = foreignHandHistory(lexical, hero);
    if (stranger) {
      throw new Error(`refusing to write next to real hand histories (${stranger}): ${lexical}`);
    }
  }
  return lexical;
}

/** A `.txt` file (two levels deep at most) dealt to someone other than `hero`. */
function foreignHandHistory(dir, hero, depth = 0) {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return null;
  }
  for (const entry of entries) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (depth < 1) {
        const found = foreignHandHistory(path, hero, depth + 1);
        if (found) return found;
      }
    } else if (entry.name.toLowerCase().endsWith(".txt")) {
      let head = "";
      try {
        if (statSync(path).size === 0) continue;
        head = readFileSync(path, "utf8").slice(0, 20000);
      } catch {
        continue;
      }
      const dealt = /^Dealt to (.+?) \[/m.exec(head);
      if (dealt && dealt[1] !== hero) return path;
    }
  }
  return null;
}
