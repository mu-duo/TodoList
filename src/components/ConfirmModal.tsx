import { useEffect, useRef, type ReactNode } from "react";

type ConfirmModalProps = {
  title: string;
  message: ReactNode;
  /** 确认按钮文案（默认「删除」）。 */
  confirmLabel?: string;
  /** 取消按钮文案（默认「取消」）。 */
  cancelLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
};

const FOCUSABLE =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * 通用确认模态：与设置模态同风格（遮罩 + 卡片）。
 * 打开时焦点进入面板（默认落在「取消」——安全默认，Enter 也只会取消）、
 * Tab 焦点陷阱、卸载时归还焦点；Esc / 点遮罩 / 点「取消」均取消。
 */
export function ConfirmModal({
  title,
  message,
  confirmLabel = "删除",
  cancelLabel = "取消",
  onConfirm,
  onCancel,
}: ConfirmModalProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  const restoreRef = useRef<HTMLElement | null>(null);

  // 记录来源焦点 → 焦点进入面板（默认「取消」）→ 卸载时归还焦点
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

  // Esc 取消（capture + 阻断冒泡，避免同时触发详情快捷键）；Tab 焦点陷阱
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        onCancel();
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
  }, [onCancel]);

  return (
    <div className="modal-backdrop" onPointerDown={onCancel}>
      <div
        ref={panelRef}
        className="modal-card"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
        onPointerDown={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>{title}</h2>
        </div>
        <div className="confirm-body">
          <p className="confirm-message">{message}</p>
          <div className="confirm-actions">
            <button type="button" data-autofocus onClick={onCancel}>
              {cancelLabel}
            </button>
            <button type="button" className="danger" onClick={onConfirm}>
              {confirmLabel}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
