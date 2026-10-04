#!/usr/bin/env node
// Dev-only hand-history generator: writes PokerStars-format hand histories
// for simulated villains (scripts/sim/profiles.json) into a test folder.
// Never part of the app or its bundle; it refuses to write into a real
// PokerStars folder or the Velora app data folder (lib/guard.mjs).
//
// Usage:
//   node scripts/sim/generate.mjs --out <folder> [--format cash|zoom|mtt|spin]
//     [--seed s] [--tables 6] [--hands 200] [--pace 0] [--clock sim|live]
//     [--start 2026-09-12T20:00:00] [--late-share 0] [--profiles id,id]
//     [--manifest]
//
// --hands is per cash table, in total for Zoom, or per tournament seat for
// mtt (9-max progressive knockouts) and spin (3-max hyper Spin & Go), where
// --tables is the number of tournaments played at once and a new one
// starts when the hero's ends. --pace is real seconds per simulated hand at
// one table (0 = as fast as possible). --late-share (mtt only, 0 to 1) is
// the share of the hero's tournaments he joins at a late-stage table
// (average stack under 20bb), so late-stage play shows up in a short run;
// 0 by default. --profiles seats only the villain profiles with these ids
// (comma-separated, from profiles.json); all of them by default. --manifest
// prints the session manifest (files, seating, tournaments, tilt episodes)
// as JSON.

import { pathToFileURL } from "node:url";
import { runSession } from "./lib/session.mjs";

const FLAGS = {
  out: "string",
  format: "string",
  seed: "string",
  tables: "int",
  hands: "int",
  pace: "number",
  clock: "string",
  start: "string",
  "late-share": "number",
  profiles: "list",
  manifest: "bool",
};

/** `late-share` -> `lateShare`. */
const optionName = (flag) => flag.replace(/-(\w)/g, (_, c) => c.toUpperCase());

export function parseArgs(argv) {
  const options = {};
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    if (!flag.startsWith("--")) throw new Error(`unexpected argument ${flag}`);
    const name = flag.slice(2);
    const kind = FLAGS[name];
    if (!kind) throw new Error(`unknown option ${flag}`);
    if (kind === "bool") {
      options[optionName(name)] = true;
      continue;
    }
    const value = argv[++i];
    if (value === undefined) throw new Error(`${flag} needs a value`);
    if (kind === "int" || kind === "number") {
      const n = Number(value);
      if (!Number.isFinite(n) || (kind === "int" && !Number.isInteger(n))) throw new Error(`${flag} needs a number`);
      options[optionName(name)] = n;
    } else if (kind === "list") {
      const ids = value.split(",").map((id) => id.trim()).filter(Boolean);
      if (ids.length === 0) throw new Error(`${flag} needs at least one id`);
      options.profileIds = ids;
    } else options[optionName(name)] = value;
  }
  if (!options.out) throw new Error("--out <folder> is required");
  if (options.clock && !["sim", "live"].includes(options.clock)) throw new Error("--clock is sim or live");
  if (options.start) {
    const start = new Date(options.start);
    if (Number.isNaN(start.getTime())) throw new Error("--start needs a date such as 2026-09-12T20:00:00");
    options.start = start;
  }
  return options;
}

async function main() {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (err) {
    console.error(`generate: ${err.message}`);
    process.exit(2);
  }
  const { manifest: printManifest, ...session } = options;
  try {
    const manifest = await runSession(session);
    if (printManifest) process.stdout.write(JSON.stringify(manifest) + "\n");
    else console.log(`generate: ${manifest.hands} ${manifest.format} hands in ${manifest.files.length} file(s) under ${manifest.out}`);
  } catch (err) {
    console.error(`generate: ${err.message}`);
    process.exit(1);
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) main();
