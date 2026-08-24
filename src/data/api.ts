import { invoke } from "@tauri-apps/api/core";
import type { ImportStatus, Player } from "./types";

export class DesktopAppRequiredError extends Error {
  constructor() {
    super(
      "Real hand history data requires running inside the Velora desktop app window (npm run tauri dev), not a plain browser tab.",
    );
    this.name = "DesktopAppRequiredError";
  }
}

function assertTauriAvailable(): void {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) {
    throw new DesktopAppRequiredError();
  }
}

export async function getPlayers(): Promise<Player[]> {
  assertTauriAvailable();
  return invoke<Player[]>("get_players");
}

export async function getImportStatus(): Promise<ImportStatus> {
  assertTauriAvailable();
  return invoke<ImportStatus>("get_import_status");
}

export async function setHandHistoryDir(path: string): Promise<ImportStatus> {
  assertTauriAvailable();
  return invoke<ImportStatus>("set_hand_history_dir", { path });
}
