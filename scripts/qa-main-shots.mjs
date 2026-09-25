// Screenshots and checks for the main window's HUD Profiles view (and the
// onboarding HUD step), from qa-harness.html with a mocked Tauri boundary.
//
// Starts Vite on a free port, then drives the installed Edge headless, the
// same way as scripts/qa-overlay-shots.mjs:
//   - --screenshot for docs/design/verification/hud-profiles-*.png at
//     1440x900 and 800x600 (plus the reset, error and empty states, and the
//     onboarding HUD step);
//   - --dump-dom to assert that "Reset seat layout" exists, that no
//     "Reposition" text is left anywhere on the page, that Reset calls
//     reset_seat_positions(null) once and confirms it on screen, that a failed
//     reset says so, that the preview respects the 25-hand gate, and that the
//     onboarding HUD step starts on Compact with Continue enabled;
//   - a real keyboard pass over the DevTools protocol: Tab reaches both
//     profile choices and Reset, Enter picks a profile, Space resets, and each
//     focused control draws a visible :focus-visible outline.
// Exits non-zero on any failure and always stops Vite and Edge.
//
// Usage: node scripts/qa-main-shots.mjs

import { spawn, execFile } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const EDGE = "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outDir = join(root, "docs", "design", "verification");

const SIZES = [
  { w: 1440, h: 900 },
  { w: 800, h: 600 },
];
// Players in qa-harness.html's roster with at least 25 hands.
const GATED_PREVIEW_CHIPS = 9;

const failures = [];
const check = (ok, label, detail = "") => {
  console.log(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? `  (${detail})` : ""}`);
  if (!ok) failures.push(label);
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

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
    await sleep(250);
  }
  throw new Error(`Vite did not answer at ${url} within ${timeoutMs} ms`);
}

function killTree(child) {
  if (!child || child.exitCode !== null) return;
  if (process.platform === "win32") {
    try {
      execFile("taskkill", ["/pid", String(child.pid), "/t", "/f"], () => {});
    } catch {
      child.kill();
    }
  } else {
    child.kill();
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
  // The last <body ...> tag: the harness's own comments mention "<body>" too.
  const body = [...dom.matchAll(/<body\b[^>]*>/gi)].pop();
  if (!body) return {};
  const attrs = {};
  for (const m of body[0].matchAll(/data-qa-([a-z-]+)="([^"]*)"/g)) {
    attrs[m[1]] = m[2].replace(/&quot;/g, '"').replace(/&amp;/g, "&").replace(/&#39;/g, "'");
  }
  return attrs;
}

/** The rendered page without <script>/<style> bodies: what a user can read. */
function visibleMarkup(dom) {
  return dom.replace(/<script\b[\s\S]*?<\/script>/gi, "").replace(/<style\b[\s\S]*?<\/style>/gi, "");
}

const pageUrl = (base, params) => `${base}/qa-harness.html?${new URLSearchParams(params)}`;

async function dump(base, size, params) {
  const dom = await edge([`--window-size=${size.w},${size.h}`, "--dump-dom", pageUrl(base, params)], { capture: true });
  const qa = qaAttributes(dom);
  if (qa.ready !== "1") {
    throw new Error(`qa-harness.html never settled for ${JSON.stringify(params)} at ${size.w}x${size.h}: ${qa.error ?? "no data-qa-ready"}`);
  }
  return { qa, dom };
}

async function shot(base, size, params, name) {
  const file = join(outDir, name);
  await edge([`--window-size=${size.w},${size.h}`, `--screenshot=${file}`, pageUrl(base, params)]);
  check(existsSync(file), `screenshot ${name}`);
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
      Enter: { code: "Enter", windowsVirtualKeyCode: 13, text: "\r" },
      " ": { code: "Space", windowsVirtualKeyCode: 32, text: " " },
    }[key];
    await this.send("Input.dispatchKeyEvent", { type: keys.text ? "keyDown" : "rawKeyDown", key, ...keys });
    await this.send("Input.dispatchKeyEvent", { type: "keyUp", key, code: keys.code, windowsVirtualKeyCode: keys.windowsVirtualKeyCode });
  }
  close() {
    try {
      this.ws.close();
    } catch {
      // already closed
    }
  }
}

// What the focused element is and whether it draws a focus ring (on itself or,
// for the stretched profile buttons, on its ::after).
const FOCUS_PROBE = `(() => {
  const el = document.activeElement;
  if (!el || el === document.body) return null;
  const own = getComputedStyle(el);
  const after = getComputedStyle(el, "::after");
  const ring = (s) => s.outlineStyle !== "none" && parseFloat(s.outlineWidth) >= 1;
  return {
    tag: el.tagName,
    text: el.textContent.trim().slice(0, 60),
    pressed: el.getAttribute("aria-pressed"),
    focusVisible: el.matches(":focus-visible"),
    ring: ring(own) || ring(after),
  };
})()`;

async function keyboardPass(base) {
  const profile = newProfile();
  const size = SIZES[0];
  const browser = spawn(
    EDGE,
    [...BASE_FLAGS, `--user-data-dir=${profile}`, "--remote-debugging-port=0", `--window-size=${size.w},${size.h}`,
      pageUrl(base, { mode: "hud" })],
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
      page = targets.find((t) => t.type === "page" && t.url.includes("qa-harness.html"));
      if (!page) await sleep(100);
    }
    if (!page) throw new Error("no qa-harness.html page target");
    cdp = await Cdp.connect(page.webSocketDebuggerUrl);
    await cdp.send("Runtime.enable");

    const readyBy = Date.now() + 20000;
    // The page may still be navigating (no body, or a context torn down mid-call); keep polling.
    const readyFlag = () => cdp.eval(`document.body?.getAttribute("data-qa-ready") ?? null`).catch(() => null);
    while ((await readyFlag()) !== "1") {
      if (Date.now() > readyBy) throw new Error("HUD Profiles view never became ready");
      await sleep(100);
    }
    // Start from the document, as after a fresh load.
    await cdp.eval(`document.activeElement?.blur(), window.focus(), true`);

    const reached = [];
    const tabTo = async (label, maxTabs = 40) => {
      for (let i = 0; i < maxTabs; i++) {
        await cdp.key("Tab");
        const focus = await cdp.eval(FOCUS_PROBE);
        if (focus) reached.push(focus.text);
        if (focus?.text.startsWith(label)) return focus;
      }
      return null;
    };

    const compact = await tabTo("Compact");
    check(!!compact, "keyboard: Tab reaches the Compact profile", reached.join(" | "));
    check(!!compact?.focusVisible && !!compact?.ring, "keyboard: Compact shows a :focus-visible ring", JSON.stringify(compact));

    const badge = await tabTo("Badge", 2);
    check(!!badge, "keyboard: the next Tab reaches the Badge profile", JSON.stringify(badge));
    check(!!badge?.ring, "keyboard: Badge shows a :focus-visible ring", JSON.stringify(badge));
    await cdp.key("Enter");
    await sleep(300);
    const afterEnter = await cdp.eval(FOCUS_PROBE);
    check(afterEnter?.text.startsWith("Badge") && afterEnter?.pressed === "true",
      "keyboard: Enter on Badge makes it the active profile, focus stays on it", JSON.stringify(afterEnter));
    const setCalls = await cdp.eval(`JSON.stringify(window.__QA__.calls.filter(c => c.cmd === "set_active_hud_profile").map(c => c.args))`);
    check(setCalls === '[{"id":"badge"}]', "keyboard: Enter sent set_active_hud_profile(badge) once", setCalls);

    const reset = await tabTo("Reset seat layout");
    check(!!reset?.ring && !!reset?.focusVisible, "keyboard: Tab reaches Reset seat layout, with a :focus-visible ring",
      JSON.stringify(reset));
    const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
    const focusShot = `hud-profiles-${size.w}x${size.h}-keyboard-focus.png`;
    writeFileSync(join(outDir, focusShot), Buffer.from(data, "base64"));
    check(existsSync(join(outDir, focusShot)), `screenshot ${focusShot}`);

    await cdp.key(" ");
    await sleep(600);
    const notice = await cdp.eval(`document.querySelector('[role="status"]')?.textContent ?? ""`);
    const resetCalls = await cdp.eval(`JSON.stringify(window.__QA__.calls.filter(c => c.cmd === "reset_seat_positions").map(c => c.args))`);
    check(resetCalls === '[{"maxPlayers":null}]', "keyboard: Space on Reset sends reset_seat_positions(null) once", resetCalls);
    check(notice.startsWith("Seat layout reset"), "keyboard: the reset confirmation is announced in the status region", notice);
    const afterSpace = await cdp.eval(FOCUS_PROBE);
    check(afterSpace?.text.startsWith("Reset seat layout"), "keyboard: focus stays on Reset after it finishes", JSON.stringify(afterSpace));
  } finally {
    if (cdp) {
      await cdp.send("Browser.close").catch(() => {});
      cdp.close();
    }
    await sleep(300);
    killTree(browser);
  }
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
  const cleanup = () => killTree(vite);
  process.on("exit", cleanup);
  process.on("SIGINT", () => process.exit(130));

  try {
    await waitForServer(`${base}/qa-harness.html`, vite);
    // First hit compiles the module graph; warm it so virtual time is spent on the page.
    await dump(base, SIZES[0], { mode: "hud" }).catch(() => {});

    for (const size of SIZES) {
      const tag = `${size.w}x${size.h}`;
      const { qa, dom } = await dump(base, size, { mode: "hud" });
      const visible = visibleMarkup(dom);
      check(qa["reset-button"] === "1" && />\s*Reset seat layout\s*</.test(visible), `${tag}: Reset seat layout button exists`);
      check(!/reposition/i.test(visible), `${tag}: no "Reposition" text anywhere on the page`);
      check(qa["profile-buttons"] === "2", `${tag}: exactly two profile choices`, `profiles=${qa["profile-buttons"]}`);
      check(qa.pressed === "Compact", `${tag}: Compact is the active profile`, `pressed=${qa.pressed}`);
      check(/>\s*Recommended\s*</.test(visible), `${tag}: a profile is marked Recommended`);
      check(qa["preview-chips"] === String(GATED_PREVIEW_CHIPS), `${tag}: preview shows only players at or over 25 hands`,
        `chips=${qa["preview-chips"]}`);
      check(qa["reset-calls"] === "0" && qa.notice === "", `${tag}: nothing reset on load`, `calls=${qa["reset-calls"]}`);
      check(/every table of the same size/.test(visible) && /directly on the table/.test(visible),
        `${tag}: hint says chips are dragged on the table and apply to every table of the same size`);
      await shot(base, size, { mode: "hud" }, `hud-profiles-${tag}.png`);
    }

    const size = SIZES[1];
    const tag = `${size.w}x${size.h}`;

    const reset = await dump(base, size, { mode: "hud", action: "reset" });
    check(reset.qa["pending-label"] === "Resetting" && reset.qa["pending-aria"] === "true",
      `${tag} reset: shows a pending state`, `${reset.qa["pending-label"]} aria-disabled=${reset.qa["pending-aria"]}`);
    check(reset.qa["reset-calls"] === "1" && reset.qa["reset-arg"] === '{"maxPlayers":null}',
      `${tag} reset: calls reset_seat_positions(null) once`, `${reset.qa["reset-calls"]} ${reset.qa["reset-arg"]}`);
    check(reset.qa.notice.startsWith("Seat layout reset") && reset.qa["notice-error"] === "0",
      `${tag} reset: visible confirmation`, reset.qa.notice);
    await shot(base, size, { mode: "hud", action: "reset" }, `hud-profiles-${tag}-reset.png`);

    const failed = await dump(base, size, { mode: "hud", action: "reset", reset: "fail" });
    check(failed.qa.notice.includes("Couldn't reset the seat layout") && failed.qa["notice-error"] === "1",
      `${tag} reset failure: the error is on screen`, failed.qa.notice);
    check(failed.qa["reset-button"] === "1", `${tag} reset failure: Reset is usable again`);
    await shot(base, size, { mode: "hud", action: "reset", reset: "fail" }, `hud-profiles-${tag}-reset-error.png`);

    const badge = await dump(base, size, { mode: "hud", action: "badge" });
    check(badge.qa.pressed === "Badge", `${tag} click Badge: it becomes the active profile`, badge.qa.pressed);

    const empty = await dump(base, size, { mode: "hud", players: "0" });
    check(/No hands imported yet/.test(visibleMarkup(empty.dom)) && empty.qa["reset-button"] === "1",
      `${tag} no hands: empty state, and Reset still available`);
    await shot(base, size, { mode: "hud", players: "0" }, `hud-profiles-${tag}-empty.png`);

    const onboarding = await dump(base, size, { mode: "green", step: "3" });
    check(onboarding.qa.pressed.startsWith("Compact") && onboarding.qa["continue-enabled"] === "1",
      `${tag} onboarding HUD step: Compact preselected, Continue enabled (one click)`,
      `pressed=${onboarding.qa.pressed} continue=${onboarding.qa["continue-enabled"]}`);
    await shot(base, size, { mode: "green", step: "3" }, `hud-profiles-${tag}-onboarding-step3.png`);

    await keyboardPass(base);
  } catch (err) {
    failures.push(err.message);
    console.error(err.message);
    if (viteLog) console.error(`--- Vite output ---\n${viteLog}`);
  } finally {
    cleanup();
    await sleep(300);
    removeProfiles();
  }

  if (failures.length) {
    console.error(`\n${failures.length} failure(s).`);
    process.exit(1);
  }
  console.log("\nAll main-window QA checks passed.");
}

main();
