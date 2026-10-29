/*
 * 「删除回收站」纯逻辑验证（无依赖；复用仓库内 typescript 即时转译 src/trash.ts）。
 *
 * 运行：`node src/trash.verify.cjs`（在仓库根或任意目录均可，路径基于 __dirname）。
 * 覆盖：时间解析（秒 / 分 / 空格 / 裸日期 / 非法日历 / 非法字符串）、剩余时间边界、
 *       剩余文案、分组顺序与组名回退、组内倒序、空组省略、过期过滤、计数、无副作用。
 * 说明：本文件不是构建产物的一部分（未被任何模块 import；tsc 不包含 .cjs）。
 */
const path = require("path");
const fs = require("fs");
const assert = require("assert");

const here = __dirname;
const ts = require("typescript");
const src = fs.readFileSync(path.join(here, "trash.ts"), "utf8");
const js = ts.transpileModule(src, {
  compilerOptions: {
    module: ts.ModuleKind.CommonJS,
    target: ts.ScriptTarget.ES2020,
  },
}).outputText;
const mod = { exports: {} };
new Function("exports", "module", "require", js)(mod.exports, mod, require);
const m = mod.exports;

// 统计断言次数（包裹 assert 方法）。
let assertionCount = 0;
for (const key of ["strictEqual", "deepStrictEqual", "notStrictEqual", "throws"]) {
  const original = assert[key];
  assert[key] = function (...args) {
    assertionCount++;
    return original.apply(assert, args);
  };
}

let pass = 0;
function ok(name, fn) {
  fn();
  pass++;
  console.log("ok -", name);
}

const NOW = new Date(2026, 9, 8, 12, 0, 0); // 本地 2026-10-08 12:00:00
const RETENTION = 24 * 60 * 60 * 1000;

/** 造一条回收站任务。 */
function task(overrides) {
  return {
    trash_id: overrides.trash_id ?? overrides.task?.id ?? "t",
    deleted: overrides.deleted ?? "2026-10-08T10:00:00",
    list_file: overrides.list_file ?? "a.md",
    list_name: overrides.list_name ?? "",
    task: { id: overrides.task?.id ?? "id", title: overrides.task?.title ?? "标题" },
  };
}

function view(tasks, lists = []) {
  return { tasks, lists };
}

// ---------------------------------------------------------------------------
// 1. 时间解析
// ---------------------------------------------------------------------------
ok("parseTrashTimestamp：支持秒 / 分 / 空格 / 裸日期", () => {
  const withSeconds = m.parseTrashTimestamp("2026-10-08T12:34:56");
  assert.deepStrictEqual(withSeconds, new Date(2026, 9, 8, 12, 34, 56));
  const withMinutes = m.parseTrashTimestamp("2026-10-08T12:34");
  assert.deepStrictEqual(withMinutes, new Date(2026, 9, 8, 12, 34, 0));
  const withSpace = m.parseTrashTimestamp("2026-10-08 12:34:56");
  assert.deepStrictEqual(withSpace, new Date(2026, 9, 8, 12, 34, 56));
  const bareDate = m.parseTrashTimestamp("2026-10-08");
  assert.deepStrictEqual(bareDate, new Date(2026, 9, 8, 0, 0, 0));
  assert.deepStrictEqual(m.parseTrashTimestamp("  2026-10-08T01:02  "), new Date(2026, 9, 8, 1, 2, 0));
});

ok("parseTrashTimestamp：非法日历 / 非法字符串 → null", () => {
  assert.strictEqual(m.parseTrashTimestamp("2026-02-31T10:00"), null);
  assert.strictEqual(m.parseTrashTimestamp("2026-13-01"), null);
  assert.strictEqual(m.parseTrashTimestamp("2026-04-31"), null);
  assert.strictEqual(m.parseTrashTimestamp("2026-10-08T25:00"), null);
  assert.strictEqual(m.parseTrashTimestamp("2026-10-08T10:60"), null);
  assert.strictEqual(m.parseTrashTimestamp("not-a-date"), null);
  assert.strictEqual(m.parseTrashTimestamp(""), null);
  assert.strictEqual(m.parseTrashTimestamp("2026/10/08"), null);
});

// ---------------------------------------------------------------------------
// 2. 剩余时间边界
// ---------------------------------------------------------------------------
ok("remainingMs：24h 整、23h59m、1h、59m、59s、未来时间", () => {
  assert.strictEqual(m.remainingMs("2026-10-07T12:00:00", NOW), 0);
  assert.strictEqual(m.remainingMs("2026-10-08T11:59:00", NOW), 23 * 3600 * 1000 + 59 * 60 * 1000);
  assert.strictEqual(m.remainingMs("2026-10-07T13:00:00", NOW), 3600 * 1000);
  assert.strictEqual(m.remainingMs("2026-10-07T12:59:00", NOW), 59 * 60 * 1000);
  assert.strictEqual(m.remainingMs("2026-10-07T12:00:59", NOW), 59 * 1000);
  assert.strictEqual(m.remainingMs("2026-10-08T12:00:00", NOW), RETENTION);
  assert.strictEqual(m.remainingMs("2026-10-09T12:00:00", NOW), 2 * RETENTION);
  assert.strictEqual(m.remainingMs("垃圾", NOW), null);
});

ok("isTrashExpired：仅剩余 <= 0 才过期，无法解析不过期", () => {
  assert.strictEqual(m.isTrashExpired("2026-10-07T12:00:00", NOW), true);
  assert.strictEqual(m.isTrashExpired("2026-10-07T11:59:59", NOW), true);
  assert.strictEqual(m.isTrashExpired("2026-10-07T12:00:01", NOW), false);
  assert.strictEqual(m.isTrashExpired("2026-10-09T12:00:00", NOW), false);
  assert.strictEqual(m.isTrashExpired("", NOW), false);
  assert.strictEqual(m.isTrashExpired("bad", NOW), false);
});

ok("TRASH_RETENTION_MS = 24 小时", () => {
  assert.strictEqual(m.TRASH_RETENTION_MS, RETENTION);
});

// ---------------------------------------------------------------------------
// 3. 剩余文案
// ---------------------------------------------------------------------------
ok("formatTrashRemaining：未知 / 即将删除 / 不足 1 分钟", () => {
  assert.strictEqual(m.formatTrashRemaining("bad", NOW), "时间未知");
  assert.strictEqual(m.formatTrashRemaining("2026-10-07T12:00:00", NOW), "即将删除");
  assert.strictEqual(m.formatTrashRemaining("2026-10-06T12:00:00", NOW), "即将删除");
  assert.strictEqual(m.formatTrashRemaining("2026-10-07T12:00:59", NOW), "不足 1 分钟");
});

ok("formatTrashRemaining：分钟向上取整", () => {
  assert.strictEqual(m.formatTrashRemaining("2026-10-07T12:59:00", NOW), "剩余 59 分钟");
  assert.strictEqual(m.formatTrashRemaining("2026-10-07T12:58:01", NOW), "剩余 59 分钟");
  // 1 小时 + 1 秒 → 剩余 1 小时 1 分
  assert.strictEqual(m.formatTrashRemaining("2026-10-07T13:00:01", NOW), "剩余 1 小时 1 分");
});

ok("formatTrashRemaining：小时与小时+分", () => {
  assert.strictEqual(m.formatTrashRemaining("2026-10-07T13:00:00", NOW), "剩余 1 小时");
  assert.strictEqual(m.formatTrashRemaining("2026-10-08T11:30:00", NOW), "剩余 23 小时 30 分");
  assert.strictEqual(m.formatTrashRemaining("2026-10-08T12:00:00", NOW), "剩余 24 小时");
});

// ---------------------------------------------------------------------------
// 4. 可见性过滤 + 倒序
// ---------------------------------------------------------------------------
ok("visibleTrashTasks：过滤过期并稳定倒序，非法删除时间垫底", () => {
  const tasks = [
    task({ task: { id: "old" }, deleted: "2026-10-07T11:59:59" }),
    task({ task: { id: "fresh" }, deleted: "2026-10-08T10:00:00" }),
    task({ task: { id: "mid" }, deleted: "2026-10-07T12:30:00" }),
    task({ task: { id: "unknown1" }, deleted: "" }),
    task({ task: { id: "unknown2" }, deleted: "bad" }),
  ];
  const visible = m.visibleTrashTasks(view(tasks), NOW);
  assert.deepStrictEqual(visible.map((t) => t.task.id), [
    "fresh",
    "mid",
    "unknown1",
    "unknown2",
  ]);
});

ok("visibleTrashLists：过滤过期并稳定倒序", () => {
  const lists = [
    { trash_id: "1", deleted: "2026-10-07T12:00:00", file: "gone.md", name: "过期", task_count: 1 },
    { trash_id: "2", deleted: "2026-10-08T10:00:00", file: "b.md", name: "新", task_count: 2 },
    { trash_id: "3", deleted: "2026-10-07T18:00:00", file: "a.md", name: "旧", task_count: 3 },
    { trash_id: "4", deleted: "bad", file: "x.md", name: "未知", task_count: 0 },
  ];
  const visible = m.visibleTrashLists(view([], lists), NOW);
  assert.deepStrictEqual(visible.map((l) => l.file), ["b.md", "a.md", "x.md"]);
});

// ---------------------------------------------------------------------------
// 5. 分组
// ---------------------------------------------------------------------------
const LISTS = [
  { file: "a.md", name: "工作" },
  { file: "b.md", name: "生活" },
  { file: "c.md", name: "想法" },
];

ok("buildTrashTaskGroups：现有清单顺序 + 缺失清单字典序追加", () => {
  const tasks = [
    task({ task: { id: "z" }, list_file: "zz.md" }),
    task({ task: { id: "b" }, list_file: "b.md" }),
    task({ task: { id: "aa" }, list_file: "aa.md" }),
    task({ task: { id: "a" }, list_file: "a.md" }),
  ];
  const groups = m.buildTrashTaskGroups(LISTS, tasks);
  assert.deepStrictEqual(groups.map((g) => g.listFile), ["a.md", "b.md", "aa.md", "zz.md"]);
});

ok("buildTrashTaskGroups：组名优先快照名，其次当前清单名，最后文件名去扩展名", () => {
  const tasks = [
    task({ task: { id: "a" }, list_file: "a.md", list_name: "旧工作" }),
    task({ task: { id: "b" }, list_file: "b.md", list_name: "" }),
    task({ task: { id: "g" }, list_file: "dir/ghost.md", list_name: "" }),
  ];
  const groups = m.buildTrashTaskGroups(LISTS, tasks);
  const names = groups.map((g) => g.listName);
  assert.deepStrictEqual(names, ["旧工作", "生活", "ghost"]);
});

ok("buildTrashTaskGroups：组内按删除时间倒序，非法 / 缺失垫底且保持稳定", () => {
  const tasks = [
    task({ task: { id: "old" }, list_file: "a.md", deleted: "2026-10-08T08:00:00" }),
    task({ task: { id: "unknown1" }, list_file: "a.md", deleted: "" }),
    task({ task: { id: "new" }, list_file: "a.md", deleted: "2026-10-08T11:00:00" }),
    task({ task: { id: "bad" }, list_file: "a.md", deleted: "nope" }),
    task({ task: { id: "mid" }, list_file: "a.md", deleted: "2026-10-08T09:00:00" }),
    task({ task: { id: "unknown2" }, list_file: "a.md", deleted: "" }),
  ];
  const groups = m.buildTrashTaskGroups(LISTS, tasks);
  assert.deepStrictEqual(groups[0].tasks.map((t) => t.task.id), [
    "new",
    "mid",
    "old",
    "unknown1",
    "bad",
    "unknown2",
  ]);
});

ok("buildTrashTaskGroups：空组不产出，空任务 → []", () => {
  const groups = m.buildTrashTaskGroups(LISTS, [
    task({ task: { id: "b" }, list_file: "b.md" }),
  ]);
  assert.deepStrictEqual(groups.map((g) => g.listFile), ["b.md"]);
  assert.deepStrictEqual(m.buildTrashTaskGroups(LISTS, []), []);
});

// ---------------------------------------------------------------------------
// 6. 计数 + 无副作用
// ---------------------------------------------------------------------------
ok("trashEntryCount：可见任务 + 可见清单", () => {
  const data = view(
    [
      task({ task: { id: "fresh" }, deleted: "2026-10-08T10:00:00" }),
      task({ task: { id: "old" }, deleted: "2026-10-07T11:00:00" }),
      task({ task: { id: "unknown" }, deleted: "bad" }),
    ],
    [
      { trash_id: "1", deleted: "2026-10-08T09:00:00", file: "b.md", name: "生活", task_count: 1 },
      { trash_id: "2", deleted: "2026-10-07T09:00:00", file: "a.md", name: "工作", task_count: 1 },
    ],
  );
  assert.strictEqual(m.trashEntryCount(data, NOW), 3);
});

ok("无副作用：可见性与分组均不修改入参", () => {
  const tasks = [
    task({ task: { id: "old" }, list_file: "a.md", deleted: "2026-10-07T08:00:00" }),
    task({ task: { id: "new" }, list_file: "a.md", deleted: "2026-10-08T11:00:00" }),
    task({ task: { id: "expired" }, list_file: "b.md", deleted: "2026-10-06T08:00:00" }),
  ];
  const lists = [
    { trash_id: "1", deleted: "2026-10-08T09:00:00", file: "b.md", name: "生活", task_count: 1 },
    { trash_id: "2", deleted: "2026-10-06T09:00:00", file: "a.md", name: "工作", task_count: 2 },
  ];
  const data = view(tasks, lists);
  const before = JSON.stringify(data);
  const beforeOrder = tasks.map((t) => t.task.id);

  m.visibleTrashTasks(data, NOW);
  m.visibleTrashLists(data, NOW);
  m.buildTrashTaskGroups(LISTS, tasks);
  m.trashEntryCount(data, NOW);

  assert.strictEqual(JSON.stringify(data), before);
  assert.deepStrictEqual(tasks.map((t) => t.task.id), beforeOrder);
  assert.strictEqual(tasks[0].task.id, "old");
  assert.strictEqual(data.tasks, tasks);
});

console.log("\n" + pass + " cases passed, " + assertionCount + " assertions passed");
