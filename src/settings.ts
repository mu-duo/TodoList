import { invoke } from "@tauri-apps/api/core";
import { isThemePref, type ThemePref } from "./theme";
import { sanitizeAppBarOrder } from "./appBar";
import { sanitizeCompletedCollapsed } from "./completedGroups";
import { sanitizeDetailCollapsed } from "./detailSections";
import type { DetailSectionKey } from "./detailSections";

export type AppSettings = {
  theme: ThemePref;
  /** 已保存的功能栏顺序；null = 默认（字段缺失 / 为空 / 非法）。 */
  appBarOrder: string[] | null;
  /** 「已完成」视图折叠的清单文件名集合；null = 默认全展开（字段缺失 / 为空 / 非法）。 */
  completedCollapsed: string[] | null;
  /** 任务详情中收起的子板块 key；null = 默认（仅「任务属性」收起）。空数组合法 = 全部展开。 */
  detailCollapsed: DetailSectionKey[] | null;
  /** 「关闭时最小化到托盘」；null = 默认开启（字段缺失 / 非法），false = 已关闭。 */
  minimizeToTray: boolean | null;
};

/**
 * 一次性读取本机偏好（主题 + 功能栏顺序 + 已完成折叠集合）。
 * 读取失败 / 后端未就绪一律回退默认，不报错；启动时只读一次，避免多处各读造成竞争。
 */
export async function loadSettings(): Promise<AppSettings> {
  try {
    const raw = await invoke<{
      theme?: unknown;
      app_bar_order?: unknown;
      completed_collapsed?: unknown;
      detail_collapsed?: unknown;
      minimize_to_tray?: unknown;
    }>("load_settings");
    return {
      theme: isThemePref(raw?.theme) ? raw.theme : "system",
      appBarOrder: sanitizeAppBarOrder(raw?.app_bar_order),
      completedCollapsed: sanitizeCompletedCollapsed(raw?.completed_collapsed),
      detailCollapsed: sanitizeDetailCollapsed(raw?.detail_collapsed),
      // 仅接受真正的布尔；缺失 / 非法 → null（= 默认开启）
      minimizeToTray:
        typeof raw?.minimize_to_tray === "boolean"
          ? raw.minimize_to_tray
          : null,
    };
  } catch {
    return {
      theme: "system",
      appBarOrder: null,
      completedCollapsed: null,
      detailCollapsed: null,
      minimizeToTray: null,
    };
  }
}
