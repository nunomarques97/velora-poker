// Screenshots and layout assertions for the overlay HUD, from qa-overlay.html.
//
// Starts Vite on a free port, then drives the installed Edge headless:
// --screenshot for the images in docs/design/verification/hud-compact-*.png
// and --dump-dom to read the numbers qa-overlay.html writes to <body>:
//   - the default layouts: no chip overlaps another chip (overlap=0) and no
//     chip covers a protected zone, i.e. board, bets, the hero's hole cards or
//     action buttons (intersections=0), nor an opponent's hole cards;
//   - a scripted drag: exactly one save_seat_position call, for the dragged
//     seat key; a cancelled drag saves nothing; a plain click opens the
//     detail panel; a save made by another overlay of the same size moves
//     this one's chip.
// Exits non-zero on any failure and always stops Vite.
//
// The table behind the HUD is drawn from seatLayout.ts's ASSUMED geometry,
// not captured from a live PokerStars client.
//
// Usage: node scripts/qa-overlay-shots.mjs

import { spawn, execFile } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
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
    attrs[m[1]] = m[2].replace(/&quot;/g, '"').replace(/&amp;/g, "&");
  }
  return attrs;
}

const pageUrl = (base, params) => `${base}/qa-overlay.html?${new URLSearchParams({ table: "1", ...params })}`;

async function dump(base, size, params) {
  const dom = await edge([`--window-size=${size.w},${size.h}`, "--dump-dom", pageUrl(base, params)], { capture: true });
  const qa = qaAttributes(dom);
  if (qa.ready !== "1") throw new Error(`qa-overlay.html never settled for ${JSON.stringify(params)} at ${size.w}x${size.h}`);
  return qa;
}

async function shot(base, size, params, name) {
  const file = join(outDir, name);
  await edge([`--window-size=${size.w},${size.h}`, `--screenshot=${file}`, pageUrl(base, params)]);
  check(existsSync(file), `screenshot ${name}`);
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

    for (const max of TABLES) {
      for (const size of SIZES) {
        const tag = `${max}max-${size.w}x${size.h}`;
        for (const frame of ["hero", "absolute"]) {
          const qa = await dump(base, size, { max: String(max), frame });
          const label = `${tag} ${frame} defaults`;
          check(qa.chips === String(max), `${label}: ${max} chips`, `chips=${qa.chips}`);
          check(qa.overlap === "0", `${label}: overlap=0`, `overlap=${qa.overlap}`);
          check(qa.intersections === "0", `${label}: intersections=0`, `intersections=${qa.intersections}`);
          check(qa["opp-hole-intersections"] === "0", `${label}: no chip over an opponent's hole cards`,
            `opp-hole=${qa["opp-hole-intersections"]}`);
          check(qa["save-count"] === "0", `${label}: nothing saved`, `saves=${qa["save-count"]}`);
          check(Number(qa["min-font-px"]) >= 10, `${label}: chip font >= 10px`, `font=${qa["min-font-px"]}`);
        }
        const badge = await dump(base, size, { max: String(max), model: "badge" });
        check(badge.overlap === "0" && badge.intersections === "0", `${tag} badge defaults: overlap=0, intersections=0`,
          `overlap=${badge.overlap} intersections=${badge.intersections}`);
        await shot(base, size, { max: String(max) }, `hud-compact-${tag}.png`);
      }

      const size = SIZES[0];
      const tag = `${max}max-${size.w}x${size.h}`;
      const drag = { max: String(max), action: "drag", seat: String(DRAG.seat), dx: String(DRAG.dx), dy: String(DRAG.dy) };
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
      await shot(base, size, drag, `hud-compact-${tag}-drag.png`);

      const cancel = await dump(base, size, { ...drag, action: "cancel" });
      check(cancel["save-count"] === "0", `${tag} pointercancel: nothing saved`, `saves=${cancel["save-count"]}`);

      const click = await dump(base, size, { max: String(max), action: "click", seat: String(DRAG.seat) });
      check(click.panel === "1" && click["save-count"] === "0", `${tag} click: opens the detail panel, saves nothing`,
        `${click["action-result"]} saves=${click["save-count"]}`);
      check(Number(click["max-zone-fraction"]) < 0.6, `${tag} click: panel hot zone is its own rect, not the window`,
        `largest=${click["max-zone-fraction"]}`);
      await shot(base, size, { max: String(max), action: "click", seat: String(DRAG.seat) }, `hud-compact-${tag}-panel.png`);

      const external = await dump(base, size, { max: String(max), action: "external", seat: String(DRAG.seat) });
      check(external["action-result"] === "followed", `${tag} another overlay's save moves this chip`, external["action-result"]);
    }
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
  console.log("\nAll overlay QA checks passed.");
}

main();
