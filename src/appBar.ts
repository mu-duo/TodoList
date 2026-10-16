/**
 * 应用功能栏（App Bar）入口元数据与顺序逻辑。
 *
 * 顺序契约：docs/pdr/10-app/02-interfaces.md §5；呈现契约：01-spec.md §8（FR-APP-22~24）。
 * 本模块全部为纯函数，便于单独验证（未知槽位 / 去重 / 新入口追置 / 恢复默认）。
 */

export type AppBarEntryKind = "action" | "config";

export type AppBarEntryMeta = {
  /** 稳定入口 id（持久化用，勿改）。 */
  id: string;
  /** 悬浮提示 / aria 名称（中文）。 */
  label: string;
  kind: AppBarEntryKind;
};

/** 已知入口；数组顺序即默认顺序（Agent 入口追加在末尾）。 */
export const APP_BAR_ENTRIES: AppBarEntryMeta[] = [
  { id: "refresh", label: "刷新", kind: "action" },
  { id: "pin", label: "置顶", kind: "action" },
  { id: "note", label: "便签", kind: "action" },
  { id: "settings", label: "设置", kind: "config" },
  { id: "agent", label: "Agent", kind: "config" },
];

/** 默认顺序：刷新 → 置顶 → 设置 → Agent。 */
export const DEFAULT_APP_BAR_ORDER: string[] = APP_BAR_ENTRIES.map((e) => e.id);

export function isKnownAppBarId(id: string): boolean {
  return APP_BAR_ENTRIES.some((entry) => entry.id === id);
}

/** 读取侧清洗：非数组 / 无有效字符串 → null（= 默认）；否则返回字符串数组。 */
export function sanitizeAppBarOrder(raw: unknown): string[] | null {
  if (!Array.isArray(raw)) return null;
  const list = raw.filter((id): id is string => typeof id === "string");
  return list.length > 0 ? list : null;
}

/**
 * 解析「有效顺序」：
 * 1. 缺失 / 为空 → 默认顺序；
 * 2. 去重（保留首次出现）；
 * 3. 未知 id 原样保留（占位，供未来版本还原）；
 * 4. 已保存顺序中缺失的已知入口，按默认顺序追加到末尾。
 */
export function resolveAppBarOrder(saved: unknown): string[] {
  const savedList = Array.isArray(saved)
    ? saved.filter((id): id is string => typeof id === "string")
    : [];
  const seen = new Set<string>();
  const result: string[] = [];
  for (const id of savedList) {
    if (seen.has(id)) continue;
    seen.add(id);
    result.push(id);
  }
  for (const id of DEFAULT_APP_BAR_ORDER) {
    if (seen.has(id)) continue;
    seen.add(id);
    result.push(id);
  }
  return result;
}

/** 仅可见（已知）入口，保持有效顺序；未知 id 隐藏。 */
export function visibleAppBarIds(order: string[]): string[] {
  return order.filter(isKnownAppBarId);
}

/**
 * 用新的「可见顺序」重排：未知 id 保留在原槽位，已知入口按 newVisible 依次填入。
 * 仅重排可见入口，不移动未知 id 的相对位置。
 */
export function mergeAppBarOrder(
  order: string[],
  newVisible: string[],
): string[] {
  const queue = newVisible.slice();
  const result = order.map((id) =>
    isKnownAppBarId(id) ? queue.shift() ?? id : id,
  );
  if (queue.length > 0) result.push(...queue);
  return result;
}

/** 可见顺序内上移 / 下移一格（越界返回原数组）。 */
export function moveAppBarVisible(
  visible: string[],
  id: string,
  dir: -1 | 1,
): string[] {
  const from = visible.indexOf(id);
  if (from < 0) return visible;
  const to = from + dir;
  if (to < 0 || to >= visible.length) return visible;
  const next = visible.slice();
  next[from] = visible[to];
  next[to] = visible[from];
  return next;
}

/** 是否存在已保存的顺序记录（非空有效数组，含未知 id）→ 决定「恢复默认」是否可点。
 *  注意：即便保存值与默认顺序完全相同，也是**实际写入的记录**，恢复默认应能清除该字段。 */
export function hasSavedAppBarOrder(saved: unknown): boolean {
  return sanitizeAppBarOrder(saved) !== null;
}
