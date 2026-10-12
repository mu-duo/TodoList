import { useEffect, useLayoutEffect, useRef, useState } from "react";

export type MenuItem = {
  key: string;
  label: string;
  danger?: boolean;
  /** 禁用（不可选）。默认 false，保持既有行为。 */
  disabled?: boolean;
  /** 勾选态（切换类菜单项）。未提供则不渲染勾选语义。默认 undefined。 */
  checked?: boolean;
  /** 子菜单项；存在时该项为父菜单（点击/悬浮展开），`onSelect` 可省。 */
  children?: MenuItem[];
  onSelect?: () => void;
};

type ContextMenuProps = {
  x: number;
  y: number;
  items: MenuItem[];
  onClose: () => void;
};

/**
 * 轻量右键菜单：贴近视口边缘时自动收进边界内；
 * 点外部 / Esc / 滚动 / 尺寸变化即关闭。
 * 支持一层（可递归）子菜单：父项 hover / 点击展开，子项选择后关闭整个菜单。
 */
export function ContextMenu({ x, y, items, onClose }: ContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });
  // 同一时刻仅展开一个子菜单
  const [openKey, setOpenKey] = useState<string | null>(null);

  // 量取尺寸后夹取到视口内（在绘制前完成，避免闪跳）
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    const pad = 8;
    const maxLeft = window.innerWidth - pad - width;
    const maxTop = window.innerHeight - pad - height;
    setPos({
      left: Math.max(pad, Math.min(x, maxLeft)),
      top: Math.max(pad, Math.min(y, maxTop)),
    });
  }, [x, y]);

  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) onClose();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    const onCloseEvent = () => onClose();
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("scroll", onCloseEvent, true);
    window.addEventListener("resize", onCloseEvent);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("scroll", onCloseEvent, true);
      window.removeEventListener("resize", onCloseEvent);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      className="context-menu"
      style={{ left: pos.left, top: pos.top }}
      role="menu"
    >
      {items.map((item) => (
        <MenuRow
          key={item.key}
          item={item}
          onClose={onClose}
          openKey={openKey}
          onOpen={setOpenKey}
        />
      ))}
    </div>
  );
}

function MenuRow({
  item,
  onClose,
  openKey,
  onOpen,
}: {
  item: MenuItem;
  onClose: () => void;
  openKey: string | null;
  onOpen: (key: string) => void;
}) {
  const [flip, setFlip] = useState(false);
  const subRef = useRef<HTMLDivElement>(null);
  const open = openKey === item.key;

  // 子菜单贴近右边界时翻到左侧
  useLayoutEffect(() => {
    if (!open) return;
    const el = subRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const pad = 8;
    setFlip(rect.right > window.innerWidth - pad);
  }, [open]);

  const hasChildren = !!item.children && item.children.length > 0;

  if (hasChildren) {
    return (
      <div className="context-menu-sub-host">
        <button
          type="button"
          role="menuitem"
          aria-haspopup="menu"
          aria-expanded={open}
          className="context-menu-item context-menu-item--parent"
          disabled={item.disabled}
          onPointerEnter={() => {
            if (!item.disabled) onOpen(item.key);
          }}
          onClick={(e) => {
            e.preventDefault();
            if (!item.disabled) onOpen(item.key);
          }}
        >
          <span>{item.label}</span>
          <span className="context-menu-caret" aria-hidden="true">
            ▸
          </span>
        </button>
        {open && (
          <div
            ref={subRef}
            className={`context-menu context-menu--sub${flip ? " is-flip" : ""}`}
            role="menu"
          >
            {item.children!.map((child) => (
              <MenuRow
                key={child.key}
                item={child}
                onClose={onClose}
                openKey={openKey}
                onOpen={onOpen}
              />
            ))}
          </div>
        )}
      </div>
    );
  }

  const hasCheck = item.checked !== undefined;
  return (
    <button
      type="button"
      role={hasCheck ? "menuitemcheckbox" : "menuitem"}
      aria-checked={hasCheck ? item.checked : undefined}
      className={`context-menu-item${item.danger ? " danger" : ""}${
        item.checked ? " is-checked" : ""
      }`}
      disabled={item.disabled}
      onClick={() => {
        onClose();
        item.onSelect?.();
      }}
    >
      {item.checked && (
        <span className="context-menu-check" aria-hidden="true">
          ✓
        </span>
      )}
      {item.label}
    </button>
  );
}
