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
  type DragMoveEvent,
  type DragStartEvent,
  type DropAnimation,
} from "@dnd-kit/core";
import {
  SortableContext,
  horizontalListSortingStrategy,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent,
  type ReactNode,
} from "react";
import { usePrefersReducedMotion } from "../hooks";
import { Z_LAYER } from "../layers";

type Axis = "vertical" | "horizontal";

/**
 * 收缩态落下：仅 100ms 淡出，关键帧位移保持原地，绝不飞回源行。
 * dnd-kit 在 dragEnd 时会同步卸载浮层，`dropAnimation={null}` 会瞬间消失、
 * 留不出淡出时间，故用零位移关键帧实现「原地消失」。
 */
const COLLAPSED_DROP_ANIMATION: DropAnimation = {
  duration: 100,
  easing: "ease-out",
  sideEffects: null,
  keyframes: ({ transform }) => {
    const at = CSS.Transform.toString(transform.initial);
    return [
      { opacity: 1, transform: at },
      { opacity: 0, transform: at },
    ];
  },
};

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
  onDragStart?: (event: DragStartEvent) => void;
  /** 拖拽结束 / 取消回调：入参为被拖拽项的 id（取消时为 null）。 */
  onDragEnd?: (activeId: string | null) => void;
  /** 拖拽过程中的回调（用于指针命中测试等，如「拖到清单条」）。 */
  onDragMove?: (event: DragMoveEvent) => void;
  /**
   * 浮层是否处于「收缩为图标」状态（仅任务列表使用）。
   * 收缩态落下时不做飞回动画，改为 100ms 原地淡出。
   */
  overlayCollapsed?: boolean;
  /** 浮层内层容器的行内样式（承载抓取点 / 收缩比例等 CSS 变量）。 */
  overlayStyle?: CSSProperties;
  /** 整行右键回调（用于清单操作菜单）。 */
  onItemContextMenu?: (id: string, event: MouseEvent) => void;
  /** 单项禁用拖拽。 */
  itemDisabled?: (id: string) => boolean;
  /**
   * 每个可拖拽 <li> 的额外 data-* 属性（如清单文件映射 `data-list-file`），
   * 供拖拽命中测试等按 DOM 查询定位。固定 footer 项不带该属性。
   */
  rowDataAttributes?: (id: string) => Record<string, string> | undefined;
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
  onDragMove,
  overlayCollapsed = false,
  overlayStyle,
  onItemContextMenu,
  itemDisabled,
  rowDataAttributes,
  interaction = "pointer",
  keyboardDrag = true,
}: SortableListProps) {
  const reduced = usePrefersReducedMotion();
  const [activeId, setActiveId] = useState<string | null>(null);
  // 当前是否处于收缩态：落下时据此选择「原地淡出」而非默认飞回。
  // 必须随 overlayCollapsed 回落为 false，否则先经过清单、再拖回空白/源清单
  // 落下时仍会错误地走收缩淡出。
  const collapsedDropRef = useRef(false);
  useEffect(() => {
    collapsedDropRef.current = overlayCollapsed;
  }, [overlayCollapsed]);
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
          <li
            key={id}
            className={rowClassName(id)}
            {...rowDataAttributes?.(id)}
            onContextMenu={(event) => onItemContextMenu?.(id, event)}
          >
            {renderItem(id)}
          </li>
        ))}
        {footer}
      </ul>
    );
  }

  function handleDragStart(event: DragStartEvent) {
    collapsedDropRef.current = false;
    // 浮层 pointer-events:none 后，拖拽光标由 body 统一兜住（任务 / 清单 / AppBar 共用）
    document.body.classList.add("is-dragging");
    setActiveId(String(event.active.id));
    onDragStart?.(event);
  }

  function handleDragEnd(event: DragEndEvent) {
    const { active, over } = event;
    document.body.classList.remove("is-dragging");
    setActiveId(null);
    onDragEnd?.(String(active.id));
    if (over && active.id !== over.id) {
      onReorder?.(String(active.id), String(over.id));
    }
  }

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      onDragStart={handleDragStart}
      onDragMove={onDragMove}
      onDragEnd={handleDragEnd}
      onDragCancel={() => {
        document.body.classList.remove("is-dragging");
        setActiveId(null);
        onDragEnd?.(null);
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
              dataAttributes={rowDataAttributes?.(id)}
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
        style={{ pointerEvents: "none" }}
        dropAnimation={
          reduced
            ? null
            : collapsedDropRef.current
              ? COLLAPSED_DROP_ANIMATION
              : undefined
        }
      >
        {activeId && renderOverlay ? (
          <div
            className={`drag-overlay${overlayCollapsed ? " is-over-list" : ""}`}
            style={overlayStyle}
          >
            {renderOverlay(activeId)}
          </div>
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
  /** 额外的 data-* 属性（拖拽命中测试用）。 */
  dataAttributes?: Record<string, string>;
  onContextMenu?: (id: string, event: MouseEvent) => void;
  /** 渲染行内容；入参为「拖拽激活节点」ref，供键盘拖拽结束后归还焦点。 */
  children: (activatorRef: (element: HTMLElement | null) => void) => ReactNode;
};

function SortableRow({
  id,
  reduced,
  className,
  disabled,
  dataAttributes,
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
      {...dataAttributes}
      onContextMenu={(event) => onContextMenu?.(id, event)}
      {...listeners}
    >
      {children(setActivatorNodeRef)}
    </li>
  );
}
