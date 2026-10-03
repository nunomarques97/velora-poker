// The e2e driver's plan: options, the eight table slots, titles, the seeder,
// app and generator command lines and the overridden environment, all
// computed without opening a window or launching anything. The dry run
// itself is a scheduled check (`node scripts/sim/e2e.mjs --dry-run`). Run
// with `npm run test:sim`.

import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { buildPlan, generatorRuns, parseArgs, tableSlots } from "../e2e.mjs";
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
  assert.deepEqual(plan.app.args, ["run", "tauri", "dev", "--", "--features", "strategic-analysis"]);
  assert.deepEqual(plan.seed.args.slice(0, 5), ["run", "--example", "sim_seed", "--features", "strategic-analysis"]);
  assert.deepEqual(plan.seed.args.slice(-3), ["--", plan.paths.db, plan.paths.handHistoryRoot]);
  // Every command line survives cmd.exe verbatim (the temp root has no spaces).
  for (const cmd of [plan.seed, plan.app]) assert.doesNotThrow(() => assertPlainArgs([cmd.command, ...cmd.args]));

  const plain = buildPlan(parseArgs([]), { area: AREA, root: root() });
  assert.deepEqual(plain.app.args, ["run", "tauri", "dev"]);
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
