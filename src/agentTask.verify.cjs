/*
 * Agent 任务 UI 纯逻辑验证（无依赖；复用仓库内 typescript 即时转译 src/agentTask.ts）。
 * 运行：`node src/agentTask.verify.cjs`
 * 覆盖：状态→色调/文案、error_class 归类、menu 禁用规则、运行排序/合并/分页、行尾图标可见性、时间格式。
 * 说明：本文件非构建产物（未被任何模块 import；tsc 不包含 .cjs）。
 */
const path = require("path");
const fs = require("fs");
const assert = require("assert");

const here = __dirname;
const ts = require("typescript");
const src = fs.readFileSync(path.join(here, "agentTask.ts"), "utf8");
const js = ts.transpileModule(src, {
  compilerOptions: {
    module: ts.ModuleKind.CommonJS,
    target: ts.ScriptTarget.ES2020,
  },
}).outputText;
const mod = { exports: {} };
new Function("exports", "module", "require", js)(mod.exports, mod, require);
const t = mod.exports;

let pass = 0;
function ok(name, fn) {
  fn();
  pass++;
  console.log("ok -", name);
}

ok("agentStateMeta：状态 → 色调/中文标签", () => {
  assert.deepStrictEqual(t.agentStateMeta("running"), { tone: "running", label: "执行中" });
  assert.deepStrictEqual(t.agentStateMeta("succeeded"), { tone: "success", label: "已完成" });
  assert.deepStrictEqual(t.agentStateMeta("failed"), { tone: "danger", label: "失败" });
  assert.deepStrictEqual(t.agentStateMeta("stopped"), { tone: "neutral", label: "已停止" });
  assert.deepStrictEqual(t.agentStateMeta("interrupted"), { tone: "neutral", label: "已中断" });
  assert.deepStrictEqual(t.agentStateMeta("pending"), { tone: "neutral", label: "待执行" });
  assert.deepStrictEqual(t.agentStateMeta(undefined), { tone: "neutral", label: "待执行" });
  assert.deepStrictEqual(t.agentStateMeta("weird"), { tone: "neutral", label: "待执行" });
});

ok("agentErrorClassLabel：后端真实取值 → 中文；未知原样保留", () => {
  assert.strictEqual(t.agentErrorClassLabel("MissingCredential"), "未配置密钥");
  assert.strictEqual(t.agentErrorClassLabel("HttpAuth"), "鉴权失败");
  assert.strictEqual(t.agentErrorClassLabel("HttpStatus"), "服务端错误");
  assert.strictEqual(t.agentErrorClassLabel("Transport"), "网络错误");
  assert.strictEqual(t.agentErrorClassLabel("Redirect"), "响应被重定向");
  assert.strictEqual(t.agentErrorClassLabel("Parse"), "响应解析失败");
  assert.strictEqual(t.agentErrorClassLabel("NoResultBlock"), "未返回搜索结果");
  assert.strictEqual(t.agentErrorClassLabel("ProviderError"), "上游返回错误");
  assert.strictEqual(t.agentErrorClassLabel("Aborted"), "已中止");
  assert.strictEqual(t.agentErrorClassLabel("UnsafePath"), "路径不安全");
  assert.strictEqual(t.agentErrorClassLabel("ResultRoot"), "结果目录不可用");
  assert.strictEqual(t.agentErrorClassLabel("StepLimit"), "达到步数上限");
  assert.strictEqual(t.agentErrorClassLabel("timeout"), "运行超时");
  assert.strictEqual(t.agentErrorClassLabel(null), "");
  assert.strictEqual(t.agentErrorClassLabel(undefined), "");
  assert.strictEqual(t.agentErrorClassLabel(""), "");
  // 未知值原样返回（兜底，不吞掉）
  assert.strictEqual(t.agentErrorClassLabel("SomeFutureClass"), "SomeFutureClass");
});

ok("agentStepsText：缺失不显示；正常／达到上限", () => {
  assert.strictEqual(t.agentStepsText(undefined, false), "");
  assert.strictEqual(t.agentStepsText(null, false), "");
  assert.strictEqual(t.agentStepsText(0, false), "");
  assert.strictEqual(t.agentStepsText(12, false), "步数 12");
  // 达到上限：分母取实测步数（不硬编码上限值）
  assert.strictEqual(t.agentStepsText(40, true), "步数 40/40（达到上限）");
  assert.strictEqual(t.agentStepsText(7, true), "步数 7/7（达到上限）");
  // 未标记达到上限则不显示分母
  assert.strictEqual(t.agentStepsText(40, false), "步数 40");
});

ok("agentStopReasonLabel：后端 STOP_* → 中文；未知原样", () => {
  assert.strictEqual(t.agentStopReasonLabel("manual"), "手动停止");
  assert.strictEqual(t.agentStopReasonLabel("disabled"), "任务已停用");
  assert.strictEqual(t.agentStopReasonLabel("completed"), "任务已完成");
  assert.strictEqual(t.agentStopReasonLabel("deleted"), "任务已删除");
  assert.strictEqual(t.agentStopReasonLabel(""), "");
  assert.strictEqual(t.agentStopReasonLabel(null), "");
  assert.strictEqual(t.agentStopReasonLabel(undefined), "");
  assert.strictEqual(t.agentStopReasonLabel("future"), "future");
});

ok("agentStateTitle：含 error_class / stopReason / 无", () => {
  assert.strictEqual(t.agentStateTitle("failed", "timeout"), "Agent：失败（运行超时）");
  assert.strictEqual(t.agentStateTitle("failed", "StepLimit"), "Agent：失败（达到步数上限）");
  assert.strictEqual(t.agentStateTitle("stopped", null, "manual"), "Agent：已停止（手动停止）");
  assert.strictEqual(t.agentStateTitle("running"), "Agent：执行中");
  assert.strictEqual(t.agentStateTitle(undefined), "Agent：待执行");
});

ok("isAgentActive：running/pending 为真", () => {
  assert.strictEqual(t.isAgentActive("running"), true);
  assert.strictEqual(t.isAgentActive("pending"), true);
  assert.strictEqual(t.isAgentActive("succeeded"), false);
  assert.strictEqual(t.isAgentActive(undefined), false);
});

ok("rowBadgeState：停用不显示；live>latest>pending", () => {
  assert.strictEqual(t.rowBadgeState(false, "running", "succeeded"), null);
  assert.strictEqual(t.rowBadgeState(true, "running", "succeeded"), "running");
  assert.strictEqual(t.rowBadgeState(true, null, "succeeded"), "succeeded");
  assert.strictEqual(t.rowBadgeState(true, null, null), "pending");
  assert.strictEqual(t.rowBadgeState(true, undefined, undefined), "pending");
});

ok("agentMenuModel：标签与禁用规则", () => {
  const off = t.agentMenuModel({ agentEnabled: false, hasRun: false, isActive: false, latestResultDir: null });
  assert.strictEqual(off.toggleLabel, "启用协助");
  assert.strictEqual(off.startLabel, "立即执行");
  assert.strictEqual(off.startEnabled, false); // 未启用
  assert.strictEqual(off.stopEnabled, false);
  assert.strictEqual(off.viewResultEnabled, false);
  assert.strictEqual(off.addFileEnabled, true);
  assert.strictEqual(off.addDirEnabled, true);

  const onIdle = t.agentMenuModel({ agentEnabled: true, hasRun: true, isActive: false, latestResultDir: "D:/r/x" });
  assert.strictEqual(onIdle.toggleLabel, "停用协助");
  assert.strictEqual(onIdle.startLabel, "重新执行");
  assert.strictEqual(onIdle.startEnabled, true);
  assert.strictEqual(onIdle.stopEnabled, false);
  assert.strictEqual(onIdle.viewResultEnabled, true);

  const running = t.agentMenuModel({ agentEnabled: true, hasRun: true, isActive: true, latestResultDir: null });
  assert.strictEqual(running.startEnabled, false); // 进行中不可再启动
  assert.strictEqual(running.stopEnabled, true);
  assert.strictEqual(running.viewResultEnabled, false);
});

ok("sortRunsDesc：按 started_at 降序、稳定", () => {
  const runs = [
    { run_id: "a", started_at: "2026-10-12T10:00" },
    { run_id: "c", started_at: "2026-10-12T12:00" },
    { run_id: "b", started_at: "2026-10-12T11:00" },
  ];
  assert.deepStrictEqual(t.sortRunsDesc(runs).map((r) => r.run_id), ["c", "b", "a"]);
});

ok("mergeRuns：按 run_id 去重、后者覆盖、降序", () => {
  const base = [
    { run_id: "a", started_at: "2026-10-12T10:00", state: "running" },
    { run_id: "b", started_at: "2026-10-12T09:00", state: "succeeded" },
  ];
  const incoming = [{ run_id: "a", started_at: "2026-10-12T10:00", state: "succeeded" }];
  const merged = t.mergeRuns(base, incoming);
  assert.strictEqual(merged.length, 2);
  assert.strictEqual(merged[0].run_id, "a");
  assert.strictEqual(merged[0].state, "succeeded"); // 被后者覆盖
});

ok("latestRun / hasMoreRuns", () => {
  const runs = [
    { run_id: "n1", started_at: "2026-10-12T08:00" },
    { run_id: "n2", started_at: "2026-10-12T09:00" },
  ];
  assert.strictEqual(t.latestRun(runs).run_id, "n2");
  assert.strictEqual(t.latestRun([]), null);
  assert.strictEqual(t.hasMoreRuns([{}, {}, {}], 3), true);
  assert.strictEqual(t.hasMoreRuns([{}, {}], 3), false);
});

ok("taskKey：不同清单/任务不冲突", () => {
  assert.strictEqual(t.taskKey("a.yaml", "T1"), t.taskKey("a.yaml", "T1"));
  assert.notStrictEqual(t.taskKey("a.yaml", "T1"), t.taskKey("b.yaml", "T1"));
});

ok("formatRunTime：本地时间格式化", () => {
  assert.strictEqual(t.formatRunTime("2026-10-12T14:05:33"), "2026-10-12 14:05");
  assert.strictEqual(t.formatRunTime(""), "");
  assert.strictEqual(t.formatRunTime(null), "");
  assert.strictEqual(t.formatRunTime("not-a-date"), "not-a-date");
});

console.log("\n" + pass + " assertions passed");
