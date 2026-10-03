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
//   - StrictMode leaves one listener per event;
//   - the villain search: a labelled box that matches screen names across
//     tables (any case), shows only the matching tables, opened, with the
//     matches marked; says so when nothing matches; Esc and clearing bring
//     back the rows that were open before; the query survives a new hand, a
//     table closing, an older refresh answering last and a failed refresh
//     followed by Retry; and, with real key presses over the DevTools
//     protocol, Tab reaches the box with a visible focus ring and Esc clears it.
// Screenshots panel-search-*.png at 480x1000 and 1440x900.
// Exits non-zero on any failure and always stops Vite and Edge.
//
// The data is a mocked snapshot, not a live PokerStars session.
//
// Usage: node scripts/qa-panel-shots.mjs

import { spawn, execFile } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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
function newProfile() {
  profileRoot ??= mkdtempSync(join(process.env.TEMP || tmpdir(), "velora-qa-edge-"));
  return join(profileRoot, String(profileSeq++));
}
function removeProfiles() {
  if (!profileRoot) return;
  try {
    rmSync(profileRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  } catch {
    // A lingering Edge process still holds a file; the OS temp cleanup gets it.
  }
  profileRoot = null;
}

const BASE_FLAGS = [
  "--headless=new",
  "--disable-gpu",
  "--no-first-run",
  "--no-default-browser-check",
  "--disable-extensions",
  "--hide-scrollbars",
  "--force-device-scale-factor=1",
];

function edge(args, { capture = false } = {}) {
  const base = [...BASE_FLAGS, `--user-data-dir=${newProfile()}`, "--virtual-time-budget=6000"];
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

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

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

async function searchChecks(base) {
  const label = "search";
  let r = fields((await dump(base, { action: "search" }))["action-result"]);
  check(r.label === "Find_a_player", `${label}: the box has a visible label`, JSON.stringify(r));
  check(r.rows === "1,2,5" && r.matches === "3" && r.marks === "3",
    `${label}: "ANNA" matches AggroAnna77 at every table she sits at, any case, marked`, JSON.stringify(r));
  check(r.expanded === "1,2,5", `${label}: matching tables open, the others hidden`, JSON.stringify(r));
  check(r.collapsed === "1,5", `${label}: a matching row can still be collapsed during the search`, JSON.stringify(r));
  check(r.line === "1_player_at_3_of_8_tables_·_Esc_clears", `${label}: the result line counts players and tables`, r.line);
  check(r.announced === '1_player_matches_"ANNA"_at_3_tables.', `${label}: the result is announced politely`, r.announced);
  check(r.clearedRows === String(TABLES) && r.restored === "4",
    `${label}: clearing the box restores every table and the rows open before`, JSON.stringify(r));
  check(r.clearedAnnounce === "Search_cleared._8_tables_open.", `${label}: clearing is announced`, r.clearedAnnounce);

  r = fields((await dump(base, { action: "searchnone" }))["action-result"]);
  check(r.state === "nomatch" && r.rows === "0" && r.empty === "1" && r.hasQuery === "1",
    `${label}: no match shows a clear empty-result message naming the query`, JSON.stringify(r));
  check(r.line === "No_match_·_Esc_clears" && r.announced === 'No_player_matches_"zzz_nobody".',
    `${label}: no match is said under the box and announced`, JSON.stringify(r));

  r = fields((await dump(base, { action: "searchclear" }))["action-result"]);
  check(r.during === "2,3" && r.query === '""' && r.rows === String(TABLES) && r.restored === "3" && r.focus === "search",
    `${label}: Esc clears the box, keeps focus there and restores the open rows`, JSON.stringify(r));

  r = fields((await dump(base, { action: "searchrefresh" }))["action-result"]);
  check(r.before === "2,3" && r.afterHand === "1,2,3/1,2,3", `${label}: a new hand refreshes the filtered list (new match shown, opened)`,
    JSON.stringify(r));
  check(r.afterClose === "1,3/1,3" && r.query === "tony", `${label}: a table closing keeps the query and drops its row`, JSON.stringify(r));
  check(r.restored === "4", `${label}: clearing after refreshes restores the rows open before the search`, JSON.stringify(r));

  r = fields((await dump(base, { action: "searchstale" }))["action-result"]);
  check(r.mid === "new" && r.end === "new" && r.query === "anna" && r.rows === "1,2,5",
    `${label}: an older refresh answering last never overwrites the newer filtered result`, JSON.stringify(r));

  r = fields((await dump(base, { action: "searchretry" }))["action-result"]);
  check(r.banner === "1" && r.kept === "anna/1,2,5", `${label}: a failed refresh keeps the query and the filtered list`, JSON.stringify(r));
  check(r.alertsAfter === "0" && r.query === "anna" && r.rows === "1,2,5" && r.fresh === "1",
    `${label}: Retry recovers with the query still applied`, JSON.stringify(r));

  for (const [size, tag] of [[NARROW, "480x1000"], [WIDE, "1440x900"]]) {
    await shot(base, size, { q: "anna" }, `panel-search-match-${tag}.png`);
    await shot(base, size, { q: "zzz_nobody" }, `panel-search-none-${tag}.png`);
  }
  await shot(base, NARROW, { action: "searchretry" }, "panel-search-retry-480x1000.png");
}

// --- Keyboard pass over the DevTools protocol -------------------------------

class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.nextId = 1;
    this.pending = new Map();
    ws.addEventListener("message", (event) => {
      const msg = JSON.parse(event.data);
      if (msg.id && this.pending.has(msg.id)) {
        const { res, rej } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) rej(new Error(msg.error.message));
        else res(msg.result);
      }
    });
  }
  static async connect(url) {
    const ws = new WebSocket(url);
    await new Promise((res, rej) => {
      ws.addEventListener("open", res, { once: true });
      ws.addEventListener("error", () => rej(new Error(`cannot open ${url}`)), { once: true });
    });
    return new Cdp(ws);
  }
  send(method, params = {}) {
    const id = this.nextId++;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((res, rej) => this.pending.set(id, { res, rej }));
  }
  async eval(expression) {
    const { result, exceptionDetails } = await this.send("Runtime.evaluate", {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    if (exceptionDetails) throw new Error(`evaluate failed: ${exceptionDetails.text}`);
    return result.value;
  }
  async key(key) {
    const keys = {
      Tab: { code: "Tab", windowsVirtualKeyCode: 9 },
      Escape: { code: "Escape", windowsVirtualKeyCode: 27 },
    }[key];
    await this.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key, ...keys });
    await this.send("Input.dispatchKeyEvent", { type: "keyUp", key, ...keys });
  }
  close() {
    try {
      this.ws.close();
    } catch {
      // already closed
    }
  }
}

const FOCUS_PROBE = `(() => {
  const el = document.activeElement;
  if (!el || el === document.body) return null;
  const s = getComputedStyle(el);
  return {
    id: el.id,
    tag: el.tagName,
    text: el.textContent.trim().slice(0, 40),
    focusVisible: el.matches(":focus-visible"),
    ring: s.outlineStyle !== "none" && parseFloat(s.outlineWidth) >= 1,
  };
})()`;

function killTree(child) {
  if (!child || child.exitCode !== null) return;
  try {
    execFile("taskkill", ["/pid", String(child.pid), "/t", "/f"], () => {});
  } catch {
    child.kill();
  }
}

async function keyboardPass(base) {
  const label = "search keyboard";
  const profile = newProfile();
  const browser = spawn(
    EDGE,
    [...BASE_FLAGS, `--user-data-dir=${profile}`, "--remote-debugging-port=0", `--window-size=${NARROW.w},${NARROW.h}`,
      pageUrl(base, { expand: "4" })],
    { stdio: "ignore", windowsHide: true },
  );
  let cdp = null;
  try {
    const portFile = join(profile, "DevToolsActivePort");
    const deadline = Date.now() + 20000;
    while (!existsSync(portFile) && Date.now() < deadline) await sleep(100);
    if (!existsSync(portFile)) throw new Error("Edge never wrote DevToolsActivePort");
    const port = readFileSync(portFile, "utf8").split(/\r?\n/)[0].trim();

    let page = null;
    while (!page && Date.now() < deadline + 10000) {
      const targets = await fetch(`http://127.0.0.1:${port}/json/list`).then((r) => r.json()).catch(() => []);
      page = targets.find((t) => t.type === "page" && t.url.includes("qa-panel.html"));
      if (!page) await sleep(100);
    }
    if (!page) throw new Error("no qa-panel.html page target");
    cdp = await Cdp.connect(page.webSocketDebuggerUrl);
    await cdp.send("Runtime.enable");

    const readyBy = Date.now() + 20000;
    const readyFlag = () => cdp.eval(`document.body?.getAttribute("data-qa-ready") ?? null`).catch(() => null);
    while ((await readyFlag()) !== "1") {
      if (Date.now() > readyBy) throw new Error("side panel never became ready");
      await sleep(100);
    }
    await cdp.eval(`document.activeElement?.blur(), window.focus(), true`);

    await cdp.key("Tab");
    const first = await cdp.eval(FOCUS_PROBE);
    check(first?.id === "villain-search", `${label}: the first Tab reaches the search box`, JSON.stringify(first));
    check(!!first?.focusVisible && !!first?.ring, `${label}: the box shows a :focus-visible ring`, JSON.stringify(first));

    await cdp.send("Input.insertText", { text: "anna" });
    await sleep(700);
    const typed = await cdp.eval(`({
      rows: [...document.querySelectorAll("li[data-table-id]")].map((li) => li.dataset.tableId).join(","),
      announce: document.querySelector("[role=status]")?.textContent ?? "",
    })`);
    check(typed.rows === "1,2,5" && typed.announce === '1 player matches "anna" at 3 tables.',
      `${label}: typing filters the tables and announces the result`, JSON.stringify(typed));
    const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
    const focusShot = "panel-search-keyboard-480x1000.png";
    writeFileSync(join(outDir, focusShot), Buffer.from(data, "base64"));
    check(existsSync(join(outDir, focusShot)), `screenshot ${focusShot}`);

    await cdp.key("Escape");
    await sleep(200);
    const cleared = await cdp.eval(`({
      value: document.getElementById("villain-search").value,
      rows: document.querySelectorAll("li[data-table-id]").length,
      expanded: [...document.querySelectorAll("li[data-table-id] button[aria-expanded=true]")].map((b) => b.closest("li").dataset.tableId).join(","),
      focus: document.activeElement?.id ?? "",
    })`);
    check(cleared.value === "" && cleared.rows === TABLES && cleared.expanded === "4" && cleared.focus === "villain-search",
      `${label}: Esc clears the query, restores the open rows and keeps focus in the box`, JSON.stringify(cleared));

    await cdp.key("Tab");
    const next = await cdp.eval(FOCUS_PROBE);
    check(next?.tag === "BUTTON" && next?.text.startsWith("Halley III") && !!next?.ring,
      `${label}: the next Tab moves on to the first table row, with a ring`, JSON.stringify(next));
  } finally {
    if (cdp) {
      await cdp.send("Browser.close").catch(() => {});
      cdp.close();
    }
    await sleep(300);
    killTree(browser);
  }
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
    await searchChecks(base);
    await keyboardPass(base);
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
