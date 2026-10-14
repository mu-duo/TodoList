import { useEffect, useRef, useState } from "react";
import {
  DEFAULT_AGENT_MAX_USES,
  DEFAULT_AGENT_MODEL,
  DEFAULT_AGENT_PROVIDER,
  clearAgentKey,
  isAgentKeyPresent,
  listAgentModels,
  listAgentProviders,
  loadAgentConfig,
  normalizeProtocol,
  pickAgentResultRoot,
  probeAgent,
  protocolLabel,
  saveAgentConfig,
  setAgentKey,
  validateAgentResultRoot,
  type AgentConfigPatch,
  type AgentModelInfo,
  type AgentProtocol,
  type ProviderPreset,
} from "../agent";
import { AgentModelCombobox } from "./AgentModelCombobox";

type AgentConfigModalProps = {
  onClose: () => void;
  /** 可选：从别处跳转进来时的说明横幅（如任务启用前置不足的引导）。 */
  notice?: string | null;
};

type Feedback = { ok: boolean; text: string } | null;

/** 模型列表加载状态（`idle` = 尚未请求，如自定义端点未填地址）。 */
type ModelsState = "idle" | "loading" | "remote" | "none" | "error";

/** 模型列表请求目标（预设只传 provider；自定义端点另带协议与地址）。 */
type ModelTarget = {
  provider: string;
  protocol?: string | null;
  baseUrl?: string | null;
};

const FOCUSABLE =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), summary, [tabindex]:not([tabindex="-1"])';

const PROTOCOL_OPTIONS: { value: AgentProtocol; label: string }[] = [
  { value: "anthropic", label: "Anthropic 兼容" },
  { value: "openai", label: "OpenAI 兼容" },
];

/** 是否为「自定义端点」预设条目（后端标记优先，其次按惯用 id 兜底）。 */
function isCustomPreset(preset: ProviderPreset | undefined): boolean {
  if (!preset) return false;
  return preset.is_custom === true || preset.id === "custom";
}

/** 归一化 `model_by_provider`：只保留非空字符串值。 */
function sanitizeModelMap(raw: unknown): Record<string, string> {
  if (!raw || typeof raw !== "object") return {};
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(raw as Record<string, unknown>)) {
    if (typeof value === "string" && value.trim()) out[key] = value;
  }
  return out;
}

/** 是否形如 http(s)://… 的地址（非 https 不阻断，仅提示）。 */
function hasHttpScheme(url: string): boolean {
  return /^https?:\/\/\S/i.test(url);
}

/** 是否 https 地址。 */
function isHttpsUrl(url: string): boolean {
  return /^https:\/\/\S/i.test(url);
}

/**
 * Agent 配置模态。
 *
 * - 复用既有模态外壳（`.modal-backdrop` / `.modal-card`）与焦点陷阱 / Esc / 遮罩规则。
 * - 打开仅**本机读取**配置（`agent_load_config`）；保存配置不联网、不触发全局刷新 / 重读设置。
 * - Provider 预设来自后端（`agent_list_providers`，单一事实源）；模型列表按需联网拉取
 *   （`agent_list_models`），失败 / 无列表则回退自由输入，**不做持久化缓存**。
 * - 密钥：password 输入、**从不回显旧值**、无“显示密码”开关，仅展示「已配置 / 未配置」；成功即清空输入。
 * - 测试连接为**显式二次费用确认**后的最小对话请求；未确认前**不 invoke**。
 * - 本机执行 `execution_enabled` 默认关闭；false→true 需**显式云端同意**（并随保存传 `consentCloud`）。
 */
export function AgentConfigModal({ onClose, notice }: AgentConfigModalProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  const restoreRef = useRef<HTMLElement | null>(null);
  const mountedRef = useRef(true);

  // 会话内模型列表缓存（**不落盘**）：key → 结果
  const modelsCacheRef = useRef<
    Map<string, { source: "remote" | "none"; models: AgentModelInfo[] }>
  >(new Map());
  // 在途请求令牌：切换 provider / 强制刷新后，旧响应作废
  const modelsReqRef = useRef(0);

  const [phase, setPhase] = useState<"loading" | "ready" | "error">("loading");
  const [loadError, setLoadError] = useState("");

  const [providers, setProviders] = useState<ProviderPreset[]>([]);
  const [provider, setProvider] = useState(DEFAULT_AGENT_PROVIDER);
  const [customProtocol, setCustomProtocol] =
    useState<AgentProtocol>("anthropic");
  const [customBaseUrl, setCustomBaseUrl] = useState("");

  const [model, setModel] = useState(DEFAULT_AGENT_MODEL);
  const [modelByProvider, setModelByProvider] = useState<
    Record<string, string>
  >({});
  const [modelOpen, setModelOpen] = useState(false);
  const [models, setModels] = useState<AgentModelInfo[]>([]);
  const [modelsState, setModelsState] = useState<ModelsState>("idle");

  const [resultRoot, setResultRoot] = useState<string | null>(null);
  const [executionEnabled, setExecutionEnabled] = useState(false);
  const [maxUsesText, setMaxUsesText] = useState(String(DEFAULT_AGENT_MAX_USES));

  const [keyPresent, setKeyPresent] = useState(false);
  const [keyInput, setKeyInput] = useState("");
  const [keyBusy, setKeyBusy] = useState(false);
  const [keyMsg, setKeyMsg] = useState<Feedback>(null);

  const [rootBusy, setRootBusy] = useState(false);
  const [rootMsg, setRootMsg] = useState<Feedback>(null);
  // 结果根是否通过本机校验；仅当「已选择且校验 ok」才算有效根（ok:false 不得视为有效）
  const [rootValid, setRootValid] = useState(false);

  const [cloudConsent, setCloudConsent] = useState(false);
  const [consentOpen, setConsentOpen] = useState(false);

  const [configBusy, setConfigBusy] = useState(false);
  const [configMsg, setConfigMsg] = useState<Feedback>(null);

  const [probeConfirm, setProbeConfirm] = useState(false);
  const [probeBusy, setProbeBusy] = useState(false);
  const [probeMsg, setProbeMsg] = useState<Feedback>(null);

  const currentPreset = providers.find((p) => p.id === provider);
  const isCustom = isCustomPreset(currentPreset) || provider === "custom";
  // 未声明支持搜索的 provider：UI 标注降级（不阻断）
  const searchSupported = currentPreset ? currentPreset.search_support : true;
  // 自定义端点允许无 Key（本地 / 自建）；预设需已配置 Key
  const keySatisfied = keyPresent || isCustom;
  const canEnable = keySatisfied && !!resultRoot && rootValid;
  // 已选择但校验未通过：不得据此开启 / 保存（后端仍会二次校验，这里只保证状态一致）
  const rootInvalid = !!resultRoot && !rootValid;

  const customBaseUrlTrimmed = customBaseUrl.trim();
  const customUrlInvalid =
    isCustom &&
    customBaseUrlTrimmed.length > 0 &&
    !hasHttpScheme(customBaseUrlTrimmed);
  // 非 https + 已有 / 已填 Key → 轻提示（不阻断）
  const showNonHttpsWarn =
    isCustom &&
    customBaseUrlTrimmed.length > 0 &&
    !isHttpsUrl(customBaseUrlTrimmed) &&
    (keyPresent || keyInput.trim().length > 0);

  const matchedModel = models.find((m) => m.id === model.trim());
  const modelUnsupported = !!matchedModel && matchedModel.supported === false;
  // 占位符：优先展示该 provider 的默认模型；无默认则给中性提示（避免误标其它 provider 的模型）
  const modelPlaceholder = currentPreset?.default_model?.trim() || "输入模型 ID";

  // 记录来源焦点 → 焦点进入面板 → 卸载时归还焦点
  useEffect(() => {
    restoreRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    panelRef.current?.focus();
    return () => {
      restoreRef.current?.focus?.();
    };
  }, []);

  // 卸载标记：异步收尾不再 setState
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // 打开仅本机读取配置（无网络）；带 mounted guard 防竞态
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [cfg, presets] = await Promise.all([
          loadAgentConfig(),
          listAgentProviders(),
        ]);
        if (cancelled) return;
        setProviders(presets);
        const initialProvider = cfg.provider || DEFAULT_AGENT_PROVIDER;
        const initialPreset = presets.find((p) => p.id === initialProvider);
        const custom = cfg.custom ?? null;
        setProvider(initialProvider);
        setCustomProtocol(normalizeProtocol(custom?.protocol));
        setCustomBaseUrl(custom?.base_url ?? "");
        const map = sanitizeModelMap(cfg.model_by_provider);
        setModelByProvider(map);
        // 当前 provider 的模型以 `cfg.model` 为准（权威值）；缺失才回退默认
        setModel(
          cfg.model?.trim() ||
            map[initialProvider]?.trim() ||
            initialPreset?.default_model?.trim() ||
            DEFAULT_AGENT_MODEL,
        );
        setExecutionEnabled(!!cfg.execution_enabled);
        setMaxUsesText(
          typeof cfg.max_uses === "number" && Number.isFinite(cfg.max_uses)
            ? String(cfg.max_uses)
            : String(DEFAULT_AGENT_MAX_USES),
        );
        setKeyPresent(!!cfg.key_present);
        // 已开启 = 此前已同意过云端发送；本次再保存不构成 false→true 转变
        setCloudConsent(!!cfg.execution_enabled);
        const root = cfg.result_root ?? null;
        setResultRoot(root);
        if (root) {
          // 本机校验（无网络）：只有 ok 才算有效根，避免旧值失效后仍可开启
          try {
            const res = await validateAgentResultRoot(root);
            if (cancelled) return;
            setRootValid(res.ok);
            if (!res.ok) setRootMsg({ ok: false, text: res.message });
          } catch {
            if (cancelled) return;
            setRootValid(false);
            setRootMsg({ ok: false, text: "结果主目录校验失败" });
          }
        } else {
          setRootValid(false);
        }
        if (cancelled) return;
        setPhase("ready");
        // 打开时拉一次当前 provider 的模型列表（自定义端点需已存地址）
        if (isCustomPreset(initialPreset)) {
          if (custom?.base_url?.trim()) {
            void loadModels({
              provider: initialProvider,
              protocol: normalizeProtocol(custom?.protocol),
              baseUrl: custom.base_url.trim(),
            });
          }
        } else {
          void loadModels({ provider: initialProvider });
        }
      } catch (e) {
        if (cancelled) return;
        setLoadError(String(e));
        setPhase("error");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // 就绪后聚焦首个字段
  useEffect(() => {
    if (phase !== "ready") return;
    panelRef.current
      ?.querySelector<HTMLElement>("[data-autofocus]")
      ?.focus();
  }, [phase]);

  // Esc：优先取消内联确认，其次关闭模态；付费探测进行中禁止关闭；Tab 焦点陷阱
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        if (probeBusy) return; // 付费探测进行中：不关闭（避免误以为已取消）
        if (modelOpen) setModelOpen(false);
        else if (consentOpen) setConsentOpen(false);
        else if (probeConfirm) setProbeConfirm(false);
        else onClose();
        return;
      }
      if (event.key !== "Tab") return;
      const panel = panelRef.current;
      if (!panel) return;
      const focusables = Array.from(
        panel.querySelectorAll<HTMLElement>(FOCUSABLE),
      );
      if (focusables.length === 0) {
        event.preventDefault();
        panel.focus();
        return;
      }
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      const active = document.activeElement;
      if (event.shiftKey) {
        if (active === first || !panel.contains(active)) {
          event.preventDefault();
          last.focus();
        }
      } else if (active === last || !panel.contains(active)) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [onClose, modelOpen, consentOpen, probeConfirm, probeBusy]);

  // 遮罩关闭：付费探测进行中不关闭（其余情况可关闭）
  function handleBackdropPointerDown() {
    if (probeBusy) return;
    onClose();
  }

  // ---- 模型列表（联网；失败 / 无列表回退自由输入）----

  function modelCacheKey(target: ModelTarget): string {
    return `${target.provider}|${target.protocol ?? ""}|${target.baseUrl ?? ""}`;
  }

  function applyModels(source: "remote" | "none", list: AgentModelInfo[]) {
    setModels(list);
    setModelsState(source);
  }

  function resetModels() {
    modelsReqRef.current += 1; // 作废在途响应
    setModels([]);
    setModelsState("idle");
  }

  /** 当前表单对应的模型请求目标；自定义端点未填地址时返回 null。 */
  function currentModelTarget(): ModelTarget | null {
    if (isCustom) {
      if (!customBaseUrlTrimmed) return null;
      return { provider, protocol: customProtocol, baseUrl: customBaseUrlTrimmed };
    }
    return { provider };
  }

  async function loadModels(target: ModelTarget, force = false) {
    const key = modelCacheKey(target);
    if (!force) {
      const cached = modelsCacheRef.current.get(key);
      if (cached) {
        modelsReqRef.current += 1;
        applyModels(cached.source, cached.models);
        return;
      }
    }
    const token = ++modelsReqRef.current;
    setModelsState("loading");
    try {
      const res = await listAgentModels({
        provider: target.provider,
        protocol: target.protocol ?? null,
        baseUrl: target.baseUrl ?? null,
      });
      if (!mountedRef.current || token !== modelsReqRef.current) return;
      const list = Array.isArray(res?.models)
        ? res.models.filter(
            (m) => m && typeof m.id === "string" && m.id.length > 0,
          )
        : [];
      const source: "remote" | "none" =
        res?.source === "remote" && list.length > 0 ? "remote" : "none";
      modelsCacheRef.current.set(key, { source, models: list });
      applyModels(source, list);
    } catch {
      if (!mountedRef.current || token !== modelsReqRef.current) return;
      setModels([]);
      setModelsState("error");
    }
  }

  /** 手动刷新（强制绕过会话缓存）。 */
  function handleRefreshModels() {
    if (modelsState === "loading") return;
    const target = currentModelTarget();
    if (!target) return;
    void loadModels(target, true);
  }

  function modelsHintText(): string {
    switch (modelsState) {
      case "loading":
        return "正在获取模型列表…";
      case "remote":
        return `已从 Provider 获取 ${models.length} 个模型。`;
      case "none":
        return "该 Provider 未提供模型列表，请手动输入模型 ID。";
      case "error":
        return "模型列表获取失败，可手动输入，或点「刷新」重试。";
      default:
        return isCustom
          ? "填写端点地址后点「刷新」获取模型列表（也可直接手动输入）。"
          : "可手动输入模型 ID。";
    }
  }

  // ---- 密钥（独立保存，不影响配置表单草稿；按当前 provider 的槽）----

  async function refreshKeyPresent(providerId: string) {
    try {
      const present = await isAgentKeyPresent(providerId);
      if (!mountedRef.current) return;
      setKeyPresent(present);
    } catch {
      if (!mountedRef.current) return;
      setKeyPresent(false);
    }
  }

  // 切换 provider：回填该 provider 上次模型（否则预设默认），刷新密钥存在性与模型列表。
  function handleChangeProvider(next: string) {
    if (next === provider) return;
    const nextMap = { ...modelByProvider };
    const draft = model.trim();
    if (draft) nextMap[provider] = draft; // 记住当前 provider 的草稿模型
    setModelByProvider(nextMap);
    setProvider(next);
    setModelOpen(false);
    const preset = providers.find((p) => p.id === next);
    const remembered = nextMap[next]?.trim();
    setModel(remembered || preset?.default_model?.trim() || "");
    setKeyInput("");
    setKeyMsg(null);
    void refreshKeyPresent(next);
    if (isCustomPreset(preset) || next === "custom") {
      if (customBaseUrlTrimmed) {
        void loadModels({
          provider: next,
          protocol: customProtocol,
          baseUrl: customBaseUrlTrimmed,
        });
      } else {
        resetModels();
      }
    } else {
      setModels([]); // 避免短暂显示上一 provider 的候选
      void loadModels({ provider: next });
    }
  }

  function handleChangeCustomProtocol(next: AgentProtocol) {
    if (next === customProtocol) return;
    setCustomProtocol(next);
    if (customBaseUrlTrimmed) {
      void loadModels(
        { provider, protocol: next, baseUrl: customBaseUrlTrimmed },
        true,
      );
    }
  }

  async function handleSaveKey() {
    if (keyBusy) return;
    if (!keyInput.trim()) {
      setKeyMsg({ ok: false, text: "请输入 Key" });
      return;
    }
    setKeyBusy(true);
    setKeyMsg(null);
    try {
      await setAgentKey(keyInput, provider);
      if (!mountedRef.current) return;
      setKeyPresent(true);
      setKeyInput(""); // 成功立即清空，绝不保留明文
      setKeyMsg({ ok: true, text: "Key 已保存" });
      // 新 Key 可能让模型列表可用：清缓存并按需重拉
      modelsCacheRef.current.clear();
      const target = currentModelTarget();
      if (target) void loadModels(target, true);
    } catch (e) {
      if (!mountedRef.current) return;
      setKeyMsg({ ok: false, text: `保存失败：${String(e)}` });
    } finally {
      if (mountedRef.current) setKeyBusy(false);
    }
  }

  async function handleClearKey() {
    if (keyBusy) return;
    setKeyBusy(true);
    setKeyMsg(null);
    try {
      await clearAgentKey(provider);
      if (!mountedRef.current) return;
      setKeyPresent(false);
      setKeyInput("");
      setKeyMsg({ ok: true, text: "Key 已清除" });
      modelsCacheRef.current.clear();
    } catch (e) {
      if (!mountedRef.current) return;
      setKeyMsg({ ok: false, text: `清除失败：${String(e)}` });
    } finally {
      if (mountedRef.current) setKeyBusy(false);
    }
  }

  // ---- 结果主目录（选择 → 校验）----

  async function handlePickRoot() {
    if (rootBusy) return;
    setRootBusy(true);
    setRootMsg(null);
    try {
      const picked = await pickAgentResultRoot();
      if (!mountedRef.current) return;
      if (picked) {
        setResultRoot(picked);
        setRootValid(false); // 校验通过前先视为无效，避免残留旧的“有效”态
        const res = await validateAgentResultRoot(picked);
        if (!mountedRef.current) return;
        setRootValid(res.ok); // 仅 ok:true 才算有效根
        setRootMsg({ ok: res.ok, text: res.message });
      }
    } catch (e) {
      if (!mountedRef.current) return;
      setRootValid(false);
      setRootMsg({ ok: false, text: `选择失败：${String(e)}` });
    } finally {
      if (mountedRef.current) setRootBusy(false);
    }
  }

  function handleClearRoot() {
    setResultRoot(null);
    setRootValid(false);
    setRootMsg(null);
  }

  // ---- 本机执行权限（false→true 需显式同意）----

  function handleToggleExecution(next: boolean) {
    if (!next) {
      setExecutionEnabled(false);
      setConsentOpen(false);
      return;
    }
    if (!canEnable) return; // 缺 Key / 结果目录：不可开启（下方有提示）
    setConsentOpen(true); // 需显式云端同意后才真正开启
  }

  function confirmConsent() {
    setExecutionEnabled(true);
    setCloudConsent(true);
    setConsentOpen(false);
  }

  // ---- 保存配置（仅本机；无网络）----

  async function handleSaveConfig() {
    if (configBusy) return;
    const modelText = model.trim();
    if (!modelText) {
      setConfigMsg({ ok: false, text: "模型不能为空" });
      return;
    }
    if (isCustom && !customBaseUrlTrimmed) {
      setConfigMsg({ ok: false, text: "请填写自定义端点地址" });
      return;
    }
    if (customUrlInvalid) {
      setConfigMsg({
        ok: false,
        text: "自定义端点地址需以 http:// 或 https:// 开头",
      });
      return;
    }
    const uses = Number.parseInt(maxUsesText, 10);
    if (!Number.isFinite(uses) || uses < 1) {
      setConfigMsg({ ok: false, text: "高级项 max_uses 需为不小于 1 的整数" });
      return;
    }
    if (executionEnabled && !canEnable) {
      setConfigMsg({
        ok: false,
        text: "开启本机执行需先配置 API Key（本地无 Key 端点除外）与有效的结果主目录",
      });
      return;
    }
    if (rootInvalid) {
      setConfigMsg({
        ok: false,
        text: "结果主目录校验未通过：请重新选择或清除后再保存",
      });
      return;
    }
    setConfigBusy(true);
    setConfigMsg(null);
    try {
      const patch: AgentConfigPatch = {
        provider,
        model: modelText,
        result_root: resultRoot,
        execution_enabled: executionEnabled,
        max_uses: uses,
        // 保存时把当前 provider 的模型并入 map（记住每个 provider 上次模型）
        model_by_provider: { ...modelByProvider, [provider]: modelText },
      };
      if (isCustom) {
        patch.custom = {
          protocol: customProtocol,
          base_url: customBaseUrlTrimmed,
        };
      }
      await saveAgentConfig(patch, executionEnabled && cloudConsent);
      if (!mountedRef.current) return;
      setModelByProvider((prev) => ({ ...prev, [provider]: modelText }));
      setConfigMsg({ ok: true, text: "配置已保存" });
    } catch (e) {
      if (!mountedRef.current) return;
      setConfigMsg({ ok: false, text: `保存失败：${String(e)}` });
    } finally {
      if (mountedRef.current) setConfigBusy(false);
    }
  }

  // ---- 测试连接（显式付费，二次确认；最小对话请求）----

  async function runProbe() {
    if (probeBusy) return;
    setProbeBusy(true);
    setProbeMsg(null);
    try {
      const res = await probeAgent(true); // confirmPaid: true（已二次确认）
      if (!mountedRef.current) return;
      const text = res.ok
        ? res.message || "连接成功"
        : `${res.message}${res.error_class ? `（${res.error_class}）` : ""}`;
      setProbeMsg({ ok: res.ok, text });
      setProbeConfirm(false);
    } catch (e) {
      if (!mountedRef.current) return;
      setProbeMsg({ ok: false, text: `测试失败：${String(e)}` });
      setProbeConfirm(false);
    } finally {
      if (mountedRef.current) setProbeBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onPointerDown={handleBackdropPointerDown}>
      <div
        ref={panelRef}
        className="modal-card modal-card--agent"
        role="dialog"
        aria-modal="true"
        aria-label="Agent 配置"
        aria-busy={probeBusy}
        tabIndex={-1}
        onPointerDown={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>Agent 配置</h2>
          <button
            type="button"
            className="modal-close"
            onClick={onClose}
            disabled={probeBusy}
            aria-label="关闭"
            title={probeBusy ? "测试进行中，请稍候" : "关闭"}
          >
            ×
          </button>
        </div>

        {notice && <p className="agent-notice">{notice}</p>}

        {phase === "loading" && <p className="hint">加载中…</p>}
        {phase === "error" && (
          <p className="agent-err">读取配置失败：{loadError}</p>
        )}

        {phase === "ready" && (
          <div className="settings-body">
            <p className="agent-note">
              配置仅保存在本机；密钥以系统保护方式加密存放、不回显。关闭应用即停止
              Agent；换用其它设备需在该设备重新授权。
            </p>

            {/* ------------------------------ Provider ------------------------------ */}
            <div className="settings-field">
              <div className="agent-field-head">
                <label className="field-label" htmlFor="agent-provider">
                  Provider
                </label>
                <span className="agent-field-spacer" />
                {searchSupported ? (
                  <span className="agent-chip agent-chip--ok">联网搜索</span>
                ) : (
                  <span className="agent-chip agent-chip--muted">
                    无联网搜索
                  </span>
                )}
              </div>
              <div className="agent-select-wrap">
                <select
                  id="agent-provider"
                  className="agent-input agent-select"
                  data-autofocus
                  value={provider}
                  onChange={(e) => handleChangeProvider(e.currentTarget.value)}
                >
                  {providers.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.label}
                      {p.search_support ? "" : "（无搜索）"}
                    </option>
                  ))}
                </select>
              </div>
              {!searchSupported && (
                <p className="agent-hint">
                  该 Provider 不支持联网搜索，Agent 将以无搜索模式运行（降级）。
                </p>
              )}
            </div>

            {/* --------------------------- 自定义端点 --------------------------- */}
            {isCustom && (
              <div className="settings-field agent-reveal">
                <span className="field-label">自定义端点</span>
                <div
                  className="segmented"
                  role="radiogroup"
                  aria-label="自定义端点协议"
                >
                  {PROTOCOL_OPTIONS.map((option) => {
                    const active = option.value === customProtocol;
                    return (
                      <button
                        key={option.value}
                        type="button"
                        role="radio"
                        aria-checked={active}
                        className="segmented-btn"
                        onClick={() => handleChangeCustomProtocol(option.value)}
                      >
                        {option.label}
                      </button>
                    );
                  })}
                </div>
                <input
                  className="agent-input agent-input--mono"
                  aria-label="自定义端点地址"
                  value={customBaseUrl}
                  placeholder="https://example.com/v1"
                  spellCheck={false}
                  autoComplete="off"
                  onChange={(e) => setCustomBaseUrl(e.currentTarget.value)}
                />
                <p className="agent-hint">
                  填写服务商的 API 基址（base URL）；协议决定请求路径的拼接方式。
                  Key 可留空，适用于本地 / 自建端点。
                </p>
                {showNonHttpsWarn && (
                  <p className="agent-caution">
                    该地址不是 https，密钥会以明文经网络传输；请确认端点可信（不会阻止保存）。
                  </p>
                )}
              </div>
            )}

            {/* ------------------------------ 模型 ------------------------------ */}
            <div className="settings-field">
              <div className="agent-field-head">
                <label className="field-label" htmlFor="agent-model">
                  模型
                </label>
                <span className="agent-field-spacer" />
                {matchedModel && (
                  <span
                    className="agent-chip agent-chip--muted"
                    title="当前模型实际使用的协议"
                  >
                    {protocolLabel(matchedModel.protocol)}
                  </span>
                )}
                <button
                  type="button"
                  className={`row-action agent-refresh${modelsState === "loading" ? " is-busy" : ""}`}
                  onClick={handleRefreshModels}
                  disabled={
                    modelsState === "loading" ||
                    (isCustom && !customBaseUrlTrimmed)
                  }
                  title={
                    isCustom && !customBaseUrlTrimmed
                      ? "请先填写自定义端点地址"
                      : "重新获取模型列表"
                  }
                >
                  <RefreshIcon />
                  <span>刷新</span>
                </button>
              </div>
              <AgentModelCombobox
                id="agent-model"
                value={model}
                onChange={setModel}
                options={models}
                open={modelOpen}
                onOpenChange={setModelOpen}
                placeholder={modelPlaceholder}
              />
              <p className="agent-hint">{modelsHintText()}</p>
              {modelUnsupported && (
                <p className="agent-caution">
                  该模型在当前协议下暂不受支持，请另选一个模型。
                </p>
              )}
            </div>

            {/* ----------------------------- API Key ----------------------------- */}
            <div className="settings-field">
              <div className="agent-field-head">
                <span className="field-label">
                  API Key
                  {isCustom && (
                    <span className="agent-label-note">（可留空）</span>
                  )}
                </span>
              </div>
              <div className="agent-key-row">
                <input
                  type="password"
                  className="agent-input"
                  value={keyInput}
                  placeholder={keyPresent ? "已配置（输入以替换）" : "未配置"}
                  autoComplete="off"
                  spellCheck={false}
                  onChange={(e) => setKeyInput(e.currentTarget.value)}
                />
                <button
                  type="button"
                  className="btn-primary agent-btn"
                  onClick={() => void handleSaveKey()}
                  disabled={keyBusy || !keyInput.trim()}
                >
                  保存 Key
                </button>
                <button
                  type="button"
                  className="agent-btn"
                  onClick={() => void handleClearKey()}
                  disabled={keyBusy || !keyPresent}
                >
                  清除
                </button>
              </div>
              <p className="agent-status">
                状态：{keyPresent ? "已配置" : "未配置"}
                {isCustom && !keyPresent && "（本地 / 自建端点可留空）"}
                {keyMsg && (
                  <span className={keyMsg.ok ? "agent-ok" : "agent-err"}>
                    {" "}
                    · {keyMsg.text}
                  </span>
                )}
              </p>
            </div>

            {/* --------------------------- 结果主目录 --------------------------- */}
            <div className="settings-field">
              <span className="field-label">结果主目录</span>
              <div className="agent-path-row">
                <code className="agent-path" title={resultRoot ?? undefined}>
                  {resultRoot ?? "未设置"}
                </code>
                <button
                  type="button"
                  className="agent-btn"
                  onClick={() => void handlePickRoot()}
                  disabled={rootBusy}
                >
                  选择目录
                </button>
                <button
                  type="button"
                  className="agent-btn"
                  onClick={handleClearRoot}
                  disabled={rootBusy || !resultRoot}
                >
                  清除
                </button>
              </div>
              {rootMsg && (
                <p className={rootMsg.ok ? "agent-ok" : "agent-err"}>
                  {rootMsg.text}
                </p>
              )}
            </div>

            {/* --------------------------- 本机执行权限 --------------------------- */}
            <div className="settings-field">
              <label className="agent-check">
                <input
                  type="checkbox"
                  checked={executionEnabled}
                  disabled={!canEnable || consentOpen}
                  onChange={(e) =>
                    handleToggleExecution(e.currentTarget.checked)
                  }
                />
                <span>允许本机执行（联网调用所选 Provider）</span>
              </label>
              {!canEnable && (
                <p className="agent-warn">
                  {!keySatisfied
                    ? "需先配置 API Key（本地 / 自建端点可留空），才能开启本机执行。"
                    : rootInvalid
                      ? "结果主目录校验未通过：请重新选择或清除后再开启。"
                      : "需先选择有效的结果主目录，才能开启本机执行。"}
                </p>
              )}
              {consentOpen && (
                <div className="agent-consent">
                  <p>
                    开启后，被启用任务的标题、备注与关联文件内容会按需发送到
                    所选 Provider 云端用于研究。执行权限仅对本机有效，任务文件同步不会自动授权其它设备；关闭应用即停止。
                  </p>
                  <div className="agent-actions">
                    <button
                      type="button"
                      onClick={() => setConsentOpen(false)}
                    >
                      取消
                    </button>
                    <button
                      type="button"
                      className="btn-primary"
                      onClick={confirmConsent}
                    >
                      确认开启
                    </button>
                  </div>
                </div>
              )}
            </div>

            <details className="agent-advanced">
              <summary>高级</summary>
              <div className="settings-field">
                <label className="field-label" htmlFor="agent-max-uses">
                  单次搜索请求内的搜索上限（max_uses）
                </label>
                <input
                  id="agent-max-uses"
                  type="number"
                  min={1}
                  className="agent-input agent-input--num"
                  value={maxUsesText}
                  onChange={(e) => setMaxUsesText(e.currentTarget.value)}
                />
                <p className="agent-hint">
                  这是单条联网研究请求内部的服务端搜索次数上限，不是运行总次数，也不是费用上限。
                </p>
              </div>
            </details>

            {configMsg && (
              <p className={configMsg.ok ? "agent-ok" : "agent-err"}>
                {configMsg.text}
              </p>
            )}

            {probeMsg && (
              <p className={probeMsg.ok ? "agent-ok" : "agent-err"}>
                测试连接：{probeMsg.text}
              </p>
            )}

            <div className="agent-actions agent-actions--footer">
              {probeConfirm ? (
                <>
                  <span className="agent-warn">
                    {probeBusy
                      ? "正在发起最小对话请求，请稍候（完成后可关闭）。"
                      : "测试连接会向所选 Provider 发送一次最小的对话请求以验证连通性，可能产生少量费用。"}
                  </span>
                  <button
                    type="button"
                    onClick={() => setProbeConfirm(false)}
                    disabled={probeBusy}
                  >
                    取消
                  </button>
                  <button
                    type="button"
                    className="btn-primary"
                    onClick={() => void runProbe()}
                    disabled={probeBusy}
                  >
                    {probeBusy ? "测试中…" : "确认测试"}
                  </button>
                </>
              ) : (
                <>
                  <button
                    type="button"
                    onClick={() => setProbeConfirm(true)}
                    disabled={probeBusy || !keySatisfied}
                    title={
                      keySatisfied
                        ? "发送一次最小对话请求以验证连通性"
                        : "需先配置 API Key（本地 / 自建端点可留空）才能测试连接"
                    }
                  >
                    测试连接
                  </button>
                  <button
                    type="button"
                    className="btn-primary"
                    onClick={() => void handleSaveConfig()}
                    disabled={configBusy || rootInvalid}
                    title={
                      rootInvalid
                        ? "结果主目录校验未通过，请重新选择或清除"
                        : undefined
                    }
                  >
                    {configBusy ? "保存中…" : "保存配置"}
                  </button>
                </>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

/** 刷新图标（循环箭头）。 */
function RefreshIcon() {
  return (
    <svg
      className="agent-refresh-icon"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M21 12a9 9 0 1 1-2.64-6.36" />
      <path d="M21 3v6h-6" />
    </svg>
  );
}
