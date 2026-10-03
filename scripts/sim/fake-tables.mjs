#!/usr/bin/env node
// Node side of the fake PokerStars tables (fake-tables.ps1): starts the
// window program, sends it commands and reads its click-count status file.
// Dev-only, used by the e2e driver; never part of the app or its bundle.
//
// Usage:
//   node scripts/sim/fake-tables.mjs --probe
//       compiles the window program and prints the monitors; opens nothing
//   node scripts/sim/fake-tables.mjs --titles <hand-history folder> [--hero SimHero]
//       prints, as JSON, every table in the folder with the window title a
//       fake table for it gets (src-tauri/tests/sim_titles_tests.rs checks
//       them against table_track's own title parsing)

import { spawn, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { fileURLToPath, pathToFileURL } from "node:url";
import { loadProfiles } from "./lib/session.mjs";
import { scanTables, tableTitle } from "./lib/tables.mjs";

export const SCRIPT = fileURLToPath(new URL("./fake-tables.ps1", import.meta.url));
export const POWERSHELL = "powershell.exe";
const BASE_ARGS = ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", SCRIPT];

/** Compiles the window program without creating a window; returns the monitors. */
export function probe({ env = process.env, timeoutMs = 60_000 } = {}) {
  const run = spawnSync(POWERSHELL, [...BASE_ARGS, "-Probe"], { env, encoding: "utf8", timeout: timeoutMs, windowsHide: true });
  if (run.error) throw new Error(`could not run ${POWERSHELL}: ${run.error.message}`);
  if (run.status !== 0) throw new Error(`fake-tables probe failed (${run.status}): ${(run.stderr || run.stdout).trim()}`);
  const line = run.stdout.trim().split(/\r?\n/).pop();
  return JSON.parse(line);
}

/** The primary monitor's work area from a probe. */
export function primaryWorkArea(probed) {
  const monitor = probed.monitors.find((m) => m.primary) ?? probed.monitors[0];
  if (!monitor) throw new Error("no monitor reported");
  return monitor.workArea;
}

/** A running window program. Every method resolves with the program's answer. */
export class FakeTables {
  constructor(child, statusPath) {
    this.child = child;
    this.statusPath = statusPath;
    this.seq = 0;
    this.pending = new Map();
    this.exited = false;
    this.stderr = "";
  }

  /** Starts fake-tables.ps1 writing its status to `statusPath`. */
  static start({ statusPath, env = process.env, timeoutMs = 60_000 }) {
    const child = spawn(POWERSHELL, [...BASE_ARGS, "-Status", statusPath], {
      env,
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });
    const tables = new FakeTables(child, statusPath);
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      tables.stderr = (tables.stderr + chunk).slice(-4000);
    });
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        tables.kill();
        reject(new Error(`fake tables did not start in ${timeoutMs} ms`));
      }, timeoutMs);
      let started = false;
      createInterface({ input: child.stdout }).on("line", (line) => {
        let message;
        try {
          message = JSON.parse(line);
        } catch {
          return;
        }
        if (message.ready && !started) {
          started = true;
          clearTimeout(timer);
          resolve(tables);
          return;
        }
        const waiter = tables.pending.get(message.seq);
        if (!waiter) return;
        tables.pending.delete(message.seq);
        if (message.ok) waiter.resolve(message.result);
        else waiter.reject(new Error(`fake tables: ${message.error}`));
      });
      child.on("error", (err) => {
        clearTimeout(timer);
        reject(new Error(`could not run ${POWERSHELL}: ${err.message}`));
      });
      child.on("exit", (code) => {
        tables.exited = true;
        clearTimeout(timer);
        for (const waiter of tables.pending.values()) waiter.reject(new Error(`fake tables exited (${code}) ${tables.stderr.trim()}`));
        tables.pending.clear();
        if (!started) reject(new Error(`fake tables exited before starting (${code}): ${tables.stderr.trim()}`));
      });
    });
  }

  send(op, args = {}, timeoutMs = 30_000) {
    if (this.exited) return Promise.reject(new Error("fake tables are not running"));
    const seq = ++this.seq;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(seq);
        reject(new Error(`fake tables: ${op} timed out`));
      }, timeoutMs);
      this.pending.set(seq, {
        resolve: (value) => (clearTimeout(timer), resolve(value)),
        reject: (err) => (clearTimeout(timer), reject(err)),
      });
      this.child.stdin.write(JSON.stringify({ seq, op, ...args }) + "\n");
    });
  }

  open(id, title, rect, seats) {
    return this.send("open", { id, title, ...rect, seats });
  }
  move(id, rect) {
    return this.send("move", { id, ...rect });
  }
  retitle(id, title) {
    return this.send("retitle", { id, title });
  }
  close(id) {
    return this.send("close", { id });
  }
  closeAll() {
    return this.send("closeAll");
  }
  /** Captures a screen region (physical pixels) to a PNG. The caller checks `path`. */
  shot(path, rect) {
    return this.send("shot", { path, ...rect });
  }
  mouse(point) {
    return this.send("mouse", point);
  }
  click(point) {
    return this.send("click", point);
  }
  /** Visible and hidden top-level windows whose title contains `title`. */
  list(title) {
    return this.send("list", { title });
  }
  /** Presses a key combination: `{ ctrl, alt, shift, vk }` (vk: Windows virtual-key code). */
  keys(combo) {
    return this.send("keys", combo);
  }
  /** ShowWindow on any top-level window (6 minimizes, 9 restores). */
  show(hwnd, cmd) {
    return this.send("show", { hwnd, cmd });
  }
  status() {
    return this.send("status");
  }

  /** The status file as last written (click counts per table). */
  readStatus() {
    for (let attempt = 0; attempt < 5; attempt++) {
      try {
        return JSON.parse(readFileSync(this.statusPath, "utf8"));
      } catch {
        // Mid-swap; read again.
      }
    }
    return null;
  }

  /** Closes every window and ends the program; kills it if it does not answer. */
  async quit(timeoutMs = 5000) {
    if (this.exited) return;
    const exited = new Promise((done) => this.child.once("exit", done));
    try {
      await this.send("closeAll", {}, timeoutMs);
      await this.send("quit", {}, timeoutMs);
    } catch {
      // Fall through to the kill.
    }
    const timer = setTimeout(() => this.kill(), timeoutMs);
    await exited;
    clearTimeout(timer);
  }

  kill() {
    if (this.exited) return;
    try {
      spawnSync("taskkill", ["/PID", String(this.child.pid), "/T", "/F"], { windowsHide: true });
    } catch {
      this.child.kill();
    }
  }
}

/** Every table in a hand-history folder with its fake window's title. */
export function titlesFor(dir, hero) {
  return scanTables(dir, hero).map((table) => ({
    file: table.file,
    table: table.name,
    kind: table.kind,
    maxSeats: table.maxSeats,
    title: tableTitle(table, hero, 0),
  }));
}

function main(argv) {
  const at = (flag) => {
    const i = argv.indexOf(flag);
    return i >= 0 ? argv[i + 1] : undefined;
  };
  if (argv.includes("--probe")) {
    process.stdout.write(JSON.stringify(probe()) + "\n");
    return 0;
  }
  const dir = at("--titles");
  if (dir) {
    const hero = at("--hero") ?? loadProfiles().hero.name;
    process.stdout.write(JSON.stringify(titlesFor(dir, hero)) + "\n");
    return 0;
  }
  console.error("usage: fake-tables.mjs --probe | --titles <folder> [--hero name]");
  return 2;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) process.exitCode = main(process.argv.slice(2));
