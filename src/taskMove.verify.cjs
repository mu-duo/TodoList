/*
 * 任务跨清单转移「纯逻辑」验证（无依赖；复用仓库内 typescript 即时转译 src/taskMove.ts）。
 *
 * 运行：`node src/taskMove.verify.cjs`（在仓库根或任意目录均可，路径基于 __dirname）。
 * 覆盖：右键目标清单计算（排除源清单 / 保序 / 无目标）、指针命中测试（边界 / 越界）、
 *       拖拽落点解析（源清单不有效 / 空指针放行既有排序）、矩形命中（边界 / 容差 / 空指针）。
 * 说明：本文件不是构建产物的一部分（未被任何模块 import；tsc 不包含 .cjs）。
 */
const path = require("path");
const fs = require("fs");
const assert = require("assert");

const here = __dirname;
const ts = require("typescript");
const src = fs.readFileSync(path.join(here, "taskMove.ts"), "utf8");
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

// 1. 右键目标清单：排除源清单、保持侧栏顺序
ok("moveTargetLists：排除源清单并保持顺序", () =>
  assert.deepStrictEqual(m.moveTargetLists(LISTS, "b.md"), [
    { file: "a.md", name: "工作" },
    { file: "c.md", name: "想法" },
  ]));
ok("moveTargetLists：排除首个清单", () =>
  assert.deepStrictEqual(m.moveTargetLists(LISTS, "a.md"), [
    { file: "b.md", name: "生活" },
    { file: "c.md", name: "想法" },
  ]));
ok("moveTargetLists：仅一个清单（即源）→ 无目标", () =>
  assert.deepStrictEqual(
    m.moveTargetLists([{ file: "a.md", name: "工作" }], "a.md"),
    [],
  ));
ok("moveTargetLists：空清单 → 无目标", () =>
  assert.deepStrictEqual(m.moveTargetLists([], "a.md"), []));
ok("moveTargetLists：未知源清单 → 全部保留", () =>
  assert.strictEqual(m.moveTargetLists(LISTS, "ghost.md").length, 3));

// 2. 指针命中测试
const RECTS = [
  { file: "a.md", left: 0, top: 0, right: 100, bottom: 30 },
  { file: "b.md", left: 0, top: 32, right: 100, bottom: 62 },
];
ok("hitTestListFile：命中首项", () =>
  assert.strictEqual(m.hitTestListFile({ x: 50, y: 15 }, RECTS), "a.md"));
ok("hitTestListFile：命中次项", () =>
  assert.strictEqual(m.hitTestListFile({ x: 50, y: 40 }, RECTS), "b.md"));
ok("hitTestListFile：边界含端点", () =>
  assert.strictEqual(m.hitTestListFile({ x: 0, y: 0 }, RECTS), "a.md"));
ok("hitTestListFile：未命中返回 null", () =>
  assert.strictEqual(m.hitTestListFile({ x: 500, y: 400 }, RECTS), null));
ok("hitTestListFile：列表为空返回 null", () =>
  assert.strictEqual(m.hitTestListFile({ x: 1, y: 1 }, []), null));

// 3. 拖拽落点解析
ok("resolveDropTarget：命中非源清单 → 有效", () =>
  assert.strictEqual(
    m.resolveDropTarget({ x: 50, y: 40 }, RECTS, "a.md"),
    "b.md",
  ));
ok("resolveDropTarget：命中源清单 → 无效", () =>
  assert.strictEqual(
    m.resolveDropTarget({ x: 50, y: 15 }, RECTS, "a.md"),
    null,
  ));
ok("resolveDropTarget：未命中清单项 → 无效（保持既有排序）", () =>
  assert.strictEqual(
    m.resolveDropTarget({ x: 500, y: 400 }, RECTS, "a.md"),
    null,
  ));
ok("resolveDropTarget：键盘拖拽（指针为空）→ 无效", () =>
  assert.strictEqual(m.resolveDropTarget(null, RECTS, "a.md"), null));

// 4. 矩形命中：含边界与容差（源任务滚动区 / 「已完成」落点共用）
const BOX = { left: 10, top: 20, right: 110, bottom: 60 };
ok("isPointInRect：内部 → true", () =>
  assert.strictEqual(m.isPointInRect({ x: 50, y: 40 }, BOX), true));
ok("isPointInRect：四角边界含端点 → true", () => {
  assert.strictEqual(m.isPointInRect({ x: 10, y: 20 }, BOX), true);
  assert.strictEqual(m.isPointInRect({ x: 110, y: 60 }, BOX), true);
  assert.strictEqual(m.isPointInRect({ x: 10, y: 60 }, BOX), true);
  assert.strictEqual(m.isPointInRect({ x: 110, y: 20 }, BOX), true);
});
ok("isPointInRect：外侧 1px → false", () => {
  assert.strictEqual(m.isPointInRect({ x: 9, y: 40 }, BOX), false);
  assert.strictEqual(m.isPointInRect({ x: 111, y: 40 }, BOX), false);
  assert.strictEqual(m.isPointInRect({ x: 50, y: 19 }, BOX), false);
  assert.strictEqual(m.isPointInRect({ x: 50, y: 61 }, BOX), false);
});
ok("isPointInRect：4px 容差内（含边界）→ true", () => {
  assert.strictEqual(m.isPointInRect({ x: 6, y: 40 }, BOX, 4), true);
  assert.strictEqual(m.isPointInRect({ x: 114, y: 40 }, BOX, 4), true);
  assert.strictEqual(m.isPointInRect({ x: 50, y: 16 }, BOX, 4), true);
  assert.strictEqual(m.isPointInRect({ x: 50, y: 64 }, BOX, 4), true);
});
ok("isPointInRect：容差外 1px → false", () => {
  assert.strictEqual(m.isPointInRect({ x: 5, y: 40 }, BOX, 4), false);
  assert.strictEqual(m.isPointInRect({ x: 115, y: 40 }, BOX, 4), false);
  assert.strictEqual(m.isPointInRect({ x: 50, y: 15 }, BOX, 4), false);
  assert.strictEqual(m.isPointInRect({ x: 50, y: 65 }, BOX, 4), false);
});
ok("isPointInRect：负容差向内收缩 → false", () =>
  assert.strictEqual(m.isPointInRect({ x: 10, y: 20 }, BOX, -1), false));
ok("isPointInRect：指针为空（键盘拖拽）→ false", () =>
  assert.strictEqual(m.isPointInRect(null, BOX, 4), false));

console.log("\n" + pass + " assertions passed");
