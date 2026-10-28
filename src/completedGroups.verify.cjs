/*
 * 「已完成」分组 / 折叠「纯逻辑」验证（无依赖；复用仓库内 typescript 即时转译 src/completedGroups.ts）。
 *
 * 运行：`node src/completedGroups.verify.cjs`（在仓库根或任意目录均可，路径基于 __dirname）。
 * 覆盖：组顺序（跟随清单）、空组过滤、单来源、组内 completed 倒序、缺失回退文件顺序（稳定）、
 *       归属未知清单的条目丢弃、折叠集合解析清洗、失效键过滤、切换计算（空集合 → null）、集合等价；
 *       时间档位边界（今天 / 昨天 / 第 7 天 / 第 8 天 / 未来）、缺失 / 非法时间戳、归档可见性与沉底、
 *       空档位 / 空组省略、计数（组头与总数）。
 * 说明：本文件不是构建产物的一部分（未被任何模块 import；tsc 不包含 .cjs）。
 */
const path = require("path");
const fs = require("fs");
const assert = require("assert");

const here = __dirname;
const ts = require("typescript");
const src = fs.readFileSync(path.join(here, "completedGroups.ts"), "utf8");
const js = ts.transpileModule(src, {
  compilerOptions: {
    module: ts.ModuleKind.CommonJS,
    target: ts.ScriptTarget.ES2020,
  },
}).outputText;
const mod = { exports: {} };
new Function("exports", "module", "require", js)(mod.exports, mod, require);
const m = mod.exports;

let pass = 0;
function ok(name, fn) {
  fn();
  pass++;
  console.log("ok -", name);
}

const LISTS = [
  { file: "a.md", name: "工作" },
  { file: "b.md", name: "生活" },
  { file: "c.md", name: "想法" },
];

/** 造一条已完成条目。 */
function entry(listFile, id, completed, archived) {
  return {
    listFile,
    listName: LISTS.find((l) => l.file === listFile)?.name,
    task: { id, completed, archived },
  };
}

// 1. 组顺序 = 清单顺序（即便条目给的是打乱顺序）
ok("组顺序跟随清单，且组内 id 正确归类", () => {
  const tasks = [
    entry("c.md", "c1", "2026-01-01"),
    entry("a.md", "a1", "2026-01-01"),
    entry("b.md", "b1", "2026-01-01"),
  ];
  const groups = m.buildCompletedGroups(LISTS, tasks);
  assert.deepStrictEqual(groups.map((g) => g.file), ["a.md", "b.md", "c.md"]);
  assert.deepStrictEqual(groups.map((g) => g.name), ["工作", "生活", "想法"]);
  assert.deepStrictEqual(groups.map((g) => g.tasks.map((e) => e.task.id)), [["a1"], ["b1"], ["c1"]]);
});

// 2. 空组过滤（仅中间清单有已完成）
ok("空组不显示（仅保留有已完成条目的清单）", () => {
  const groups = m.buildCompletedGroups(LISTS, [entry("b.md", "b1", "2026-02-01")]);
  assert.deepStrictEqual(groups.map((g) => g.file), ["b.md"]);
  assert.strictEqual(groups[0].tasks.length, 1);
});

// 3. 仅一个来源也照常成形（不因单组而省略组头数据）
ok("仅一个来源：仍产出单个分组", () => {
  const groups = m.buildCompletedGroups(LISTS, [entry("a.md", "a1", null)]);
  assert.strictEqual(groups.length, 1);
  assert.strictEqual(groups[0].file, "a.md");
});

// 4. 组内 completed 倒序（字符串比较）
ok("组内按 completed 倒序", () => {
  const tasks = [
    entry("a.md", "old", "2026-01-01"),
    entry("a.md", "new", "2026-03-01"),
    entry("a.md", "mid", "2026-02-01"),
  ];
  const groups = m.buildCompletedGroups(LISTS, tasks);
  assert.deepStrictEqual(groups[0].tasks.map((e) => e.task.id), ["new", "mid", "old"]);
});

// 5. 缺失 completed：回退文件内顺序（稳定），并排在有时刻者之后
ok("缺失 completed 回退文件顺序且排在末尾", () => {
  const tasks = [
    entry("a.md", "miss1", null),
    entry("a.md", "dated-old", "2026-01-01"),
    entry("a.md", "miss2", undefined),
    entry("a.md", "dated-new", "2026-05-01"),
  ];
  const groups = m.buildCompletedGroups(LISTS, tasks);
  assert.deepStrictEqual(
    groups[0].tasks.map((e) => e.task.id),
    ["dated-new", "dated-old", "miss1", "miss2"],
  );
});

// 6. 全部缺失时保持文件顺序
ok("全部缺失 completed 时保持文件顺序", () => {
  const tasks = [entry("a.md", "x", null), entry("a.md", "y", null), entry("a.md", "z", null)];
  const groups = m.buildCompletedGroups(LISTS, tasks);
  assert.deepStrictEqual(groups[0].tasks.map((e) => e.task.id), ["x", "y", "z"]);
});

// 7. 不在任何清单内的条目被丢弃（不产出幽灵组）
ok("归属未知清单的条目被丢弃", () => {
  const groups = m.buildCompletedGroups(LISTS, [
    entry("ghost.md", "g1", "2026-01-01"),
    entry("a.md", "a1", "2026-01-01"),
  ]);
  assert.deepStrictEqual(groups.map((g) => g.file), ["a.md"]);
});

// 8. 无条目 → 无分组
ok("无已完成条目 → 无分组", () =>
  assert.deepStrictEqual(m.buildCompletedGroups(LISTS, []), []));

// 9. 折叠集合解析清洗
ok("sanitizeCompletedCollapsed：非数组 → null", () => {
  assert.strictEqual(m.sanitizeCompletedCollapsed(undefined), null);
  assert.strictEqual(m.sanitizeCompletedCollapsed(null), null);
  assert.strictEqual(m.sanitizeCompletedCollapsed("a.md"), null);
});
ok("sanitizeCompletedCollapsed：空 / 空白 / 去重后为空 → null", () => {
  assert.strictEqual(m.sanitizeCompletedCollapsed([]), null);
  assert.strictEqual(m.sanitizeCompletedCollapsed(["", "   "]), null);
  assert.strictEqual(m.sanitizeCompletedCollapsed([1, {}, null]), null);
});
ok("sanitizeCompletedCollapsed：过滤非法项、去重、保留原值", () => {
  assert.deepStrictEqual(
    m.sanitizeCompletedCollapsed(["a.md", 1, "a.md", "  ", "b.md"]),
    ["a.md", "b.md"],
  );
});

// 10. 失效键过滤（重命名 / 删除后的旧键静默忽略）
ok("activeCollapsedFiles：仅保留仍存在的清单文件", () => {
  assert.deepStrictEqual(
    m.activeCollapsedFiles(["a.md", "gone.md", "b.md"], LISTS),
    ["a.md", "b.md"],
  );
  assert.deepStrictEqual(m.activeCollapsedFiles(["gone.md"], LISTS), []);
});

// 11. 切换计算：空集合 → null（恢复默认全展开）
ok("nextCollapsed：加入键", () => {
  assert.deepStrictEqual(m.nextCollapsed([], "a.md", true), ["a.md"]);
  assert.deepStrictEqual(m.nextCollapsed(["a.md"], "b.md", true), ["a.md", "b.md"]);
});
ok("nextCollapsed：移出键，空集合返回 null", () => {
  assert.deepStrictEqual(m.nextCollapsed(["a.md", "b.md"], "a.md", false), ["b.md"]);
  assert.strictEqual(m.nextCollapsed(["a.md"], "a.md", false), null);
});
ok("nextCollapsed：已存在的键保留原位（不重排）", () => {
  assert.deepStrictEqual(m.nextCollapsed(["b.md", "a.md"], "a.md", true), ["b.md", "a.md"]);
});

// 12. 集合等价（与顺序无关）
ok("sameCollapsed：忽略顺序，长度不同即不等", () => {
  assert.strictEqual(m.sameCollapsed(["a.md", "b.md"], ["b.md", "a.md"]), true);
  assert.strictEqual(m.sameCollapsed(["a.md"], ["a.md", "b.md"]), false);
  assert.strictEqual(m.sameCollapsed(["a.md"], ["b.md"]), false);
  assert.strictEqual(m.sameCollapsed([], []), true);
});

// ---------------------------------------------------------------------------
// FR-APP-36：时间分桶 + 归档
// ---------------------------------------------------------------------------

const NOW = new Date(2026, 9, 8, 20, 0, 0); // 本地 2026-10-08 20:00

// 13. 时间档位边界：今天 / 昨天 / 第 7 天 / 第 8 天
ok("timeBucketOf：今天、昨天、第 7 天、第 8 天边界", () => {
  assert.strictEqual(m.timeBucketOf("2026-10-08T00:00", NOW), "today");
  assert.strictEqual(m.timeBucketOf("2026-10-08 23:59:59", NOW), "today");
  assert.strictEqual(m.timeBucketOf("2026-10-07T23:59", NOW), "last7");
  assert.strictEqual(m.timeBucketOf("2026-10-01T00:00", NOW), "last7");
  assert.strictEqual(m.timeBucketOf("2026-09-30T23:59", NOW), "older");
});

// 14. 未来时间戳归「今天」（不产生未来档位）；裸日期可解析
ok("timeBucketOf：未来时间戳归今天，裸日期按本地零点解析", () => {
  assert.strictEqual(m.timeBucketOf("2026-10-09T09:00", NOW), "today");
  assert.strictEqual(m.timeBucketOf("2026-12-31", NOW), "today");
  assert.strictEqual(m.timeBucketOf("2026-10-08", NOW), "today");
});

// 15. 缺失 / 无法解析 → 更早
ok("timeBucketOf：缺失或无法解析归更早", () => {
  assert.strictEqual(m.timeBucketOf(undefined, NOW), "older");
  assert.strictEqual(m.timeBucketOf(null, NOW), "older");
  assert.strictEqual(m.timeBucketOf("", NOW), "older");
  assert.strictEqual(m.timeBucketOf("not-a-date", NOW), "older");
  assert.strictEqual(m.timeBucketOf("2026-02-31T10:00", NOW), "older");
  assert.strictEqual(m.timeBucketOf("2026-13-01", NOW), "older");
});

// 16. 固定档位顺序与中文标签
ok("档位顺序与标签固定：今天 / 七天内 / 更早", () => {
  assert.deepStrictEqual(m.COMPLETED_BUCKETS, ["today", "last7", "older"]);
  assert.deepStrictEqual(m.COMPLETED_BUCKET_LABELS, {
    today: "今天",
    last7: "七天内",
    older: "更早",
  });
});

// 17. 关闭「显示归档」：归档项被剔除、计数不含、空档位不产出
ok("bucketCompletedGroups：关闭显示归档时剔除归档项且空档位隐藏", () => {
  const groups = m.buildCompletedGroups(LISTS, [
    entry("a.md", "a-today", "2026-10-08T09:00"),
    entry("a.md", "a-old", "2026-09-01T09:00"),
    entry("a.md", "a-arch", "2026-10-08T10:00", "2026-10-08T11:00"),
  ]);
  const bucketed = m.bucketCompletedGroups(groups, false, NOW);
  assert.strictEqual(bucketed.length, 1);
  assert.strictEqual(bucketed[0].listFile, "a.md");
  assert.strictEqual(bucketed[0].listName, "工作");
  assert.deepStrictEqual(bucketed[0].buckets.map((b) => b.bucket), ["today", "older"]);
  assert.deepStrictEqual(bucketed[0].buckets.map((b) => b.label), ["今天", "更早"]);
  assert.deepStrictEqual(
    bucketed[0].buckets[0].tasks.map((e) => e.task.id),
    ["a-today"],
  );
  assert.strictEqual(bucketed[0].count, 2);
  assert.strictEqual(m.totalVisibleCompleted(bucketed), 2);
});

// 18. 开启「显示归档」：归档项留在原档位、沉到末尾，组内各自 completed 倒序
ok("bucketCompletedGroups：开启后归档项沉底且计数包含", () => {
  const groups = m.buildCompletedGroups(LISTS, [
    entry("a.md", "a-today-new", "2026-10-08T12:00"),
    entry("a.md", "a-today-old", "2026-10-08T09:00"),
    entry("a.md", "a-arch-new", "2026-10-08T13:00", "2026-10-08T14:00"),
    entry("a.md", "a-arch-old", "2026-10-08T07:00", "2026-10-08T08:00"),
  ]);
  const bucketed = m.bucketCompletedGroups(groups, true, NOW);
  assert.deepStrictEqual(
    bucketed[0].buckets[0].tasks.map((e) => e.task.id),
    ["a-today-new", "a-today-old", "a-arch-new", "a-arch-old"],
  );
  assert.strictEqual(bucketed[0].count, 4);
  assert.strictEqual(m.totalVisibleCompleted(bucketed), 4);
});

// 19. 清单全部归档且关闭显示 → 整组不渲染
ok("bucketCompletedGroups：全部归档且隐藏时整组省略", () => {
  const groups = m.buildCompletedGroups(LISTS, [
    entry("b.md", "b-arch", "2026-10-08T10:00", "2026-10-08T11:00"),
  ]);
  assert.deepStrictEqual(m.bucketCompletedGroups(groups, false, NOW), []);
  const shown = m.bucketCompletedGroups(groups, true, NOW);
  assert.strictEqual(shown.length, 1);
  assert.deepStrictEqual(shown[0].buckets.map((b) => b.bucket), ["today"]);
  assert.strictEqual(m.totalVisibleCompleted(shown), 1);
});

// 20. 缺失 completed 归「更早」且排在有时刻者之后
ok("bucketCompletedGroups：缺失 completed 归更早并垫底", () => {
  const groups = m.buildCompletedGroups(LISTS, [
    entry("c.md", "c-miss", null),
    entry("c.md", "c-old", "2026-01-01T08:00"),
  ]);
  const bucketed = m.bucketCompletedGroups(groups, false, NOW);
  assert.deepStrictEqual(bucketed[0].buckets.map((b) => b.bucket), ["older"]);
  assert.deepStrictEqual(
    bucketed[0].buckets[0].tasks.map((e) => e.task.id),
    ["c-old", "c-miss"],
  );
});

// 21. 多清单计数与顺序：跟随清单顺序，总数 = 各组可见项之和
ok("bucketCompletedGroups：多组顺序与总数正确", () => {
  const groups = m.buildCompletedGroups(LISTS, [
    entry("c.md", "c1", "2026-10-08T09:00"),
    entry("a.md", "a1", "2026-10-08T09:00"),
    entry("a.md", "a2", "2026-10-07T09:00"),
    entry("b.md", "b1", "2026-08-01T09:00"),
  ]);
  const bucketed = m.bucketCompletedGroups(groups, false, NOW);
  assert.deepStrictEqual(bucketed.map((g) => g.listFile), ["a.md", "b.md", "c.md"]);
  assert.deepStrictEqual(bucketed.map((g) => g.count), [2, 1, 1]);
  assert.deepStrictEqual(
    bucketed[0].buckets.map((b) => b.bucket),
    ["today", "last7"],
  );
  assert.strictEqual(m.totalVisibleCompleted(bucketed), 4);
});

// 22. visibleTasksForGroup：独立可用，未归档在前、归档在后
ok("visibleTasksForGroup：归档沉底，隐藏时仅未归档", () => {
  const tasks = [
    entry("a.md", "arch", "2026-10-08T12:00", "2026-10-08T12:30"),
    entry("a.md", "plain-old", "2026-10-08T08:00"),
    entry("a.md", "plain-new", "2026-10-08T11:00"),
  ];
  assert.deepStrictEqual(
    m.visibleTasksForGroup(tasks, true).map((e) => e.task.id),
    ["plain-new", "plain-old", "arch"],
  );
  assert.deepStrictEqual(
    m.visibleTasksForGroup(tasks, false).map((e) => e.task.id),
    ["plain-new", "plain-old"],
  );
});

console.log("\n" + pass + " assertions passed");
