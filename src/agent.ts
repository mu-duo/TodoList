import { invoke } from "@tauri-apps/api/core";

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
