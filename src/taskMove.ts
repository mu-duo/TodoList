/**
 * 任务跨清单转移的前端纯逻辑（无 React / DOM 依赖，便于独立断言验证）。
 *
 * 覆盖：
 * - 右键「移动到清单」子菜单的目标清单计算；
 * - 拖拽任务时对清单条项做指针命中测试，解析出有效落点；
 * - 源任务滚动区 / 「已完成」落点等矩形命中判定（含边界与容差）。
 */

/** 侧栏清单的最小引用（顺序即展示顺序）。 */
export type MoveListRef = { file: string; name: string };

/** 可转移的目标清单。 */
export type MoveTarget = { file: string; name: string };

/** 与 DOMRect 对齐的最小矩形信息。 */
export type RectLike = {
  left: number;
  top: number;
  right: number;
  bottom: number;
};

/** 指针坐标（视口坐标系）。 */
export type PointerPos = { x: number; y: number };

/**
 * 指针是否落在矩形内（含边界）；`margin` 向外扩张容差（px）。
 * 无指针（键盘拖拽）→ false。
 */
export function isPointInRect(
  pointer: PointerPos | null,
  rect: RectLike,
  margin = 0,
): boolean {
  if (!pointer) return false;
  return (
    pointer.x >= rect.left - margin &&
    pointer.x <= rect.right + margin &&
    pointer.y >= rect.top - margin &&
    pointer.y <= rect.bottom + margin
  );
}

/**
 * 右键子菜单的目标清单：除任务所在清单外的全部真实清单，保持侧栏顺序。
 * 「已完成」是虚拟聚合项，不在 `lists` 中，天然不会出现。
 */
export function moveTargetLists(
  lists: readonly MoveListRef[],
  sourceListFile: string,
): MoveTarget[] {
  return lists
    .filter((list) => list.file !== sourceListFile)
    .map((list) => ({ file: list.file, name: list.name }));
}

/**
 * 指针命中测试：返回指针所在清单的文件名；不在任何项内返回 null。
 * 传入顺序即优先级：若多项重叠（正常不会），取最先命中者。
 */
export function hitTestListFile(
  pointer: PointerPos,
  rects: ReadonlyArray<RectLike & { file: string }>,
): string | null {
  for (const rect of rects) {
    if (isPointInRect(pointer, rect)) {
      return rect.file;
    }
  }
  return null;
}

/**
 * 拖拽落点解析：只有命中某个清单项、且该清单不是源清单时才算有效转移目标。
 * 指针为空（如键盘拖拽）时返回 null，保持既有同清单排序行为。
 */
export function resolveDropTarget(
  pointer: PointerPos | null,
  rects: ReadonlyArray<RectLike & { file: string }>,
  sourceListFile: string,
): string | null {
  if (!pointer) return null;
  const hit = hitTestListFile(pointer, rects);
  if (!hit || hit === sourceListFile) return null;
  return hit;
}
