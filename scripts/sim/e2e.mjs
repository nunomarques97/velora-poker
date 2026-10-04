#!/usr/bin/env node
// Dev-only end-to-end driver: runs the real app (npm run tauri dev) against
// fake PokerStars tables (fake-tables.ps1) and the hand-history generator,
// captures screenshots, hovers chips and clicks the felt outside HUD
// elements. Never part of the app or its bundle; never touches the real
// PokerStars client.
//
// Isolation: everything lives in a temporary root. The app runs with
// APPDATA, LOCALAPPDATA, WEBVIEW2_USER_DATA_FOLDER and TEMP/TMP inside it
// (its database path comes from APPDATA, lib.rs), on a database seeded there
// through the app's own migrations (examples/sim_seed.rs). The driver
// refuses to start if any of those paths is the real Velora data folder or
// a PokerStars folder, and every file it writes is checked against the root
// and the screenshot folder (lib/e2e-guard.mjs). Cargo's own build output
// (src-tauri/target) is the normal dev build, as with any `tauri dev`.
//
// Usage:
//   node scripts/sim/e2e.mjs --dry-run [options]   checks and prints the plan; opens nothing
//   node scripts/sim/e2e.mjs [options]             runs the session (opens windows!)
// Options:
//   --features strategic-analysis   build with the opponent engine (reads, hover card)
//   --root <dir>        temporary root (default: a new folder under the system temp)
//   --shots <dir>       screenshot folder (default: <root>/shots)
//   --seed <s>          generator seed (default e2e)
//   --cash <n>          cash 6-max tables (default 4)
//   --mtt <n>           MTT 9-max tables (default 4)
//   --hands <n>         hands per cash table / per tournament seat (default 60)
//   --backlog <n>       hands per cash table imported before launch (default 0)
//   --pace <s>          real seconds per hand at one table (default 1.5)
//   --shot-every <n>    screenshot round every n hands written (default 50)
//   --launch-timeout <s> wait for the app's main window (default 900)
//   --keep-main         leave the app's main window up (default: minimized)
//   --observe           no mouse, keyboard or screenshots: the session runs and
//                       records the windows only (also on a locked session,
//                       where nothing on screen can be checked)
//   --inspect <port>    also open the app's WebView2 DevTools port on loopback
//                       (its own temporary environment only) and, every round,
//                       read each overlay's chips from its DOM, render each
//                       table with its HUD on top (PrintWindow + the page's
//                       own capture, no screen involved), hover a chip, open
//                       a drawer once, show the side panel and search it.
//                       Works on a locked session too.

import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, openSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { FakeTables, primaryWorkArea, probe } from "./fake-tables.mjs";
import {
  appEnvironment,
  assertSafeE2ePaths,
  assertUsableRoot,
  createWriteScope,
  e2ePaths,
  ROOT_MARKER,
} from "./lib/e2e-guard.mjs";
import { CdpPage, inspectArguments, listPages, OVERLAY_STATE, PANEL_STATE, pageKind, panelSearch } from "./lib/cdp.mjs";
import { assertPlainArgs, killTree, runUnderVcvars, spawnUnderVcvars, VCVARS } from "./lib/msvc.mjs";
import { loadProfiles } from "./lib/session.mjs";
import {
  chipHoverPoints,
  FELT_CLICK_POINT,
  planLayout,
  plannedCashTables,
  pointIn,
  scanTables,
  TABLE_CLASS,
  tableTitle,
} from "./lib/tables.mjs";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const GENERATOR = join(REPO, "scripts", "sim", "generate.mjs");
export const ALLOWED_FEATURES = ["strategic-analysis", "auto-classification"];
/** Ctrl+Alt+P: the side panel's default shortcut (settings::SETTING_PANEL_SHORTCUT). */
const PANEL_SHORTCUT = { ctrl: true, alt: true, shift: false, vk: 0x50 };
const MAIN_WINDOW_TITLE = "Velora Poker";

const FLAGS = {
  "dry-run": "bool",
  features: "string",
  root: "string",
  shots: "string",
  seed: "string",
  cash: "int",
  mtt: "int",
  hands: "int",
  backlog: "int",
  pace: "number",
  "shot-every": "int",
  "launch-timeout": "int",
  "keep-main": "bool",
  observe: "bool",
  inspect: "int",
};

export function parseArgs(argv) {
  const o = {
    dryRun: false,
    features: [],
    root: null,
    shots: null,
    seed: "e2e",
    cash: 4,
    mtt: 4,
    hands: 60,
    backlog: 0,
    pace: 1.5,
    shotEvery: 50,
    launchTimeout: 900,
    keepMain: false,
    observe: false,
    inspect: 0,
  };
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    const name = flag.startsWith("--") ? flag.slice(2) : null;
    const kind = name && FLAGS[name];
    if (!kind) throw new Error(`unknown option ${flag}`);
    const key = name.replace(/-([a-z])/g, (_, c) => c.toUpperCase());
    if (kind === "bool") {
      o[key] = true;
      continue;
    }
    const value = argv[++i];
    if (value === undefined) throw new Error(`${flag} needs a value`);
    if (kind === "int" || kind === "number") {
      const n = Number(value);
      if (!Number.isFinite(n) || n < 0 || (kind === "int" && !Number.isInteger(n))) throw new Error(`${flag} needs a number`);
      o[key] = n;
    } else if (name === "features") {
      o.features = value.split(",").filter(Boolean);
      for (const f of o.features) if (!ALLOWED_FEATURES.includes(f)) throw new Error(`unsupported feature ${f}`);
    } else o[key] = value;
  }
  if (o.cash + o.mtt < 1) throw new Error("at least one table is needed");
  if (o.cash + o.mtt > 12) throw new Error("12 tables at most");
  if (o.hands < 1) throw new Error("--hands must be at least 1");
  if (o.shotEvery < 1) throw new Error("--shot-every must be at least 1");
  if (!/^[A-Za-z0-9_-]{1,40}$/.test(o.seed)) throw new Error("--seed: letters, digits, - and _ only");
  if (o.inspect) inspectArguments(o.inspect);
  return o;
}

/** Window slots: cash 6-max and MTT 9-max, alternating minimum and medium size. */
export function tableSlots({ cash, mtt }) {
  const slots = [];
  for (let i = 0; i < cash; i++) slots.push({ kind: "cash", seats: 6, size: i % 2 === 0 ? "min" : "medium" });
  for (let i = 0; i < mtt; i++) slots.push({ kind: "mtt", seats: 9, size: i % 2 === 0 ? "min" : "medium" });
  return slots;
}

/** The generator command lines of the live session (and the optional backlog). */
export function generatorRuns(o, paths) {
  const common = ["--out", paths.handHistory, "--hands", String(o.hands), "--pace", String(o.pace), "--clock", "live"];
  const runs = [];
  if (o.backlog > 0 && o.cash > 0) {
    runs.push({
      phase: "backlog",
      args: ["--out", paths.handHistory, "--format", "cash", "--seed", `${o.seed}-backlog`, "--tables", String(o.cash)]
        .concat(["--hands", String(o.backlog), "--pace", "0", "--clock", "sim", "--start", "2026-09-01T20:00:00"]),
    });
  }
  if (o.cash > 0) runs.push({ phase: "live", format: "cash", args: [...common, "--format", "cash", "--seed", o.seed, "--tables", String(o.cash)] });
  if (o.mtt > 0) runs.push({ phase: "live", format: "mtt", args: [...common, "--format", "mtt", "--seed", o.seed, "--tables", String(o.mtt)] });
  return runs;
}

/**
 * Everything the session will do, computed without side effects: paths,
 * environment overrides, window slots with their rects and titles, the
 * generator, seeder and app command lines, and the steps.
 */
export function buildPlan(o, { area, root, profiles = loadProfiles() }) {
  const hero = profiles.hero.name;
  const paths = e2ePaths(root, hero);
  const shots = o.shots ? resolve(o.shots) : join(paths.root, "shots");
  const featureArgs = o.features.length ? ["--features", o.features.join(",")] : [];
  const cashTables = plannedCashTables({ seed: o.seed, tables: o.cash, stakes: profiles.stakes.cash });
  const slots = planLayout(tableSlots(o), area).map((slot) => {
    if (slot.kind !== "cash") return { ...slot, table: null, title: null };
    const table = cashTables[slot.index];
    return { ...slot, table: table.name, title: tableTitle(table, hero, 0) };
  });
  return {
    hero,
    paths,
    shots,
    area,
    features: o.features,
    env: {
      APPDATA: paths.appData,
      LOCALAPPDATA: paths.localAppData,
      WEBVIEW2_USER_DATA_FOLDER: paths.webview,
      TEMP: paths.tmp,
      TMP: paths.tmp,
      ...(o.inspect ? { WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: inspectArguments(o.inspect) } : {}),
    },
    slots,
    seed: { command: "cargo", args: ["run", "--example", "sim_seed", ...featureArgs, "--", paths.db, paths.handHistoryRoot] },
    // --no-watch: an edit under src-tauri during the session must not
    // rebuild and restart the app halfway through it.
    app: { command: "npm", args: ["run", "tauri", "dev", "--", "--no-watch", ...featureArgs] },
    generators: generatorRuns(o, paths),
    steps: [
      `guard every path, create ${paths.root} with ${ROOT_MARKER}`,
      `seed ${paths.db} with the app's migrations (cargo run --example sim_seed, under vcvars)`,
      o.backlog > 0 ? `write a backlog of ${o.backlog} hands per cash table (simulated clock)` : null,
      `start the fake tables (${TABLE_CLASS}); open the ${o.cash} cash table(s) before their first hand`,
      `launch the app (npm run tauri dev -- --no-watch${featureArgs.length ? ` ${featureArgs.join(" ")}` : ""}) with the overridden environment; wait for "${MAIN_WINDOW_TITLE}"${o.keepMain ? "" : ", then minimize it"}`,
      `start the generators (live clock, ${o.pace}s per hand); open an MTT window when its tournament file appears, close it when the hero's tournament ends`,
      o.observe
        ? `every ${o.shotEvery} hands: list the overlay windows (observe only: no mouse, keyboard or screenshots)`
        : `every ${o.shotEvery} hands: desktop and per-table screenshots, hover a villain chip, click the felt outside HUD elements and read the click counter, list the overlay windows; once: toggle the side panel (Ctrl+Alt+P)`,
      o.inspect
        ? `every round also, through the app's WebView2 DevTools port ${o.inspect} (loopback): read each overlay's chips, render each table with its HUD on top (no screen involved), hover a chip; once: open and close a drawer; from the second round: show the side panel, read it and search a villain`
        : null,
      "after the second round, once: move a cash table and put it back, close another and reopen it, listing the overlay windows after each step",
      `write ${paths.summary}; close every fake window, the generators and the app`,
    ].filter(Boolean),
  };
}

/**
 * Why the desktop cannot be driven, from a fake-tables probe, or null. A
 * locked session shows the lock screen over every window and takes the mouse
 * and keyboard: screenshots would show the lock screen and no click or
 * shortcut would reach a table, the HUD or the side panel.
 */
export function desktopBlocker(probed) {
  if (probed?.locked === true) {
    return "the Windows session is locked: the lock screen covers every window and takes the mouse and keyboard, so screenshots show the lock screen and no click or shortcut reaches a table; unlock the session and leave the computer alone while the driver runs";
  }
  return null;
}

const inside = (rect, p) => p.x >= rect.x && p.x < rect.x + rect.width && p.y >= rect.y && p.y < rect.y + rect.height;

/**
 * The open tables (in opening order, so the last is on top) whose points
 * `pointsOf(entry)` no later window covers: on a small screen the windows
 * overlap, and a hover or click on a covered point lands on another table.
 */
export function exposedTables(entries, pointsOf) {
  return entries.filter(([, e], i) => {
    const later = entries.slice(i + 1).map(([, l]) => l.slot.rect);
    return pointsOf(e).every((p) => !later.some((r) => inside(r, p)));
  });
}

// ---------------------------------------------------------------- dry run

function check(name, fn) {
  try {
    const detail = fn();
    return { name, ok: true, detail: detail ?? "" };
  } catch (err) {
    return { name, ok: false, detail: err.message };
  }
}

function underVcvars(command, args) {
  const run = runUnderVcvars(command, args, { cwd: join(REPO, "src-tauri"), timeoutMs: 120_000 });
  if (run.error) throw run.error;
  if (run.status !== 0) throw new Error((run.stderr || run.stdout || `exit ${run.status}`).trim().split(/\r?\n/).pop());
  return run.stdout.trim().split(/\r?\n/)[0];
}

async function dryRun(o) {
  const checks = [];
  checks.push(check("Windows", () => {
    if (process.platform !== "win32") throw new Error(`platform ${process.platform}`);
  }));
  checks.push(check("Node 22+", () => {
    if (Number(process.versions.node.split(".")[0]) < 22) throw new Error(process.version);
    return process.version;
  }));
  checks.push(check("VS2022 BuildTools vcvars64.bat", () => {
    if (!existsSync(VCVARS)) throw new Error(`not found: ${VCVARS}`);
    return VCVARS;
  }));
  checks.push(check("MSVC compiler under vcvars", () => underVcvars("where", ["cl.exe"])));
  checks.push(check("cargo under vcvars", () => underVcvars("cargo", ["--version"])));
  checks.push(check("npm", () => underVcvars("npm", ["--version"])));
  checks.push(check("Tauri CLI installed", () => {
    const cli = join(REPO, "node_modules", "@tauri-apps", "cli", "package.json");
    if (!existsSync(cli)) throw new Error("run npm install first");
    return JSON.parse(readFileSync(cli, "utf8")).version;
  }));

  // The C# compile needs a TEMP; give it a throwaway root of the dry run's own.
  const scratch = mkdtempSync(join(tmpdir(), "velora-e2e-dryrun-"));
  let probed = null;
  checks.push(check("PowerShell + C# compile of the fake tables (no window)", () => {
    probed = probe({ env: { ...process.env, TEMP: scratch, TMP: scratch } });
    return `${probed.monitors.length} monitor(s), class ${probed.className}`;
  }));
  checks.push(check("Fake table titles read back whole by other processes (what table_track sees)", () => {
    if (!probed) throw new Error("no probe");
    if (probed.titleRoundTrip !== true) throw new Error("a window title reads back truncated: table_track would track no table");
    return "ok";
  }));
  rmSync(scratch, { recursive: true, force: true });

  // Not a failure of the dry run: the environment is ready, only the
  // moment is wrong. A real run refuses to start until it is cleared.
  const blocker = probed ? desktopBlocker(probed) : null;
  const area = probed ? primaryWorkArea(probed) : { x: 0, y: 0, width: 1920, height: 1040 };
  const root = o.root ? resolve(o.root) : join(tmpdir(), "velora-e2e-<timestamp>");
  const plan = buildPlan(o, { area, root });
  checks.push(check("Path guards (temp data, never the real Velora or PokerStars folders)", () => {
    assertSafeE2ePaths(plan.paths, { shots: plan.shots, hero: plan.hero });
    if (o.root) assertUsableRoot(plan.paths.root);
    appEnvironment(plan.paths);
    return "ok";
  }));
  checks.push(check("Root path passes to cmd.exe verbatim (no spaces or metacharacters)", () => {
    assertPlainArgs([o.root ? resolve(o.root) : tmpdir()]);
    return "ok";
  }));

  const line = (s = "") => console.log(s);
  line("Velora e2e driver: dry run (no window opened, app not launched)");
  line();
  line("Checks");
  for (const c of checks) line(`  [${c.ok ? "ok" : "FAIL"}] ${c.name}${c.detail ? `: ${c.detail}` : ""}`);
  line(blocker ? `  [warn] Desktop can be driven now: no, ${blocker}` : "  [ok] Desktop can be driven now (session not locked)");
  line();
  line("Paths");
  for (const [k, v] of Object.entries(plan.paths)) line(`  ${k.padEnd(16)}${v}`);
  line(`  ${"shots".padEnd(16)}${plan.shots}`);
  line();
  line("App environment overrides");
  for (const [k, v] of Object.entries(plan.env)) line(`  ${k}=${v}`);
  line();
  line(`Tables (primary work area ${area.width}x${area.height} at ${area.x},${area.y})`);
  for (const s of plan.slots) {
    const r = s.rect;
    const title = s.title ?? `(set when its tournament file appears, e.g. "${tableTitle(
      { kind: "mtt", name: "4100000001 7", maxSeats: 9, sb: 75, bb: 150, buyIn: [500, 500, 100] },
      plan.hero,
    )}")`;
    line(`  #${s.index + 1} ${s.kind} ${s.seats}-max ${s.size} ${r.width}x${r.height} at ${r.x},${r.y}`);
    line(`      ${title}`);
  }
  line();
  line("Commands");
  line(`  seed:  ${plan.seed.command} ${plan.seed.args.join(" ")}   (cwd src-tauri, under vcvars)`);
  line(`  app:   ${plan.app.command} ${plan.app.args.join(" ")}   (under vcvars, overridden environment)`);
  for (const g of plan.generators) line(`  gen:   node scripts/sim/generate.mjs ${g.args.join(" ")}`);
  line();
  line("Steps");
  plan.steps.forEach((s, i) => line(`  ${i + 1}. ${s}`));
  const failed = checks.filter((c) => !c.ok);
  line();
  line(failed.length ? `dry run: ${failed.length} check(s) failed` : "dry run: ready");
  return failed.length ? 1 : 0;
}

// ---------------------------------------------------------------- live run

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

/** The player a chip belongs to, from its accessible name ("Name, 30 hands, ..."). */
export const chipName = (chip) => (chip.label ?? "").split(",")[0].trim();

/** Chips of players other than the hero. */
export const villainChips = (chips, hero) => chips.filter((c) => chipName(c) && chipName(c) !== hero);

/** The chip to hover in round `n`: rotating over villains with a tag, else over any villain. */
export function pickChip(chips, n, hero) {
  const villains = villainChips(chips, hero);
  const tagged = villains.filter((c) => /, tag [^,]+,/.test(c.label ?? ""));
  const pool = tagged.length ? tagged : villains;
  return pool[(n - 1) % pool.length] ?? null;
}

/** Round log suffix for --inspect. */
function inspected(x) {
  if (!x) return "";
  const chips = x.overlays.reduce((sum, o) => sum + o.chips.length, 0);
  const parts = [`${x.overlays.length} overlay page(s), ${chips} chip(s)`];
  if (x.hover) parts.push(`hover card ${x.hover.card ? "shown" : "NOT shown"}`);
  if (x.drawer) parts.push(`drawer ${x.drawer.text ? "opened" : "NOT opened"}`);
  if (x.panel) parts.push(`panel ${x.panel.search ? `search "${x.panel.search.query}": ${x.panel.search.result}` : "read"}`);
  if (x.errors.length) parts.push(`${x.errors.length} error(s): ${x.errors[0]}`);
  return `; inspect: ${parts.join(", ")}`;
}

class Session {
  constructor(o, plan) {
    this.o = o;
    this.plan = plan;
    this.paths = plan.paths;
    this.scope = createWriteScope({ root: this.paths.root, shots: plan.shots });
    this.children = [];
    this.app = null;
    this.tables = null;
    this.open = new Map(); // fake window id -> { slot, table, title, closing }
    this.events = [];
    this.rounds = [];
    this.counter = 0;
    this.cleaned = false;
    this.env = appEnvironment(this.paths);
    if (plan.env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS) {
      this.env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = plan.env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS;
    }
  }

  log(message) {
    const text = `[e2e ${new Date().toISOString().slice(11, 19)}] ${message}`;
    console.log(text);
  }

  event(op, detail) {
    this.events.push({ at: new Date().toISOString(), op, ...detail });
  }

  logFile(name) {
    return openSync(this.scope.allow(join(this.paths.logs, name)), "a");
  }

  prepare() {
    assertSafeE2ePaths(this.paths, { shots: this.plan.shots, hero: this.plan.hero });
    assertUsableRoot(this.paths.root);
    assertPlainArgs([this.paths.db, this.paths.handHistoryRoot]);
    for (const dir of [this.paths.root, this.paths.appData, this.paths.localAppData, this.paths.webview, this.paths.handHistory, this.paths.logs, this.paths.tmp, this.plan.shots]) {
      mkdirSync(this.scope.allow(dir), { recursive: true });
    }
    writeFileSync(this.scope.allow(this.paths.marker), "Velora e2e driver root. Safe to delete.\n");
  }

  seed() {
    if (existsSync(this.paths.db)) throw new Error(`refusing: ${this.paths.db} already exists`);
    this.log("seeding the temporary database (cargo run --example sim_seed)");
    const run = runUnderVcvars(this.plan.seed.command, this.plan.seed.args, {
      cwd: join(REPO, "src-tauri"),
      env: this.env,
      timeoutMs: 1_800_000,
    });
    writeFileSync(this.scope.allow(join(this.paths.logs, "seed.log")), `${run.stdout ?? ""}\n${run.stderr ?? ""}`);
    if (run.status !== 0) throw new Error(`seeding failed (${run.status}); see ${join(this.paths.logs, "seed.log")}`);
  }

  generator(run, name) {
    const out = this.logFile(`${name}.log`);
    const child = spawn(process.execPath, [GENERATOR, ...run.args], {
      cwd: REPO,
      env: this.env,
      stdio: ["ignore", out, out],
      windowsHide: true,
    });
    const entry = { name, child, exit: null };
    child.on("exit", (code) => (entry.exit = code ?? -1));
    this.children.push(entry);
    return entry;
  }

  async backlog() {
    const run = this.plan.generators.find((g) => g.phase === "backlog");
    if (!run) return;
    this.log(`writing a backlog of ${this.o.backlog} hands per cash table`);
    const entry = this.generator(run, "generator-backlog");
    while (entry.exit === null) await sleep(200);
    if (entry.exit !== 0) throw new Error("the backlog generator failed; see logs/generator-backlog.log");
  }

  async startTables() {
    this.tables = await FakeTables.start({ statusPath: this.scope.allow(this.paths.status), env: this.env });
    for (const slot of this.plan.slots.filter((s) => s.kind === "cash")) await this.openTable(slot, slot.table, slot.title);
    if (!this.o.observe) await this.calibrate();
  }

  /**
   * Before the app starts: a left click on the first table's felt must reach
   * it, or nothing the session measures would mean anything (a lock screen
   * or a window on top takes the clicks).
   */
  async calibrate() {
    // The last window opened is on top.
    const [id, e] = [...this.open].at(-1) ?? [];
    if (!id) return;
    await sleep(800);
    const felt = pointIn(e.slot.rect, FELT_CLICK_POINT);
    const count = async () => (await this.tables.status()).tables.find((t) => t.id === id)?.clicks ?? 0;
    const before = await count();
    await this.tables.click(felt);
    await sleep(400);
    const after = await count();
    const shot = await this.shot("r00-calibration.png", e.slot.rect);
    this.event("calibrate", { id, point: felt, before, after, shot });
    if (after <= before) {
      throw new Error(`a left click on the felt of ${id} did not reach it (${before} -> ${after} clicks): another window covers it or the session is locked; see ${shot}`);
    }
    this.log(`calibration click reached ${id}`);
  }

  async openTable(slot, table, title) {
    const id = `t${slot.index + 1}-${++this.counter}`;
    await this.tables.open(id, title, slot.rect, slot.seats);
    this.open.set(id, { slot, table, title, closing: null });
    this.event("open", { id, table, title, rect: slot.rect });
    this.log(`opened ${id}: ${title}`);
  }

  async closeTable(id) {
    const entry = this.open.get(id);
    await this.tables.close(id);
    this.open.delete(id);
    this.event("close", { id, table: entry?.table });
    this.log(`closed ${id} (${entry?.table})`);
  }

  async launchApp() {
    this.log(`launching the app: ${this.plan.app.command} ${this.plan.app.args.join(" ")}`);
    const out = this.logFile("app.log");
    this.app = spawnUnderVcvars(this.plan.app.command, this.plan.app.args, { cwd: REPO, env: this.env, stdio: ["ignore", out, out] });
    let appExit = null;
    this.app.on("exit", (code) => (appExit = code ?? -1));
    const deadline = Date.now() + this.o.launchTimeout * 1000;
    while (Date.now() < deadline) {
      if (appExit !== null) throw new Error(`the app exited (${appExit}) before its window appeared; see logs/app.log`);
      const windows = await this.tables.list(MAIN_WINDOW_TITLE);
      const main = windows.find((w) => w.title === MAIN_WINDOW_TITLE && w.visible && w.className !== TABLE_CLASS);
      if (main) {
        this.event("app-window", { hwnd: main.hwnd, pid: main.pid, rect: main.rect });
        this.log(`app window up (pid ${main.pid})`);
        if (!this.o.keepMain) await this.tables.show(main.hwnd, 6);
        await sleep(5000);
        return;
      }
      await sleep(2000);
    }
    throw new Error(`the app's window did not appear in ${this.o.launchTimeout}s; see logs/app.log`);
  }

  /** Opens/closes tournament windows to follow the hand-history folder. */
  async followFolder() {
    const tables = scanTables(this.paths.handHistory, this.plan.hero);
    const byName = new Map([...this.open].map(([id, e]) => [e.table, id]));
    const now = Date.now();
    for (const t of tables) {
      if (t.kind !== "mtt" && t.kind !== "spin") continue;
      const id = byName.get(t.name);
      if (id && t.heroOut) {
        const entry = this.open.get(id);
        entry.closing ??= now + 5000;
        if (now >= entry.closing) await this.closeTable(id);
      } else if (!id && !t.heroOut && !this.seen?.has(t.name)) {
        const used = new Set([...this.open.values()].map((e) => e.slot.index));
        const slot = this.plan.slots.find((s) => s.kind === "mtt" && !used.has(s.index));
        if (!slot) continue;
        (this.seen ??= new Set()).add(t.name);
        await this.openTable(slot, t.name, tableTitle(t, this.plan.hero, 0));
      }
    }
    return tables.reduce((sum, t) => sum + t.hands, 0);
  }

  shotPath(name) {
    return this.scope.allow(join(this.plan.shots, name));
  }

  async shot(name, rect) {
    await this.tables.shot(this.shotPath(name), rect);
    return name;
  }

  async overlayWindows() {
    return (await this.tables.list("Velora")).map(({ hwnd, title, visible, rect }) => ({ hwnd, title, visible, rect }));
  }

  async round(hands, { panel = false } = {}) {
    const n = this.rounds.length + 1;
    const tag = String(n).padStart(2, "0");
    const record = { round: n, hands, at: new Date().toISOString(), shots: [], hover: null, click: null, overlays: null, panel: null };
    const area = this.plan.area;
    if (this.o.inspect) record.inspect = await this.inspect(n, tag);
    if (this.o.observe) {
      record.overlays = await this.overlayWindows();
      record.tables = [...this.open].map(([id, e]) => ({ id, table: e.table }));
      this.rounds.push(record);
      this.log(`round ${n}: ${hands} hands, ${record.overlays.length} overlay window(s), ${record.overlays.filter((w) => w.visible).length} visible${inspected(record.inspect)}`);
      return;
    }
    record.shots.push(await this.shot(`r${tag}-desktop.png`, area));
    for (const [id, e] of this.open) {
      record.shots.push(await this.shot(`r${tag}-${id}-${e.slot.kind}-${e.slot.seats}max-${e.slot.size}.png`, e.slot.rect));
    }
    const hoverOf = (e) => {
      const points = chipHoverPoints(e.slot.seats);
      return pointIn(e.slot.rect, points[(n - 1) % points.length]);
    };
    const entries = exposedTables([...this.open], (e) => [hoverOf(e), pointIn(e.slot.rect, FELT_CLICK_POINT)]);
    if (!entries.length && this.open.size) this.log(`round ${n}: every table is partly covered; no hover or click this round`);
    if (entries.length) {
      const [id, e] = entries[(n - 1) % entries.length];
      const target = hoverOf(e);
      await this.tables.mouse(target);
      await sleep(900);
      record.hover = { id, table: e.table, point: target, shot: await this.shot(`r${tag}-${id}-hover.png`, e.slot.rect) };
      const felt = pointIn(e.slot.rect, FELT_CLICK_POINT);
      const before = (await this.tables.status()).tables.find((t) => t.id === id)?.clicks ?? null;
      await this.tables.click(felt);
      await sleep(400);
      const after = (await this.tables.status()).tables.find((t) => t.id === id)?.clicks ?? null;
      record.click = { id, table: e.table, point: felt, before, after, reached: after !== null && before !== null && after > before };
      await this.tables.mouse({ x: area.x + area.width - 2, y: area.y + area.height - 2 });
    }
    record.overlays = await this.overlayWindows();
    if (panel) {
      await this.tables.keys(PANEL_SHORTCUT);
      await sleep(1500);
      const shown = await this.shot(`r${tag}-panel-shown.png`, area);
      await this.tables.keys(PANEL_SHORTCUT);
      await sleep(1000);
      record.panel = { shown, hidden: await this.shot(`r${tag}-panel-hidden.png`, area) };
    }
    this.rounds.push(record);
    this.log(`round ${n}: ${hands} hands, ${record.shots.length} shots${record.click ? `, felt click ${record.click.reached ? "reached" : "did NOT reach"} ${record.click.id}` : ""}`);
  }

  /**
   * --inspect: what the real app's pages show, read through the WebView2
   * DevTools port: every overlay's chips (matched to its fake table by
   * window position), each table rendered with that HUD on top, one chip
   * hovered (rotating), one drawer opened and closed once, and from the
   * second round the side panel shown, read and searched. Pointer events go
   * to the page, not the desktop. Errors are recorded, never fatal.
   */
  async inspect(n, tag) {
    const out = { overlays: [], hover: null, drawer: null, panel: null, errors: [] };
    const fail = (what, err) => out.errors.push(`${what}: ${err.message}`);
    let pages = [];
    try {
      pages = await listPages(this.o.inspect);
    } catch (err) {
      fail("list pages", err);
      return out;
    }
    const overlays = [];
    for (const page of pages.filter((p) => pageKind(p.url) === "overlay")) {
      let cdp;
      try {
        cdp = await CdpPage.connect(page.ws);
        const state = await cdp.evaluate(
          `(() => { const s = ${OVERLAY_STATE}; s.screen = { x: screenX, y: screenY }; s.label = window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label ?? null; return s; })()`,
        );
        const fake = [...this.open].find(([, e]) => e.slot.rect.x === state.screen.x && e.slot.rect.y === state.screen.y);
        const entry = { label: state.label, url: state.url, size: state.size, screen: state.screen, id: fake?.[0] ?? null, table: fake?.[1].table ?? null, chips: state.chips };
        if (fake && state.chips.length) {
          const e = fake[1];
          entry.shot = await this.composite(cdp, fake, `r${tag}-${fake[0]}-${e.slot.kind}-${e.slot.seats}max-${e.slot.size}`);
        }
        out.overlays.push(entry);
        overlays.push({ cdp, entry, fake });
      } catch (err) {
        fail(`overlay ${page.url}`, err);
        cdp?.close();
      }
    }
    try {
      const withChips = overlays.filter((x) => x.fake && villainChips(x.entry.chips, this.plan.hero).length);
      if (withChips.length) {
        const { cdp, entry, fake } = withChips[(n - 1) % withChips.length];
        const chip = pickChip(entry.chips, n, this.plan.hero);
        const at = { x: chip.rect.x + chip.rect.width / 2, y: chip.rect.y + chip.rect.height / 2 };
        await cdp.mouseMove(at.x, at.y);
        await sleep(900);
        const state = await cdp.evaluate(OVERLAY_STATE);
        out.hover = { id: fake[0], table: entry.table, chip: chip.text, label: chip.label, card: state.hoverCard };
        out.hover.shot = await this.composite(cdp, fake, `r${tag}-${fake[0]}-hover`);
        await cdp.mouseMove(1, 1);
        await sleep(400);
        out.hover.closedOnLeave = (await cdp.evaluate(OVERLAY_STATE)).hoverCard === null;
        if (!this.drawerDone) {
          this.drawerDone = true;
          await cdp.click(at.x, at.y);
          await sleep(1200);
          const opened = await cdp.evaluate(OVERLAY_STATE);
          out.drawer = { id: fake[0], table: entry.table, chip: chip.text, text: opened.drawer?.slice(0, 1500) ?? null };
          out.drawer.shot = await this.composite(cdp, fake, `r${tag}-${fake[0]}-drawer`);
          const hit = `(() => { const el = document.elementFromPoint(${at.x}, ${at.y}); return el ? el.tagName + (el.closest("[data-hud-chip]") ? " in chip" : "") + (el.closest("aside") ? " in drawer" : "") : null; })()`;
          out.drawer.secondClickHits = await cdp.evaluate(hit);
          await cdp.click(at.x, at.y);
          await sleep(800);
          out.drawer.closedAgain = (await cdp.evaluate(OVERLAY_STATE)).drawer === null;
        }
      }
    } catch (err) {
      fail("hover/drawer", err);
    }
    for (const { cdp } of overlays) cdp.close();
    const seated = out.overlays.flatMap((o) => villainChips(o.chips, this.plan.hero)).map(chipName);
    if (n >= 2) out.panel = await this.inspectPanel(pages, tag, seated).catch((err) => (fail("panel", err), null));
    return out;
  }

  /** Renders fake table `id` with the page's own capture (its HUD) on top. */
  async composite(cdp, [id], name) {
    const hud = this.shotPath(`${name}-hud.png`);
    writeFileSync(hud, await cdp.screenshot());
    const file = `${name}.png`;
    await this.tables.print(id, this.shotPath(file), hud);
    return file;
  }

  /** Shows the side panel (the command HUD Profiles → Open side panel calls), reads it and searches a villain. */
  async inspectPanel(pages, tag, seated) {
    const main = pages.find((p) => pageKind(p.url) === "main");
    const panelPage = pages.find((p) => pageKind(p.url) === "panel");
    if (!panelPage) throw new Error("no side panel page");
    if (!this.panelShown && main) {
      const cdp = await CdpPage.connect(main.ws);
      try {
        await cdp.evaluate(`window.__TAURI_INTERNALS__.invoke("show_side_panel").then(() => true)`);
      } finally {
        cdp.close();
      }
      this.panelShown = true;
      await sleep(1500);
    }
    const cdp = await CdpPage.connect(panelPage.ws);
    try {
      const out = { before: await cdp.evaluate(PANEL_STATE) };
      out.shot = `r${tag}-panel.png`;
      writeFileSync(this.shotPath(out.shot), await cdp.screenshot());
      // Rows start collapsed: search for a villain an overlay shows, rotating.
      const players = loadProfiles().profiles.flatMap((p) => p.players).filter((p) => seated.includes(p));
      const name = players[(Number(tag) - 2) % Math.max(players.length, 1)];
      if (name) {
        const query = name.slice(1, 6).toLowerCase();
        await cdp.evaluate(panelSearch(query));
        await sleep(700);
        const { text, ...found } = await cdp.evaluate(PANEL_STATE);
        out.search = { player: name, query, ...found, shot: `r${tag}-panel-search.png` };
        writeFileSync(this.shotPath(out.search.shot), await cdp.screenshot());
        await cdp.evaluate(panelSearch(""));
      }
      return out;
    } finally {
      cdp.close();
    }
  }

  /**
   * Once per run: moves a cash table and puts it back, then closes another
   * and reopens it, listing the overlay windows after each step. The HUD must
   * follow the move, hide the closed table's overlay without destroying it
   * (the window count stays) and reuse a pooled window for the reopened one.
   */
  async churn() {
    const cash = [...this.open].filter(([, e]) => e.slot.kind === "cash");
    if (cash.length < 2) return;
    const record = { at: new Date().toISOString(), steps: [] };
    const step = async (op, detail, waitMs, shotRect) => {
      await sleep(waitMs);
      const entry = { op, ...detail, overlays: await this.overlayWindows() };
      if (shotRect && !this.o.observe) entry.shot = await this.shot(`churn-${op}.png`, shotRect);
      record.steps.push(entry);
    };
    await step("before", {}, 0);
    const [moveId, moved] = cash[0];
    const home = moved.slot.rect;
    const away = { ...home, x: home.x + 30, y: home.y + 20 };
    await this.tables.move(moveId, away);
    this.event("move", { id: moveId, table: moved.table, rect: away });
    await step("moved", { id: moveId, rect: away }, 2000, away);
    await this.tables.move(moveId, home);
    this.event("move", { id: moveId, table: moved.table, rect: home });
    await step("moved-back", { id: moveId, rect: home }, 2000);
    const [closeId, closed] = cash.at(-1);
    await this.closeTable(closeId);
    await step("closed", { id: closeId, table: closed.table }, 3000);
    await this.openTable(closed.slot, closed.table, closed.title);
    await step("reopened", { table: closed.table }, 4000, closed.slot.rect);
    this.churned = record;
    const counts = record.steps.map((x) => `${x.op} ${x.overlays.length}/${x.overlays.filter((w) => w.visible).length}`);
    this.log(`churn (overlay windows total/visible): ${counts.join(", ")}`);
  }

  async play() {
    const live = this.plan.generators.filter((g) => g.phase === "live").map((g) => this.generator(g, `generator-${g.format}`));
    let nextShot = this.o.shotEvery;
    let panelDone = false;
    let hands = 0;
    while (live.some((g) => g.exit === null)) {
      hands = await this.followFolder();
      if (hands >= nextShot) {
        await this.round(hands, { panel: !panelDone && this.rounds.length >= 1 });
        panelDone ||= this.rounds.at(-1).panel !== null;
        if (this.rounds.length === 2 && !this.churned) await this.churn();
        while (nextShot <= hands) nextShot += this.o.shotEvery;
      }
      await sleep(1000);
    }
    for (const g of live) if (g.exit !== 0) this.log(`generator ${g.name} exited with ${g.exit}`);
    await sleep(8000); // the last hands reach the HUD
    hands = await this.followFolder();
    await this.round(hands, { panel: !panelDone });
    return hands;
  }

  writeSummary(extra) {
    const summary = {
      options: this.o,
      paths: this.paths,
      shots: this.plan.shots,
      slots: this.plan.slots,
      events: this.events,
      rounds: this.rounds,
      churn: this.churned ?? null,
      generators: this.children.map((c) => ({ name: c.name, exit: c.exit })),
      ...extra,
    };
    writeFileSync(this.scope.allow(this.paths.summary), JSON.stringify(summary, null, 2));
  }

  /** Closes every fake window, the generators and the app; safe to call twice. */
  async cleanup() {
    if (this.cleaned) return;
    this.cleaned = true;
    for (const c of this.children) if (c.exit === null) killTree(c.child.pid);
    if (this.tables) await this.tables.quit().catch(() => this.tables.kill());
    if (this.app) killTree(this.app.pid);
    this.log("closed the fake tables, the generators and the app");
  }

  /** Last-resort synchronous cleanup on process exit. */
  cleanupSync() {
    if (this.cleaned) return;
    this.cleaned = true;
    for (const c of this.children) if (c.exit === null) killTree(c.child.pid);
    if (this.tables) this.tables.kill();
    if (this.app) killTree(this.app.pid);
  }
}

async function liveRun(o) {
  if (process.platform !== "win32") throw new Error("the e2e driver is Windows-only");
  const root = o.root ? resolve(o.root) : mkdtempSync(join(tmpdir(), "velora-e2e-"));
  // Guards first: nothing is written before every path has been checked.
  const session = new Session(o, buildPlan(o, { area: { x: 0, y: 0, width: 1920, height: 1040 }, root }));
  session.prepare();
  const probed = probe({ env: session.env });
  const area = primaryWorkArea(probed);
  const plan = buildPlan(o, { area, root });
  session.plan = plan;
  const onSignal = (signal) => {
    session.log(`${signal}: cleaning up`);
    session.cleanup().finally(() => process.exit(130));
  };
  for (const signal of ["SIGINT", "SIGTERM", "SIGBREAK"]) process.on(signal, onSignal);
  process.on("exit", () => session.cleanupSync());
  let error = null;
  let hands = 0;
  try {
    session.log(`root ${plan.paths.root}`);
    const blocker = desktopBlocker(probed);
    if (blocker && !o.observe) throw new Error(`refusing to start: ${blocker}`);
    if (blocker) session.log(`observe only: ${blocker}`);
    session.seed();
    await session.backlog();
    await session.startTables();
    await session.launchApp();
    hands = await session.play();
  } catch (err) {
    error = err;
    session.log(`error: ${err.message}`);
  } finally {
    try {
      session.writeSummary({ hands, error: error ? error.message : null });
    } catch (err) {
      session.log(`could not write the summary: ${err.message}`);
    }
    await session.cleanup();
  }
  session.log(`${hands} hands; summary ${plan.paths.summary}; screenshots ${plan.shots}`);
  return error ? 1 : 0;
}

async function main() {
  let o;
  try {
    o = parseArgs(process.argv.slice(2));
  } catch (err) {
    console.error(`e2e: ${err.message}`);
    return 2;
  }
  try {
    return o.dryRun ? await dryRun(o) : await liveRun(o);
  } catch (err) {
    console.error(`e2e: ${err.message}`);
    return 1;
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) process.exitCode = await main();
