import { invoke } from "@tauri-apps/api/core";
import { isThemePref, type ThemePref } from "./theme";
import { sanitizeAppBarOrder } from "./appBar";

export type AppSettings = {
  theme: ThemePref;
  /** 已保存的功能栏顺序；null = 默认（字段缺失 / 为空 / 非法）。 */
  appBarOrder: string[] | null;
};

/**
 * 一次性读取本机偏好（主题 + 功能栏顺序）。
 * 读取失败 / 后端未就绪一律回退默认，不报错；启动时只读一次，避免多处各读造成竞争。
 */
export async function loadSettings(): Promise<AppSettings> {
  try {
    const raw = await invoke<{ theme?: unknown; app_bar_order?: unknown }>(
      "load_settings",
    );
    return {
      theme: isThemePref(raw?.theme) ? raw.theme : "system",
      appBarOrder: sanitizeAppBarOrder(raw?.app_bar_order),
    };
  } catch {
    return { theme: "system", appBarOrder: null };
  }
}
