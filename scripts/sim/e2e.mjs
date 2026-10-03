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
    },
    slots,
    seed: { command: "cargo", args: ["run", "--example", "sim_seed", ...featureArgs, "--", paths.db, paths.handHistoryRoot] },
    app: { command: "npm", args: ["run", "tauri", "dev", ...(featureArgs.length ? ["--", ...featureArgs] : [])] },
    generators: generatorRuns(o, paths),
    steps: [
      `guard every path, create ${paths.root} with ${ROOT_MARKER}`,
      `seed ${paths.db} with the app's migrations (cargo run --example sim_seed, under vcvars)`,
      o.backlog > 0 ? `write a backlog of ${o.backlog} hands per cash table (simulated clock)` : null,
      `start the fake tables (${TABLE_CLASS}); open the ${o.cash} cash table(s) before their first hand`,
      `launch the app (npm run tauri dev${featureArgs.length ? ` -- ${featureArgs.join(" ")}` : ""}) with the overridden environment; wait for "${MAIN_WINDOW_TITLE}"${o.keepMain ? "" : ", then minimize it"}`,
      `start the generators (live clock, ${o.pace}s per hand); open an MTT window when its tournament file appears, close it when the hero's tournament ends`,
      `every ${o.shotEvery} hands: desktop and per-table screenshots, hover a villain chip, click the felt outside HUD elements and read the click counter, list the overlay windows; once: toggle the side panel (Ctrl+Alt+P)`,
      `write ${paths.summary}; close every fake window, the generators and the app`,
    ].filter(Boolean),
  };
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
  rmSync(scratch, { recursive: true, force: true });

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

  async round(hands, { panel = false } = {}) {
    const n = this.rounds.length + 1;
    const tag = String(n).padStart(2, "0");
    const record = { round: n, hands, at: new Date().toISOString(), shots: [], hover: null, click: null, overlays: null, panel: null };
    const area = this.plan.area;
    record.shots.push(await this.shot(`r${tag}-desktop.png`, area));
    for (const [id, e] of this.open) {
      record.shots.push(await this.shot(`r${tag}-${id}-${e.slot.kind}-${e.slot.seats}max-${e.slot.size}.png`, e.slot.rect));
    }
    const entries = [...this.open];
    if (entries.length) {
      const [id, e] = entries[(n - 1) % entries.length];
      const points = chipHoverPoints(e.slot.seats);
      const target = pointIn(e.slot.rect, points[(n - 1) % points.length]);
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
    record.overlays = (await this.tables.list("Velora")).map(({ hwnd, title, visible, rect }) => ({ hwnd, title, visible, rect }));
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
  const area = primaryWorkArea(probe({ env: session.env }));
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
