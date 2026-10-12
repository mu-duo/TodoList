import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  MouseSensor,
  PointerSensor,
  TouchSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
  type DragStartEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  horizontalListSortingStrategy,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useState, type CSSProperties, type MouseEvent, type ReactNode } from "react";
import { usePrefersReducedMotion } from "../hooks";
import { Z_LAYER } from "../layers";

type Axis = "vertical" | "horizontal";

type SortableListProps = {
  /** 可排序项的 id（顺序即展示顺序）。 */
  ids: string[];
  axis?: Axis;
  /** 传给内部 <ul> 的类名（滚动 / 间距由调用方决定）。 */
  className?: string;
  /** false 时退化为普通列表（无拖拽），用于「已完成」等只读视图。 */
  sortable?: boolean;
  /** 每个 <li> 的类名（含选中 / 完成等状态）。 */
  rowClassName: (id: string) => string;
  /**
   * <li> 内部内容。第二个参数是可选的「拖拽激活节点」ref：
   * 绑定到实际的拖拽把手/按钮上，键盘拖拽结束会把焦点还给该节点
   * （否则 dnd-kit 会聚焦行内第一个可聚焦元素，任务行会落到勾选框）。
   */
  renderItem: (
    id: string,
    activatorRef?: (element: HTMLElement | null) => void,
  ) => ReactNode;
  /** 拖拽浮层内容（不提供则不显示浮层）。 */
  renderOverlay?: (id: string) => ReactNode;
  /** 固定尾项（如「已完成」），不可拖拽。 */
  footer?: ReactNode;
  onReorder?: (activeId: string, overId: string) => void;
  onDragStart?: () => void;
  onDragEnd?: () => void;
  /** 整行右键回调（用于清单操作菜单）。 */
  onItemContextMenu?: (id: string, event: MouseEvent) => void;
  /** 单项禁用拖拽。 */
  itemDisabled?: (id: string) => boolean;
  /**
   * 拖拽激活方式：
   * - `pointer`（默认，保持既有行为）：鼠标 / 触屏均为「移动超过阈值」触发。
   * - `mouseTouch`：鼠标移动超过阈值触发；触屏改为**长按**触发（避免与滚动竞争）。
   */
  interaction?: "pointer" | "mouseTouch";
  /**
   * 是否注册键盘拖拽（Space 拾起 / 放下）。默认 true。
   * 入口本身即按钮的场景（应用功能栏）设为 false，令 Space / Enter 都作原生点击。
   */
  keyboardDrag?: boolean;
};

/**
 * 可复用的列表容器：滚动区 + 行渲染 + dnd-kit 拖拽。
 * 任务列表与清单条共用同一套实现（轴向可切换）。
 */
export function SortableList({
  ids,
  axis = "vertical",
  className,
  sortable = true,
  rowClassName,
  renderItem,
  renderOverlay,
  footer,
  onReorder,
  onDragStart,
  onDragEnd,
  onItemContextMenu,
  itemDisabled,
  interaction = "pointer",
  keyboardDrag = true,
}: SortableListProps) {
  const reduced = usePrefersReducedMotion();
  const [activeId, setActiveId] = useState<string | null>(null);
  // 无条件建立各传感器描述符，按 interaction 选择注册，保持 hooks 调用顺序稳定。
  const pointerSensor = useSensor(PointerSensor, {
    activationConstraint: { distance: 6 },
  });
  const mouseSensor = useSensor(MouseSensor, {
    activationConstraint: { distance: 6 },
  });
  const touchSensor = useSensor(TouchSensor, {
    activationConstraint: { delay: 220, tolerance: 6 },
  });
  const keyboardSensor = useSensor(KeyboardSensor, {
    coordinateGetter: sortableKeyboardCoordinates,
    // 键盘拖拽只由 Space 激活/落定，Esc 取消。
    // dnd-kit 默认还含 Enter，会抢走行内按钮的原生回车（任务行＝打开详情），故在此收敛。
    keyboardCodes: {
      start: ["Space"],
      cancel: ["Escape"],
      end: ["Space"],
    },
  });
  const sensors = useSensors(
    ...(interaction === "mouseTouch"
      ? keyboardDrag
        ? [mouseSensor, touchSensor, keyboardSensor]
        : [mouseSensor, touchSensor]
      : keyboardDrag
        ? [pointerSensor, keyboardSensor]
        : [pointerSensor]),
  );

  const strategy =
    axis === "horizontal"
      ? horizontalListSortingStrategy
      : verticalListSortingStrategy;

  if (!sortable) {
    return (
      <ul className={className}>
        {ids.map((id) => (
          <li key={id} className={rowClassName(id)}>
            {renderItem(id)}
          </li>
        ))}
        {footer}
      </ul>
    );
  }

  function handleDragStart(event: DragStartEvent) {
    setActiveId(String(event.active.id));
    onDragStart?.();
  }

  function handleDragEnd(event: DragEndEvent) {
    const { active, over } = event;
    setActiveId(null);
    onDragEnd?.();
    if (over && active.id !== over.id) {
      onReorder?.(String(active.id), String(over.id));
    }
  }

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      onDragStart={handleDragStart}
      onDragEnd={handleDragEnd}
      onDragCancel={() => {
        setActiveId(null);
        onDragEnd?.();
      }}
    >
      <SortableContext items={ids} strategy={strategy}>
        <ul className={className}>
          {ids.map((id) => (
            <SortableRow
              key={id}
              id={id}
              reduced={reduced}
              className={rowClassName(id)}
              disabled={itemDisabled?.(id) ?? false}
              onContextMenu={onItemContextMenu}
            >
              {(activatorRef) => renderItem(id, activatorRef)}
            </SortableRow>
          ))}
          {footer}
        </ul>
      </SortableContext>
      <DragOverlay
        zIndex={Z_LAYER.drag}
        dropAnimation={reduced ? null : undefined}
      >
        {activeId && renderOverlay ? (
          <div className="drag-overlay">{renderOverlay(activeId)}</div>
        ) : null}
      </DragOverlay>
    </DndContext>
  );
}

type SortableRowProps = {
  id: string;
  reduced: boolean;
  className: string;
  disabled: boolean;
  onContextMenu?: (id: string, event: MouseEvent) => void;
  /** 渲染行内容；入参为「拖拽激活节点」ref，供键盘拖拽结束后归还焦点。 */
  children: (activatorRef: (element: HTMLElement | null) => void) => ReactNode;
};

function SortableRow({
  id,
  reduced,
  className,
  disabled,
  onContextMenu,
  children,
}: SortableRowProps) {
  const {
    listeners,
    setNodeRef,
    setActivatorNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({
    id,
    disabled,
  });

  const style: CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition: reduced ? undefined : transition,
  };

  return (
    <li
      ref={setNodeRef}
      style={style}
      className={`${className}${isDragging ? " is-dragging" : ""}`}
      onContextMenu={(event) => onContextMenu?.(id, event)}
      {...listeners}
    >
      {children(setActivatorNodeRef)}
    </li>
  );
}
