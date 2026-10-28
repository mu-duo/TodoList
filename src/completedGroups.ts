/**
 * 「已完成」视图的分组 / 时间分桶 / 折叠纯逻辑（无 React / DOM 依赖，便于独立断言验证）。
 *
 * 覆盖三件事：
 * - 分组：按来源清单分组，组顺序严格跟随侧栏清单顺序；空组不显示；
 *   组内按完成时间（`completed`）倒序，缺失该字段的条目回退文件内顺序（稳定排序）。
 * - 时间分桶（FR-APP-36）：每个来源清单内按本地自然日再分「今天 / 七天内 / 更早」；
 *   归档项（task.archived 存在）默认不参与，开启「显示归档」后沉到所属时间组末尾。
 * - 折叠：以清单文件名为键的集合；读取清洗、失效键过滤、切换时计算下一份集合。
 *
 * 契约见 `docs/issues/feature/history/已完成清单分组折叠-done.md`（FR-APP-18 的补充）与
 * `docs/issues/feature/agent/已完成清单归档与时间分组.md`（FR-APP-36）。
 */

/** 已完成视图任务的最小结构（`archived` 为可选字段，旧数据缺失即未归档）。 */
export type CompletedTaskRef = {
  id: string;
  completed?: string | null;
  /** 归档时间戳（本地时间字符串）；字段存在（含空串）即视为已归档。 */
  archived?: string | null;
};

/** 已完成视图中的一条任务（结构最小化，便于纯逻辑断言）。 */
export type CompletedEntry = {
  listFile: string;
  /** 来源清单名（可选；分组视图的组头承载来源，行内不再使用）。 */
  listName?: string;
  task: CompletedTaskRef;
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

/* -------------------------------------------------------------------------- */
/* FR-APP-36：时间分桶                                                         */
/* -------------------------------------------------------------------------- */

/** 时间档位（固定顺序：今天 → 七天内 → 更早）。 */
export type TimeBucket = "today" | "last7" | "older";

/** 时间档位的固定展示顺序。 */
export const COMPLETED_BUCKETS: readonly TimeBucket[] = ["today", "last7", "older"];

/** 时间档位的中文标签。 */
export const COMPLETED_BUCKET_LABELS: Readonly<Record<TimeBucket, string>> = {
  today: "今天",
  last7: "七天内",
  older: "更早",
};

/** 本地时间戳：`YYYY-MM-DDTHH:mm` / `YYYY-MM-DD HH:mm:ss` / 裸日期 `YYYY-MM-DD`。 */
const LOCAL_TIMESTAMP_RE =
  /^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,3})\d*)?)?)?$/;

/** 把本地墙上时间字符串解析为 Date；格式不符 / 日历非法 → null（不臆测时区）。 */
function parseLocalTimestamp(value: string): Date | null {
  const match = LOCAL_TIMESTAMP_RE.exec(value.trim());
  if (!match) return null;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const hour = match[4] === undefined ? 0 : Number(match[4]);
  const minute = match[5] === undefined ? 0 : Number(match[5]);
  const second = match[6] === undefined ? 0 : Number(match[6]);
  const millis = match[7] === undefined ? 0 : Number(match[7].padEnd(3, "0"));
  if (
    month < 1 ||
    month > 12 ||
    day < 1 ||
    day > 31 ||
    hour > 23 ||
    minute > 59 ||
    second > 59
  ) {
    return null;
  }
  const date = new Date(year, month - 1, day, hour, minute, second, millis);
  // 拒绝 2026-02-31 这类被 Date 自动进位的日期。
  if (
    date.getFullYear() !== year ||
    date.getMonth() !== month - 1 ||
    date.getDate() !== day
  ) {
    return null;
  }
  return date;
}

/** 本地自然日序号（用本地年月日折算，避免夏令时影响天数差）。 */
function localDayOrdinal(date: Date): number {
  return Math.floor(
    Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / 86_400_000,
  );
}

/**
 * 完成时间所属档位（以 `now` 的本地自然日为基准）：
 * - 今天：同一自然日；**未来时间戳也归「今天」**（不产生未来档位）；
 * - 七天内：昨天至第 7 天前（day-1..day-7，不含今天）；
 * - 更早：第 8 天前及更早；缺失 / 无法解析 → 更早。
 */
export function timeBucketOf(
  completed: string | undefined | null,
  now: Date,
): TimeBucket {
  if (typeof completed !== "string") return "older";
  const parsed = parseLocalTimestamp(completed);
  if (!parsed) return "older";
  const diff = localDayOrdinal(now) - localDayOrdinal(parsed);
  if (diff <= 0) return "today";
  if (diff <= 7) return "last7";
  return "older";
}

/**
 * 已归档判定：`archived` 字段存在且为字符串（含空串）即已归档；
 * null / undefined（旧数据）视为未归档。
 */
function isArchived(entry: CompletedEntry): boolean {
  return typeof entry.task.archived === "string";
}

/** 可见项排序：未归档在前、已归档在后；同组内按 `completed` 倒序（缺失垫底）。 */
function compareVisible(a: CompletedEntry, b: CompletedEntry): number {
  const archivedA = isArchived(a) ? 1 : 0;
  const archivedB = isArchived(b) ? 1 : 0;
  if (archivedA !== archivedB) return archivedA - archivedB;
  return compareByCompletedDesc(a, b);
}

/**
 * 单个来源清单内的可见项与顺序：关闭「显示归档」时剔除归档项；
 * 开启时保留但沉到末尾。两组内部各自按 `completed` 倒序（稳定，缺失垫底）。
 */
export function visibleTasksForGroup(
  tasks: readonly CompletedEntry[],
  showArchived: boolean,
): CompletedEntry[] {
  const visible = showArchived
    ? tasks.slice()
    : tasks.filter((entry) => !isArchived(entry));
  return visible.sort(compareVisible);
}

/** 一个来源清单内的单个时间档位。 */
export type BucketedTimeGroup = {
  bucket: TimeBucket;
  /** 中文标签（「今天」/「七天内」/「更早」）。 */
  label: string;
  /** 该档位内可见项（未归档在前、归档沉底，各自 `completed` 倒序）。 */
  tasks: CompletedEntry[];
};

/** 时间分桶后的来源清单分组（视图直接消费）。 */
export type BucketedCompletedGroup = {
  listFile: string;
  listName: string;
  /** 该组当前可见项总数（= 各非空档位之和）。 */
  count: number;
  /** 仅包含非空档位，顺序固定为 今天 → 七天内 → 更早。 */
  buckets: BucketedTimeGroup[];
};

/**
 * 把来源清单分组进一步按时间分档：
 * - 档位顺序固定 `COMPLETED_BUCKETS`；空档位不产出；
 * - 某清单下无可见项时整组不产出（`showArchived === false` 且全部归档即属此列）；
 * - 组内可见项总数写入 `count`，供组头「名称 · 数量」直接使用。
 */
export function bucketCompletedGroups(
  groups: readonly CompletedGroup[],
  showArchived: boolean,
  now: Date,
): BucketedCompletedGroup[] {
  const result: BucketedCompletedGroup[] = [];
  for (const group of groups) {
    const ordered = visibleTasksForGroup(group.tasks, showArchived);
    if (ordered.length === 0) continue;
    const byBucket = new Map<TimeBucket, CompletedEntry[]>();
    for (const entry of ordered) {
      const bucket = timeBucketOf(entry.task.completed, now);
      const bucketTasks = byBucket.get(bucket);
      if (bucketTasks) bucketTasks.push(entry);
      else byBucket.set(bucket, [entry]);
    }
    const buckets: BucketedTimeGroup[] = [];
    for (const bucket of COMPLETED_BUCKETS) {
      const bucketTasks = byBucket.get(bucket);
      if (!bucketTasks || bucketTasks.length === 0) continue;
      buckets.push({
        bucket,
        label: COMPLETED_BUCKET_LABELS[bucket],
        tasks: bucketTasks,
      });
    }
    result.push({
      listFile: group.file,
      listName: group.name,
      count: ordered.length,
      buckets,
    });
  }
  return result;
}

/**
 * 侧栏「已完成」总数：与视图同源，只统计当前渲染的可见项。
 * 直接对 `bucketCompletedGroups` 的结果求和，避免与视图计数口径漂移。
 */
export function totalVisibleCompleted(
  groups: readonly BucketedCompletedGroup[],
): number {
  let total = 0;
  for (const group of groups) total += group.count;
  return total;
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
