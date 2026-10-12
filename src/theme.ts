import { getCurrentWindow } from "@tauri-apps/api/window";

/** 主题偏好：跟随系统 / 亮色 / 暗色。 */
export type ThemePref = "system" | "light" | "dark";

/** 校验后端返回的主题值是否合法。 */
export function isThemePref(value: unknown): value is ThemePref {
  return value === "system" || value === "light" || value === "dark";
}

/**
 * 把偏好应用到窗口：`system` → `setTheme(null)`（跟随系统），
 * 否则钉死为对应主题；同步 `color-scheme` 令原生控件（滚动条等）随之变化。
 * 权限缺失时 `setTheme` 会抛错，由调用方记入 actionError。
 */
export async function applyThemePref(pref: ThemePref): Promise<void> {
  const win = getCurrentWindow();
  await win.setTheme(pref === "system" ? null : pref);
  document.documentElement.style.colorScheme =
    pref === "system" ? "light dark" : pref;
}
