export const THEME_MODE_STORAGE_KEY = "agentmail-theme-mode";
export const WORKSPACE_SPLIT_STORAGE_KEY = "agentmail-workspace-split-percent";
export const ACTIVITY_LOG_STORAGE_KEY = "agentmail-show-activity-log";
export const DEFAULT_WORKSPACE_SPLIT_PERCENT = 45;

export type ThemeMode = "dark" | "light";

export function readStoredThemeMode(storage: Pick<Storage, "getItem"> | null | undefined): ThemeMode {
  try {
    return storage?.getItem(THEME_MODE_STORAGE_KEY) === "light" ? "light" : "dark";
  } catch {
    return "dark";
  }
}

export function getNextThemeMode(mode: ThemeMode): ThemeMode {
  return mode === "dark" ? "light" : "dark";
}

export function readStoredActivityLogVisibility(storage: Pick<Storage, "getItem"> | null | undefined) {
  try {
    return storage?.getItem(ACTIVITY_LOG_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

export function writeStoredActivityLogVisibility(storage: Pick<Storage, "setItem"> | null | undefined, visible: boolean) {
  try {
    storage?.setItem(ACTIVITY_LOG_STORAGE_KEY, visible ? "true" : "false");
  } catch {
    // Storage can be unavailable in hardened desktop/webview contexts.
  }
}

export function applyThemeModeToDocument(
  root: Pick<HTMLElement, "dataset" | "style">,
  storage: Pick<Storage, "setItem"> | null | undefined,
  mode: ThemeMode
) {
  root.dataset.theme = mode;
  root.style.colorScheme = mode;
  try {
    storage?.setItem(THEME_MODE_STORAGE_KEY, mode);
  } catch {
    // Storage can be unavailable in hardened desktop/webview contexts.
  }
}

export function readStoredWorkspaceSplitPercent(storage: Pick<Storage, "getItem"> | null | undefined) {
  try {
    const stored = storage?.getItem(WORKSPACE_SPLIT_STORAGE_KEY);
    if (!stored) return DEFAULT_WORKSPACE_SPLIT_PERCENT;
    const percent = Number(stored);
    return Number.isFinite(percent) && percent > 0 && percent < 100 ? percent : DEFAULT_WORKSPACE_SPLIT_PERCENT;
  } catch {
    return DEFAULT_WORKSPACE_SPLIT_PERCENT;
  }
}

export function writeStoredWorkspaceSplitPercent(storage: Pick<Storage, "setItem"> | null | undefined, percent: number) {
  try {
    storage?.setItem(WORKSPACE_SPLIT_STORAGE_KEY, String(percent));
  } catch {
    // Storage can be unavailable in hardened desktop/webview contexts.
  }
}
