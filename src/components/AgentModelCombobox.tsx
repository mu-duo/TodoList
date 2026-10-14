import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { normalizeProtocol, protocolLabel, type AgentModelInfo } from "../agent";

type AgentModelComboboxProps = {
  id: string;
  /** 当前模型 id（自由输入，可不在列表中）。 */
  value: string;
  onChange: (next: string) => void;
  /** 候选模型（source:"none" / 失败时为空数组 → 退化为纯输入框）。 */
  options: AgentModelInfo[];
  /** 展开态由父级持有：便于 Esc「先收列表、再关模态」。 */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  placeholder?: string;
  disabled?: boolean;
  autoFocus?: boolean;
};

/**
 * 可编辑模型组合框：自由输入 + 候选下拉。
 *
 * - 候选**不进入 Tab 序**（`tabindex` 不设），键盘走 `aria-activedescendant` 模式，
 *   焦点始终留在输入框上，与模态焦点陷阱兼容。
 * - `supported:false` 的模型**照常展示但不可选**（标注「不可用」），仍允许手动输入。
 * - 混合协议清单（同一 provider 多个协议）时，逐项标注协议。
 * - 无候选时不展开下拉（退化为普通输入框）。
 */
export function AgentModelCombobox({
  id,
  value,
  onChange,
  options,
  open,
  onOpenChange,
  placeholder,
  disabled,
  autoFocus,
}: AgentModelComboboxProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [activeId, setActiveId] = useState<string | null>(null);

  const listboxId = `${id}-listbox`;
  const optId = (index: number) => `${id}-opt-${index}`;

  // 是否混合协议（决定是否逐项标注协议）
  const mixedProtocols = useMemo(() => {
    const set = new Set(options.map((o) => normalizeProtocol(o.protocol)));
    return set.size > 1;
  }, [options]);

  // 过滤：空查询展示全部；当前值恰好命中某模型时也展示全部（便于切换）
  const filtered = useMemo(() => {
    const q = value.trim().toLowerCase();
    if (!q) return options;
    if (options.some((o) => o.id.toLowerCase() === q)) return options;
    return options.filter((o) => o.id.toLowerCase().includes(q));
  }, [options, value]);

  const selectable = useMemo(
    () => filtered.filter((o) => o.supported !== false),
    [filtered],
  );

  const canOpen = !disabled && options.length > 0;
  const isOpen = open && canOpen;

  // 展开态变化时重置高亮
  useEffect(() => {
    if (!isOpen) setActiveId(null);
  }, [isOpen]);

  // 高亮项失效（过滤变化 / 选项变化）时清空
  useEffect(() => {
    if (activeId && !filtered.some((o) => o.id === activeId)) {
      setActiveId(null);
    }
  }, [filtered, activeId]);

  // 点击容器外关闭（capture：面板内 stopPropagation 不影响判定）
  useEffect(() => {
    if (!isOpen) return;
    const onPointerDown = (event: PointerEvent) => {
      if (!containerRef.current?.contains(event.target as Node)) {
        onOpenChange(false);
      }
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    return () =>
      document.removeEventListener("pointerdown", onPointerDown, true);
  }, [isOpen, onOpenChange]);

  function commit(nextId: string) {
    onChange(nextId);
    onOpenChange(false);
    setActiveId(null);
  }

  function moveActive(direction: 1 | -1) {
    if (selectable.length === 0) return;
    const currentIndex = selectable.findIndex((o) => o.id === activeId);
    let nextIndex: number;
    if (currentIndex < 0) {
      nextIndex = direction === 1 ? 0 : selectable.length - 1;
    } else {
      nextIndex =
        (currentIndex + direction + selectable.length) % selectable.length;
    }
    setActiveId(selectable[nextIndex].id);
  }

  function handleKeyDown(event: ReactKeyboardEvent<HTMLInputElement>) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!isOpen && canOpen) {
        onOpenChange(true);
        if (selectable.length > 0) {
          setActiveId(
            event.key === "ArrowDown"
              ? selectable[0].id
              : selectable[selectable.length - 1].id,
          );
        }
        return;
      }
      moveActive(event.key === "ArrowDown" ? 1 : -1);
      return;
    }
    if (event.key === "Enter") {
      if (isOpen && activeId) {
        event.preventDefault();
        commit(activeId);
      }
      return;
    }
    if (event.key === "Escape") {
      if (isOpen) {
        // 只收列表，不冒泡到模态（避免误关弹窗）
        event.preventDefault();
        event.stopPropagation();
        onOpenChange(false);
      }
      return;
    }
    if (event.key === "Tab") {
      if (isOpen) onOpenChange(false);
    }
  }

  const activeIndex = filtered.findIndex((o) => o.id === activeId);

  return (
    <div
      ref={containerRef}
      className={`combo${isOpen ? " is-open" : ""}`}
    >
      <div className="combo-control">
        <input
          id={id}
          className="agent-input"
          role="combobox"
          aria-expanded={isOpen}
          aria-controls={isOpen ? listboxId : undefined}
          aria-autocomplete="list"
          aria-activedescendant={
            isOpen && activeIndex >= 0 ? optId(activeIndex) : undefined
          }
          value={value}
          placeholder={placeholder}
          disabled={disabled}
          spellCheck={false}
          autoComplete="off"
          data-autofocus={autoFocus ? "" : undefined}
          onChange={(e) => {
            onChange(e.currentTarget.value);
            if (canOpen && !isOpen) onOpenChange(true);
          }}
          onKeyDown={handleKeyDown}
          onClick={() => {
            if (canOpen) onOpenChange(!isOpen);
          }}
        />
        {canOpen && <span className="combo-caret" aria-hidden="true" />}
      </div>

      {isOpen && (
        <ul
          id={listboxId}
          className="combo-list"
          role="listbox"
          aria-label="模型列表"
        >
          {filtered.length === 0 ? (
            <li className="combo-empty" role="presentation">
              无匹配模型
            </li>
          ) : (
            filtered.map((option, index) => {
              const isSelectable = option.supported !== false;
              const isActive = option.id === activeId;
              return (
                <li
                  key={option.id}
                  id={optId(index)}
                  role="option"
                  aria-selected={isActive}
                  aria-disabled={!isSelectable}
                  className={`combo-opt${isActive ? " is-active" : ""}`}
                  title={
                    isSelectable
                      ? option.id
                      : `${option.id}（当前协议下暂不支持）`
                  }
                  onPointerDown={(e) => {
                    e.preventDefault();
                    if (isSelectable) commit(option.id);
                  }}
                  onMouseEnter={() => {
                    if (isSelectable) setActiveId(option.id);
                  }}
                >
                  <span className="combo-opt-id">{option.id}</span>
                  {mixedProtocols && (
                    <span className="combo-opt-proto">
                      {protocolLabel(option.protocol)}
                    </span>
                  )}
                  {!isSelectable && (
                    <span className="combo-opt-tag">不可用</span>
                  )}
                </li>
              );
            })
          )}
        </ul>
      )}
    </div>
  );
}
