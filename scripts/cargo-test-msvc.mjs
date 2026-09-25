// Runs `cargo test` from src-tauri inside the VS2022 BuildTools environment.
//
// A plain shell on this machine fails to compile the bundled SQLite with
// "cannot open include file: 'excpt.h'": the VS18 Community MSVC install is
// broken, and cargo picks it up unless vcvars64.bat of the working VS2022
// BuildTools has set the environment first. That is not a code error, so
// this helper always sets it up. Checks cannot run .bat files or shell chains
// directly, hence node -> cmd.exe with verbatim arguments.
//
// Usage: node scripts/cargo-test-msvc.mjs [extra cargo test args...]
// Exits with cargo's own exit code.

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const VCVARS =
  "C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Auxiliary\\Build\\vcvars64.bat";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const cwd = resolve(root, "src-tauri");

if (process.platform !== "win32") {
  console.error("cargo-test-msvc: this helper is Windows-only.");
  process.exit(1);
}
if (!existsSync(VCVARS)) {
  console.error(`cargo-test-msvc: VS2022 BuildTools not found at ${VCVARS}`);
  process.exit(1);
}

// Extra arguments are forwarded to cargo. Anything with cmd metacharacters is
// refused rather than escaped: the command line is passed verbatim.
const extra = process.argv.slice(2);
for (const arg of extra) {
  if (/[\s"&|<>^%!()]/.test(arg)) {
    console.error(`cargo-test-msvc: unsupported argument ${JSON.stringify(arg)}`);
    process.exit(2);
  }
}

// `/s /c "<line>"`: cmd strips exactly the outer quotes and runs the rest,
// so the quoted .bat path inside survives intact.
const line = `"${VCVARS}" >nul && cargo test ${extra.join(" ")}`.trimEnd();
const comspec = process.env.ComSpec || "C:\\Windows\\System32\\cmd.exe";

const child = spawn(comspec, ["/d", "/s", "/c", `"${line}"`], {
  cwd,
  stdio: "inherit",
  windowsVerbatimArguments: true,
});

child.on("error", (err) => {
  console.error(`cargo-test-msvc: could not start cmd.exe: ${err.message}`);
  process.exit(1);
});
child.on("exit", (code, signal) => {
  if (signal) {
    console.error(`cargo-test-msvc: cargo terminated by ${signal}`);
    process.exit(1);
  }
  process.exit(code ?? 1);
});
