import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Agent 配置前端接入层（Phase 1）。
 *
 * 契约冻结于 docs/pdr/10-app/02-interfaces.md §6 与 ADR-0008。
 * - 命令参数用 camelCase（Tauri 约定）；返回值字段为 **snake_case**（后端 serde）。
 * - `agent_save_config` 的 `config` 为**已知字段补丁**，内部字段名与后端一致（snake_case）。
 * - 本模块**只在被调用时**发起 invoke；导入/渲染/保存配置均**不联网**。
 * - 密钥值只在 `agent_set_key` 请求体中出现，绝不回显 / 记录。
 */

/** 本机全局 Agent 配置（= `agent_load_config` 返回）。 */
export type AgentConfig = {
  model: string;
  result_root: string | null;
  execution_enabled: boolean;
  max_uses: number;
  key_present: boolean;
};

/** 保存补丁：仅已知字段，字段名与后端 `agent` 段一致（snake_case）。 */
export type AgentConfigPatch = {
  model?: string;
  result_root?: string | null;
  execution_enabled?: boolean;
  max_uses?: number;
};

export type AgentValidateResult = {
  ok: boolean;
  message: string;
};

export type AgentProbeResult = {
  ok: boolean;
  error_class?: string;
  message: string;
};

export const DEFAULT_AGENT_MODEL = "deepseek-flash";
export const DEFAULT_AGENT_MAX_USES = 5;

/** 读取本机配置（纯本地，无网络）。 */
export function loadAgentConfig(): Promise<AgentConfig> {
  return invoke<AgentConfig>("agent_load_config");
}

/**
 * 保存已知字段补丁。`execution_enabled` false → true 时，`consentCloud` 必须为 true，
 * 否则后端报错不写；其余情况 `consentCloud` 值不影响结果。
 */
export function saveAgentConfig(
  config: AgentConfigPatch,
  consentCloud: boolean,
): Promise<void> {
  return invoke<void>("agent_save_config", { config, consentCloud });
}

/** 设置 / 替换密钥（值只在请求体出现，成功由 UI 立即清空输入）。 */
export function setAgentKey(key: string): Promise<void> {
  return invoke<void>("agent_set_key", { key });
}

/** 清除密钥。 */
export function clearAgentKey(): Promise<void> {
  return invoke<void>("agent_clear_key");
}

/** 是否已配置密钥（仅布尔，不含值）。 */
export function isAgentKeyPresent(): Promise<boolean> {
  return invoke<boolean>("agent_key_present");
}

/** 校验结果根目录（本机校验，无网络）。 */
export function validateAgentResultRoot(
  path: string,
): Promise<AgentValidateResult> {
  return invoke<AgentValidateResult>("agent_validate_result_root", { path });
}

/**
 * 显式付费探测（测试连接）。**仅当用户已二次确认费用后**传入 `confirmPaid: true`；
 * `false` 时后端直接拒绝、不发网络。
 */
export function probeAgent(confirmPaid: boolean): Promise<AgentProbeResult> {
  return invoke<AgentProbeResult>("agent_probe", { confirmPaid });
}

/** 原生目录选择（Rust dialog 桥）；取消返回 null。 */
export function pickAgentResultRoot(): Promise<string | null> {
  return invoke<string | null>("agent_pick_result_root");
}

/** 原生文件 / 目录选择（Phase 2 任务关联用）；本 Phase 仅提供桥，不接线。 */
export function pickAgentPaths(directories: boolean): Promise<string[]> {
  return invoke<string[]>("agent_pick_paths", { directories });
}

// ============================ Phase 2 ============================

/** 运行状态（状态机）。 */
export type AgentRunState =
  | "pending"
  | "running"
  | "succeeded"
  | "failed"
  | "stopped"
  | "interrupted";

/** 一次运行的计量统计。 */
export type AgentRunStats = {
  chat_calls?: number;
  search_calls?: number;
  server_search_requests?: number;
  bytes_read?: number;
  /** 主循环实际步数（防失控护栏的观测值；**非**调用次数帽，不参与预算）。 */
  steps?: number;
};

/** 运行摘要（= `agent_list_runs` 元素 / `agent_get_status().active`）。字段 snake_case。 */
export type RunSummary = {
  run_id: string;
  task_id: string;
  list_file: string;
  /** 运行态（后端可能返回未知值，前端按字符匹配 + 兜底）。 */
  state: string;
  attempt: number;
  error_class?: string | null;
  /** 停止原因（`state == stopped` 时）：manual|disabled|completed|deleted。 */
  stop_reason?: string | null;
  started_at: string;
  ended_at?: string | null;
  result_dir: string;
  partial: boolean;
  stats?: AgentRunStats | null;
  model: string;
};

/** 队列条目（= `agent_get_status().queue`）。后端**不返回** state，队列语义＝待执行。 */
export type RunRef = {
  run_id: string;
  task_id: string;
  list_file: string;
  attempt: number;
};

export type AgentStatus = {
  active: RunSummary | null;
  queue: RunRef[];
};

/**
 * `agent://state` 事件载荷（**无密钥**）。
 * 后端 `StateEvent` 用 `#[serde(rename_all = "camelCase")]`，故发 `errorClass` / `stopReason`。
 */
export type AgentStateEvent = {
  runId: string;
  listFile: string;
  taskId: string;
  state: string;
  attempt: number;
  errorClass?: string;
  stopReason?: string;
  startedAt?: string;
  endedAt?: string;
};

/** 立即执行 / 重新执行（`force` 绕过夜间窗口）。 */
export function startAgentTask(
  listFile: string,
  taskId: string,
  force: boolean,
): Promise<{ runId: string }> {
  return invoke<{ runId: string }>("agent_start_task", {
    listFile,
    taskId,
    force,
  });
}

/** 停止当前运行 / 取消排队。 */
export function stopAgentTask(listFile: string, taskId: string): Promise<void> {
  return invoke<void>("agent_stop_task", { listFile, taskId });
}

/** 兜底：全局运行状态（active + queue）。 */
export function getAgentStatus(): Promise<AgentStatus> {
  return invoke<AgentStatus>("agent_get_status");
}

/** 某任务的运行历史分页。 */
export function listAgentRuns(
  listFile: string,
  taskId: string,
  offset: number,
  limit: number,
): Promise<RunSummary[]> {
  return invoke<RunSummary[]>("agent_list_runs", {
    listFile,
    taskId,
    offset,
    limit,
  });
}

/** 打开某次运行的结果目录（路径由后端从记录推导）。 */
export function openAgentResultDir(runId: string): Promise<void> {
  return invoke<void>("agent_open_result_dir", { runId });
}

/** 任务级 Agent 意图（启用状态 + 关联原路径）。 */
export function setAgentTaskIntent(
  listFile: string,
  taskId: string,
  enabled: boolean,
  paths: string[],
): Promise<void> {
  return invoke<void>("agent_set_task_intent", {
    listFile,
    taskId,
    enabled,
    paths,
  });
}

/** 订阅 `agent://state`；返回取消订阅函数。载荷无密钥。 */
export function subscribeAgentState(
  handler: (event: AgentStateEvent) => void,
): Promise<UnlistenFn> {
  return listen<AgentStateEvent>("agent://state", (event) =>
    handler(event.payload),
  );
}
