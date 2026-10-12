import type { RunSummary } from "./agent";

/**
 * Agent 任务 UI 的**纯逻辑**（无副作用，便于断言）：
 * 状态 → 色调/文案、任务菜单禁用规则、运行历史排序/分页拼接、行尾图标可见性。
 */

/** 状态色调：中性 / 执行中（靛蓝）/ 成功（绿）/ 失败（红）。 */
export type AgentTone = "neutral" | "running" | "success" | "danger";

export type AgentStateMeta = {
  tone: AgentTone;
  /** 中文短标签（行尾 aria/title 与详情共用）。 */
  label: string;
};

/** 状态 → 色调与中文标签；未知/空 → 待执行（中性）。 */
export function agentStateMeta(state: string | null | undefined): AgentStateMeta {
  switch (state) {
    case "running":
      return { tone: "running", label: "执行中" };
    case "succeeded":
      return { tone: "success", label: "已完成" };
    case "failed":
      return { tone: "danger", label: "失败" };
    case "stopped":
      return { tone: "neutral", label: "已停止" };
    case "interrupted":
      return { tone: "neutral", label: "已中断" };
    case "pending":
      return { tone: "neutral", label: "待执行" };
    default:
      return { tone: "neutral", label: "待执行" };
  }
}

/**
 * `error_class` 中文标签。取值与后端实现一致：
 * provider.rs `CLASS_*`（MissingCredential/HttpAuth/HttpStatus/Transport/Redirect/Parse/NoResultBlock/ProviderError/Aborted）
 * 与 runner.rs（UnsafePath/ResultRoot/StepLimit/timeout）。
 * **未知值原样返回**（兜底，绝不吞掉）。
 */
export function agentErrorClassLabel(errorClass: string | null | undefined): string {
  switch (errorClass) {
    case null:
    case undefined:
    case "":
      return "";
    case "MissingCredential":
      return "未配置密钥";
    case "HttpAuth":
      return "鉴权失败";
    case "HttpStatus":
      return "服务端错误";
    case "Transport":
      return "网络错误";
    case "Redirect":
      return "响应被重定向";
    case "Parse":
      return "响应解析失败";
    case "NoResultBlock":
      return "未返回搜索结果";
    case "ProviderError":
      return "上游返回错误";
    case "Aborted":
      return "已中止";
    case "UnsafePath":
      return "路径不安全";
    case "ResultRoot":
      return "结果目录不可用";
    case "StepLimit":
      return "达到步数上限";
    case "timeout":
      return "运行超时";
    default:
      return errorClass;
  }
}

/** 停止原因（record.rs `STOP_*`）→ 中文；空→""；未知原样返回。 */
export function agentStopReasonLabel(reason: string | null | undefined): string {
  switch (reason) {
    case null:
    case undefined:
    case "":
      return "";
    case "manual":
      return "手动停止";
    case "disabled":
      return "任务已停用";
    case "completed":
      return "任务已完成";
    case "deleted":
      return "任务已删除";
    default:
      return reason;
  }
}

/**
 * 步数展示（读 `stats.steps`）：
 * 缺失 / 非正数 → ""（不显示）；正常 → `步数 N`；
 * 达到上限（`error_class === "StepLimit"`）→ `步数 N/N（达到上限）`。
 * 分母取**实测步数**，不硬编码上限值——上限是后端防失控护栏常量，前端不据此推导剩余次数。
 */
export function agentStepsText(
  steps: number | null | undefined,
  reachedLimit: boolean,
): string {
  if (typeof steps !== "number" || !Number.isFinite(steps) || steps <= 0) {
    return "";
  }
  return reachedLimit ? `步数 ${steps}/${steps}（达到上限）` : `步数 ${steps}`;
}

/** 供 title/aria 的完整说明（不能只靠颜色）；优先错误类，其次停止原因。 */
export function agentStateTitle(
  state: string | null | undefined,
  errorClass?: string | null,
  stopReason?: string | null,
): string {
  const meta = agentStateMeta(state);
  const detail =
    agentErrorClassLabel(errorClass) || agentStopReasonLabel(stopReason);
  return detail ? `Agent：${meta.label}（${detail}）` : `Agent：${meta.label}`;
}

/** 运行是否处于「进行中」（占用并发槽）。 */
export function isAgentActive(state: string | null | undefined): boolean {
  return state === "running" || state === "pending";
}

/** 任务行尾图标的显示状态：停用 → null（不显示）；否则 live 优先，其次历史，最后待执行。 */
export function rowBadgeState(
  enabled: boolean,
  liveState?: string | null,
  latestRunState?: string | null,
): string | null {
  if (!enabled) return null;
  return liveState || latestRunState || "pending";
}

export type AgentMenuInput = {
  agentEnabled: boolean;
  /** 是否有已知的历史运行（决定「立即执行 / 重新执行」）。 */
  hasRun: boolean;
  /** 当前是否进行中（active/queued）。 */
  isActive: boolean;
  latestResultDir?: string | null;
};

export type AgentMenuModel = {
  toggleLabel: string;
  addFileEnabled: boolean;
  addDirEnabled: boolean;
  startLabel: string;
  startEnabled: boolean;
  stopEnabled: boolean;
  viewResultEnabled: boolean;
};

/** 任务菜单中 Agent 子菜单的标签与禁用规则。 */
export function agentMenuModel(input: AgentMenuInput): AgentMenuModel {
  return {
    toggleLabel: input.agentEnabled ? "停用协助" : "启用协助",
    addFileEnabled: true,
    addDirEnabled: true,
    startLabel: input.hasRun ? "重新执行" : "立即执行",
    startEnabled: input.agentEnabled && !input.isActive,
    stopEnabled: input.isActive,
    viewResultEnabled: !!input.latestResultDir,
  };
}

/** 任务键（清单文件 + 任务 id 唯一标识一条任务）。 */
export function taskKey(listFile: string, taskId: string): string {
  return `${listFile}\u0000${taskId}`;
}

/** 运行记录排序键：优先 started_at，其次 run_id。 */
function runSortKey(run: RunSummary): string {
  return run.started_at || run.run_id || "";
}

/** 按开始时间降序（新→旧），稳定。 */
export function sortRunsDesc(runs: RunSummary[]): RunSummary[] {
  return runs
    .map((run, index) => ({ run, index }))
    .sort((a, b) => {
      const ka = runSortKey(a.run);
      const kb = runSortKey(b.run);
      if (ka === kb) return a.index - b.index;
      return ka < kb ? 1 : -1;
    })
    .map(({ run }) => run);
}

/** 合并两组运行记录：按 run_id 去重（后者覆盖），再按时间降序。 */
export function mergeRuns(
  base: RunSummary[],
  incoming: RunSummary[],
): RunSummary[] {
  const byId = new Map<string, RunSummary>();
  for (const run of base) byId.set(run.run_id, run);
  for (const run of incoming) byId.set(run.run_id, run);
  return sortRunsDesc([...byId.values()]);
}

/** 最新一条运行（按时间降序取首）。 */
export function latestRun(
  runs: RunSummary[],
): RunSummary | null {
  return sortRunsDesc(runs)[0] ?? null;
}

/** 分页是否还有更多：该页取满即可再拉一页。 */
export function hasMoreRuns(page: RunSummary[], pageSize: number): boolean {
  return page.length >= pageSize;
}

/** 本地时间显示：`YYYY-MM-DDTHH:MM:SS` → `YYYY-MM-DD HH:MM`；非法原样返回。 */
export function formatRunTime(value: string | null | undefined): string {
  if (!value) return "";
  const m = /^(\d{4}-\d{2}-\d{2})T(\d{2}:\d{2})/.exec(value);
  return m ? `${m[1]} ${m[2]}` : value;
}
