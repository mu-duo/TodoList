import type { RefObject } from "react";
import type { RunSummary } from "../agent";
import { agentErrorClassLabel, agentStateMeta, agentStateTitle, agentStepsText, agentStopReasonLabel, formatRunTime } from "../agentTask";

type AgentTaskPanelProps = {
  enabled: boolean;
  paths: string[];
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
  onAddPaths: (directories: boolean) => void;
  onRemovePath: (path: string) => void;
  onStart: () => void;
  onStop: () => void;
  onOpenResult: (runId: string) => void;
  onLoadMore: () => void;
};

/**
 * 详情内的 Agent 区：状态、关联材料（可增删）、运行历史、打开结果目录。
 * 只读展示 + 经命令写入；不做聊天面板。
 */
export function AgentTaskPanel({
  enabled,
  paths,
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
  onAddPaths,
  onRemovePath,
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
          启用后，Agent 会在本机执行时按需读取下方关联材料（只读），并把任务内容发送到
          DeepSeek 云端。
        </p>
      )}

      <div className="agent-field">
        <div className="agent-field-head">
          <span className="field-label">关联材料</span>
          <div className="agent-actions">
            <button
              type="button"
              className="row-action"
              onClick={() => onAddPaths(false)}
            >
              添加文件
            </button>
            <button
              type="button"
              className="row-action"
              onClick={() => onAddPaths(true)}
            >
              添加目录
            </button>
          </div>
        </div>
        {paths.length === 0 ? (
          <p className="agent-hint">暂无关联材料。</p>
        ) : (
          <ul className="agent-path-list">
            {paths.map((p) => (
              <li key={p} className="agent-path-item">
                <span className="agent-path-text" title={p}>
                  {p}
                </span>
                <button
                  type="button"
                  className="agent-path-remove"
                  onClick={() => onRemovePath(p)}
                  aria-label={`移除 ${p}`}
                  title="移除"
                >
                  ×
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

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
