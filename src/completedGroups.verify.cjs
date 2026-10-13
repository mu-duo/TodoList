/*
 * 「已完成」分组 / 折叠「纯逻辑」验证（无依赖；复用仓库内 typescript 即时转译 src/completedGroups.ts）。
 *
 * 运行：`node src/completedGroups.verify.cjs`（在仓库根或任意目录均可，路径基于 __dirname）。
 * 覆盖：组顺序（跟随清单）、空组过滤、单来源、组内 completed 倒序、缺失回退文件顺序（稳定）、
 *       归属未知清单的条目丢弃、折叠集合解析清洗、失效键过滤、切换计算（空集合 → null）、集合等价。
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
function entry(listFile, id, completed) {
  return { listFile, listName: LISTS.find((l) => l.file === listFile)?.name, task: { id, completed } };
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

console.log("\n" + pass + " assertions passed");
