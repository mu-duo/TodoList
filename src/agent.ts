import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Agent 配置前端接入层（Phase 1）。
 *
 * 契约冻结于 docs/pdr/10-app/02-interfaces.md §6 与 ADR-0008；多 Provider（预设端点 /
 * 凭据分槽）经 **ADR-0009** 修订 ADR-0008 的 provider 限定条款（其余安全边界不变）。
 * 细节切片冻结：`agent_list_providers` 增 `protocol` / `is_custom`，新增 `agent_list_models`，
 * settings 增 `agent.custom:{protocol, base_url}` 与 `agent.model_by_provider`。
 * - 命令参数用 camelCase（Tauri 约定）；返回值字段为 **snake_case**（后端 serde）。
 * - `agent_save_config` 的 `config` 为**已知字段补丁**，内部字段名与后端一致（snake_case）。
 * - 本模块**只在被调用时**发起 invoke；导入/渲染/保存配置均**不联网**。
 * - 密钥值只在 `agent_set_key` 请求体中出现，绝不回显 / 记录；预设清单来自后端（避免双源）。
 */

/** 协议族（预设 `protocol` / 自定义端点 `protocol`）。 */
export type AgentProtocol = "anthropic" | "openai";

/**
 * 归一化协议标识：统一为 `"anthropic"` / `"openai"`（后端 `Protocol::as_str` 口径）；
 * 容错 `anthropic_compat` / `AnthropicCompat` / `openai_compat` 等旧式写法；
 * 未知 / 空按 Anthropic 兼容处理（与后端默认一致）。
 */
export function normalizeProtocol(raw: unknown): AgentProtocol {
  const s = String(raw ?? "").toLowerCase();
  return s.includes("openai") ? "openai" : "anthropic";
}

/** 协议的中文展示名。 */
export function protocolLabel(protocol: string): string {
  return normalizeProtocol(protocol) === "openai"
    ? "OpenAI 兼容"
    : "Anthropic 兼容";
}

/** Provider 预设（= `agent_list_providers` 元素；**不含**密钥）。 */
export type ProviderPreset = {
  id: string;
  label: string;
  /** 协议族（后端 `"anthropic"` / `"openai"`；用 `normalizeProtocol` 归一化）。 */
  protocol: string;
  default_model: string;
  search_support: boolean;
  /** 是否为「自定义端点」条目（由后端标记，前端不硬编码）。 */
  is_custom?: boolean;
};

/** 自定义端点（settings `agent.custom`）。 */
export type AgentCustomEndpoint = {
  protocol: string;
  base_url: string;
};

/** 本机全局 Agent 配置（= `agent_load_config` 返回）。 */
export type AgentConfig = {
  provider: string;
  model: string;
  result_root: string | null;
  execution_enabled: boolean;
  max_uses: number;
  key_present: boolean;
  /** 自定义端点；后端未返回时为 undefined（前端按空处理）。 */
  custom?: AgentCustomEndpoint | null;
  /** 各 provider 上次使用的模型（provider id → model）。 */
  model_by_provider?: Record<string, string> | null;
};

/** 保存补丁：仅已知字段，字段名与后端 `agent` 段一致（snake_case）。 */
export type AgentConfigPatch = {
  provider?: string;
  model?: string;
  result_root?: string | null;
  execution_enabled?: boolean;
  max_uses?: number;
  /** 自定义端点（仅 provider=自定义时写入；其余情况保留后端旧值）。 */
  custom?: AgentCustomEndpoint;
  /** provider id → 上次模型（保存时把当前 provider 的模型并入）。 */
  model_by_provider?: Record<string, string>;
};

/** 模型条目（= `agent_list_models.models` 元素）。 */
export type AgentModelInfo = {
  id: string;
  /** 该模型实际使用的协议（混合网关按模型映射）。 */
  protocol: string;
  /** 当前协议族下是否受支持（false = 列表标注不可用，如 responses 模型）。 */
  supported: boolean;
};

/** 模型列表结果（= `agent_list_models` 返回）。 */
export type AgentModelList = {
  /** remote = 来自端点；none = 端点无列表（前端回退自由输入）。 */
  source: "remote" | "none";
  models: AgentModelInfo[];
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
export const DEFAULT_AGENT_PROVIDER = "deepseek";
export const DEFAULT_AGENT_MAX_USES = 5;

/** 读取本机配置（纯本地，无网络）。 */
export function loadAgentConfig(): Promise<AgentConfig> {
  return invoke<AgentConfig>("agent_load_config");
}

/** 内置 provider 预设清单（纯本地，无网络）；预设为后端单一事实源。 */
export function listAgentProviders(): Promise<ProviderPreset[]> {
  return invoke<ProviderPreset[]>("agent_list_providers");
}

/**
 * 拉取某 provider 的模型列表（**会联网**：向端点请求模型清单）。
 * - 预设：只传 `provider`（后端按预设解析端点）。
 * - 自定义端点：另传 `protocol` 与 `baseUrl`。
 * - `source:"none"` = 端点未提供列表；调用失败由调用方捕获并回退自由输入。
 * - 结果**不做持久化缓存**（仅当前会话内存持有）。
 */
export function listAgentModels(params: {
  provider: string;
  protocol?: string | null;
  baseUrl?: string | null;
}): Promise<AgentModelList> {
  return invoke<AgentModelList>("agent_list_models", {
    provider: params.provider,
    protocol: params.protocol ?? null,
    baseUrl: params.baseUrl ?? null,
  });
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

/**
 * 设置 / 替换密钥（值只在请求体出现，成功由 UI 立即清空输入）。
 * `provider` 省略 = 后端当前 provider；显式传入以读写指定槽。
 */
export function setAgentKey(key: string, provider?: string): Promise<void> {
  return invoke<void>("agent_set_key", { key, provider: provider ?? null });
}

/** 清除密钥（`provider` 省略 = 后端当前 provider）。 */
export function clearAgentKey(provider?: string): Promise<void> {
  return invoke<void>("agent_clear_key", { provider: provider ?? null });
}

/** 是否已配置密钥（仅布尔，不含值；`provider` 省略 = 后端当前 provider）。 */
export function isAgentKeyPresent(provider?: string): Promise<boolean> {
  return invoke<boolean>("agent_key_present", { provider: provider ?? null });
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

/**
 * 打开一个外部目标（详情「关联材料」行点击）。
 * 后端按目标类型分派：http(s) / file → 浏览器；目录 → 打开；文件 → 资源管理器定位。
 * 目标不存在 / 非法时 reject（中文错误），由调用方提示。
 */
export function openExternal(target: string): Promise<void> {
  return invoke<void>("agent_open_external", { target });
}

/** 任务级 Agent 意图（启用状态 + 关联原路径 + 关联链接）。 */
export function setAgentTaskIntent(
  listFile: string,
  taskId: string,
  enabled: boolean,
  paths: string[],
  links: string[],
): Promise<void> {
  return invoke<void>("agent_set_task_intent", {
    listFile,
    taskId,
    enabled,
    paths,
    links,
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
