import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { applyThemePref, type ThemePref } from "./theme";
import {
  NOTE_COLORS,
  NOTE_COLOR_COUNT,
  NOTE_TONES,
  type NoteScheme,
} from "./noteColors";
import "./note.css";

/** 便签窗口 props（与窗口创建侧契约一致，见 docs/issues/feature/agent/便签功能.md）。 */
type NoteAppProps = {
  colorIndex: number;
  themePref: ThemePref;
};

/**
 * 缩放方向：取值与 `@tauri-apps/api/window` 的 `ResizeDirection` 逐字一致
 * （node_modules/@tauri-apps/api/window.d.ts:89；该别名未随模块导出，
 * 故此处本地镜像，传给 `startResizeDragging` 结构上即可匹配）。
 */
type ResizeDirection =
  | "North"
  | "NorthEast"
  | "East"
  | "SouthEast"
  | "South"
  | "SouthWest"
  | "West"
  | "NorthWest";

/** 8 向热区：方向 ↔ 定位类（几何见 note.css 顶部说明）。 */
const RESIZE_ZONES: readonly {
  direction: ResizeDirection;
  className: string;
}[] = [
  { direction: "North", className: "note-resize-n" },
  { direction: "West", className: "note-resize-w" },
  { direction: "East", className: "note-resize-e" },
  { direction: "South", className: "note-resize-s" },
  { direction: "NorthWest", className: "note-resize-nw" },
  { direction: "NorthEast", className: "note-resize-ne" },
  { direction: "SouthWest", className: "note-resize-sw" },
  { direction: "SouthEast", className: "note-resize-se" },
];

const DARK_QUERY = "(prefers-color-scheme: dark)";

/** colorIndex 取模归一到 [0, NOTE_COLOR_COUNT)。 */
function normalizeColorIndex(value: number): number {
  if (!Number.isFinite(value)) return 0;
  const index = Math.trunc(value);
  return ((index % NOTE_COLOR_COUNT) + NOTE_COLOR_COUNT) % NOTE_COLOR_COUNT;
}

/** 当前明暗：窗口主题被 setTheme 钉住，因此只需读 media feature。 */
function prefersDark(): boolean {
  return window.matchMedia(DARK_QUERY).matches;
}

/**
 * 便签窗口根组件：整窗一张便签纸。
 * - 颜色：灰阶档位由 colorIndex 决定（跨明暗身份不变），明暗组由 media feature 决定。
 * - 主题：主窗口广播 `theme-pref-changed`；进入时按 URL 带来的偏好先钉一次。
 * - pin / close 走 Tauri 原生窗口 API；拖动靠 `data-tauri-drag-region`、
 *   缩放靠 8 个边缘热区（两者几何与层级约束见 note.css）。
 * 内容不持久化：关闭即丢弃，不写入任何存储。
 */
export default function NoteApp({ colorIndex, themePref }: NoteAppProps) {
  const tone = NOTE_TONES[normalizeColorIndex(colorIndex)];
  const [scheme, setScheme] = useState<NoteScheme>(() =>
    prefersDark() ? "dark" : "light",
  );
  const [pinned, setPinned] = useState(false);
  const pinBusy = useRef(false);
  const textRef = useRef<HTMLTextAreaElement>(null);

  const palette = NOTE_COLORS[scheme][tone];

  // 明暗组实时切换（「跟随系统」时随系统变化，钉住时随 setTheme 变化）。
  useEffect(() => {
    const media = window.matchMedia(DARK_QUERY);
    const sync = () => setScheme(media.matches ? "dark" : "light");
    sync();
    media.addEventListener("change", sync);
    return () => media.removeEventListener("change", sync);
  }, []);

  // 进入时先按 URL 里的偏好钉住主题；失败静默（权限缺失时退回跟随系统）。
  useEffect(() => {
    applyThemePref(themePref).catch(() => {});
  }, [themePref]);

  // 主窗口主题变更广播；订阅失败静默。
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    listen<ThemePref>("theme-pref-changed", (event) => {
      applyThemePref(event.payload).catch(() => {});
    })
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // 挂载即聚焦：打开就能打字。
  useEffect(() => {
    textRef.current?.focus();
  }, []);

  async function togglePin() {
    if (pinBusy.current) return;
    pinBusy.current = true;
    const next = !pinned;
    setPinned(next); // 先反馈，失败再回退
    try {
      await getCurrentWindow().setAlwaysOnTop(next);
    } catch {
      setPinned(!next);
    } finally {
      pinBusy.current = false;
    }
  }

  function closeNote() {
    // 立即关闭、丢弃内容、无确认。
    getCurrentWindow().close().catch(() => {});
  }

  // 边缘热区按下 → 交给系统拖拽缩放；失败静默（权限缺失 / 窗口不可缩放时不动）。
  function beginResize(
    event: ReactPointerEvent<HTMLDivElement>,
    direction: ResizeDirection,
  ) {
    if (event.button !== 0) return;
    event.preventDefault(); // 不触发文本选择 / 抢焦点
    getCurrentWindow()
      .startResizeDragging(direction)
      .catch(() => {});
  }

  const pinLabel = pinned ? "取消置顶" : "置顶";

  return (
    <div
      className="note-root"
      data-tauri-drag-region
      style={
        {
          "--note-bg": palette.bg,
          "--note-border": palette.border,
          "--note-text": palette.text,
        } as CSSProperties
      }
    >
      <div className="note-bar" data-tauri-drag-region>
        <span className="note-grip" aria-hidden="true" />
        <button
          type="button"
          className="note-btn"
          aria-pressed={pinned}
          aria-label={pinLabel}
          title={pinLabel}
          onClick={() => void togglePin()}
        >
          <PinIcon solid={pinned} />
        </button>
        <button
          type="button"
          className="note-btn"
          aria-label="关闭便签"
          title="关闭"
          onClick={closeNote}
        >
          <CloseIcon />
        </button>
      </div>

      <textarea
        ref={textRef}
        className="note-text"
        aria-label="便签内容"
        spellCheck={false}
        wrap="soft"
      />

      {/* 8 向缩放热区：纯指针手势（窗口边缘即控件，无键盘语义，故 aria-hidden） */}
      <div className="note-resize" aria-hidden="true">
        {RESIZE_ZONES.map(({ direction, className }) => (
          <div
            key={direction}
            className={`note-resize-zone ${className}`}
            onPointerDown={(event) => beginResize(event, direction)}
          >
            {direction === "SouthEast" && <ResizeGrip />}
          </div>
        ))}
      </div>
    </div>
  );
}

/** 图钉：描边态 ↔ 实心态（与 AppBar 图标同构，激活时填充）。 */
function PinIcon({ solid }: { solid: boolean }) {
  return (
    <svg
      className="note-icon"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path
        d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"
        fill={solid ? "currentColor" : "none"}
      />
      <path d="M12 17v5" />
    </svg>
  );
}

/** 关闭（×）：与图钉同一笔触。 */
function CloseIcon() {
  return (
    <svg
      className="note-icon"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M6.75 6.75l10.5 10.5" />
      <path d="M17.25 6.75L6.75 17.25" />
    </svg>
  );
}

/** 右下角缩放握把：三条低 alpha 墨色斜纹（风格与顶部把手一致，纯装饰）。 */
function ResizeGrip() {
  return (
    <svg
      className="note-resize-grip"
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
      strokeLinecap="round"
      aria-hidden="true"
    >
      <path d="M10.5 2.5 2.5 10.5" />
      <path d="M10.5 5.5 5.5 10.5" />
      <path d="M10.5 8.5 8.5 10.5" />
    </svg>
  );
}
