/**
 * 全局层叠层级（z-index）单一来源。
 *
 * 顺序（从小到大）：内容 < 右键菜单 < 拖拽浮层 < 窄屏抽屉 < 设置模态。
 * - 窄屏抽屉必须高于拖拽浮层，否则拖动时浮层会盖住抽屉。
 * - dnd-kit 的 DragOverlay 默认 z-index 是 999，会压过抽屉，故用 Z_LAYER.drag 显式覆盖。
 *
 * `src/App.css` 的 `--z-*` 变量与此保持一致（CSS 无法 import TS，数值需同步）。
 */
export const Z_LAYER = {
  /** 右键菜单 */
  menu: 100,
  /** 拖拽浮层（DragOverlay） */
  drag: 300,
  /** 窄屏任务详情抽屉 */
  drawer: 400,
  /** 设置模态 */
  modal: 500,
} as const;
