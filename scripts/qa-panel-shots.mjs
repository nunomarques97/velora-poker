// Screenshots and behaviour assertions for the side panel, from qa-panel.html.
//
// Starts Vite on a free port, then drives the installed Edge headless:
// --screenshot for docs/design/verification/panel-*.png (480x1000, a narrow
// second-monitor column, and 1440x900) and --dump-dom to read the numbers
// qa-panel.html writes to <body>:
//   - 8 tables render, one row each, with quality score and label (or the
//     D102 unavailable text in the default build);
//   - multi-table flags match the mock world and name the other tables;
//   - expand buttons are native buttons with aria-expanded/aria-controls,
//     focusable, and expand/collapse their villain list;
//   - a new hand (hands-imported) and a table change (tracked-tables-changed)
//     refresh the panel and keep expanded rows by tableId;
//   - an older refresh answering last is ignored;
//   - a failed first load shows an error with Retry, which recovers and moves
//     focus to the heading; a failed refresh keeps the list, flagged, and
//     Retry clears it; an empty panel fills in when tables open;
//   - StrictMode leaves one listener per event.
// Exits non-zero on any failure and always stops Vite.
//
// The data is a mocked snapshot, not a live PokerStars session.
//
// Usage: node scripts/qa-panel-shots.mjs

import { spawn, execFile } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const EDGE = "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "docs", "design", "verification");

const NARROW = { w: 480, h: 1000 };
const WIDE = { w: 1440, h: 900 };
const TABLES = 8;

const failures = [];
const check = (ok, label, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? `  (${detail})` : ""}`);
  if (!ok) failures.push(label);
};

function freePort() {
  return new Promise((res, rej) => {
    const srv = createServer();
    srv.unref();
    srv.on("error", rej);
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close(() => res(port));
    });
  });
}

async function waitForServer(url, vite, timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (vite.exitCode !== null) throw new Error(`Vite exited early with code ${vite.exitCode}`);
    try {
      const r = await fetch(url);
      if (r.ok) return;
    } catch {
      // not listening yet
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error(`Vite did not answer at ${url} within ${timeoutMs} ms`);
}

function stopVite(vite) {
  if (!vite || vite.exitCode !== null) return;
  // Vite runs under node without a shell; kill the whole tree to be sure.
  if (process.platform === "win32") {
    try {
      execFile("taskkill", ["/pid", String(vite.pid), "/t", "/f"], () => {});
    } catch {
      vite.kill();
    }
  } else {
    vite.kill();
  }
}

// Private Edge profiles, one per launch, under one temporary root that is
// removed at exit: never touches (or waits on) the user's running Edge.
let profileRoot = null;
let profileSeq = 0;
function removeProfiles() {
  if (!profileRoot) return;
  try {
    rmSync(profileRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  } catch {
    // A lingering Edge process still holds a file; the OS temp cleanup gets it.
  }
  profileRoot = null;
}

function edge(args, { capture = false } = {}) {
  profileRoot ??= mkdtempSync(join(process.env.TEMP || tmpdir(), "velora-qa-edge-"));
  const profile = join(profileRoot, String(profileSeq++));
  const base = [
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-extensions",
    "--hide-scrollbars",
    "--force-device-scale-factor=1",
    `--user-data-dir=${profile}`,
    "--virtual-time-budget=6000",
  ];
  return new Promise((res, rej) => {
    execFile(
      EDGE,
      [...base, ...args],
      { maxBuffer: 32 * 1024 * 1024, timeout: 60000, windowsHide: true },
      (err, stdout, stderr) => {
        if (err && !capture) return rej(new Error(`Edge failed: ${err.message}\n${stderr}`));
        if (err && !stdout) return rej(new Error(`Edge failed: ${err.message}\n${stderr}`));
        res(stdout);
      },
    );
  });
}

function qaAttributes(dom) {
  // The last <body ...> tag: the page's own comment mentions "<body>" too.
  const body = [...dom.matchAll(/<body\b[^>]*>/gi)].pop();
  if (!body) return {};
  const attrs = {};
  for (const m of body[0].matchAll(/data-qa-([a-z-]+)="([^"]*)"/g)) {
    attrs[m[1]] = m[2]
      .replace(/&quot;/g, '"')
      .replace(/&lt;/g, "<")
      .replace(/&gt;/g, ">")
      .replace(/&amp;/g, "&");
  }
  return attrs;
}

const pageUrl = (base, params) => `${base}/qa-panel.html?${new URLSearchParams(params)}`;

async function dump(base, params, size = NARROW) {
  const dom = await edge([`--window-size=${size.w},${size.h}`, "--dump-dom", pageUrl(base, params)], { capture: true });
  const qa = qaAttributes(dom);
  if (qa.ready !== "1") throw new Error(`qa-panel.html never settled for ${JSON.stringify(params)}`);
  return qa;
}

async function shot(base, size, params, name) {
  const file = join(outDir, name);
  await edge([`--window-size=${size.w},${size.h}`, `--screenshot=${file}`, pageUrl(base, params)]);
  check(existsSync(file), `screenshot ${name}`);
}

const short = (s, n = 160) => (s && s.length > n ? `${s.slice(0, n)}…` : s);

/** `a=1 b=x` → { a: "1", b: "x" } */
const fields = (s) => Object.fromEntries((s ?? "").split(" ").filter(Boolean).map((kv) => kv.split("=")));

async function strategicChecks(base) {
  const label = "strategic";
  const qa = await dump(base, {});
  check(qa.state === "ready" && qa.rows === String(TABLES), `${label}: ${TABLES} table rows`, `state=${qa.state} rows=${qa.rows}`);
  check(qa.expanded === "0", `${label}: rows start collapsed`, `expanded=${qa.expanded}`);
  check(/68/.test(qa.quality) && /Soft/.test(qa.quality), `${label}: quality score with its label`, qa.quality);
  check(Number(qa.flags) > 0 && qa.flags === qa["expected-flags"], `${label}: multi-table flags match the world`,
    `flags=${qa.flags} expected=${qa["expected-flags"]}`);
  check(/Also at Ross II, Medusa II/.test(qa["flag-text"]) && /Also at Halley III, Medusa II/.test(qa["flag-text"]),
    `${label}: flags name the other tables`, short(qa["flag-text"]));
  check(!/Unnamed table/.test(qa["flag-text"]), `${label}: an unnamed table is never named as another table`, short(qa["flag-text"]));
  check(Number(qa.tags) > 0 && Number(qa.reads) > 0 && qa.unavailable === "0", `${label}: tags and top reads, nothing unavailable`,
    `tags=${qa.tags} reads=${qa.reads} unavailable=${qa.unavailable}`);
  const listeners = JSON.parse(qa.listeners || "{}");
  check(listeners["hands-imported"] === 1 && listeners["tracked-tables-changed"] === 1,
    `${label}: one listener per event after StrictMode setup/cleanup/setup`, qa.listeners);
  check(/8 tables open/.test(qa.announce), `${label}: the live region announces the table count`, qa.announce);

  let r = fields((await dump(base, { action: "expand" }))["action-result"]);
  check(r.before === "false" && r.hiddenBefore === "1" && r.after === "true" && r.shown === "1"
    && r.collapsed === "false" && r.hiddenAgain === "1",
    `${label}: expand button toggles aria-expanded and its villain list`, JSON.stringify(r));
  check(r.native === "1" && r.focusable === "1" && r.focusKept === "1",
    `${label}: expand is a focusable native button (Enter/Space) and keeps focus`, JSON.stringify(r));

  r = fields((await dump(base, { action: "refresh" }))["action-result"]);
  check(r.refreshed === "1" && r.updated === "1" && r.villains === "5",
    `${label}: a new hand refreshes the panel (new read, new villain)`, JSON.stringify(r));
  check(r.expanded === "1,3", `${label}: expanded rows survive the refresh`, JSON.stringify(r));

  r = fields((await dump(base, { action: "tracked" }))["action-result"]);
  check(r.rows === String(TABLES) && r.row9 === "1" && r.row2 === "0" && r.expanded === "1",
    `${label}: a table change refreshes rows and keeps expanded rows by tableId`, JSON.stringify(r));

  r = fields((await dump(base, { action: "stale" }))["action-result"]);
  check(r.mid === "new" && r.end === "new" && r.expanded === "1", `${label}: an older refresh answering last is ignored`,
    JSON.stringify(r));

  const err = await dump(base, { start: "error" });
  check(err.state === "error" && err.rows === "0", `${label}: a failed first load shows the error state`, `state=${err.state}`);
  r = fields((await dump(base, { start: "error", action: "retry" }))["action-result"]);
  check(r.before === "error" && r.alert === "1" && r.hadRetry === "1" && r.after === "ready" && r.rows === String(TABLES),
    `${label}: error -> Retry recovers the list`, JSON.stringify(r));
  check(r.focus === "heading", `${label}: focus moves to the heading when Retry disappears`, JSON.stringify(r));

  r = fields((await dump(base, { action: "refreshfail" }))["action-result"]);
  check(r.banner === "1" && r.kept === String(TABLES) && r.keptExpanded === "1",
    `${label}: a failed refresh keeps the list, flagged, with Retry`, JSON.stringify(r));
  check(r.alertsAfter === "0" && r.rows === String(TABLES), `${label}: Retry after a failed refresh clears the error`,
    JSON.stringify(r));

  const empty = await dump(base, { start: "empty" });
  check(empty.state === "empty" && /No PokerStars tables open/.test(empty.announce), `${label}: empty state, announced`,
    `state=${empty.state} announce=${empty.announce}`);
  r = fields((await dump(base, { start: "empty", action: "emptyrefresh" }))["action-result"]);
  check(r.before === "empty" && r.nextStep === "1" && r.after === "ready" && r.rows === String(TABLES),
    `${label}: empty (with the next step) -> populated after tables open`, JSON.stringify(r));

  await shot(base, NARROW, { expand: "1,3" }, "panel-8tables-480x1000.png");
  await shot(base, WIDE, { expand: "1" }, "panel-8tables-1440x900.png");
  await shot(base, NARROW, {}, "panel-8tables-collapsed-480x1000.png");
  await shot(base, NARROW, { action: "refreshbanner" }, "panel-refresh-error-480x1000.png");
  await shot(base, NARROW, { start: "empty" }, "panel-empty-480x1000.png");
  await shot(base, NARROW, { start: "error" }, "panel-error-480x1000.png");
}

async function defaultBuildChecks(base) {
  const label = "default build";
  const qa = await dump(base, { build: "default" });
  check(qa.state === "ready" && qa.rows === String(TABLES), `${label}: ${TABLES} table rows`, `rows=${qa.rows}`);
  check(qa.quality === "Quality unavailable in this build", `${label}: quality shows the D102 unavailable text`, qa.quality);
  check(qa.tags === "0" && qa.reads === "0", `${label}: no tag, no read`, `tags=${qa.tags} reads=${qa.reads}`);
  // One quality text per row plus one "Tags and reads" note per villain list.
  check(qa.unavailable === String(TABLES * 2), `${label}: every row says what this build lacks`, `unavailable=${qa.unavailable}`);
  check(Number(qa.flags) > 0 && qa.flags === qa["expected-flags"], `${label}: multi-table flags still shown`,
    `flags=${qa.flags} expected=${qa["expected-flags"]}`);
  await shot(base, NARROW, { build: "default", expand: "1" }, "panel-default-8tables-480x1000.png");
  await shot(base, WIDE, { build: "default", expand: "1" }, "panel-default-8tables-1440x900.png");
}

async function main() {
  if (!existsSync(EDGE)) throw new Error(`Edge not found at ${EDGE}`);
  mkdirSync(outDir, { recursive: true });

  const port = await freePort();
  const base = `http://127.0.0.1:${port}`;
  const vite = spawn(
    process.execPath,
    [join(root, "node_modules", "vite", "bin", "vite.js"), "--port", String(port), "--strictPort", "--host", "127.0.0.1"],
    { cwd: root, stdio: ["ignore", "pipe", "pipe"], windowsHide: true },
  );
  let viteLog = "";
  vite.stdout.on("data", (d) => (viteLog += d));
  vite.stderr.on("data", (d) => (viteLog += d));
  const cleanup = () => stopVite(vite);
  process.on("exit", cleanup);
  process.on("SIGINT", () => process.exit(130));

  try {
    await waitForServer(`${base}/qa-panel.html`, vite);
    // First hit compiles the module graph; warm it so virtual time is spent on the page.
    await dump(base, {}).catch(() => {});

    await strategicChecks(base);
    await defaultBuildChecks(base);
  } catch (err) {
    failures.push(err.message);
    console.error(err.message);
    if (viteLog) console.error(`--- Vite output ---\n${viteLog}`);
  } finally {
    cleanup();
    removeProfiles();
  }

  if (failures.length) {
    console.error(`\n${failures.length} failure(s).`);
    process.exit(1);
  }
  console.log("\nAll side panel QA checks passed.");
}

main();
