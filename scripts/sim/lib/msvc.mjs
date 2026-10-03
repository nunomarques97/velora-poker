// Runs a command inside the VS2022 BuildTools environment (vcvars64.bat),
// like scripts/cargo-test-msvc.mjs: a plain shell on this machine picks up
// the broken VS18 MSVC install and fails on excpt.h. node -> cmd.exe with
// verbatim arguments, so arguments with cmd metacharacters are refused.

import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";

export const VCVARS =
  "C:\\Program Files (x86)\\Microsoft Visual Studio\\2022\\BuildTools\\VC\\Auxiliary\\Build\\vcvars64.bat";
const COMSPEC = () => process.env.ComSpec || "C:\\Windows\\System32\\cmd.exe";

/** Throws on an argument cmd.exe would reinterpret. */
export function assertPlainArgs(args) {
  for (const arg of args) {
    if (/[\s"&|<>^%!()]/.test(arg)) throw new Error(`unsupported argument ${JSON.stringify(arg)}`);
  }
}

/** The `cmd /s /c` line: vcvars, then the command. */
export function vcvarsLine(command, args = []) {
  assertPlainArgs([command, ...args]);
  return `"${VCVARS}" >nul && ${[command, ...args].join(" ")}`;
}

const cmdArgs = (line) => ["/d", "/s", "/c", `"${line}"`];

/** Starts `command args` under vcvars; returns the child process. */
export function spawnUnderVcvars(command, args, { cwd, env, stdio = "pipe" } = {}) {
  if (!existsSync(VCVARS)) throw new Error(`VS2022 BuildTools not found at ${VCVARS}`);
  return spawn(COMSPEC(), cmdArgs(vcvarsLine(command, args)), {
    cwd,
    env,
    stdio,
    windowsVerbatimArguments: true,
    windowsHide: true,
  });
}

/** Runs `command args` under vcvars to completion. */
export function runUnderVcvars(command, args, { cwd, env, timeoutMs = 120_000 } = {}) {
  if (!existsSync(VCVARS)) throw new Error(`VS2022 BuildTools not found at ${VCVARS}`);
  return spawnSync(COMSPEC(), cmdArgs(vcvarsLine(command, args)), {
    cwd,
    env,
    encoding: "utf8",
    timeout: timeoutMs,
    windowsVerbatimArguments: true,
    windowsHide: true,
  });
}

/** Ends a process and its whole tree (cmd -> npm -> cargo -> app). */
export function killTree(pid) {
  if (!pid) return;
  spawnSync("taskkill", ["/PID", String(pid), "/T", "/F"], { windowsHide: true, encoding: "utf8" });
}
