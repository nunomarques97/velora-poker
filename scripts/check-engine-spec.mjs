// Traceability checker for docs/specs/opponent-engine.md.
//
// Parses the scenario catalogue (the table between the `catalogue:start` and
// `catalogue:end` markers) and fails when:
//   - a required format or context tag is covered by no row,
//   - a row lacks a rule id or a test name, or has a malformed cell,
//   - a scenario id or a rule id is duplicated,
//   - a row marked `implemented` names a rule id that does not appear as a
//     string literal ("rule.id") under src-tauri/src, or a test that does not
//     exist as `fn <name>` under src-tauri,
//   - the rule-quality matrix (section 17.4, between the `rule-matrix:start`
//     and `rule-matrix:end` markers) does not list every rule id of `RULES`
//     in src-tauri/src/engine/rules.rs exactly once, or lists a row id that
//     is neither a rule nor a string literal under src-tauri/src.
//
// Usage:
//   node scripts/check-engine-spec.mjs                 structure + implemented rows
//   node scripts/check-engine-spec.mjs --require-all   also fails on any row not implemented
//   node scripts/check-engine-spec.mjs --self-test     runs the checker against embedded
//                                                      broken samples; exits 0 only if every
//                                                      one is rejected (and a valid one passes)

import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SPEC_PATH = join(root, "docs", "specs", "opponent-engine.md");
const TAURI_DIR = join(root, "src-tauri");
const RULES_PATH = join(TAURI_DIR, "src", "engine", "rules.rs");
const SKIP_DIRS = new Set(["target", "gen", "node_modules", ".git"]);

export const FORMATS = ["MTT", "6-max cash", "Zoom", "Spin & Go"];
export const REQUIRED_CONTEXTS = [
  "preflop-open", "steal", "blind-defense", "3bet-ip", "3bet-oop", "4bet", "squeeze",
  "limp", "iso", "limp-reraise", "cold-call", "cbet-flop", "cbet-turn", "cbet-river",
  "fold-to-cbet-flop", "fold-to-cbet-turn", "fold-to-cbet-river", "delayed-cbet",
  "check-raise", "donk", "float", "probe", "barrel", "wtsd", "wsd", "wwsf",
  "river-aggression", "push-fold", "reshove", "deep", "stage", "bounty", "head-to-head",
  "showdown-memory", "sizing-tell", "recency-tilt", "sample-honesty",
];
const COLUMNS = [
  "ID", "Formats", "Context", "Pro exploit", "Stat keys (opportunity / success)",
  "Rule ids", "Tests", "Status",
];
const STATUSES = ["planned", "implemented"];
const START = "<!-- catalogue:start -->";
const END = "<!-- catalogue:end -->";
const MATRIX_START = "<!-- rule-matrix:start -->";
const MATRIX_END = "<!-- rule-matrix:end -->";

const ID_RE = /^[A-Z][A-Z0-9]*[0-9]$/;
const RULE_RE = /^[a-z][a-z0-9_]*(\.[a-z0-9_]+)+$/;
const TEST_RE = /^[a-z_][a-z0-9_]*$/;
const CONTEXT_RE = /^[a-z0-9][a-z0-9-]*$/;

function splitRow(line) {
  let body = line.trim();
  if (!body.startsWith("|") || !body.endsWith("|")) return null;
  body = body.slice(1, -1);
  return body.split("|").map((cell) => cell.trim());
}

function backticked(cell) {
  return [...cell.matchAll(/`([^`]*)`/g)].map((m) => m[1].trim());
}

/** Parses the catalogue out of the spec text. Returns { rows, errors }. */
export function parseCatalogue(text) {
  const errors = [];
  const start = text.indexOf(START);
  const end = text.indexOf(END);
  if (start < 0 || end < 0 || end < start) {
    return { rows: [], errors: [`catalogue markers ${START} / ${END} not found in order`] };
  }
  const lines = text
    .slice(start + START.length, end)
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
  if (lines.length < 3) {
    return { rows: [], errors: ["catalogue has no header, separator and rows"] };
  }
  const header = splitRow(lines[0]);
  if (!header || header.join("|") !== COLUMNS.join("|")) {
    errors.push(`catalogue header must be exactly: | ${COLUMNS.join(" | ")} |`);
  }
  const sep = splitRow(lines[1]);
  if (!sep || sep.length !== COLUMNS.length || !sep.every((c) => /^:?-{3,}:?$/.test(c))) {
    errors.push("catalogue separator row is malformed");
  }
  const rows = [];
  lines.slice(2).forEach((line, i) => {
    const cells = splitRow(line);
    const where = `catalogue row ${i + 1}`;
    if (!cells) {
      errors.push(`${where}: not a table row`);
      return;
    }
    if (cells.length !== COLUMNS.length) {
      errors.push(`${where}: expected ${COLUMNS.length} cells, found ${cells.length}`);
      return;
    }
    const [id, formats, context, exploit, statKeys, ruleCell, testCell, status] = cells;
    rows.push({
      line: i + 1,
      id,
      formats: formats.split(",").map((f) => f.trim()).filter(Boolean),
      contexts: backticked(context),
      exploit,
      statKeys,
      ruleIds: backticked(ruleCell),
      tests: backticked(testCell),
      status,
    });
  });
  return { rows, errors };
}

function escapeRe(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Validates parsed rows. `sources` = { ruleSources: string[], testSources: string[] }
 * holds the contents of .rs files under src-tauri/src and under src-tauri.
 */
export function validateRows(rows, sources, { requireAll = false } = {}) {
  const errors = [];
  if (rows.length === 0) errors.push("catalogue has no scenario rows");
  const seenIds = new Map();
  const seenRules = new Map();
  const formatsCovered = new Set();
  const contextsCovered = new Set();

  for (const row of rows) {
    const where = `row ${row.id || `#${row.line}`}`;
    if (!row.id) errors.push(`${where}: missing scenario id`);
    else if (!ID_RE.test(row.id)) errors.push(`${where}: malformed scenario id`);
    else if (seenIds.has(row.id)) errors.push(`${where}: duplicate scenario id`);
    else seenIds.set(row.id, row);

    if (row.formats.length === 0) errors.push(`${where}: no format`);
    for (const f of row.formats) {
      if (f === "all") FORMATS.forEach((x) => formatsCovered.add(x));
      else if (FORMATS.includes(f)) formatsCovered.add(f);
      else errors.push(`${where}: unknown format "${f}"`);
    }

    if (row.contexts.length === 0) errors.push(`${where}: no context tag`);
    for (const c of row.contexts) {
      if (!CONTEXT_RE.test(c)) errors.push(`${where}: malformed context tag "${c}"`);
      contextsCovered.add(c);
    }

    if (!row.exploit) errors.push(`${where}: missing pro exploit`);
    if (!row.statKeys) errors.push(`${where}: missing stat keys`);

    if (row.ruleIds.length === 0) errors.push(`${where}: no rule id`);
    for (const r of row.ruleIds) {
      if (!RULE_RE.test(r)) errors.push(`${where}: malformed rule id "${r}"`);
      else if (seenRules.has(r)) errors.push(`${where}: rule id "${r}" already used by row ${seenRules.get(r)}`);
      else seenRules.set(r, row.id);
    }

    if (row.tests.length === 0) errors.push(`${where}: no test name`);
    for (const t of row.tests) {
      if (!TEST_RE.test(t)) errors.push(`${where}: malformed test name "${t}"`);
    }

    if (!STATUSES.includes(row.status)) {
      errors.push(`${where}: status must be one of ${STATUSES.join("|")}, found "${row.status}"`);
    } else if (row.status === "implemented") {
      for (const r of row.ruleIds) {
        const literal = `"${r}"`;
        if (!sources.ruleSources.some((src) => src.includes(literal))) {
          errors.push(`${where}: implemented but rule id ${literal} is not a string literal under src-tauri/src`);
        }
      }
      for (const t of row.tests) {
        if (!TEST_RE.test(t)) continue;
        const fnRe = new RegExp(`\\bfn\\s+${escapeRe(t)}\\s*[(<]`);
        if (!sources.testSources.some((src) => fnRe.test(src))) {
          errors.push(`${where}: implemented but test fn ${t} does not exist under src-tauri`);
        }
      }
    } else if (requireAll) {
      errors.push(`${where}: --require-all but status is "${row.status}"`);
    }
  }

  for (const f of FORMATS) {
    if (!formatsCovered.has(f)) errors.push(`required format "${f}" is covered by no row`);
  }
  for (const c of REQUIRED_CONTEXTS) {
    if (!contextsCovered.has(c)) errors.push(`required context "${c}" is covered by no row`);
  }
  return errors;
}

/** The first-cell rule ids of the rule-quality matrix. Returns { ids, errors }. */
export function parseRuleMatrix(text) {
  const start = text.indexOf(MATRIX_START);
  const end = text.indexOf(MATRIX_END);
  if (start < 0 || end < 0 || end < start) {
    return { ids: [], errors: [`rule-quality matrix markers ${MATRIX_START} / ${MATRIX_END} not found in order`] };
  }
  const ids = [];
  for (const line of text.slice(start + MATRIX_START.length, end).split(/\r?\n/)) {
    const m = /^\|\s*`([^`]+)`\s*\|/.exec(line.trim());
    if (m) ids.push(m[1].trim());
  }
  return { ids, errors: ids.length === 0 ? ["rule-quality matrix has no rule rows"] : [] };
}

/**
 * The rule ids of `RULES` in rules.rs: each `rule("id", …)` or
 * `rule(CONST, …)`, with a constant resolved through its
 * `const CONST: &str = "…";` in any source under src-tauri/src.
 */
export function codeRuleIds(rulesFile, ruleSources) {
  const errors = [];
  const open = rulesFile.indexOf("pub const RULES: &[RuleDef] = &[");
  const close = open < 0 ? -1 : rulesFile.indexOf("\n];", open);
  if (open < 0 || close < 0) return { ids: [], errors: ["RULES not found in src-tauri/src/engine/rules.rs"] };
  const ids = [];
  for (const m of rulesFile.slice(open, close).matchAll(/\brule\(\s*(?:"([^"]+)"|([A-Z][A-Z0-9_]*))\s*,/g)) {
    if (m[1]) {
      ids.push(m[1]);
      continue;
    }
    const constRe = new RegExp(`\\bconst\\s+${m[2]}\\s*:\\s*&(?:'static\\s+)?str\\s*=\\s*"([^"]+)"`);
    const found = ruleSources.map((src) => constRe.exec(src)).find(Boolean);
    if (found) ids.push(found[1]);
    else errors.push(`RULES uses ${m[2]}, which resolves to no string constant under src-tauri/src`);
  }
  if (ids.length === 0 && errors.length === 0) errors.push("RULES holds no rule");
  return { ids, errors };
}

/** Every rule id once in the matrix; any other row id must exist in the code. */
export function validateMatrix(matrixIds, ruleIds, ruleSources) {
  const errors = [];
  for (const id of ruleIds) {
    const count = matrixIds.filter((m) => m === id).length;
    if (count !== 1) errors.push(`rule-quality matrix lists rule id "${id}" ${count} times (expected exactly once)`);
  }
  for (const id of new Set(matrixIds)) {
    if (ruleIds.includes(id)) continue;
    if (!ruleSources.some((src) => src.includes(`"${id}"`))) {
      errors.push(`rule-quality matrix row "${id}" is neither a rule of RULES nor a string literal under src-tauri/src`);
    } else if (matrixIds.filter((m) => m === id).length > 1) {
      errors.push(`rule-quality matrix lists "${id}" more than once`);
    }
  }
  return errors;
}

export function checkSpec(text, sources, options) {
  const { rows, errors } = parseCatalogue(text);
  const matrix = parseRuleMatrix(text);
  const code = codeRuleIds(sources.rulesFile ?? "", sources.ruleSources);
  const matrixErrors = [...matrix.errors, ...code.errors];
  if (matrixErrors.length === 0) matrixErrors.push(...validateMatrix(matrix.ids, code.ids, sources.ruleSources));
  return { rows, errors: [...errors, ...validateRows(rows, sources, options), ...matrixErrors] };
}

function collectRustFiles(dir, out = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (!SKIP_DIRS.has(entry.name)) collectRustFiles(join(dir, entry.name), out);
    } else if (entry.name.endsWith(".rs")) {
      out.push(join(dir, entry.name));
    }
  }
  return out;
}

function loadSources() {
  const files = collectRustFiles(TAURI_DIR);
  const srcDir = join(TAURI_DIR, "src");
  const ruleSources = [];
  const testSources = [];
  for (const file of files) {
    const content = readFileSync(file, "utf8");
    testSources.push(content);
    if (!relative(srcDir, file).startsWith("..")) ruleSources.push(content);
  }
  return { ruleSources, testSources, rulesFile: readFileSync(RULES_PATH, "utf8") };
}

// ---------------------------------------------------------------------------
// Self-test: embedded samples. Every broken sample must be rejected; the valid
// samples must pass, so a checker that rejects everything cannot pass either.

const DEFAULT_MATRIX = ["pf.sample.one", "pf.sample.two"];

function sampleSpec(rows, { header = COLUMNS, markers = true, matrix = DEFAULT_MATRIX, matrixMarkers = true } = {}) {
  const table = [
    `| ${header.join(" | ")} |`,
    `|${header.map(() => "---").join("|")}|`,
    ...rows,
  ].join("\n");
  const catalogue = markers ? `${START}\n${table}\n${END}\n` : `${table}\n`;
  const matrixTable = ["| Rule id | Verdict |", "|---|---|", ...matrix.map((id) => `| \`${id}\` | kept |`)].join("\n");
  const matrixPart = matrixMarkers ? `${MATRIX_START}\n${matrixTable}\n${MATRIX_END}\n` : `${matrixTable}\n`;
  return `# Sample\n\n${catalogue}\n## Matrix\n\n${matrixPart}`;
}

const ALL_CONTEXTS = REQUIRED_CONTEXTS.map((c) => `\`${c}\``).join(", ");
const row = ({
  id = "A01",
  formats = "all",
  context = `${ALL_CONTEXTS} coverage row`,
  exploit = "Exploit text",
  stats = "`vpip` — opp: dealt in; success: voluntary money",
  rules = "`pf.sample.one`",
  tests = "`sample_test_one`",
  status = "planned",
} = {}) => `| ${id} | ${formats} | ${context} | ${exploit} | ${stats} | ${rules} | ${tests} | ${status} |`;

// RULES with a literal id and one through a constant, as rules.rs does with
// the seat-relation rule; the constant is resolved from `ruleSources`.
const SAMPLE_RULES_FILE = [
  "pub const RULES: &[RuleDef] = &[",
  '    rule("pf.sample.one", "A01", F::Preflop, T, None, "Obs.", "Advice."),',
  '    left(rule(SAMPLE_TWO, "A02", F::Stage, T, None, "Obs.", "Advice.")),',
  "];",
].join("\n");
const SAMPLE_CONST = 'pub const SAMPLE_TWO: &str = "pf.sample.two";';
const GOOD_SOURCES = {
  ruleSources: [`pub const RULE: &str = "pf.sample.one";\nconst B: &str = "pf.sample.two";\n${SAMPLE_CONST}`],
  testSources: ["#[test]\nfn sample_test_one() {}\n#[test]\nfn sample_test_two() {}\n"],
  rulesFile: SAMPLE_RULES_FILE,
};
const EMPTY_SOURCES = { ruleSources: [SAMPLE_CONST], testSources: [], rulesFile: SAMPLE_RULES_FILE };
const NEAR_MISS_SOURCES = {
  ruleSources: [`const A: &str = "pf.sample.one_more";\n// pf.sample.one without quotes\n${SAMPLE_CONST}`],
  testSources: ["fn sample_test_one_extra() {}\n// fn sample_test_one is a comment mention? no paren\n"],
  rulesFile: SAMPLE_RULES_FILE,
};

const SAMPLES = [
  { name: "valid planned catalogue", expect: "pass", text: sampleSpec([row()]), sources: EMPTY_SOURCES },
  {
    name: "valid implemented catalogue under --require-all",
    expect: "pass",
    text: sampleSpec([
      row({ status: "implemented" }),
      row({ id: "A02", formats: "MTT", context: "`stage`", rules: "`pf.sample.two`", tests: "`sample_test_two`", status: "implemented" }),
    ]),
    sources: GOOD_SOURCES,
    options: { requireAll: true },
  },
  { name: "missing catalogue markers", text: sampleSpec([row()], { markers: false }) },
  { name: "header with a missing column", text: sampleSpec([row()], { header: COLUMNS.filter((c) => c !== "Pro exploit") }) },
  { name: "row with too few cells", text: sampleSpec([row(), "| A02 | all | `stage` | x | y | `a.b` | `t_a` |"]) },
  { name: "required format missing", text: sampleSpec([row({ formats: "MTT, 6-max cash, Zoom" })]) },
  { name: "unknown format token", text: sampleSpec([row({ formats: "all, Omaha" })]) },
  {
    name: "required context missing",
    text: sampleSpec([row({ context: REQUIRED_CONTEXTS.filter((c) => c !== "squeeze").map((c) => `\`${c}\``).join(", ") })]),
  },
  { name: "row lacking rule id", text: sampleSpec([row(), row({ id: "A02", rules: "" })]) },
  { name: "row lacking test name", text: sampleSpec([row(), row({ id: "A02", rules: "`pf.sample.two`", tests: "" })]) },
  { name: "duplicate scenario id", text: sampleSpec([row(), row({ rules: "`pf.sample.two`" })]) },
  { name: "duplicate rule id across rows", text: sampleSpec([row(), row({ id: "A02" })]) },
  { name: "unknown status", text: sampleSpec([row({ status: "done" })]) },
  { name: "malformed rule id", text: sampleSpec([row({ rules: "`NotARule`" })]) },
  { name: "implemented row, rule literal absent", text: sampleSpec([row({ status: "implemented" })]), sources: EMPTY_SOURCES },
  {
    name: "implemented row, test fn absent",
    text: sampleSpec([row({ status: "implemented" })]),
    sources: { ruleSources: GOOD_SOURCES.ruleSources, testSources: [] },
  },
  {
    name: "implemented row, only near-miss rule literal and test fn",
    text: sampleSpec([row({ status: "implemented" })]),
    sources: NEAR_MISS_SOURCES,
  },
  {
    name: "rule literal only outside src-tauri/src",
    text: sampleSpec([row({ status: "implemented" })]),
    sources: { ruleSources: [], testSources: [...GOOD_SOURCES.testSources, 'const R: &str = "pf.sample.one";'] },
  },
  { name: "--require-all with a planned row", text: sampleSpec([row()]), sources: EMPTY_SOURCES, options: { requireAll: true } },
  {
    name: "matrix with a derivation row that is a literal in the code",
    expect: "pass",
    text: sampleSpec([row()], { matrix: [...DEFAULT_MATRIX, "pf.sample.one_more"] }),
    sources: { ...EMPTY_SOURCES, ruleSources: [SAMPLE_CONST, 'const D: &str = "pf.sample.one_more";'] },
  },
  { name: "missing rule-matrix markers", text: sampleSpec([row()], { matrixMarkers: false }), sources: EMPTY_SOURCES },
  { name: "matrix lacking a rule id of RULES", text: sampleSpec([row()], { matrix: ["pf.sample.one"] }), sources: EMPTY_SOURCES },
  {
    name: "matrix lacking the rule whose id is a constant",
    text: sampleSpec([row()], { matrix: ["pf.sample.one", "pf.sample.three"] }),
    sources: EMPTY_SOURCES,
  },
  {
    name: "matrix listing a rule id twice",
    text: sampleSpec([row()], { matrix: [...DEFAULT_MATRIX, "pf.sample.one"] }),
    sources: EMPTY_SOURCES,
  },
  {
    name: "matrix row that is no rule and no code literal",
    text: sampleSpec([row()], { matrix: [...DEFAULT_MATRIX, "pf.sample.ghost"] }),
    sources: EMPTY_SOURCES,
  },
  {
    name: "RULES constant that resolves to no string",
    text: sampleSpec([row()]),
    sources: { ...EMPTY_SOURCES, ruleSources: [] },
  },
  { name: "rules.rs without RULES", text: sampleSpec([row()]), sources: { ...EMPTY_SOURCES, rulesFile: "fn main() {}" } },
];

function selfTest() {
  let failures = 0;
  for (const sample of SAMPLES) {
    const expect = sample.expect ?? "reject";
    const { errors } = checkSpec(sample.text, sample.sources ?? GOOD_SOURCES, sample.options ?? {});
    const got = errors.length > 0 ? "reject" : "pass";
    const ok = got === expect;
    if (!ok) failures += 1;
    const detail = errors.length > 0 ? ` (${errors[0]})` : "";
    console.log(`${ok ? "ok  " : "FAIL"} ${sample.name}: expected ${expect}, got ${got}${detail}`);
  }
  console.log(failures === 0
    ? `check-engine-spec self-test: all ${SAMPLES.length} samples behaved as expected`
    : `check-engine-spec self-test: ${failures} of ${SAMPLES.length} samples misbehaved`);
  return failures === 0 ? 0 : 1;
}

function main(argv) {
  const known = new Set(["--require-all", "--self-test"]);
  const unknown = argv.filter((a) => !known.has(a));
  if (unknown.length > 0) {
    console.error(`check-engine-spec: unknown argument(s): ${unknown.join(" ")}`);
    return 2;
  }
  if (argv.includes("--self-test")) return selfTest();

  const requireAll = argv.includes("--require-all");
  const text = readFileSync(SPEC_PATH, "utf8");
  const sources = loadSources();
  const { rows, errors } = checkSpec(text, sources, { requireAll });
  if (errors.length > 0) {
    console.error(`check-engine-spec: ${errors.length} problem(s) in ${relative(root, SPEC_PATH)}`);
    for (const e of errors) console.error(`  - ${e}`);
    return 1;
  }
  const implemented = rows.filter((r) => r.status === "implemented").length;
  console.log(
    `check-engine-spec: OK — ${rows.length} scenarios, ${implemented} implemented, ` +
      `${rows.length - implemented} planned${requireAll ? " (--require-all)" : ""}; ` +
      `rule-quality matrix covers all ${codeRuleIds(sources.rulesFile, sources.ruleSources).ids.length} rules`,
  );
  return 0;
}

process.exitCode = main(process.argv.slice(2));
