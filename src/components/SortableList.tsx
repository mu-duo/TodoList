import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  PointerSensor,
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
  /** <li> 内部内容。 */
  renderItem: (id: string) => ReactNode;
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
}: SortableListProps) {
  const reduced = usePrefersReducedMotion();
  const [activeId, setActiveId] = useState<string | null>(null);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
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
              {renderItem(id)}
            </SortableRow>
          ))}
          {footer}
        </ul>
      </SortableContext>
      <DragOverlay dropAnimation={reduced ? null : undefined}>
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
  children: ReactNode;
};

function SortableRow({
  id,
  reduced,
  className,
  disabled,
  onContextMenu,
  children,
}: SortableRowProps) {
  const { listeners, setNodeRef, transform, transition, isDragging } = useSortable({
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
      {children}
    </li>
  );
}
