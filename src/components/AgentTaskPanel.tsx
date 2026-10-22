import type { RefObject } from "react";
import type { RunSummary } from "../agent";
import { agentErrorClassLabel, agentStateMeta, agentStateTitle, agentStepsText, agentStopReasonLabel, formatRunTime } from "../agentTask";

type AgentTaskPanelProps = {
  enabled: boolean;
  /** 当前已知的运行态（事件 / 状态轮询）；无则 null。 */
  liveState?: string | null;
  liveErrorClass?: string | null;
  /** 当前已知的停止原因（manual|disabled|completed|deleted）。 */
  liveStopReason?: string | null;
  isActive: boolean;
  /** 该任务的启动 / 停止在途。 */
  busy: boolean;
  runs: RunSummary[];
  runsLoading: boolean;
  runsHasMore: boolean;
  sectionRef: RefObject<HTMLElement | null>;
  onToggleEnabled: (enabled: boolean) => void;
  onStart: () => void;
  onStop: () => void;
  onOpenResult: (runId: string) => void;
  onLoadMore: () => void;
};

/**
 * 详情内的 Agent 区：启用开关、运行状态、运行历史与打开结果目录。
 * 关联材料已独立为详情「关联材料」板块（见 `MaterialsPanel`），此处不再管理。
 */
export function AgentTaskPanel({
  enabled,
  liveState,
  liveErrorClass,
  liveStopReason,
  isActive,
  busy,
  runs,
  runsLoading,
  runsHasMore,
  sectionRef,
  onToggleEnabled,
  onStart,
  onStop,
  onOpenResult,
  onLoadMore,
}: AgentTaskPanelProps) {
  const meta = liveState ? agentStateMeta(liveState) : null;
  const hasRun = runs.length > 0;

  return (
    <section
      ref={sectionRef}
      className="agent-section"
      aria-label="Agent 协助"
      tabIndex={-1}
    >
      <div className="agent-section-head">
        <span className="field-label">Agent 协助</span>
        {meta && (
          <span
            className={`agent-status-badge agent-tone-${meta.tone}`}
            title={agentStateTitle(liveState, liveErrorClass, liveStopReason)}
          >
            {meta.label}
          </span>
        )}
      </div>

      <label className="agent-check">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => onToggleEnabled(e.currentTarget.checked)}
        />
        <span>启用 Agent 协助</span>
      </label>
      {!enabled && (
        <p className="agent-hint">
          启用后，Agent 会在本机执行时只读关联材料（本地路径与链接，链接需能联网抓取），
          并把任务内容发送到所选 Provider 云端；无写入、无命令执行。
        </p>
      )}

      <div className="agent-actions">
        <button
          type="button"
          onClick={onStart}
          disabled={!enabled || isActive || busy}
          title={!enabled ? "需先启用 Agent 协助" : undefined}
        >
          {hasRun ? "重新执行" : "立即执行"}
        </button>
        <button type="button" onClick={onStop} disabled={!isActive || busy}>
          停止
        </button>
      </div>

      <div className="agent-field">
        <div className="agent-field-head">
          <span className="field-label">运行历史</span>
          {runsLoading && <span className="agent-hint">加载中…</span>}
        </div>
        {runs.length === 0 && !runsLoading ? (
          <p className="agent-hint">暂无运行记录。</p>
        ) : (
          <ul className="agent-run-list">
            {runs.map((run) => {
              const m = agentStateMeta(run.state);
              const err = agentErrorClassLabel(run.error_class);
              const stop = agentStopReasonLabel(run.stop_reason);
              const steps = agentStepsText(
                run.stats?.steps,
                run.error_class === "StepLimit",
              );
              return (
                <li key={run.run_id} className="agent-run-item">
                  <div className="agent-run-line">
                    <span className={`agent-status-badge agent-tone-${m.tone}`}>
                      {m.label}
                    </span>
                    <span className="agent-run-time">
                      {formatRunTime(run.started_at) || "—"}
                    </span>
                    <span className="agent-run-attempt">第 {run.attempt} 次</span>
                    {steps && <span className="agent-run-steps">{steps}</span>}
                  </div>
                  {err && <p className="agent-run-error">错误：{err}</p>}
                  {stop && <p className="agent-run-note">停止原因：{stop}</p>}
                  {run.partial && (
                    <p className="agent-run-note">（部分结果，非最终成果）</p>
                  )}
                  <div className="agent-run-line">
                    <span className="agent-run-dir mono" title={run.result_dir}>
                      {run.result_dir || "（无结果目录）"}
                    </span>
                    <button
                      type="button"
                      className="row-action"
                      onClick={() => onOpenResult(run.run_id)}
                      disabled={!run.result_dir}
                    >
                      打开结果目录
                    </button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
        {runs.length > 0 && (
          <p className="agent-hint">
            步数为单次运行主循环护栏的观测值；护栏上限由后端控制，不代表调用次数上限。
          </p>
        )}
        {runsHasMore && (
          <div className="agent-actions">
            <button
              type="button"
              className="row-action"
              onClick={onLoadMore}
              disabled={runsLoading}
            >
              加载更多
            </button>
          </div>
        )}
      </div>
    </section>
  );
}
