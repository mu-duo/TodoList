/**
 * 「已完成」视图的分组与折叠纯逻辑（无 React / DOM 依赖，便于独立断言验证）。
 *
 * 覆盖两件事：
 * - 分组：按来源清单分组，组顺序严格跟随侧栏清单顺序；空组不显示；
 *   组内按完成时间（`completed`）倒序，缺失该字段的条目回退文件内顺序（稳定排序）。
 * - 折叠：以清单文件名为键的集合；读取清洗、失效键过滤、切换时计算下一份集合。
 *
 * 契约见 `docs/issues/feature/agent/已完成清单分组折叠.md`（FR-APP-18 的补充）。
 */

/** 已完成视图中的一条任务（结构最小化，便于纯逻辑断言）。 */
export type CompletedEntry = {
  listFile: string;
  /** 来源清单名（可选；分组视图的组头承载来源，行内不再使用）。 */
  listName?: string;
  task: { id: string; completed?: string | null };
};

/** 侧栏清单的最小引用（数组顺序即展示顺序）。 */
export type CompletedListRef = { file: string; name: string };

/** 一个来源分组。 */
export type CompletedGroup = {
  /** 来源清单文件名（折叠键）。 */
  file: string;
  /** 来源清单名（组头）。 */
  name: string;
  /** 组内条目（按完成时间倒序；缺失回退文件内顺序）。 */
  tasks: CompletedEntry[];
};

/**
 * 按来源清单分组：
 * 1. 组顺序严格跟随 `lists`（不额外排序）；只产出非空组；
 * 2. 组内按 `completed` 字符串倒序；缺失该字段者回退文件内顺序并排在末尾
 *    （依赖 `Array.prototype.sort` 的稳定性）。
 * 不在任何清单内的条目（理论上不会出现）被丢弃。
 */
export function buildCompletedGroups(
  lists: readonly CompletedListRef[],
  tasks: readonly CompletedEntry[],
): CompletedGroup[] {
  const byFile = new Map<string, CompletedEntry[]>();
  for (const entry of tasks) {
    const bucket = byFile.get(entry.listFile);
    if (bucket) bucket.push(entry);
    else byFile.set(entry.listFile, [entry]);
  }
  const groups: CompletedGroup[] = [];
  for (const list of lists) {
    const bucket = byFile.get(list.file);
    if (!bucket || bucket.length === 0) continue;
    groups.push({
      file: list.file,
      name: list.name,
      tasks: bucket.slice().sort(compareByCompletedDesc),
    });
  }
  return groups;
}

/**
 * `completed` 倒序比较：值大者在前；相等（含双方都缺失）返回 0，交由稳定排序
 * 保持原文件内顺序。缺失（null / undefined）视作空串，排在最后。
 */
function compareByCompletedDesc(a: CompletedEntry, b: CompletedEntry): number {
  const ka = a.task.completed ?? "";
  const kb = b.task.completed ?? "";
  if (ka === kb) return 0;
  return ka < kb ? 1 : -1;
}

/** 读取清洗：非数组 → null（= 默认全展开）；过滤空白 / 非字符串；去重后为空 → null。 */
export function sanitizeCompletedCollapsed(raw: unknown): string[] | null {
  if (!Array.isArray(raw)) return null;
  const seen = new Set<string>();
  const list: string[] = [];
  for (const item of raw) {
    if (typeof item !== "string" || item.trim().length === 0) continue;
    if (seen.has(item)) continue;
    seen.add(item);
    list.push(item);
  }
  return list.length > 0 ? list : null;
}

/**
 * 过滤失效键：仅保留当前仍存在的清单文件名。
 * 清单重命名 / 删除后遗留的旧键由此静默忽略（不报错、也不影响展开）。
 */
export function activeCollapsedFiles(
  collapsed: readonly string[],
  lists: readonly CompletedListRef[],
): string[] {
  const known = new Set(lists.map((list) => list.file));
  return collapsed.filter((file) => known.has(file));
}

/**
 * 计算下一份折叠集合：把 `file` 加入 / 移出折叠集合。
 * 结果为空时返回 null（后端据此恢复默认全展开）；已存在的键保留原位，新键追加末尾。
 */
export function nextCollapsed(
  collapsed: readonly string[],
  file: string,
  collapse: boolean,
): string[] | null {
  const set = new Set(collapsed);
  if (collapse) set.add(file);
  else set.delete(file);
  const next = [...set];
  return next.length > 0 ? next : null;
}

/** 两个折叠集合是否等价（与元素顺序无关）。 */
export function sameCollapsed(
  a: readonly string[],
  b: readonly string[],
): boolean {
  if (a.length !== b.length) return false;
  const set = new Set(a);
  return b.every((file) => set.has(file));
}
