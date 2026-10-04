// Screenshots and layout assertions for the overlay HUD, from qa-overlay.html.
//
// Starts Vite on a free port, then drives the installed Edge headless over
// the DevTools protocol, with the page itself set to each size (device
// metrics), for the images in docs/design/verification/hud-*.png and the
// numbers qa-overlay.html writes to <body>:
//   - the default layouts: no chip overlaps another chip (overlap=0) and no
//     chip covers a protected zone, i.e. board, bets, the hero's hole cards or
//     action buttons (intersections=0), nor an opponent's hole cards, nor
//     another seat's name plate;
//   - a scripted drag: exactly one save_seat_position call, for the dragged
//     seat key; a cancelled drag saves nothing; a plain click opens the
//     detail panel; a save made by another overlay of the same size moves
//     this one's chip;
//   - the strategic-analysis overlay (engine=1), on the real table of
//     src-tauri/tests/fixtures/qa_engine_reads.json (engine output, kept in
//     step by qa_fixture_tests.rs): tagged chips still overlap
//     nothing and no other seat's name plate, tag text >= 10px and in the
//     accessible name; the sample behind each read tag is on the chip,
//     >= 10px, and in words in the accessible name; every hover card stays
//     clear of protected zones, every seat's hole cards (hero and
//     opponents), its chip and the window edge, takes no pointer input and
//     closes on leave; at 800x570 every chip with reads gets one; it also closes on the native
//     pointer-left event, Esc, blur, drag start, the drawer opening and a
//     refresh that drops the player; a new hand refreshes it; an older
//     refresh answering last is ignored; StrictMode leaves one listener per
//     event; at 800x570 no observation or advice line of any fixture
//     villain's card is cut short by the two-line clamp (at 483x359 the
//     clamped count is reported); the card shows the fixture's top read.
//     Screenshots: hud-tagged-*, hud-hover-*, hud-engine-drawer-*.
// Independent page loads run QA_JOBS (default 4) Edge processes at a time;
// the log still prints in plan order.
// Exits non-zero on any failure and always stops Vite.
//
// The table behind the HUD is drawn from seatLayout.ts's ASSUMED geometry,
// not captured from a live PokerStars client.
//
// Usage: node scripts/qa-overlay-shots.mjs

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
  { w: 483, h: 359 }, // PokerStars' minimum table window (ASSUMED)
  { w: 800, h: 570 }, // default table window (ASSUMED)
];
const TABLES = [6, 9];
const DRAG = { seat: 2, dx: 40, dy: 14 };

// Independent Edge launches run this many at a time (QA_JOBS overrides).
// One after another, the ~90 launches took minutes on a loaded machine.
const JOBS = Math.max(1, Number(process.env.QA_JOBS) || 4);

const failures = [];

/**
 * A group is a run of Edge launches that belong together (one page's checks,
 * its screenshot). Groups run JOBS at a time; each one writes to its own
 * buffer and the buffers print in plan order, so the log reads as if serial.
 */
const plan = [];
const group = (run) => plan.push(run);

function reporter() {
  const lines = [];
  return {
    lines,
    check(ok, label, detail = "") {
      lines.push(`${ok ? "PASS" : "FAIL"}  ${label}${detail ? `  (${detail})` : ""}`);
      if (!ok) failures.push(label);
    },
    info(text) {
      lines.push(`INFO  ${text}`);
    },
  };
}

async function runPlan() {
  const done = new Array(plan.length);
  let printed = 0;
  const flush = () => {
    while (printed < plan.length && done[printed]) {
      for (const line of done[printed].lines) (line.startsWith("ERROR") ? console.error : console.log)(line);
      printed++;
    }
  };
  let next = 0;
  const worker = async () => {
    while (next < plan.length) {
      const i = next++;
      const t = reporter();
      try {
        await plan[i](t);
      } catch (err) {
        failures.push(err.message);
        t.lines.push(`ERROR ${err.message}`);
      }
      done[i] = t;
      flush();
    }
  };
  await Promise.all(Array.from({ length: Math.min(JOBS, plan.length) }, worker));
}

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

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

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
    // A call Edge never answers (a page that stopped rendering) fails the
    // run instead of hanging it.
    return new Promise((res, rej) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        rej(new Error(`DevTools ${method} timed out`));
      }, 30000);
      this.pending.set(id, {
        res: (v) => (clearTimeout(timer), res(v)),
        rej: (e) => (clearTimeout(timer), rej(e)),
      });
    });
  }
  async eval(expression) {
    const { result, exceptionDetails } = await this.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
    if (exceptionDetails) throw new Error(`evaluate failed: ${exceptionDetails.text}`);
    return result.value;
  }
  close() {
    try {
      this.ws.close();
    } catch {
      // already closed
    }
  }
}

function killTree(child) {
  if (!child || child.exitCode !== null) return;
  try {
    execFile("taskkill", ["/pid", String(child.pid), "/t", "/f"], () => {});
  } catch {
    child.kill();
  }
}

// The page's data-qa-* attributes by their attribute names: qaCardText -> card-text.
const READ_QA = `Object.fromEntries(Object.entries(document.body.dataset)
  .filter(([k]) => k.startsWith("qa"))
  .map(([k, v]) => [k.slice(2).replace(/[A-Z]/g, (c, i) => (i ? "-" : "") + c.toLowerCase()), v]))`;

/**
 * Loads `url` in a private headless Edge whose page is exactly `size`, set
 * through DevTools device metrics, waits for the harness to settle and
 * returns its data-qa-* attributes; with `file`, also saves a screenshot.
 * --window-size sizes the window, not the page: this Edge gave an 800x570
 * window a 776x478 page and never a page under 492px wide, and --screenshot
 * resized the page to the window only at capture, after the scripted action
 * had run at the other size.
 */
async function runPage(url, size, file = null) {
  const profile = newProfile();
  const browser = spawn(
    EDGE,
    [...BASE_FLAGS, `--user-data-dir=${profile}`, "--remote-debugging-port=0", `--window-size=${size.w + 100},${size.h + 200}`, "about:blank"],
    { stdio: "ignore", windowsHide: true },
  );
  let cdp = null;
  try {
    const portFile = join(profile, "DevToolsActivePort");
    const deadline = Date.now() + 30000;
    // Edge may still hold the file open while writing it.
    let port = "";
    while (!port && Date.now() < deadline) {
      try {
        port = readFileSync(portFile, "utf8").split(/\r?\n/)[0].trim();
      } catch {
        // not written yet
      }
      if (!port) await sleep(100);
    }
    if (!port) throw new Error("Edge never wrote DevToolsActivePort");
    let page = null;
    while (!page && Date.now() < deadline) {
      const targets = await fetch(`http://127.0.0.1:${port}/json/list`).then((r) => r.json()).catch(() => []);
      page = targets.find((t) => t.type === "page");
      if (!page) await sleep(100);
    }
    if (!page) throw new Error("no page target in Edge");
    cdp = await Cdp.connect(page.webSocketDebuggerUrl);
    await cdp.send("Emulation.setDeviceMetricsOverride", { width: size.w, height: size.h, deviceScaleFactor: 1, mobile: false });
    // A page driven over DevTools is not the focused window; focus and blur
    // events need it to be.
    await cdp.send("Emulation.setFocusEmulationEnabled", { enabled: true });
    await cdp.send("Page.enable");
    // Edge sometimes opens a second tab; a background tab never runs
    // requestAnimationFrame.
    await cdp.send("Page.bringToFront");
    await cdp.send("Page.navigate", { url });
    // Longer than the page's own 25s wait for its chips.
    const readyBy = Date.now() + 45000;
    const ready = () => cdp.eval(`document.body?.dataset.qaReady ?? null`).catch(() => null);
    while ((await ready()) !== "1") {
      if (Date.now() > readyBy) throw new Error(`qa-overlay.html never settled: ${url} at ${size.w}x${size.h}`);
      await sleep(100);
    }
    const qa = await cdp.eval(READ_QA);
    if (file) {
      // Two frames after the harness settles, so the capture shows its final state.
      await cdp.eval(`new Promise((r) => { requestAnimationFrame(() => requestAnimationFrame(() => r(true))); setTimeout(() => r(true), 500); })`);
      const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
      writeFileSync(file, Buffer.from(data, "base64"));
    }
    return qa;
  } finally {
    if (cdp) {
      // Edge may close the socket before it answers.
      await Promise.race([cdp.send("Browser.close").catch(() => {}), sleep(1000)]);
      cdp.close();
    }
    killTree(browser);
  }
}

const pageUrl = (base, params) => `${base}/qa-overlay.html?${new URLSearchParams({ table: "1", ...params })}`;

async function dump(base, size, params) {
  const qa = await runPage(pageUrl(base, params), size);
  if (qa.viewport !== `${size.w}x${size.h}`) throw new Error(`page is ${qa.viewport}, not ${size.w}x${size.h}`);
  return qa;
}

async function shot(t, base, size, params, name) {
  const file = join(outDir, name);
  const qa = await runPage(pageUrl(base, params), size, file);
  t.check(existsSync(file) && qa.viewport === `${size.w}x${size.h}`, `screenshot ${name}`, `page ${qa.viewport}`);
}

/** `a=1 b=x` → { a: "1", b: "x" } */
const fields = (s) => Object.fromEntries((s ?? "").split(" ").filter(Boolean).map((kv) => kv.split("=")));

// qa-overlay.html's engine table at each size, from the same fixture: the
// opponents with a tag, with a sample on the chip (PlayerHudCard's
// tagSample: the sample of the read the tag stands for; a stack or bounty
// fact has none) and with a card.
const FIXTURE = JSON.parse(readFileSync(join(root, "src-tauri", "tests", "fixtures", "qa_engine_reads.json"), "utf8"));
const ENGINE = Object.fromEntries(
  TABLES.map((max) => {
    const table = FIXTURE.tables.find((t) => t.maxPlayers === max);
    const villains = table.players.filter((p) => p.seatOffset !== 0 && p.engine);
    const sample = (p) => (p.engine.tag ? (p.engine.reads.find((r) => r.ruleId === p.engine.tag.ruleId)?.sample ?? null) : null);
    return [max, {
      tags: villains.filter((p) => p.engine.tag).length,
      samples: villains.filter(sample).length,
      cards: villains.length,
      at: (key) => villains.find((p) => p.seatOffset === key),
    }];
  }),
);

/** The card's confidence text, as HoverReadCard's confidenceText prints it. */
const TIER = { high: "High", medium: "Medium", low: "Low", insufficientData: "Low data" };
const confidence = (read) => `${TIER[read.confidenceTier]}${read.confidencePct === null ? "" : ` ${Math.round(read.confidencePct)}%`}`;
/** qa-overlay.html writes the card text with " as '. */
const asCardText = (text) => text.replace(/"/g, "'");

/**
 * The seat whose card the hover checks open: the dragged seat's, or, when
 * that card fits nowhere at this size (9-max at the minimum size, see
 * hoverall), the first chip whose card does. One lookup per table and size.
 */
const hoverSeats = new Map();
function hoverSeat(base, max, size) {
  const key = `${max}/${size.w}`;
  if (!hoverSeats.has(key)) {
    hoverSeats.set(key, (async () => {
      if (size.w !== SIZES[0].w) return DRAG.seat;
      const all = fields((await dump(base, size, { max: String(max), engine: "1", action: "hoverall" }))["action-result"]);
      const keys = (all.keys ?? "").split(",").map(Number).filter(Boolean);
      return keys.includes(DRAG.seat) || !keys.length ? DRAG.seat : keys[0];
    })());
  }
  return hoverSeats.get(key);
}

/** The `strategic-analysis` overlay: tagged chips, the hover card and its lifecycle. */
function planEngineChecks(base) {
  for (const max of TABLES) {
    const expected = ENGINE[max];
    for (const size of SIZES) {
      const tag = `${max}max-${size.w}x${size.h}`;
      for (const frame of ["hero", "absolute"]) {
        group(async ({ check, info }) => {
          const label = `${tag} ${frame} engine`;
          const qa = await dump(base, size, { max: String(max), frame, engine: "1" });
          check(qa.chips === String(max), `${label}: ${max} chips`, `chips=${qa.chips}`);
          check(qa.tags === String(expected.tags), `${label}: ${expected.tags} tagged chips`, `tags=${qa.tags}`);
          check(qa.overlap === "0" && qa.intersections === "0", `${label}: tag-width chips overlap no chip and no protected zone`,
            `overlap=${qa.overlap} intersections=${qa.intersections}`);
          check(qa["plate-intersections"] === "0", `${label}: no tagged chip over another seat's name plate`,
            `plates=${qa["plate-intersections"]}`);
          // Opponents' card backs are a soft zone for chips (seatLayout.ts). The
          // wider tagged chip still avoids them at the medium size; at the
          // minimum size it may touch some, which is reported, not failed.
          if (size.w === SIZES[0].w) {
            info(`${label}: tagged chips over an opponent's card backs: ${qa["opp-hole-intersections"]}`);
          } else {
            check(qa["opp-hole-intersections"] === "0", `${label}: no tagged chip over an opponent's card backs`,
              `opp-hole=${qa["opp-hole-intersections"]}`);
          }
          check(Number(qa["min-tag-font-px"]) >= 10, `${label}: tag text >= 10px`, `tag font=${qa["min-tag-font-px"]}`);
          check(qa["tag-labels"] === qa.tags, `${label}: every tagged chip's accessible name carries its tag`,
            `${qa["tag-labels"]}/${qa.tags}`);
          check(qa.samples === String(expected.samples), `${label}: ${expected.samples} chips show the sample behind their tag`,
            `samples=${qa.samples}`);
          check(Number(qa["min-sample-font-px"]) >= 10, `${label}: sample text >= 10px`, `sample font=${qa["min-sample-font-px"]}`);
          check(qa["sample-labels"] === qa.samples, `${label}: every sample is stated in words in the chip's accessible name`,
            `${qa["sample-labels"]}/${qa.samples}`);
          const listeners = JSON.parse(qa.listeners || "{}");
          check(listeners["hands-imported"] === 1 && listeners["overlay-pointer-left"] === 1,
            `${label}: one listener per event after StrictMode setup/cleanup/setup`, qa.listeners);

          const all = await dump(base, size, { max: String(max), frame, engine: "1", action: "hoverall" });
          const r = fields(all["action-result"]);
          // At the minimum size a 9-max table has no room left for some cards
          // once every seat's hole cards are hard zones (seatLayout.test.ts);
          // those chips show no card, which is reported, not failed.
          if (size.w === SIZES[1].w) {
            check(r.opened === String(expected.cards), `${label}: a card opens on every chip with an engine payload`, all["action-result"]);
          } else {
            info(`${label}: cards placed ${r.opened}/${expected.cards} (the rest fit nowhere clear of every seat's cards), at seat keys ${r.keys}`);
          }
          check(r.violations === "0" && r.hole === "0",
            `${label}: no card covers a protected zone, any seat's hole cards, its chip or the window edge`, all["action-result"]);
          check(r.hot === "0", `${label}: no card takes pointer input or becomes a hot zone`, all["action-result"]);
          check(r.stuck === "0", `${label}: every card closes on pointer leave`, all["action-result"]);
          info(`${label}: cards with two reads: ${r.twoReads}/${r.opened}`);
          // Every read's observation and advice wraps to at most two lines; the
          // clamp cuts a longer one short. The medium window must show them in
          // full; at the minimum one some may not fit, which is reported.
          if (size.w === SIZES[1].w) {
            check(r.clamped === "0", `${label}: no observation or advice of any card is clamped`,
              `clamped=${r.clamped}/${r.blocks} (${r.clampedWho})`);
          } else {
            info(`${label}: clamped observation/advice blocks: ${r.clamped}/${r.blocks} (${r.clampedWho})`);
          }
        });
      }
      group(async (t) => {
        const { check, info } = t;
        const seat = await hoverSeat(base, max, size);
        if (seat !== DRAG.seat) info(`${tag} hover: seat ${DRAG.seat}'s card fits nowhere at this size; hovering seat ${seat}`);
        const hover = { max: String(max), engine: "1", action: "hover", seat: String(seat) };
        const qa = await dump(base, size, hover);
        check(qa.card === "1" && qa["card-reads"] !== "0" && qa["card-violations"] === "0" && qa["card-hole"] === "0" && qa["card-hot"] === "0",
          `${tag} hover: card open, clear of protected zones and every seat's hole cards, not a hot zone`,
          `card=${qa.card} reads=${qa["card-reads"]} violations=${qa["card-violations"]} hole=${qa["card-hole"]} hot=${qa["card-hot"]}`);
        const villain = expected.at(seat);
        const top = villain.engine.topReads[0];
        const text = qa["card-text"] ?? "";
        check(text.includes(asCardText(top.observation)) && text.includes(asCardText(top.advice)) && text.includes(confidence(top)),
          `${tag} hover: card shows the fixture's top read (observation, advice, ${confidence(top)})`, text);
        // The card may sit away from its chip (never over any seat's cards), so it names its player.
        check(text.startsWith(villain.name), `${tag} hover: card names the player first`, text);
        await shot(t, base, size, { max: String(max), engine: "1" }, `hud-tagged-${tag}.png`);
        await shot(t, base, size, hover, `hud-hover-${tag}.png`);
      });
    }

    // Each lifecycle action is its own page load, so each is its own group.
    const size = SIZES[0];
    const tag = `${max}max-${size.w}x${size.h}`;
    const act = (action, verify) =>
      group(async ({ check }) => {
        const seat = await hoverSeat(base, max, size);
        verify(check, fields((await dump(base, size, { max: String(max), engine: "1", action, seat: String(seat) }))["action-result"]));
      });
    act("clampprobe", (check, r) =>
      check(r.card === "1" && Number(r.clamped) >= 1, `${tag} the clamp check counts a read too long for two lines`, JSON.stringify(r)));
    act("leave", (check, r) =>
      check(r.before === "1" && r.after === "0", `${tag} card closes on pointer leave`, JSON.stringify(r)));
    act("nativeleave", (check, r) =>
      check(r.before === "1" && r.otherTable === "1" && r.after === "0",
        `${tag} card closes on the native pointer-left event for its own table only (click-through window)`, JSON.stringify(r)));
    act("esc", (check, r) =>
      check(r.before === "1" && r.after === "0", `${tag} card closes on Esc`, JSON.stringify(r)));
    act("focus", (check, r) =>
      check(r.before === "1" && r.described === "1" && r.after === "0",
        `${tag} keyboard focus opens the card (aria-describedby), blur closes it`, JSON.stringify(r)));
    act("hoverdrag", (check, r) =>
      check(r.before === "1" && r.during === "0" && r.passOver === "0" && r.after === "0" && r.panel === "0",
        `${tag} drag start closes the card; none opens during the drag`, JSON.stringify(r)));
    act("hoverclick", (check, r) =>
      check(r.before === "1" && r.panel === "1" && r.after === "0" && r.enterWithDrawer === "0" && r.autoLabel === "1",
        `${tag} click opens the drawer (auto-notes labelled Auto) and closes the card; no card while it is open`, JSON.stringify(r)));
    act("refresh", (check, r) =>
      check(r.before === "1" && r.after === "1" && r.updated === "1", `${tag} a new hand refreshes the open card`, JSON.stringify(r)));
    act("drop", (check, r) =>
      check(r.before === "1" && r.after === "0", `${tag} a refresh that drops the player closes his card`, JSON.stringify(r)));
    act("outoforder", (check, r) =>
      check(r.before === "1" && r.mid === "new" && r.end === "new" && r.open === "1",
        `${tag} an older refresh answering last is ignored`, JSON.stringify(r)));
    act("burst", (check, r) =>
      check(r.before === "1" && r.inFlight === "1" && r.total === "2" && r.updated === "1" && r.open === "1",
        `${tag} a burst of new hands during a refresh collapses into one more refresh with the newest read`, JSON.stringify(r)));
  }
  group(async (t) => {
    await shot(t, base, SIZES[1], { max: "6", engine: "1", action: "click", seat: String(DRAG.seat) }, "hud-engine-drawer-6max-800x570.png");
    await shot(t, base, SIZES[1], { max: "6", engine: "1", action: "click", seat: String(DRAG.seat), scroll: "end" },
      "hud-engine-drawer-6max-800x570-end.png");
  });
}

/** The default build: chip layouts, drag, click and another overlay's save. */
function planDefaultChecks(base) {
  for (const max of TABLES) {
    for (const size of SIZES) {
      const tag = `${max}max-${size.w}x${size.h}`;
      for (const frame of ["hero", "absolute"]) {
        group(async ({ check }) => {
          const qa = await dump(base, size, { max: String(max), frame });
          const label = `${tag} ${frame} defaults`;
          check(qa.chips === String(max), `${label}: ${max} chips`, `chips=${qa.chips}`);
          check(qa.overlap === "0", `${label}: overlap=0`, `overlap=${qa.overlap}`);
          check(qa.intersections === "0", `${label}: intersections=0`, `intersections=${qa.intersections}`);
          check(qa["opp-hole-intersections"] === "0", `${label}: no chip over an opponent's hole cards`,
            `opp-hole=${qa["opp-hole-intersections"]}`);
          check(qa["plate-intersections"] === "0", `${label}: no chip over another seat's name plate`,
            `plates=${qa["plate-intersections"]}`);
          check(qa["save-count"] === "0", `${label}: nothing saved`, `saves=${qa["save-count"]}`);
          check(Number(qa["min-font-px"]) >= 10, `${label}: chip font >= 10px`, `font=${qa["min-font-px"]}`);
          check(qa.tags === "0" && qa.samples === "0" && qa.card === "0", `${label}: no engine (default build), no tag, no sample, no card`,
            `tags=${qa.tags} samples=${qa.samples}`);
        });
      }
      group(async (t) => {
        const badge = await dump(base, size, { max: String(max), model: "badge" });
        t.check(badge.overlap === "0" && badge.intersections === "0" && badge["plate-intersections"] === "0",
          `${tag} badge defaults: overlap=0, intersections=0, no other seat's plate`,
          `overlap=${badge.overlap} intersections=${badge.intersections} plates=${badge["plate-intersections"]}`);
        await shot(t, base, size, { max: String(max) }, `hud-compact-${tag}.png`);
      });
    }

    const size = SIZES[0];
    const tag = `${max}max-${size.w}x${size.h}`;
    const drag = { max: String(max), action: "drag", seat: String(DRAG.seat), dx: String(DRAG.dx), dy: String(DRAG.dy) };
    group(async (t) => {
      const { check } = t;
      const qa = await dump(base, size, drag);
      const last = qa["last-save"] ? JSON.parse(qa["last-save"]) : null;
      check(qa["save-count"] === "1", `${tag} drag: exactly one save_seat_position`, `saves=${qa["save-count"]}`);
      check(
        last?.maxPlayers === max && last?.frame === "hero" && last?.seatKey === DRAG.seat,
        `${tag} drag: saved seat key ${DRAG.seat} for ${max}-max hero`,
        qa["last-save"],
      );
      const expX = DRAG.dx / size.w;
      const expY = DRAG.dy / size.h;
      const moved = /moved=(-?[\d.]+),(-?[\d.]+) panel=(\d)/.exec(qa["action-result"] ?? "");
      check(
        !!moved && Math.abs(Number(moved[1]) - DRAG.dx) < 2 && Math.abs(Number(moved[2]) - DRAG.dy) < 2,
        `${tag} drag: chip moved by the drag distance`,
        `${qa["action-result"]} expected ~${DRAG.dx},${DRAG.dy} (${expX.toFixed(3)},${expY.toFixed(3)})`,
      );
      check(moved?.[3] === "0" && qa.panel === "0", `${tag} drag: release does not open the detail panel`, qa["action-result"]);
      check(Number(qa["max-zone-fraction"]) < 0.1, `${tag} drag: no hot zone near window size`, `largest=${qa["max-zone-fraction"]}`);
      await shot(t, base, size, drag, `hud-compact-${tag}-drag.png`);
    });
    group(async ({ check }) => {
      const cancel = await dump(base, size, { ...drag, action: "cancel" });
      check(cancel["save-count"] === "0", `${tag} pointercancel: nothing saved`, `saves=${cancel["save-count"]}`);
    });
    group(async (t) => {
      const { check } = t;
      const click = await dump(base, size, { max: String(max), action: "click", seat: String(DRAG.seat) });
      check(click.panel === "1" && click["save-count"] === "0", `${tag} click: opens the detail panel, saves nothing`,
        `${click["action-result"]} saves=${click["save-count"]}`);
      check(Number(click["max-zone-fraction"]) < 0.6, `${tag} click: panel hot zone is its own rect, not the window`,
        `largest=${click["max-zone-fraction"]}`);
      await shot(t, base, size, { max: String(max), action: "click", seat: String(DRAG.seat) }, `hud-compact-${tag}-panel.png`);
    });
    group(async ({ check }) => {
      const external = await dump(base, size, { max: String(max), action: "external", seat: String(DRAG.seat) });
      check(external["action-result"] === "followed", `${tag} another overlay's save moves this chip`, external["action-result"]);
    });
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
  const cleanup = () => stopVite(vite);
  process.on("exit", cleanup);
  process.on("SIGINT", () => process.exit(130));

  try {
    await waitForServer(`${base}/qa-overlay.html`, vite);
    // First hit compiles the module graph; warm it so virtual time is spent on the page.
    await dump(base, SIZES[0], { max: "6" }).catch(() => {});

    // QA_ONLY=engine skips the default-build checks while iterating on the engine ones.
    if (process.env.QA_ONLY !== "engine") planDefaultChecks(base);
    planEngineChecks(base);
    await runPlan();
  } catch (err) {
    failures.push(err.message);
    console.error(err.message);
  } finally {
    if (failures.length && viteLog) console.error(`--- Vite output ---\n${viteLog}`);
    cleanup();
    removeProfiles();
  }

  if (failures.length) {
    console.error(`\n${failures.length} failure(s).`);
    process.exit(1);
  }
  console.log("\nAll overlay QA checks passed.");
}

main();
