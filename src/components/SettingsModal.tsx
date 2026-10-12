import { useEffect, useRef, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { arrayMove } from "@dnd-kit/sortable";
import { SortableList } from "./SortableList";
import { moveAppBarVisible, type AppBarEntryMeta } from "../appBar";
import type { ThemePref } from "../theme";

type SettingsModalProps = {
  theme: ThemePref;
  onThemeChange: (theme: ThemePref) => void;
  onClose: () => void;
  /** 应用功能栏入口（可见、按当前顺序）。 */
  appBarEntries: AppBarEntryMeta[];
  /** 重排可见入口（拖拽 / 上移 / 下移共用；由上层合并未知槽位并落盘）。 */
  onAppBarReorder: (nextVisibleIds: string[]) => void;
  /** 恢复默认顺序。 */
  onAppBarReset: () => void;
  /** 顺序保存中：禁用重排 / 恢复默认。 */
  appBarBusy: boolean;
  /** 顺序保存失败信息（就地展示）。 */
  appBarError: string | null;
  /** 是否存在非默认顺序（决定「恢复默认」是否可点）。 */
  canResetAppBar: boolean;
};

const THEME_OPTIONS: { value: ThemePref; label: string }[] = [
  { value: "system", label: "跟随系统" },
  { value: "light", label: "亮色" },
  { value: "dark", label: "暗色" },
];

const FOCUSABLE =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * 设置模态卡片：带遮罩、居中；点遮罩 / Esc / 关闭按钮关闭；Enter 不做任何处理。
 * 打开时焦点进入面板并做焦点陷阱（Tab 不外泄），关闭后把焦点还给触发元素。
 * 含「应用功能栏顺序」：拖拽 + 上移 / 下移 + 恢复默认（键盘可用）。
 */
export function SettingsModal({
  theme,
  onThemeChange,
  onClose,
  appBarEntries,
  onAppBarReorder,
  onAppBarReset,
  appBarBusy,
  appBarError,
  canResetAppBar,
}: SettingsModalProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  const restoreRef = useRef<HTMLElement | null>(null);
  const [version, setVersion] = useState<string | null>(null);

  // 记录来源焦点 → 移动焦点入面板 → 关闭时归还焦点
  useEffect(() => {
    restoreRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    const panel = panelRef.current;
    const target =
      panel?.querySelector<HTMLElement>("[data-autofocus]") ?? panel;
    target?.focus();
    return () => {
      restoreRef.current?.focus?.();
    };
  }, []);

  // 版本号：失败就整行省略，不报错
  useEffect(() => {
    let cancelled = false;
    getVersion()
      .then((v) => {
        if (!cancelled) setVersion(v);
      })
      .catch(() => {
        /* 读取失败：省略版本行 */
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Esc 关闭（capture + 阻断冒泡，避免同时触发详情快捷键）；Tab 焦点陷阱
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        onClose();
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
  }, [onClose]);

  const orderIds = appBarEntries.map((e) => e.id);

  function moveEntry(id: string, dir: -1 | 1) {
    if (appBarBusy) return;
    const next = moveAppBarVisible(orderIds, id, dir);
    if (next !== orderIds) onAppBarReorder(next);
  }

  return (
    <div className="modal-backdrop" onPointerDown={onClose}>
      <div
        ref={panelRef}
        className="modal-card"
        role="dialog"
        aria-modal="true"
        aria-label="设置"
        tabIndex={-1}
        onPointerDown={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>设置</h2>
          <button
            type="button"
            className="modal-close"
            onClick={onClose}
            aria-label="关闭设置"
            title="关闭"
          >
            ×
          </button>
        </div>

        <div className="settings-body">
          <div className="settings-field">
            <span className="field-label">主题</span>
            <div className="segmented" role="radiogroup" aria-label="主题">
              {THEME_OPTIONS.map((option) => {
                const active = option.value === theme;
                return (
                  <button
                    key={option.value}
                    type="button"
                    role="radio"
                    aria-checked={active}
                    className="segmented-btn"
                    data-autofocus={active ? "" : undefined}
                    onClick={() => onThemeChange(option.value)}
                  >
                    {option.label}
                  </button>
                );
              })}
            </div>
          </div>

          <div className="settings-field">
            <span className="field-label">功能栏顺序</span>
            <p className="settings-hint">
              拖拽，或用「上移 / 下移」调整入口顺序（宽屏竖栏与窄屏顶部共用）。
            </p>
            {appBarError && (
              <p className="settings-inline-error">排序保存失败：{appBarError}</p>
            )}
            <SortableList
              className="app-bar-order-list"
              ids={orderIds}
              axis="vertical"
              interaction="mouseTouch"
              rowClassName={() => "app-bar-order-item"}
              itemDisabled={() => appBarBusy}
              onReorder={(activeId, overId) => {
                const from = orderIds.indexOf(activeId);
                const to = orderIds.indexOf(overId);
                if (from < 0 || to < 0 || from === to) return;
                onAppBarReorder(arrayMove(orderIds, from, to));
              }}
              renderItem={(id, activatorRef) => {
                const meta = appBarEntries.find((e) => e.id === id);
                if (!meta) return null;
                const index = orderIds.indexOf(id);
                const isFirst = index === 0;
                const isLast = index === orderIds.length - 1;
                return (
                  <div className="app-bar-order-row">
                    <button
                      type="button"
                      ref={activatorRef}
                      className="app-bar-order-handle"
                      aria-label={`拖拽排序：${meta.label}`}
                      title="拖拽排序"
                    >
                      <GripIcon />
                    </button>
                    <span className="app-bar-order-label">{meta.label}</span>
                    <button
                      type="button"
                      className="row-action"
                      onClick={() => moveEntry(id, -1)}
                      disabled={appBarBusy || isFirst}
                      aria-label={`上移 ${meta.label}`}
                      title="上移"
                    >
                      上移
                    </button>
                    <button
                      type="button"
                      className="row-action"
                      onClick={() => moveEntry(id, 1)}
                      disabled={appBarBusy || isLast}
                      aria-label={`下移 ${meta.label}`}
                      title="下移"
                    >
                      下移
                    </button>
                  </div>
                );
              }}
            />
            <div className="app-bar-order-actions">
              <button
                type="button"
                className="row-action"
                onClick={onAppBarReset}
                disabled={appBarBusy || !canResetAppBar}
                title="恢复默认顺序"
              >
                恢复默认
              </button>
            </div>
          </div>

          {version && <p className="settings-version">版本 {version}</p>}
        </div>
      </div>
    </div>
  );
}

/** 拖拽把手图标（六点）。 */
function GripIcon() {
  return (
    <svg
      className="app-bar-order-grip"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden="true"
    >
      <circle cx="9" cy="6" r="1.5" />
      <circle cx="15" cy="6" r="1.5" />
      <circle cx="9" cy="12" r="1.5" />
      <circle cx="15" cy="12" r="1.5" />
      <circle cx="9" cy="18" r="1.5" />
      <circle cx="15" cy="18" r="1.5" />
    </svg>
  );
}
