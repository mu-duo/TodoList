import { useEffect, useLayoutEffect, useRef, useState } from "react";

export type MenuItem = {
  key: string;
  label: string;
  danger?: boolean;
  onSelect: () => void;
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
 */
export function ContextMenu({ x, y, items, onClose }: ContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });

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
        <button
          key={item.key}
          type="button"
          role="menuitem"
          className={`context-menu-item${item.danger ? " danger" : ""}`}
          onClick={() => {
            onClose();
            item.onSelect();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}
