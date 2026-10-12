import { useEffect, useRef, useState, type ReactNode } from "react";
import { arrayMove } from "@dnd-kit/sortable";
import { SortableList } from "./SortableList";
import { ContextMenu } from "./ContextMenu";
import { type AppBarEntryMeta } from "../appBar";

type AppBarProps = {
  /** 可见入口（已解析顺序，仅已知 id）。 */
  entries: AppBarEntryMeta[];
  /** vertical = 宽屏最左竖栏；horizontal = 窄屏顶部横排。 */
  orientation: "vertical" | "horizontal";
  /** 激活某个入口（执行动作 / 打开配置）。 */
  onActivate: (id: string) => void;
  /** 该入口是否禁用（忙碌）。 */
  isDisabled?: (id: string) => boolean;
  /** 该入口是否为「按下」态（仅置顶使用；其余返回 undefined）。 */
  isPressed?: (id: string) => boolean | undefined;
  /** 保存中：禁用重排，避免异步乱序。 */
  reorderDisabled?: boolean;
  /** 拖拽 / 溢出后新的完整可见顺序。 */
  onReorder: (nextVisibleIds: string[]) => void;
};

// 与 CSS 中按钮尺寸 / 间距保持一致（用于顶部溢出估算）。
const BUTTON_PX = 40;
const GAP_PX = 8;

/**
 * 应用功能栏：宽屏为最左竖栏，窄屏为顶部横排（复用同一顺序与渲染）。
 * - 入口图标无常驻文字，悬浮显示中文名（title），保留 aria 名称。
 * - 直接拖拽重排：鼠标移动超过阈值 / 触屏长按触发；拖拽结束不触发点击。
 *   （本栏按钮即操作入口，不注册键盘拖拽——Space / Enter 均作原生点击；键盘重排在设置内提供。）
 * - 窄屏顶部空间不足时，溢出入口收进「更多」菜单（不挤压标题、不产生整页横向滚动）；
 *   菜单项与栏上入口共用同一「禁用 / 切换态」规则。
 */
export function AppBar({
  entries,
  orientation,
  onActivate,
  isDisabled,
  isPressed,
  reorderDisabled = false,
  onReorder,
}: AppBarProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [available, setAvailable] = useState(0);
  const [morePos, setMorePos] = useState<{ x: number; y: number } | null>(null);
  const moreBtnRef = useRef<HTMLButtonElement>(null);

  const entryById = new Map(entries.map((e) => [e.id, e]));

  // 量取可用宽度：决定窄屏顶部是否需要「更多」收起。
  useEffect(() => {
    if (orientation !== "horizontal") {
      setAvailable(0);
      return;
    }
    const el = rootRef.current;
    if (!el) return;
    const measure = () => setAvailable(el.clientWidth);
    measure();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", measure);
      return () => window.removeEventListener("resize", measure);
    }
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, [orientation]);

  // 计算实际渲染的入口与溢出入口
  let shownEntries = entries;
  let overflowEntries: AppBarEntryMeta[] = [];
  if (orientation === "horizontal" && available > 0) {
    const fit = Math.max(
      1,
      Math.floor((available + GAP_PX) / (BUTTON_PX + GAP_PX)),
    );
    if (fit < entries.length) {
      const shownCount = Math.max(0, fit - 1); // 留一格给「更多」
      shownEntries = entries.slice(0, shownCount);
      overflowEntries = entries.slice(shownCount);
    }
  }

  function handleReorder(activeId: string, overId: string) {
    const ids = shownEntries.map((e) => e.id);
    const from = ids.indexOf(activeId);
    const to = ids.indexOf(overId);
    if (from < 0 || to < 0 || from === to) return;
    const moved = arrayMove(ids, from, to);
    onReorder([...moved, ...overflowEntries.map((e) => e.id)]);
  }

  function openMoreMenu() {
    const rect = moreBtnRef.current?.getBoundingClientRect();
    if (!rect) return;
    setMorePos({ x: rect.left, y: rect.bottom + 4 });
  }

  return (
    <div
      ref={rootRef}
      className={`app-bar app-bar--${orientation}`}
      role="toolbar"
      aria-orientation={orientation}
      aria-label="应用功能栏"
    >
      <SortableList
        className="app-bar-list"
        ids={shownEntries.map((e) => e.id)}
        axis={orientation === "vertical" ? "vertical" : "horizontal"}
        interaction="mouseTouch"
        keyboardDrag={false}
        rowClassName={() => "app-bar-item"}
        itemDisabled={() => reorderDisabled}
        onReorder={handleReorder}
        renderItem={(id, activatorRef) => {
          const meta = entryById.get(id);
          if (!meta) return null;
          const pressed = isPressed?.(id);
          return (
            <button
              type="button"
              ref={activatorRef}
              className={`app-bar-btn${pressed ? " is-pressed" : ""}`}
              onClick={() => onActivate(id)}
              disabled={isDisabled?.(id) ?? false}
              aria-pressed={pressed}
              aria-label={meta.label}
              title={meta.label}
            >
              <AppBarIcon id={id} />
            </button>
          );
        }}
      />

      {overflowEntries.length > 0 && (
        <button
          ref={moreBtnRef}
          type="button"
          className="app-bar-btn app-bar-more"
          onClick={openMoreMenu}
          aria-haspopup="menu"
          aria-expanded={morePos !== null}
          aria-label="更多"
          title="更多"
        >
          <MoreIcon />
        </button>
      )}

      {morePos && overflowEntries.length > 0 && (
        <ContextMenu
          x={morePos.x}
          y={morePos.y}
          items={overflowEntries.map((entry) => ({
            key: entry.id,
            label: entry.label,
            disabled: isDisabled?.(entry.id) ?? false,
            checked: isPressed?.(entry.id),
            onSelect: () => onActivate(entry.id),
          }))}
          onClose={() => setMorePos(null)}
        />
      )}
    </div>
  );
}

/** 入口图标（内联 SVG，沿用 24×24 / fill:none / currentColor / 1.8 / round 的既有风格）。 */
function AppBarIcon({ id }: { id: string }) {
  const className = "app-bar-icon";
  if (id === "refresh") {
    return (
      <svg
        className={className}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        <path d="M21 12a9 9 0 1 1-2.64-6.36" />
        <path d="M21 3v6h-6" />
      </svg>
    );
  }
  if (id === "pin") {
    return (
      <svg
        className={className}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        <path d="M12 17v5" />
        <path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" />
      </svg>
    );
  }
  if (id === "settings") {
    return (
      <svg
        className={className}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        <circle cx="12" cy="12" r="3" />
        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
      </svg>
    );
  }
  if (id === "agent") {
    return (
      <svg
        className={className}
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
      >
        <rect x="4" y="8" width="16" height="11" rx="3" />
        <path d="M12 8V5.2" />
        <circle cx="12" cy="4" r="1.1" />
        <path d="M9 12v2" />
        <path d="M15 12v2" />
        <path d="M9.5 16.5h5" />
        <path d="M4 12.5H2.6" />
        <path d="M20 12.5h1.4" />
      </svg>
    );
  }
  return null;
}

/** 「更多」图标：三点（实心，尺寸小）。 */
function MoreIcon(): ReactNode {
  return (
    <svg
      className="app-bar-icon"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden="true"
    >
      <circle cx="5.5" cy="12" r="1.7" />
      <circle cx="12" cy="12" r="1.7" />
      <circle cx="18.5" cy="12" r="1.7" />
    </svg>
  );
}
