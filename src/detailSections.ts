/**
 * 任务详情子板块的折叠纯逻辑（无 React / DOM 依赖）。
 *
 * 契约见 `docs/issues/feature/agent/详情面板统一子板块与折叠.md` 与
 * `docs/product/10-app/02-interfaces.md` 的 `detail.collapsed`。
 *
 * 与「已完成」分组折叠的差异：此处**默认不是全展开**，缺 key = 仅「任务属性」收起，
 * 因此空数组合法（= 用户把「任务属性」也展开了），后端必须把空数组写盘。
 */

/** 子板块 key（数组顺序即详情面板渲染顺序）。 */
export const DETAIL_SECTION_KEYS = [
  "title",
  "metadata",
  "subtasks",
  "notes",
  "meta",
  "materials",
  "agent",
] as const;

export type DetailSectionKey = (typeof DETAIL_SECTION_KEYS)[number];

/** 首次（`settings.yaml` 无 `detail.collapsed`）的默认收起集合：仅「任务属性」。 */
export const DEFAULT_DETAIL_COLLAPSED: readonly DetailSectionKey[] = [
  "metadata",
];

/** 是否是已知的子板块 key（用于过滤历史遗留 / 手改 YAML 的未知值）。 */
export function isDetailSectionKey(value: unknown): value is DetailSectionKey {
  return (
    typeof value === "string" &&
    (DETAIL_SECTION_KEYS as readonly string[]).includes(value)
  );
}

/**
 * 读取清洗：非数组 → null（= 默认，仅「任务属性」收起）；
 * 过滤未知 key 与重复项；**保留空数组**（= 全部展开）。
 */
export function sanitizeDetailCollapsed(
  raw: unknown,
): DetailSectionKey[] | null {
  if (!Array.isArray(raw)) return null;
  const seen = new Set<DetailSectionKey>();
  const list: DetailSectionKey[] = [];
  for (const item of raw) {
    if (!isDetailSectionKey(item) || seen.has(item)) continue;
    seen.add(item);
    list.push(item);
  }
  return list;
}

/** 计算下一份收起集合：加入 / 移出某个 key；结果可为空数组（= 全部展开）。 */
export function nextDetailCollapsed(
  collapsed: readonly string[],
  key: DetailSectionKey,
  collapse: boolean,
): DetailSectionKey[] {
  const set = new Set(collapsed.filter(isDetailSectionKey));
  if (collapse) set.add(key);
  else set.delete(key);
  return [...set];
}

/** 两个收起集合是否等价（与元素顺序无关）。 */
export function sameDetailCollapsed(
  a: readonly string[],
  b: readonly string[],
): boolean {
  if (a.length !== b.length) return false;
  const set = new Set(a);
  return b.every((key) => set.has(key));
}
