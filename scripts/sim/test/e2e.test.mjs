// The e2e driver's plan: options, the eight table slots, titles, the seeder,
// app and generator command lines and the overridden environment, all
// computed without opening a window or launching anything. The dry run
// itself is a scheduled check (`node scripts/sim/e2e.mjs --dry-run`). Run
// with `npm run test:sim`.

import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { buildPlan, chipName, desktopBlocker, exposedTables, generatorRuns, parseArgs, pickChip, tableSlots, villainChips } from "../e2e.mjs";
import { CdpPage, inspectArguments, listPages, pageKind, panelSearch } from "../lib/cdp.mjs";
import { parseArgs as parseGeneratorArgs } from "../generate.mjs";
import { assertPlainArgs, vcvarsLine } from "../lib/msvc.mjs";
import { LOGGED_IN_MARKER, TABLE_SIZES } from "../lib/tables.mjs";

const AREA = { x: 0, y: 0, width: 2560, height: 1400 };
const root = () => mkdtempSync(join(tmpdir(), "velora-e2e-plan-"));

test("options: defaults, features and refused values", () => {
  const o = parseArgs([]);
  assert.equal(o.dryRun, false);
  assert.deepEqual(o.features, []);
  assert.equal(o.cash + o.mtt, 8);
  assert.ok(o.hands * (o.cash + o.mtt) >= 300, "the default session is at least 300 hands");
  const s = parseArgs(["--dry-run", "--features", "strategic-analysis", "--cash", "2", "--mtt", "6", "--pace", "0.5", "--keep-main"]);
  assert.equal(s.dryRun, true);
  assert.deepEqual(s.features, ["strategic-analysis"]);
  assert.equal(s.cash, 2);
  assert.equal(s.pace, 0.5);
  assert.equal(s.keepMain, true);
  assert.throws(() => parseArgs(["--features", "strategic-analysis&calc"]), /unsupported feature/);
  assert.throws(() => parseArgs(["--features", "evil"]), /unsupported feature/);
  assert.throws(() => parseArgs(["--seed", "a b"]), /seed/);
  assert.throws(() => parseArgs(["--cash", "0", "--mtt", "0"]), /at least one table/);
  assert.throws(() => parseArgs(["--cash", "-1"]), /number/);
  assert.throws(() => parseArgs(["--root"]), /needs a value/);
  assert.throws(() => parseArgs(["--launch"]), /unknown option/);
});

test("the default slots mix 6-max and 9-max at minimum and medium size", () => {
  const slots = tableSlots(parseArgs([]));
  const combos = new Set(slots.map((s) => `${s.seats}:${s.size}`));
  assert.deepEqual(combos, new Set(["6:min", "6:medium", "9:min", "9:medium"]));
  assert.equal(slots.filter((s) => s.kind === "cash").every((s) => s.seats === 6), true);
  assert.equal(slots.filter((s) => s.kind === "mtt").every((s) => s.seats === 9), true);
});

test("the plan: cash windows titled before launch, data paths and environment in the root", () => {
  const o = parseArgs(["--features", "strategic-analysis"]);
  const plan = buildPlan(o, { area: AREA, root: root() });
  assert.equal(plan.slots.length, 8);
  for (const slot of plan.slots) {
    assert.deepEqual({ width: slot.rect.width, height: slot.rect.height }, TABLE_SIZES[slot.size]);
    if (slot.kind === "cash") {
      assert.ok(slot.title.endsWith(`${LOGGED_IN_MARKER}${plan.hero}`));
      assert.ok(slot.title.includes(` - ${slot.table} - `), slot.title);
    } else assert.equal(slot.title, null, "tournament titles come from their files");
  }
  for (const value of Object.values(plan.env)) assert.ok(value.startsWith(plan.paths.root), value);
  assert.ok(plan.shots.startsWith(plan.paths.root));
  assert.deepEqual(plan.app.args, ["run", "tauri", "dev", "--", "--no-watch", "--features", "strategic-analysis"]);
  assert.deepEqual(plan.seed.args.slice(0, 5), ["run", "--example", "sim_seed", "--features", "strategic-analysis"]);
  assert.deepEqual(plan.seed.args.slice(-3), ["--", plan.paths.db, plan.paths.handHistoryRoot]);
  // Every command line survives cmd.exe verbatim (the temp root has no spaces).
  for (const cmd of [plan.seed, plan.app]) assert.doesNotThrow(() => assertPlainArgs([cmd.command, ...cmd.args]));

  const plain = buildPlan(parseArgs([]), { area: AREA, root: root() });
  assert.deepEqual(plain.app.args, ["run", "tauri", "dev", "--", "--no-watch"]);
  assert.deepEqual(plain.seed.args.slice(0, 4), ["run", "--example", "sim_seed", "--"]);
});

test("generator runs write live-clock hands into the hero's folder; the backlog uses the simulated clock", () => {
  const o = parseArgs(["--backlog", "40"]);
  const plan = buildPlan(o, { area: AREA, root: root() });
  const runs = generatorRuns(o, plan.paths);
  assert.deepEqual(runs.map((r) => r.phase), ["backlog", "live", "live"]);
  for (const run of runs) {
    const args = parseGeneratorArgs(run.args);
    assert.equal(args.out, plan.paths.handHistory);
    assert.equal(args.clock, run.phase === "live" ? "live" : "sim");
  }
  const [backlog, cash, mtt] = runs.map((r) => parseGeneratorArgs(r.args));
  assert.equal(backlog.hands, 40);
  assert.equal(backlog.pace, 0);
  assert.notEqual(backlog.seed, cash.seed);
  assert.equal(cash.format, "cash");
  assert.equal(cash.tables, o.cash);
  assert.equal(mtt.format, "mtt");
  assert.equal(mtt.tables, o.mtt);
  assert.deepEqual(generatorRuns(parseArgs(["--cash", "0", "--mtt", "2"]), plan.paths).map((r) => r.format), ["mtt"]);
});

test("vcvars command lines refuse cmd metacharacters", () => {
  assert.match(vcvarsLine("cargo", ["--version"]), /vcvars64\.bat" >nul && cargo --version$/);
  for (const bad of ["a b", 'a"b', "a&b", "a|b", "a>b", "%PATH%", "a^b", "(x)"]) {
    assert.throws(() => vcvarsLine("cargo", [bad]), /unsupported argument/, bad);
  }
});

test("a locked session blocks the run with the reason; unlocked or unknown does not", () => {
  assert.match(desktopBlocker({ locked: true }), /session is locked/);
  assert.equal(desktopBlocker({ locked: false }), null);
  assert.equal(desktopBlocker({ locked: null }), null);
  assert.equal(desktopBlocker(null), null);
});

test("hover and click target only tables no later window covers", () => {
  const entry = (id, x, y) => [id, { slot: { rect: { x, y, width: 483, height: 359 } } }];
  const centre = (e) => [{ x: e.slot.rect.x + 200, y: e.slot.rect.y + 200 }];
  // Cascaded 40px apart, as on a small screen: only the top window is exposed.
  const cascade = [entry("a", 0, 0), entry("b", 40, 40), entry("c", 80, 80)];
  assert.deepEqual(exposedTables(cascade, centre).map(([id]) => id), ["c"]);
  // Side by side: all exposed.
  const tiled = [entry("a", 0, 0), entry("b", 483, 0), entry("c", 966, 0)];
  assert.deepEqual(exposedTables(tiled, centre).map(([id]) => id), ["a", "b", "c"]);
  // A later window covering a corner the points do not use does not count.
  assert.deepEqual(exposedTables([entry("a", 0, 0), entry("b", 400, 300)], centre).map(([id]) => id), ["a", "b"]);
  assert.deepEqual(exposedTables([], centre), []);
});

test("observe mode: no mouse, keyboard or screenshots in the plan", () => {
  assert.equal(parseArgs([]).observe, false);
  const o = parseArgs(["--observe"]);
  assert.equal(o.observe, true);
  const steps = buildPlan(o, { area: AREA, root: root() }).steps.join(" | ");
  assert.match(steps, /observe only/);
  assert.doesNotMatch(steps, /hover a villain chip/);
  assert.match(buildPlan(parseArgs([]), { area: AREA, root: root() }).steps.join(" | "), /hover a villain chip/);
});

test("inspect mode: loopback DevTools port in the app's own environment only", () => {
  assert.equal(parseArgs([]).inspect, 0);
  const plain = buildPlan(parseArgs([]), { area: AREA, root: root() });
  assert.equal(plain.env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS, undefined);
  assert.doesNotMatch(plain.steps.join(" | "), /DevTools/);
  const o = parseArgs(["--observe", "--inspect", "9333"]);
  assert.equal(o.inspect, 9333);
  const plan = buildPlan(o, { area: AREA, root: root() });
  const args = plan.env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS;
  assert.match(args, /--remote-debugging-port=9333/);
  assert.match(args, /--remote-debugging-address=127\.0\.0\.1/);
  assert.match(plan.steps.join(" | "), /DevTools port 9333 \(loopback\)/);
  // The driver's own process never gets it: only the plan's app environment.
  assert.equal(process.env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS, undefined);
  for (const bad of ["80", "70000", "x"]) assert.throws(() => parseArgs(["--inspect", bad]));
  assert.throws(() => inspectArguments(1023));
});

test("inspect mode: page kinds, chip picking and the search script", () => {
  assert.equal(pageKind("http://localhost:1420/overlay.html?table=4"), "overlay");
  assert.equal(pageKind("http://localhost:1420/panel.html"), "panel");
  assert.equal(pageKind("http://localhost:1420/"), "main");
  assert.equal(pageKind("https://example.com/x.html"), null);
  assert.equal(pageKind("not a url"), null);
  const chip = (label) => ({ label, rect: { x: 0, y: 0, width: 10, height: 10 } });
  const hero = chip("SimHero, 41 hands, VPIP 17, PFR 15, 3-Bet 5. Open details");
  const plain = chip("RockSolidRui, 32 hands, VPIP 3, PFR 3, 3-Bet 0. Open details");
  const cc = chip("CallMeMaybe77, 30 hands, VPIP 40, PFR 0, 3-Bet 0, tag CC+, Cold-calls 50% (8/16) of opens., based on 16 opportunities. Open details");
  const lag = chip("TripleBarrelTom, 21 hands, VPIP 38, PFR 33, 3-Bet 25, under 25 hands, small sample, tag LAG, Plays 38%. Open details");
  assert.equal(chipName(cc), "CallMeMaybe77");
  assert.deepEqual(villainChips([hero, plain, cc], "SimHero").map(chipName), ["RockSolidRui", "CallMeMaybe77"]);
  // Tagged villains first, rotating by round; never the hero.
  assert.equal(pickChip([hero, plain, cc, lag], 1, "SimHero"), cc);
  assert.equal(pickChip([hero, plain, cc, lag], 2, "SimHero"), lag);
  assert.equal(pickChip([hero, plain], 1, "SimHero"), plain);
  assert.equal(pickChip([hero], 1, "SimHero"), null);
  // The query is embedded as a JSON string: quotes cannot break out of it.
  const script = panelSearch('a"); alert(1); ("');
  assert.match(script, /set\.call\(input, "a\\"\); alert\(1\); \(\\""\)/);
});

test("inspect mode: only loopback DevTools targets are listed or connected to", async () => {
  const targets = [
    { id: "a", type: "page", url: "http://localhost:1420/overlay.html?table=1", title: "o", webSocketDebuggerUrl: "ws://127.0.0.1:9333/devtools/page/a" },
    { id: "b", type: "page", url: "http://localhost:1420/panel.html", title: "p", webSocketDebuggerUrl: "ws://192.168.1.20:9333/devtools/page/b" },
    { id: "c", type: "service_worker", url: "http://localhost:1420/sw.js", title: "w", webSocketDebuggerUrl: "ws://127.0.0.1:9333/devtools/page/c" },
    { id: "d", type: "page", url: "http://localhost:1420/", title: "m", webSocketDebuggerUrl: "ws://evil.example:9333/devtools/page/d" },
  ];
  const server = createServer((req, res) => {
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify(req.url === "/json/list" ? targets : []));
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    const pages = await listPages(server.address().port);
    assert.deepEqual(pages.map((p) => p.id), ["a"]);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
  await assert.rejects(CdpPage.connect("ws://evil.example:9333/devtools/page/d"), /non-loopback/);
  await assert.rejects(CdpPage.connect("ws://127.0.0.1.evil.example:9333/devtools/page/d"), /non-loopback/);
});
