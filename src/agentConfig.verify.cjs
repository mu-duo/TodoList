/*
 * Agent 多 Provider 配置纯逻辑验证（无依赖；复用仓库内 typescript 即时转译 src/agent.ts）。
 * 运行：`node src/agentConfig.verify.cjs`（在仓库根或任意目录均可，路径基于 __dirname）。
 * 覆盖：协议归一化 normalizeProtocol（容错旧式写法 / 未知回退）、protocolLabel 中文标签、
 *       默认常量口径（与后端 presets / config 默认一致）。
 * 说明：本文件非构建产物（未被任何模块 import；tsc 不包含 .cjs）。
 *       src/agent.ts 顶部 import 了 @tauri-apps/api（本机桥）；这里用桩 require 隔离，仅取纯函数，
 *       不触网、不调用 invoke（模块顶层无副作用）。
 */
const path = require("path");
const fs = require("fs");
const assert = require("assert");

const here = __dirname;
const ts = require("typescript");
const src = fs.readFileSync(path.join(here, "agent.ts"), "utf8");
const js = ts.transpileModule(src, {
  compilerOptions: {
    module: ts.ModuleKind.CommonJS,
    target: ts.ScriptTarget.ES2020,
  },
}).outputText;

// 桩 require：仅屏蔽 Tauri 本机桥（不执行任何 invoke）；其余走真实 require。
function stubRequire(name) {
  if (name === "@tauri-apps/api/core") return { invoke: () => {} };
  if (name === "@tauri-apps/api/event") return { listen: () => {} };
  return require(name);
}

const mod = { exports: {} };
new Function("exports", "module", "require", js)(mod.exports, mod, stubRequire);
const a = mod.exports;

let pass = 0;
function ok(name, fn) {
  fn();
  pass++;
  console.log("ok -", name);
}

ok("normalizeProtocol：标准口径直通", () => {
  assert.strictEqual(a.normalizeProtocol("anthropic"), "anthropic");
  assert.strictEqual(a.normalizeProtocol("openai"), "openai");
});

ok("normalizeProtocol：大小写 / 空白容错", () => {
  assert.strictEqual(a.normalizeProtocol("OpenAI"), "openai");
  assert.strictEqual(a.normalizeProtocol("  OpenAI  "), "openai");
  assert.strictEqual(a.normalizeProtocol("ANTHROPIC"), "anthropic");
});

ok("normalizeProtocol：旧式兼容写法归一到 openai / anthropic", () => {
  assert.strictEqual(a.normalizeProtocol("openai_compat"), "openai");
  assert.strictEqual(a.normalizeProtocol("OpenAiCompat"), "openai");
  assert.strictEqual(a.normalizeProtocol("anthropic_compat"), "anthropic");
  assert.strictEqual(a.normalizeProtocol("AnthropicCompat"), "anthropic");
});

ok("normalizeProtocol：未知 / 空 / 缺失回退 anthropic（与后端默认一致）", () => {
  assert.strictEqual(a.normalizeProtocol(""), "anthropic");
  assert.strictEqual(a.normalizeProtocol("   "), "anthropic");
  assert.strictEqual(a.normalizeProtocol(null), "anthropic");
  assert.strictEqual(a.normalizeProtocol(undefined), "anthropic");
  assert.strictEqual(a.normalizeProtocol("weird"), "anthropic");
  assert.strictEqual(a.normalizeProtocol(123), "anthropic");
});

ok("protocolLabel：中文展示名（经归一化）", () => {
  assert.strictEqual(a.protocolLabel("openai"), "OpenAI 兼容");
  assert.strictEqual(a.protocolLabel("OpenAiCompat"), "OpenAI 兼容");
  assert.strictEqual(a.protocolLabel("anthropic"), "Anthropic 兼容");
  assert.strictEqual(a.protocolLabel("anthropic_compat"), "Anthropic 兼容");
  assert.strictEqual(a.protocolLabel("weird"), "Anthropic 兼容");
  assert.strictEqual(a.protocolLabel(""), "Anthropic 兼容");
});

ok("默认常量：与后端默认 provider / model 口径一致", () => {
  assert.strictEqual(a.DEFAULT_AGENT_PROVIDER, "deepseek");
  assert.strictEqual(a.DEFAULT_AGENT_MODEL, "deepseek-flash");
  assert.strictEqual(a.DEFAULT_AGENT_MAX_USES, 5);
});

console.log("\n" + pass + " assertions passed");
