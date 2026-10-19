/**
 * 便签窗口创建管线。
 *
 * 契约见 docs/issues/feature/agent/便签功能.md：
 * - 标签 `note-<n>`（n 从 1 起，占用则跳过）；
 * - 颜色按成功创建次数在 `NOTE_COLOR_COUNT` 上取模轮换；
 * - 首个居中，其后相对「上一张便签窗口」右下 +28/+28 级联（逻辑坐标）；
 * - 创建失败原样抛出，由调用方（App Bar 动作）记入 actionError。
 */
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { NOTE_COLOR_COUNT } from "./noteColors";
import type { ThemePref } from "./theme";

/** 成功创建的颜色序号计数（仅成功创建后递增）。 */
let noteColorCounter = 0;
/** 最近一次成功创建的便签窗口标签，用于级联定位。 */
let lastNoteLabel: string | null = null;

/** 级联偏移（逻辑像素）。 */
const CASCADE_OFFSET = 28;

/** 找一个未被占用的 `note-<n>` 标签（n 从 1 起）。 */
async function allocateNoteLabel(): Promise<string> {
  for (let n = 1; ; n += 1) {
    const label = `note-${n}`;
    const existing = await WebviewWindow.getByLabel(label);
    if (!existing) return label;
  }
}

/**
 * 计算新窗口初始位置：上一张便签存在且读取成功 → 其右下 +CASCADE_OFFSET（逻辑坐标）；
 * 否则返回 null，由调用方回退 `center: true`。
 */
async function resolveCascadePosition(): Promise<{ x: number; y: number } | null> {
  if (!lastNoteLabel) return null;
  try {
    const prev = await WebviewWindow.getByLabel(lastNoteLabel);
    if (!prev) return null;
    const scaleFactor = await prev.scaleFactor();
    const logical = (await prev.outerPosition()).toLogical(scaleFactor);
    return { x: logical.x + CASCADE_OFFSET, y: logical.y + CASCADE_OFFSET };
  } catch {
    return null;
  }
}

/** 等待窗口创建结果：成功 resolve，失败 reject（原样抛出底层错误）。 */
function awaitCreation(win: WebviewWindow): Promise<void> {
  return new Promise<void>((resolve, reject) => {
    void win.once("tauri://created", () => {
      resolve();
    });
    void win.once("tauri://error", (event) => {
      reject(event.payload instanceof Error ? event.payload : new Error(String(event.payload)));
    });
  });
}

/** 串行化队列尾：保证多次调用按序执行，避免并行分配相同标签 / colorIndex。 */
let pending: Promise<void> = Promise.resolve();

/**
 * 新建一个便签窗口；失败原样抛出。
 *
 * 快速连点时按调用顺序串行：每次调用把「分配标签 / 位置 → 创建 → 计数递增」
 * 整段链到队列尾部；单次失败只影响该次返回的 Promise，队列继续可用。
 */
export function openNoteWindow(themePref: ThemePref): Promise<void> {
  const run = pending.then(() => createNoteWindow(themePref));
  pending = run.catch(() => {});
  return run;
}

async function createNoteWindow(themePref: ThemePref): Promise<void> {
  const colorIndex = noteColorCounter % NOTE_COLOR_COUNT;
  const label = await allocateNoteLabel();
  const position = await resolveCascadePosition();
  // 创建即带主题，避免首帧按 OS 明暗闪色；system 不传（保持跟随系统）。
  const themeOption =
    themePref === "system" ? {} : ({ theme: themePref } as const);

  const win = new WebviewWindow(label, {
    url: `/?note=${colorIndex}&theme=${themePref}`,
    title: "便签",
    width: 280,
    height: 320,
    ...(position ? { x: position.x, y: position.y } : { center: true }),
    ...themeOption,
    decorations: false,
    resizable: true,
    minWidth: 200,
    minHeight: 160,
    maximizable: false,
    shadow: true,
    alwaysOnTop: false,
  });

  await awaitCreation(win);

  // 仅成功创建后递增计数与记录级联锚点。
  noteColorCounter += 1;
  lastNoteLabel = label;
}
