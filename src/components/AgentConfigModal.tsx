import { useEffect, useRef, useState } from "react";
import {
  DEFAULT_AGENT_MAX_USES,
  DEFAULT_AGENT_MODEL,
  clearAgentKey,
  loadAgentConfig,
  pickAgentResultRoot,
  probeAgent,
  saveAgentConfig,
  setAgentKey,
  validateAgentResultRoot,
} from "../agent";

type AgentConfigModalProps = {
  onClose: () => void;
  /** 可选：从别处跳转进来时的说明横幅（如任务启用前置不足的引导）。 */
  notice?: string | null;
};

type Feedback = { ok: boolean; text: string } | null;

const FOCUSABLE =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), summary, [tabindex]:not([tabindex="-1"])';

/**
 * Agent 配置模态（Phase 1）。
 *
 * - 复用既有模态外壳（`.modal-backdrop` / `.modal-card`）与焦点陷阱 / Esc / 遮罩规则。
 * - 打开仅**本机读取**配置（`agent_load_config`）；保存配置不联网、不触发全局刷新 / 重读设置。
 * - 密钥：password 输入、**从不回显旧值**、无“显示密码”开关，仅展示「已配置 / 未配置」；成功即清空输入。
 * - 测试连接为**显式二次费用确认**后的付费探测；未确认前**不 invoke**。
 * - 本机执行 `execution_enabled` 默认关闭；false→true 需**显式云端同意**（并随保存传 `consentCloud`）。
 */
export function AgentConfigModal({ onClose, notice }: AgentConfigModalProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  const restoreRef = useRef<HTMLElement | null>(null);
  const mountedRef = useRef(true);

  const [phase, setPhase] = useState<"loading" | "ready" | "error">("loading");
  const [loadError, setLoadError] = useState("");

  const [model, setModel] = useState(DEFAULT_AGENT_MODEL);
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

  const canEnable = keyPresent && !!resultRoot && rootValid;
  // 已选择但校验未通过：不得据此开启 / 保存（后端仍会二次校验，这里只保证状态一致）
  const rootInvalid = !!resultRoot && !rootValid;

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
        const cfg = await loadAgentConfig();
        if (cancelled) return;
        setModel(cfg.model || DEFAULT_AGENT_MODEL);
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
        if (consentOpen) setConsentOpen(false);
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
  }, [onClose, consentOpen, probeConfirm, probeBusy]);

  // 遮罩关闭：付费探测进行中不关闭（其余情况可关闭）
  function handleBackdropPointerDown() {
    if (probeBusy) return;
    onClose();
  }

  // ---- 密钥（独立保存，不影响配置表单草稿）----

  async function handleSaveKey() {
    if (keyBusy) return;
    if (!keyInput.trim()) {
      setKeyMsg({ ok: false, text: "请输入 Key" });
      return;
    }
    setKeyBusy(true);
    setKeyMsg(null);
    try {
      await setAgentKey(keyInput);
      if (!mountedRef.current) return;
      setKeyPresent(true);
      setKeyInput(""); // 成功立即清空，绝不保留明文
      setKeyMsg({ ok: true, text: "Key 已保存" });
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
      await clearAgentKey();
      if (!mountedRef.current) return;
      setKeyPresent(false);
      setKeyInput("");
      setKeyMsg({ ok: true, text: "Key 已清除" });
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
    const uses = Number.parseInt(maxUsesText, 10);
    if (!Number.isFinite(uses) || uses < 1) {
      setConfigMsg({ ok: false, text: "高级项 max_uses 需为不小于 1 的整数" });
      return;
    }
    if (executionEnabled && !canEnable) {
      setConfigMsg({ ok: false, text: "开启本机执行需先配置 API Key 与有效的结果主目录" });
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
      await saveAgentConfig(
        {
          model: modelText,
          result_root: resultRoot,
          execution_enabled: executionEnabled,
          max_uses: uses,
        },
        executionEnabled && cloudConsent,
      );
      if (!mountedRef.current) return;
      setConfigMsg({ ok: true, text: "配置已保存" });
    } catch (e) {
      if (!mountedRef.current) return;
      setConfigMsg({ ok: false, text: `保存失败：${String(e)}` });
    } finally {
      if (mountedRef.current) setConfigBusy(false);
    }
  }

  // ---- 测试连接（显式付费，二次确认）----

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

            <div className="settings-field">
              <label className="field-label" htmlFor="agent-model">
                模型
              </label>
              <input
                id="agent-model"
                className="agent-input"
                data-autofocus
                value={model}
                placeholder={DEFAULT_AGENT_MODEL}
                spellCheck={false}
                autoComplete="off"
                onChange={(e) => setModel(e.currentTarget.value)}
              />
            </div>

            <div className="settings-field">
              <span className="field-label">API Key</span>
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
                {keyMsg && (
                  <span className={keyMsg.ok ? "agent-ok" : "agent-err"}>
                    {" "}
                    · {keyMsg.text}
                  </span>
                )}
              </p>
            </div>

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
                <span>允许本机执行（联网调用 DeepSeek）</span>
              </label>
              {!canEnable && (
                <p className="agent-warn">
                  {!keyPresent
                    ? "需先配置 API Key，才能开启本机执行。"
                    : rootInvalid
                      ? "结果主目录校验未通过：请重新选择或清除后再开启。"
                      : "需先选择有效的结果主目录，才能开启本机执行。"}
                </p>
              )}
              {consentOpen && (
                <div className="agent-consent">
                  <p>
                    开启后，被启用任务的标题、备注与关联文件内容会按需发送到
                    DeepSeek 云端用于研究。执行权限仅对本机有效，任务文件同步不会自动授权其它设备；关闭应用即停止。
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
                      ? "正在发起付费调用，请稍候（完成后可关闭）。"
                      : "测试连接会发起一次真实的付费调用（DeepSeek）。"}
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
                    disabled={probeBusy || !keyPresent}
                    title={
                      keyPresent ? undefined : "需先配置 API Key 才能测试连接"
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
