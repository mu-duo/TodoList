/**
 * 「删除回收站」纯逻辑（FR-APP-37；无 React / DOM 依赖，便于独立断言验证）。
 *
 * 后端把任务 / 清单删除到 `lists/.trash.yaml`，24 小时内可恢复；本模块负责：
 * - 解析后端返回的本地墙上时间戳（`YYYY-MM-DDTHH:MM:SS` 等）；
 * - 计算剩余保留时长并格式化为中文文案；
 * - 过滤已过期条目（无法解析的时间戳一律视为未过期，避免误删）；
 * - 按来源清单把任务分组（现有清单顺序优先，已消失清单追加在后）。
 *
 * 注意：解析失败时 `isTrashExpired` 返回 false，宁可多显示也不误判过期。
 */

/** 回收站保留时长：24 小时。 */
export const TRASH_RETENTION_MS: number = 24 * 60 * 60 * 1000;

/** 回收站中的任务条目（与后端 `tasks` 条目同形；`task` 只声明 UI 用到的字段）。 */
export type TrashTaskRef = {
  trash_id: string;
  /** 删除时间（本地墙上时间字符串）。 */
  deleted: string;
  /** 来源清单文件名。 */
  list_file: string;
  /** 来源清单名（删除时的快照）。 */
  list_name: string;
  task: {
    id: string;
    title: string;
    status?: string;
    completed?: string | null;
    archived?: string | null;
    notes?: string | null;
    subtasks?: Array<{ id: string; title: string; status: string }> | null;
  };
};

/** 回收站中的清单条目。 */
export type TrashListRef = {
  trash_id: string;
  /** 删除时间（本地墙上时间字符串）。 */
  deleted: string;
  /** 清单文件名。 */
  file: string;
  /** 清单名。 */
  name: string;
  /** 该清单内的任务数。 */
  task_count: number;
};

/** 后端回收站视图。 */
export type TrashView = {
  tasks: TrashTaskRef[];
  lists: TrashListRef[];
};

/** 一个来源清单的回收站任务分组。 */
export type TrashTaskGroup = {
  /** 来源清单文件名。 */
  listFile: string;
  /** 组头显示名。 */
  listName: string;
  /** 组内任务（按删除时间倒序，无法解析者垫底）。 */
  tasks: TrashTaskRef[];
};

/** 本地时间戳：`YYYY-MM-DDTHH:mm[:ss]` / `YYYY-MM-DD HH:mm[:ss]` / 裸日期 `YYYY-MM-DD`。 */
const TRASH_TIMESTAMP_RE =
  /^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,3})\d*)?)?)?$/;

/**
 * 把本地墙上时间字符串解析为 Date。
 * 格式不符 / 日历非法（如 2026-02-31）→ null（不臆测时区、不自动进位）。
 */
export function parseTrashTimestamp(value: string): Date | null {
  if (typeof value !== "string") return null;
  const match = TRASH_TIMESTAMP_RE.exec(value.trim());
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
  // 拒绝被 Date 自动进位的非法日历日期。
  if (
    date.getFullYear() !== year ||
    date.getMonth() !== month - 1 ||
    date.getDate() !== day
  ) {
    return null;
  }
  return date;
}

/**
 * 距自动清除的剩余毫秒数：`deleted + 24h - now`。
 * 无法解析 → null。
 */
export function remainingMs(deleted: string, now: Date): number | null {
  const parsed = parseTrashTimestamp(deleted);
  if (!parsed) return null;
  return parsed.getTime() + TRASH_RETENTION_MS - now.getTime();
}

/**
 * 是否已过期（剩余 <= 0）。无法解析 → false（不误删）。
 */
export function isTrashExpired(deleted: string, now: Date): boolean {
  const remaining = remainingMs(deleted, now);
  return remaining === null ? false : remaining <= 0;
}

/**
 * 剩余时长文案：
 * - 无法解析 → `时间未知`
 * - `<= 0` → `即将删除`
 * - `< 1 分钟` → `不足 1 分钟`
 * - `< 1 小时` → `剩余 N 分钟`（N 向上取整）
 * - 其余 → `剩余 H 小时` / `剩余 H 小时 M 分`
 *   （分钟向上取整；满 60 分时进位为小时，不出现「60 分」）。
 */
export function formatTrashRemaining(deleted: string, now: Date): string {
  const remaining = remainingMs(deleted, now);
  if (remaining === null) return "时间未知";
  if (remaining <= 0) return "即将删除";
  if (remaining < 60_000) return "不足 1 分钟";
  if (remaining < 3_600_000) {
    return `剩余 ${Math.ceil(remaining / 60_000)} 分钟`;
  }
  const totalMinutes = Math.ceil(remaining / 60_000);
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return minutes > 0 ? `剩余 ${hours} 小时 ${minutes} 分` : `剩余 ${hours} 小时`;
}

/** 按 `deleted` 倒序比较；无法解析（含空串）者垫底，双方都无时间返回 0 保持稳定顺序。 */
function compareByDeletedDesc(a: string, b: string): number {
  const pa = parseTrashTimestamp(a);
  const pb = parseTrashTimestamp(b);
  if (pa && pb) {
    const diff = pb.getTime() - pa.getTime();
    return diff === 0 ? 0 : diff;
  }
  if (pa) return -1;
  if (pb) return 1;
  return 0;
}

/** 可见任务：剔除已过期，按删除时间倒序（稳定）。不修改入参。 */
export function visibleTrashTasks(view: TrashView, now: Date): TrashTaskRef[] {
  return view.tasks
    .filter((task) => !isTrashExpired(task.deleted, now))
    .sort((a, b) => compareByDeletedDesc(a.deleted, b.deleted));
}

/** 可见清单：剔除已过期，按删除时间倒序（稳定）。不修改入参。 */
export function visibleTrashLists(view: TrashView, now: Date): TrashListRef[] {
  return view.lists
    .filter((list) => !isTrashExpired(list.deleted, now))
    .sort((a, b) => compareByDeletedDesc(a.deleted, b.deleted));
}

/** 去掉目录与扩展名的文件名（用于组名回退）。 */
function baseNameWithoutExt(file: string): string {
  const cleaned = file.replace(/\\/g, "/");
  const base = cleaned.slice(cleaned.lastIndexOf("/") + 1);
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(0, dot) : base;
}

/**
 * 把回收站任务按来源清单分组：
 * 1. 组顺序：先按传入 `lists` 的顺序，再追加「原清单已不存在」的组（按 `list_file` 字典序）；
 * 2. 组名：优先 `task.list_name`，其次当前清单名，最后文件名去扩展名；
 * 3. 组内按 `deleted` 倒序（缺失 / 非法垫底，稳定）；空组不产出。
 */
export function buildTrashTaskGroups(
  lists: readonly { file: string; name: string }[],
  tasks: readonly TrashTaskRef[],
): TrashTaskGroup[] {
  const byFile = new Map<string, TrashTaskRef[]>();
  for (const task of tasks) {
    const bucket = byFile.get(task.list_file);
    if (bucket) bucket.push(task);
    else byFile.set(task.list_file, [task]);
  }

  const listOrder = new Map<string, number>();
  const listNames = new Map<string, string>();
  lists.forEach((list, index) => {
    if (!listOrder.has(list.file)) listOrder.set(list.file, index);
    if (!listNames.has(list.file)) listNames.set(list.file, list.name);
  });

  const files = [...byFile.keys()];
  const known: string[] = [];
  const missing: string[] = [];
  for (const file of files) {
    if (listOrder.has(file)) known.push(file);
    else missing.push(file);
  }
  known.sort((a, b) => (listOrder.get(a) as number) - (listOrder.get(b) as number));
  missing.sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));

  const groups: TrashTaskGroup[] = [];
  for (const file of [...known, ...missing]) {
    const bucket = byFile.get(file);
    if (!bucket || bucket.length === 0) continue;
    const snapshotName = bucket.find(
      (task) => typeof task.list_name === "string" && task.list_name.trim().length > 0,
    )?.list_name;
    const listName =
      snapshotName ?? listNames.get(file) ?? baseNameWithoutExt(file);
    groups.push({
      listFile: file,
      listName,
      tasks: bucket
        .slice()
        .sort((a, b) => compareByDeletedDesc(a.deleted, b.deleted)),
    });
  }
  return groups;
}

/** 可见条目总数：可见任务数 + 可见清单数。 */
export function trashEntryCount(view: TrashView, now: Date): number {
  return visibleTrashTasks(view, now).length + visibleTrashLists(view, now).length;
}
