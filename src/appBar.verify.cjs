/*
 * 应用功能栏「顺序逻辑」纯函数验证（无依赖；复用仓库内 typescript 即时转译 src/appBar.ts）。
 *
 * 运行：`node src/appBar.verify.cjs`（在仓库根或任意目录均可，路径基于 __dirname）。
 * 覆盖：默认 / 去重 / 未知槽位保留 / 新入口(Agent)追加末尾 / merge / move / 恢复默认判据。
 * 说明：本文件不是构建产物的一部分（未被任何模块 import；tsc 不包含 .cjs）。
 */
const path = require("path");
const fs = require("fs");
const assert = require("assert");

const here = __dirname;
const ts = require("typescript");
const src = fs.readFileSync(path.join(here, "appBar.ts"), "utf8");
const js = ts.transpileModule(src, {
  compilerOptions: {
    module: ts.ModuleKind.CommonJS,
    target: ts.ScriptTarget.ES2020,
  },
}).outputText;
const mod = { exports: {} };
new Function("exports", "module", "require", js)(mod.exports, mod, require);
const ab = mod.exports;

const DEFAULT4 = ["refresh", "pin", "settings", "agent"];

let pass = 0;
function ok(name, fn) {
  fn();
  pass++;
  console.log("ok -", name);
}

// 1. 默认：null / 空 / 非法（现含 agent，末尾）
ok("null → 默认（含 agent 末尾）", () =>
  assert.deepStrictEqual(ab.resolveAppBarOrder(null), DEFAULT4));
ok("[] → 默认", () =>
  assert.deepStrictEqual(ab.resolveAppBarOrder([]), DEFAULT4));
ok("非法元素 → 默认（无有效 id）", () =>
  assert.deepStrictEqual(ab.resolveAppBarOrder([1, null, {}]), DEFAULT4));

// 2. 读取清洗
ok("sanitize：非数组 → null", () =>
  assert.strictEqual(ab.sanitizeAppBarOrder("x"), null));
ok("sanitize：空数组 → null", () =>
  assert.strictEqual(ab.sanitizeAppBarOrder([]), null));
ok("sanitize：过滤非字符串", () =>
  assert.deepStrictEqual(ab.sanitizeAppBarOrder(["a", 1, "b"]), ["a", "b"]));

// 3. 去重取首次
ok("去重（保留首次）", () =>
  assert.deepStrictEqual(ab.resolveAppBarOrder(["pin", "pin", "refresh"]), ["pin", "refresh", "settings", "agent"]));

// 4. 未知保留槽位 + 缺失已知（含 agent）追加末尾
ok("未知 id 保留槽位 + 缺失已知追加末尾", () =>
  assert.deepStrictEqual(ab.resolveAppBarOrder(["refresh", "ghost", "pin"]), ["refresh", "ghost", "pin", "settings", "agent"]));
ok("旧顺序（无 agent）→ agent 追加末尾", () =>
  assert.deepStrictEqual(ab.resolveAppBarOrder(["refresh", "pin", "settings"]), ["refresh", "pin", "settings", "agent"]));
ok("新增未配置入口追加到用户顺序末尾", () =>
  assert.deepStrictEqual(ab.resolveAppBarOrder(["settings"]), ["settings", "refresh", "pin", "agent"]));

// 5. 可见过滤
ok("visible 仅保留已知且保序", () =>
  assert.deepStrictEqual(ab.visibleAppBarIds(["ghost", "pin", "refresh"]), ["pin", "refresh"]));

// 6. merge：未知保留原槽位
ok("merge：未知 id 保持原槽位，已知按新可见顺序填入", () =>
  assert.deepStrictEqual(
    ab.mergeAppBarOrder(["refresh", "ghost", "pin", "settings", "agent"], ["agent", "refresh", "pin", "settings"]),
    ["agent", "ghost", "refresh", "pin", "settings"],
  ));
ok("merge：无未知时等价于直接重排", () =>
  assert.deepStrictEqual(
    ab.mergeAppBarOrder(DEFAULT4.slice(), ["agent", "settings", "refresh", "pin"]),
    ["agent", "settings", "refresh", "pin"],
  ));

// 7. 上移 / 下移
ok("move：上移 / 下移 / 越界原样", () => {
  assert.deepStrictEqual(ab.moveAppBarVisible(["a", "b", "c"], "b", -1), ["b", "a", "c"]);
  assert.deepStrictEqual(ab.moveAppBarVisible(["a", "b", "c"], "b", 1), ["a", "c", "b"]);
  const orig = ["a", "b", "c"];
  assert.strictEqual(ab.moveAppBarVisible(orig, "a", -1), orig);
  assert.strictEqual(ab.moveAppBarVisible(orig, "c", 1), orig);
});

// 8. 恢复默认判据：只要存在「实际保存记录」（非空有效数组）即可清除字段，含未知
ok("reset 判据：hasSavedAppBarOrder", () => {
  assert.strictEqual(ab.hasSavedAppBarOrder(null), false);
  assert.strictEqual(ab.hasSavedAppBarOrder([]), false);
  assert.strictEqual(ab.hasSavedAppBarOrder("x"), false);
  assert.strictEqual(ab.hasSavedAppBarOrder([1, {}, null]), false);
  assert.strictEqual(ab.hasSavedAppBarOrder(DEFAULT4), true);
  assert.strictEqual(ab.hasSavedAppBarOrder(["pin", "refresh", "settings"]), true);
  assert.strictEqual(ab.hasSavedAppBarOrder(["refresh", "pin", "settings", "ghost"]), true);
});

console.log("\n" + pass + " assertions passed");
