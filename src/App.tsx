import { useEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { DragMoveEvent, DragStartEvent } from "@dnd-kit/core";
import { arrayMove } from "@dnd-kit/sortable";
import { SortableList } from "./components/SortableList";
import { ContextMenu, type MenuItem } from "./components/ContextMenu";
import {
  isPointInRect,
  moveTargetLists,
  resolveDropTarget,
  type RectLike,
} from "./taskMove";
import { SettingsModal } from "./components/SettingsModal";
import { ConfirmModal } from "./components/ConfirmModal";
import { AppBar } from "./components/AppBar";
import { AgentConfigModal } from "./components/AgentConfigModal";
import { AgentTaskPanel } from "./components/AgentTaskPanel";
import { MaterialsPanel } from "./components/MaterialsPanel";
import { DetailSection } from "./components/DetailSection";
import { applyThemePref, type ThemePref } from "./theme";
import { openNoteWindow } from "./noteWindow";
import { materialLinkError, normalizeMaterialLink } from "./materialLink";
import { loadSettings } from "./settings";
import {
  getAgentStatus,
  listAgentRuns,
  loadAgentConfig,
  openAgentResultDir,
  openExternal,
  pickAgentPaths,
  setAgentTaskIntent,
  startAgentTask,
  stopAgentTask,
  subscribeAgentState,
  type AgentStateEvent,
  type RunSummary,
} from "./agent";
import {
  agentMenuModel,
  agentStateMeta,
  agentStateTitle,
  hasMoreRuns,
  isAgentActive,
  latestRun,
  mergeRuns,
  rowBadgeState,
  sortRunsDesc,
  taskKey,
} from "./agentTask";
import {
  APP_BAR_ENTRIES,
  DEFAULT_APP_BAR_ORDER,
  hasSavedAppBarOrder,
  mergeAppBarOrder,
  resolveAppBarOrder,
  visibleAppBarIds,
  type AppBarEntryMeta,
} from "./appBar";
import {
  activeCollapsedFiles,
  bucketCompletedGroups,
  buildCompletedGroups,
  nextCollapsed,
  sameCollapsed,
  totalVisibleCompleted,
} from "./completedGroups";
import {
  DEFAULT_DETAIL_COLLAPSED,
  nextDetailCollapsed,
  sameDetailCollapsed,
  type DetailSectionKey,
} from "./detailSections";
import { useMediaQuery, usePrefersReducedMotion } from "./hooks";
import {
  buildTrashTaskGroups,
  formatTrashRemaining,
  trashEntryCount,
  visibleTrashLists,
  visibleTrashTasks,
  type TrashListRef,
  type TrashTaskRef,
  type TrashView,
} from "./trash";
import "./App.css";

type Subtask = {
  id: string;
  title: string;
  status: string;
};

type Task = {
  id: string;
  title: string;
  status: string;
  completed: string | null;
  priority: string | null;
  due: string | null;
  tags: string[] | null;
  reminder?: false | null;
  /**
   * 归档时间（本地时间字符串，`FR-APP-36`）；字段存在即已归档，删除该键即取消归档。
   * 只对 `status: "done"` 有意义；旧数据缺失 → 未归档。
   */
  archived?: string | null;
  notes: string | null;
  subtasks: Subtask[] | null;
  /**
   * 任务级 Agent 意图（启用状态 + 关联原路径 + 关联链接）；由后端 `load_lists` 返回。
   * 旧数据可能没有 `links`，读取处一律按 `?? []` 处理。
   */
  agent?: { enabled: boolean; paths: string[]; links?: string[] } | null;
};

type TaskList = {
  name: string;
  file: string;
  tasks: Task[];
};

/**
 * 回收站条目的 `task` 实际是后端序列化的完整任务（含优先级 / 截止 / 标签 / 提醒）；
 * `trash.ts` 只声明了 UI 行渲染用到的子集，详情只读展示需要更完整的字段。
 */
type TrashTaskFull = Omit<TrashTaskRef, "task"> & { task: Task };

type LoadResult = {
  lists: TaskList[];
  warnings: string[];
};

/** 视图中的一条任务（含其来源清单）。 */
type ViewTask = { task: Task; listFile: string; listName?: string };

/** 任务浮层收缩几何：抓取点相对源行左上角，以及源行实际宽高。 */
type DragMetrics = {
  grabX: number;
  grabY: number;
  width: number;
  height: number;
};

/** 激活事件里可能携带的指针坐标（鼠标 / 触摸）。 */
type ActivatorPointerEvent = {
  clientX?: number;
  clientY?: number;
  touches?: ArrayLike<{ clientX: number; clientY: number }>;
};

const PRIORITY_LABEL: Record<string, string> = {
  high: "高",
  medium: "中",
  low: "低",
};

/** 「已完成」虚拟清单的哨兵 id（非真实文件）。 */
const COMPLETED_ID = "__completed__";

/** 「已删除」虚拟清单的哨兵 id（非真实文件）。 */
const TRASH_ID = "__trash__";

/** 回收站倒计时 / 过期清理的刷新节奏。 */
const TRASH_TICK_MS = 30_000;
const TRASH_SWEEP_MS = 60_000;

/** 详情 / 行尾共用的运行态快照（来自事件 / 状态 / 历史）。 */
type AgentLiveState = {
  runId: string;
  state: string;
  attempt: number;
  errorClass?: string | null;
  /** 停止原因（`state == stopped`）：manual|disabled|completed|deleted。 */
  stopReason?: string | null;
  resultDir?: string | null;
};

/** 任务右键菜单的位置与目标。 */
type TaskMenuState = {
  listFile: string;
  taskId: string;
  x: number;
  y: number;
};

/** 运行历史每页条数。 */
const AGENT_RUNS_PAGE = 50;

function App() {
  const [lists, setLists] = useState<TaskList[]>([]);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  // 「关联材料」板块内的添加错误（就地展示，不飘到全局横幅）
  const [materialsError, setMaterialsError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // 窗口置顶（pin）
  const [pinned, setPinned] = useState(false);
  const [pinBusy, setPinBusy] = useState(false);

  // 设置面板（模态）与主题偏好
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [theme, setTheme] = useState<ThemePref>("system");
  // 「关闭时最小化到托盘」；null = 默认开启
  const [minimizeToTray, setMinimizeToTray] = useState<boolean | null>(null);
  // Agent 配置模态（Phase 1）
  const [agentConfigOpen, setAgentConfigOpen] = useState(false);

  // 待确认删除的清单（非空即显示确认模态）
  const [pendingDeleteList, setPendingDeleteList] = useState<TaskList | null>(
    null,
  );

  // 有便签时的退出确认（便签数；null = 模态关闭）
  const [exitNoteCount, setExitNoteCount] = useState<number | null>(null);

  // 应用功能栏（App Bar）：顺序（含未知槽位）、最后成功顺序、保存态与错误
  const [appBarOrder, setAppBarOrder] = useState<string[]>(() => [
    ...DEFAULT_APP_BAR_ORDER,
  ]);
  const [appBarSaved, setAppBarSaved] = useState<string[] | null>(null);
  const [appBarSaving, setAppBarSaving] = useState(false);
  const [appBarError, setAppBarError] = useState<string | null>(null);
  // 同步互斥：同一渲染内快速连调用时，仅靠 state 守卫会漏；ref 在 finally 释放
  const appBarSavingRef = useRef(false);
  // 手动刷新按钮的忙碌态（入口忙碌时禁用并可反馈）
  const [refreshing, setRefreshing] = useState(false);

  // 「已完成」视图：折叠的清单文件名集合（键 = 清单文件；空 = 全展开）
  const [completedCollapsed, setCompletedCollapsed] = useState<string[]>([]);
  const [completedCollapsedError, setCompletedCollapsedError] = useState<
    string | null
  >(null);
  // 当前期望集合（供串行保存与失败回滚读取最新值）
  const completedCollapsedRef = useRef<string[]>([]);
  // 最近一次成功落盘的集合（失败时回滚目标）
  const completedCollapsedSavedRef = useRef<string[]>([]);
  // 串行写盘链：保证「始终保存最新完整集合」，避免并发乱序覆盖
  const completedSaveChainRef = useRef<Promise<void>>(Promise.resolve());

  // 「已完成」视图是否渲染已归档任务（本机偏好；默认关）
  const [completedShowArchived, setCompletedShowArchived] = useState(false);
  const [completedShowArchivedError, setCompletedShowArchivedError] = useState<
    string | null
  >(null);
  // 当前期望值 / 最近一次成功落盘值 / 串行写盘链（与折叠集合同款）
  const completedShowArchivedRef = useRef(false);
  const completedShowArchivedSavedRef = useRef(false);
  const showArchivedSaveChainRef = useRef<Promise<void>>(Promise.resolve());

  // 归档撤销提示：单槽位，新的归档覆盖旧的；5 秒后自隐
  const [archiveToast, setArchiveToast] = useState<{
    listFile: string;
    taskIds: string[];
  } | null>(null);
  const archiveToastTimerRef = useRef<number | null>(null);

  // ---- 「已删除」回收站（FR-APP-37）----
  // 后端视图快照（含已过期条目；展示前再按 now 过滤）
  const [trashView, setTrashView] = useState<TrashView>({ tasks: [], lists: [] });
  const [trashError, setTrashError] = useState<string | null>(null);
  // 倒计时基准：进入回收站后每 30 秒推进
  const [trashNow, setTrashNow] = useState<Date>(() => new Date());
  // 恢复中的条目 id（防重复点击；ref 供异步回调同步判断）
  const [restoringIds, setRestoringIds] = useState<Set<string>>(new Set());
  const restoringIdsRef = useRef<Set<string>>(new Set());
  // 彻底删除中的条目 id（与恢复互斥判断共用一份「在途」语义，但单独记录便于禁用）
  const [purgingIds, setPurgingIds] = useState<Set<string>>(new Set());
  const purgingIdsRef = useRef<Set<string>>(new Set());
  // 回收站右键菜单（任务行 / 清单行共用同一份状态；与普通任务菜单相互独立）
  const [trashMenu, setTrashMenu] = useState<{
    trashId: string;
    title: string;
    kind: "task" | "list";
    taskCount?: number;
    x: number;
    y: number;
  } | null>(null);
  // 待确认彻底删除的条目（非空即显示确认模态）
  const [purgeTarget, setPurgeTarget] = useState<{
    trashId: string;
    title: string;
    kind: "task" | "list";
    taskCount?: number;
  } | null>(null);
  // 「已删除」视图折叠的来源清单文件名集合（键 = 清单文件；空 = 全展开）
  const [trashCollapsed, setTrashCollapsed] = useState<string[]>([]);
  const [trashCollapsedError, setTrashCollapsedError] = useState<string | null>(
    null,
  );
  // 当前期望集合 / 最近一次成功落盘的集合 / 串行写盘链（与已完成折叠同款）
  const trashCollapsedRef = useRef<string[]>([]);
  const trashCollapsedSavedRef = useRef<string[]>([]);
  const trashSaveChainRef = useRef<Promise<void>>(Promise.resolve());
  // 「选择恢复到的清单」模态
  const [restoreTarget, setRestoreTarget] = useState<{
    trashId: string;
    title: string;
  } | null>(null);
  const [restoreTargetFile, setRestoreTargetFile] = useState("");
  // 删除任务后的轻量提示（独立单槽，不影响归档撤销提示）
  const [deleteToast, setDeleteToast] = useState(false);
  const deleteToastTimerRef = useRef<number | null>(null);

  // 任务详情子板块折叠集合（key 见 detailSections；空数组 = 全部展开，合法）
  const [detailCollapsed, setDetailCollapsed] = useState<string[]>([]);
  const [detailCollapsedError, setDetailCollapsedError] = useState<
    string | null
  >(null);
  // 当前期望集合（供串行保存与失败回滚读取最新值）
  const detailCollapsedRef = useRef<string[]>([]);
  // 最近一次成功落盘的集合（失败时回滚目标）
  const detailCollapsedSavedRef = useRef<string[]>([]);
  // 串行写盘链：与已完成分组同款，避免并发乱序覆盖
  const detailSaveChainRef = useRef<Promise<void>>(Promise.resolve());

  // ---- Agent 任务 UI（Phase 2）----
  // 任务键 → 已知运行态（事件 / 状态轮询 / 历史最新）
  const [agentStates, setAgentStates] = useState<Map<string, AgentLiveState>>(
    new Map(),
  );
  // 任务右键菜单（位置 + 目标）
  const [taskMenu, setTaskMenu] = useState<TaskMenuState | null>(null);
  // 选中任务的运行历史
  const [agentRuns, setAgentRuns] = useState<RunSummary[]>([]);
  const [agentRunsLoading, setAgentRunsLoading] = useState(false);
  const [agentRunsHasMore, setAgentRunsHasMore] = useState(false);
  // 启动 / 停止在途的任务键
  const [agentBusyKeys, setAgentBusyKeys] = useState<Set<string>>(new Set());
  // 启用前置不足时的引导提示（随 Agent 配置模态展示）
  const [agentGateNotice, setAgentGateNotice] = useState<string | null>(null);
  // 点击行尾图标后定位到详情 Agent 区
  const [agentFocusTick, setAgentFocusTick] = useState(0);

  const agentStatesRef = useRef(agentStates);
  agentStatesRef.current = agentStates;
  const agentBusyRef = useRef<Set<string>>(new Set());
  // 已尝试拉取过最新状态的任务键（避免每次视图变化重复拉取）
  const seededAgentKeysRef = useRef<Set<string>>(new Set());
  const agentSectionRef = useRef<HTMLElement>(null);
  const agentEventRef = useRef<((event: AgentStateEvent) => void) | null>(null);
  const agentRunsRefetchRef = useRef<number | null>(null);

  const [newListName, setNewListName] = useState("");
  const [newTaskTitles, setNewTaskTitles] = useState<Record<string, string>>({});

  // 当前选中的清单（可为真实清单文件或「已完成」哨兵）/ 任务
  const [selectedListFile, setSelectedListFile] = useState<string | null>(null);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  // 窄屏下任务详情以抽屉形式从右侧滑出
  const [drawerOpen, setDrawerOpen] = useState(false);
  // 正在播放「移除动画」的任务 id → 目标完成态
  const [leaving, setLeaving] = useState<Map<string, boolean>>(new Map());
  // 清单右键菜单位置
  const [menu, setMenu] = useState<{ file: string; x: number; y: number } | null>(
    null,
  );
  // 「已完成」来源清单组头的右键菜单位置（与侧栏清单菜单相互独立）
  const [completedGroupMenu, setCompletedGroupMenu] = useState<{
    listFile: string;
    x: number;
    y: number;
  } | null>(null);

  // 任务详情面板的编辑草稿
  const [editTitle, setEditTitle] = useState("");
  const [editNotes, setEditNotes] = useState("");
  // Metadata patches share one YAML read-modify-write path: serialize per app session.
  const metadataQueueRef = useRef<Promise<boolean>>(Promise.resolve(true));

  // 详情即时保存：输入值镜像到 ref，供防抖与 flush 读取最新值
  const editTitleRef = useRef("");
  const editNotesRef = useRef("");
  const saveTimerRef = useRef<number | null>(null);
  // 同一错误去重/节流：记住上一次已提示的保存错误
  const lastSaveErrorRef = useRef<string | null>(null);

  // ---- 子任务编辑（独立即时写盘通道，无防抖）----
  // 本地镜像：增 / 删 / 勾选成功后按命令返回值就地更新，不做整表刷新
  const [subtasks, setSubtasks] = useState<Subtask[]>([]);
  // 就地录入框常存的草稿（失焦保留、关详情 / 切任务丢弃）
  const [subtaskDraft, setSubtaskDraft] = useState("");
  // 正在等待落盘的子任务 id（禁用重复触发 + 视觉降透明度）
  const [subtaskBusyIds, setSubtaskBusyIds] = useState<Set<string>>(new Set());
  const subtaskBusyRef = useRef<Set<string>>(new Set());
  const addSubtaskPendingRef = useRef(false);
  const subtaskInputRef = useRef<HTMLInputElement>(null);
  // 子任务错误单独去重，避免与标题/备注保存通道互相干扰
  const lastSubtaskErrorRef = useRef<string | null>(null);

  // 子任务改名（双击进入编辑态）：正在编辑的 id、草稿、输入框、防重复触发
  const [editingSubtaskId, setEditingSubtaskId] = useState<string | null>(null);
  const [subtaskEditDraft, setSubtaskEditDraft] = useState("");
  const subtaskEditRef = useRef<HTMLInputElement>(null);
  // 本轮编辑是否已被处理（Enter/Esc/失焦只认一次，避免错误二次提交）
  const subtaskEditHandledRef = useRef(false);
  // 最新编辑 id 的镜像：异步收尾时判断是否已切走，避免关掉后一个编辑态
  const editingSubtaskIdRef = useRef<string | null>(null);
  editingSubtaskIdRef.current = editingSubtaskId;

  // 任务内容行内编辑（双击 / 右键「编辑」）：正在编辑的任务 id、草稿、输入框、防双提交
  const [editingTaskId, setEditingTaskId] = useState<string | null>(null);
  const [taskEditDraft, setTaskEditDraft] = useState("");
  const taskEditRef = useRef<HTMLInputElement>(null);
  // 本轮编辑是否已被处理（Enter/Esc/失焦只认一次，避免错误二次提交）
  const taskEditHandledRef = useRef(false);
  // 最新编辑 id 的镜像：异步收尾时判断是否已切走
  const editingTaskIdRef = useRef<string | null>(null);
  editingTaskIdRef.current = editingTaskId;
  // 行内编辑来源清单（提交时用；仅事件读写，不触发渲染）
  const editingTaskListFileRef = useRef<string | null>(null);

  const [editingListFile, setEditingListFile] = useState<string | null>(null);
  const [editListName, setEditListName] = useState("");

  // 拖拽结束后的「点击抑制」：避免松手时误触发选中
  const suppressClickRef = useRef(false);

  // 选中任务 id 的镜像：异步收尾（如跨清单移动）需读取最新选中，避免过期闭包
  const selectedTaskIdRef = useRef<string | null>(null);
  selectedTaskIdRef.current = selectedTaskId;

  // ---- 任务拖拽到「清单条」：目标高亮 + 落点判定 ----
  // 高亮的目标清单文件（null = 无有效目标）；state 驱动行类名，ref 供事件回调同步读取
  const [dragDropTarget, setDragDropTarget] = useState<string | null>(null);
  const dragDropTargetRef = useRef<string | null>(null);
  // 指针是否已离开源任务滚动区：浮层据此收缩为图标，露出清单名与高亮
  const [dragOutsideSource, setDragOutsideSource] = useState(false);
  const dragOutsideSourceRef = useRef(false);
  // 指针是否落在「已完成」固定项上：松手等同勾选完成
  const [dragOverCompleted, setDragOverCompleted] = useState(false);
  const dragOverCompletedRef = useRef(false);
  // 源任务滚动区 / 「已完成」固定项（拖拽命中测试）
  const taskScrollRef = useRef<HTMLDivElement | null>(null);
  const completedFooterRef = useRef<HTMLLIElement | null>(null);
  // 本次任务拖拽的抓取点 / 源行实际宽高（收缩动画的 transform-origin 与非等比比例）
  const [dragMetrics, setDragMetrics] = useState<DragMetrics | null>(null);
  // 本次拖拽是否已按「跨清单转移 / 勾选完成」处理（用于压掉随后的同清单排序回调）
  const dragMoveHandledRef = useRef(false);

  // 详情由键盘打开（click.detail===0）→ 关闭时保留焦点；指针打开 → 清焦点环
  const openedByKeyboardRef = useRef(true);
  // 置顶前的最小化 / 缩放 / 最大化快照（用于逐条还原）
  const pinSnapshotRef = useRef<{
    minimizable: boolean;
    resizable: boolean;
    maximizable: boolean;
  } | null>(null);

  const isNarrow = useMediaQuery("(max-width: 760px)");
  const reduceMotion = usePrefersReducedMotion();

  const isCompletedView = selectedListFile === COMPLETED_ID;
  const isTrashView = selectedListFile === TRASH_ID;
  const selectedList = lists.find((l) => l.file === selectedListFile) ?? null;

  // 聚合所有清单的已完成任务（「已完成」虚拟视图）
  const completedTasks = useMemo<ViewTask[]>(
    () =>
      lists.flatMap((list) =>
        list.tasks
          .filter((task) => task.status === "done")
          .map((task) => ({ task, listFile: list.file, listName: list.name })),
      ),
    [lists],
  );

  // 「已完成」视图按来源清单分组（组顺序 = 侧栏清单顺序；空组不显示）
  const completedGroups = useMemo(
    () => buildCompletedGroups(lists, completedTasks),
    [lists, completedTasks],
  );

  // 组内再按完成时间分「今天 / 七天内 / 更早」；归档项按开关决定是否参与。
  // 时间基准只在进入视图 / 数据变化时重算（不加午夜定时器）。
  const completedBucketedGroups = useMemo(
    () => bucketCompletedGroups(completedGroups, completedShowArchived, new Date()),
    // 进入已完成视图时重算一次，跨天后档位不会停在旧值
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [completedGroups, completedShowArchived, isCompletedView],
  );

  // 侧栏与组头共用同一口径：只统计当前渲染的可见项
  const completedVisibleCount = useMemo(
    () => totalVisibleCompleted(completedBucketedGroups),
    [completedBucketedGroups],
  );

  // ---- 回收站派生数据（可见项 = 未过期，按删除时间倒序）----
  const trashTasksVisible = useMemo(
    () => visibleTrashTasks(trashView, trashNow),
    [trashView, trashNow],
  );
  const trashListsVisible = useMemo(
    () => visibleTrashLists(trashView, trashNow),
    [trashView, trashNow],
  );
  const trashCount = useMemo(
    () => trashEntryCount(trashView, trashNow),
    [trashView, trashNow],
  );
  // 任务按来源清单分组：现有清单顺序优先，原清单已不存在的追加在后
  const trashTaskGroups = useMemo(
    () => buildTrashTaskGroups(lists, trashTasksVisible),
    [lists, trashTasksVisible],
  );

  // 当前生效的折叠集合（过滤重命名 / 删除后的失效键，静默忽略）
  const collapsedCompletedSet = useMemo(
    () => new Set(activeCollapsedFiles(completedCollapsed, lists)),
    [completedCollapsed, lists],
  );

  // 回收站折叠集合：原清单可能已不存在，不能按 `lists` 过滤；失效键静默忽略
  const collapsedTrashSet = useMemo(
    () => new Set(trashCollapsed),
    [trashCollapsed],
  );

  // 详情面板中收起的子板块集合（顺序无关；未知 key 已在读取时过滤）
  const collapsedSectionSet = useMemo(
    () => new Set(detailCollapsed),
    [detailCollapsed],
  );

  // 当前视图的任务集合
  const viewTasks = useMemo<ViewTask[]>(() => {
    if (isCompletedView) return completedTasks;
    if (!selectedList) return [];
    return selectedList.tasks
      .filter((task) => task.status !== "done")
      .map((task) => ({ task, listFile: selectedList.file }));
  }, [isCompletedView, completedTasks, selectedList]);

  const selectedEntry = useMemo<ViewTask | null>(() => {
    if (!selectedTaskId) return null;
    if (isCompletedView) {
      return completedTasks.find((e) => e.task.id === selectedTaskId) ?? null;
    }
    const task = selectedList?.tasks.find((t) => t.id === selectedTaskId);
    return task && selectedList ? { task, listFile: selectedList.file } : null;
  }, [isCompletedView, completedTasks, selectedList, selectedTaskId]);

  // 回收站选中项：按 trash_id 匹配当前可见条目（条目消失即视为未选中）
  const selectedTrashEntry = useMemo<TrashTaskFull | null>(() => {
    if (!isTrashView || !selectedTaskId) return null;
    const found = trashTasksVisible.find((t) => t.trash_id === selectedTaskId);
    return (found as TrashTaskFull | undefined) ?? null;
  }, [isTrashView, trashTasksVisible, selectedTaskId]);
  const selectedTask = selectedEntry?.task ?? null;
  const selectedTaskListFile = selectedEntry?.listFile ?? null;

  // 选中任务在 Agent 状态表中的键与快照
  const selectedAgentKey =
    selectedTask && selectedTaskListFile
      ? taskKey(selectedTaskListFile, selectedTask.id)
      : null;
  const selectedAgentLive = selectedAgentKey
    ? agentStates.get(selectedAgentKey)
    : undefined;
  const selectedAgentKeyRef = useRef<string | null>(null);
  selectedAgentKeyRef.current = selectedAgentKey;

  // 磁盘侧子任务的「指纹」：选中任务切换、或外部/刷新/本地改动子任务时重新同步本地镜像
  const selectedSubtasksKey = JSON.stringify(selectedTask?.subtasks ?? null);
  // 切换任务：重置录入草稿与改名编辑态（仅在换任务时，避免本地改动子任务时误清空）
  useEffect(() => {
    setSubtaskDraft("");
    setEditingSubtaskId(null);
    setSubtaskEditDraft("");
    setMaterialsError(null);
    subtaskEditHandledRef.current = true;
  }, [selectedTaskId]);

  // 子任务数据变化（详情内增 / 删 / 勾 / 改名，或外部刷新）→ 同步本地镜像
  useEffect(() => {
    setSubtasks(selectedTask?.subtasks ?? []);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedTaskId, selectedSubtasksKey]);

  // 改名框出现即聚焦并全选（双击进入编辑态）
  useEffect(() => {
    if (editingSubtaskId) {
      const el = subtaskEditRef.current;
      if (el) {
        el.focus();
        el.select();
      }
    }
  }, [editingSubtaskId]);

  // 任务行内编辑框出现即聚焦，光标置尾（不全选；与子任务的有意差异）
  useEffect(() => {
    if (editingTaskId) {
      const el = taskEditRef.current;
      if (el) {
        el.focus();
        const len = el.value.length;
        el.setSelectionRange(len, len);
      }
    }
  }, [editingTaskId]);

  const listIds = useMemo(() => lists.map((l) => l.file), [lists]);
  const taskIds = useMemo(() => viewTasks.map((e) => e.task.id), [viewTasks]);
  const entryById = useMemo(
    () => new Map(viewTasks.map((e) => [e.task.id, e])),
    [viewTasks],
  );
  const listById = useMemo(() => new Map(lists.map((l) => [l.file, l])), [lists]);

  // 应用功能栏可见入口（仅已知 id，按当前顺序）
  const appBarEntries = useMemo<AppBarEntryMeta[]>(() => {
    return visibleAppBarIds(appBarOrder)
      .map((id) => APP_BAR_ENTRIES.find((e) => e.id === id))
      .filter((e): e is AppBarEntryMeta => !!e);
  }, [appBarOrder]);

  async function refresh() {
    // 清单与回收站并行读取：回收站失败只提示自身，不影响清单的成功 / 告警
    const [listsResult, trashResult] = await Promise.allSettled([
      invoke<LoadResult>("load_lists"),
      invoke<TrashView>("load_trash"),
    ]);

    if (trashResult.status === "fulfilled") {
      setTrashView({
        tasks: trashResult.value?.tasks ?? [],
        lists: trashResult.value?.lists ?? [],
      });
      setTrashError(null);
    } else {
      setTrashError(String(trashResult.reason));
    }

    if (listsResult.status === "fulfilled") {
      setLists(listsResult.value.lists);
      setWarnings(listsResult.value.warnings);
      setError(null);
    } else {
      setError(String(listsResult.reason));
      // 硬失败时清空可能过期的告警
      setWarnings([]);
    }
    setLoading(false);
  }

  // 最新 refresh 的镜像：供回收站轮询定时器调用，避免因函数重建而重启 interval
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  // ---- Agent 任务 UI（Phase 2）：状态同步 / 历史 / 意图 / 启动停止 ----

  function markAgentBusy(key: string, busy: boolean) {
    const set = agentBusyRef.current;
    if (busy) set.add(key);
    else set.delete(key);
    setAgentBusyKeys(new Set(set));
  }

  // 用一条运行记录播种最新状态；不覆盖「进行中」的活状态
  function seedLatestFromRun(key: string, run: RunSummary) {
    setAgentStates((prev) => {
      const cur = prev.get(key);
      if (cur && isAgentActive(cur.state)) return prev;
      const next = new Map(prev);
      next.set(key, {
        runId: run.run_id,
        state: run.state,
        attempt: run.attempt,
        errorClass: run.error_class ?? null,
        stopReason: run.stop_reason ?? null,
        resultDir: run.result_dir || null,
      });
      return next;
    });
  }

  // 兜底：全局状态（active + queue）
  async function refreshAgentStatus() {
    try {
      const status = await getAgentStatus();
      setAgentStates((prev) => {
        const next = new Map(prev);
        if (status.active) {
          const a = status.active;
          const k = taskKey(a.list_file, a.task_id);
          next.set(k, {
            runId: a.run_id,
            state: a.state,
            attempt: a.attempt,
            errorClass: a.error_class ?? null,
            stopReason: a.stop_reason ?? null,
            resultDir: a.result_dir || next.get(k)?.resultDir || null,
          });
        }
        // 队列项＝待执行（后端 RunRef 不含 state 字段）
        for (const q of status.queue) {
          const k = taskKey(q.list_file, q.task_id);
          next.set(k, {
            runId: q.run_id,
            state: "pending",
            attempt: q.attempt,
            errorClass: null,
            stopReason: next.get(k)?.stopReason ?? null,
            resultDir: next.get(k)?.resultDir ?? null,
          });
        }
        return next;
      });
    } catch {
      // 状态不可用时保持现状（不刷屏）
    }
  }

  async function reloadSelectedRuns() {
    const task = selectedTask;
    const listFile = selectedTaskListFile;
    if (!task || !listFile) return;
    try {
      const runs = await listAgentRuns(listFile, task.id, 0, AGENT_RUNS_PAGE);
      setAgentRuns(sortRunsDesc(runs));
      setAgentRunsHasMore(hasMoreRuns(runs, AGENT_RUNS_PAGE));
      const top = latestRun(runs);
      if (top) seedLatestFromRun(taskKey(listFile, task.id), top);
    } catch {
      // 忽略
    }
  }

  // 防抖重取选中任务历史；按任务键校验，避免 400ms 窗口内切换任务时用旧闭包刷新
  function scheduleReloadSelectedRuns(key: string) {
    if (agentRunsRefetchRef.current !== null) {
      window.clearTimeout(agentRunsRefetchRef.current);
    }
    agentRunsRefetchRef.current = window.setTimeout(() => {
      agentRunsRefetchRef.current = null;
      if (selectedAgentKeyRef.current !== key) return; // 选中已切换：丢弃过期刷新
      void reloadSelectedRuns();
    }, 400);
  }

  function onAgentEvent(event: AgentStateEvent) {
    const key = taskKey(event.listFile, event.taskId);
    setAgentStates((prev) => {
      const cur = prev.get(key);
      const next = new Map(prev);
      next.set(key, {
        runId: event.runId,
        state: event.state,
        attempt: event.attempt,
        // 事件发 camelCase errorClass / stopReason（后端不发送 error_class）
        errorClass: event.errorClass ?? cur?.errorClass ?? null,
        stopReason: event.stopReason ?? cur?.stopReason ?? null,
        resultDir: cur?.resultDir ?? null,
      });
      return next;
    });
    if (selectedAgentKeyRef.current === key) scheduleReloadSelectedRuns(key);
  }
  agentEventRef.current = onAgentEvent;

  // 订阅 agent://state（一次；载荷无密钥）
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    subscribeAgentState((event) => agentEventRef.current?.(event))
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // 订阅失败：靠状态 / 历史兜底
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // 订阅 reminder-disabled（通知按钮关闭提醒后回写）→ 刷新列表与选中详情
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen<{ listFile: string; taskId: string }>("reminder-disabled", () => {
      void refresh();
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // 订阅失败：静默降级（手动刷新仍可用）
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 订阅 exit-requested-with-notes（有便签时主窗关闭 / 托盘「退出」）→ 打开退出确认模态
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen<{ noteCount: number }>("exit-requested-with-notes", (event) => {
      setExitNoteCount(event.payload.noteCount);
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // 订阅失败：静默降级
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 挂载后兜底拉取全局状态
  useEffect(() => {
    void refreshAgentStatus();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 当前视图内启用中、且尚未获知状态的任务 → 拉最新一次运行做行尾着色
  const agentSeedSignature = useMemo(
    () =>
      viewTasks
        .filter((e) => e.task.agent?.enabled)
        .map((e) => taskKey(e.listFile, e.task.id))
        .join("|"),
    [viewTasks],
  );
  useEffect(() => {
    const pending = viewTasks.filter((entry) => {
      if (!entry.task.agent?.enabled) return false;
      const key = taskKey(entry.listFile, entry.task.id);
      if (agentStatesRef.current.has(key)) return false;
      if (seededAgentKeysRef.current.has(key)) return false;
      return true;
    });
    if (pending.length === 0) return;
    let cancelled = false;
    void (async () => {
      for (const entry of pending) {
        const key = taskKey(entry.listFile, entry.task.id);
        seededAgentKeysRef.current.add(key);
        try {
          const runs = await listAgentRuns(entry.listFile, entry.task.id, 0, 1);
          if (cancelled) return;
          const top = latestRun(runs);
          if (top) seedLatestFromRun(key, top);
        } catch {
          // 忽略：保持中性
        }
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentSeedSignature]);

  // 选中任务变化 → 拉取历史首页
  useEffect(() => {
    // 切换选中即作废未触发的防抖刷新（避免用旧闭包刷新历史）
    if (agentRunsRefetchRef.current !== null) {
      window.clearTimeout(agentRunsRefetchRef.current);
      agentRunsRefetchRef.current = null;
    }
    setAgentRuns([]);
    setAgentRunsHasMore(false);
    const task = selectedEntry?.task ?? null;
    const listFile = selectedEntry?.listFile ?? null;
    if (!task || !listFile) return;
    let cancelled = false;
    setAgentRunsLoading(true);
    listAgentRuns(listFile, task.id, 0, AGENT_RUNS_PAGE)
      .then((runs) => {
        if (cancelled) return;
        setAgentRuns(sortRunsDesc(runs));
        setAgentRunsHasMore(hasMoreRuns(runs, AGENT_RUNS_PAGE));
        const top = latestRun(runs);
        if (top) seedLatestFromRun(taskKey(listFile, task.id), top);
      })
      .catch(() => {
        if (cancelled) return;
        setAgentRuns([]);
        setAgentRunsHasMore(false);
      })
      .finally(() => {
        if (!cancelled) setAgentRunsLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedTaskId, selectedTaskListFile]);

  // 点击行尾图标 → 详情滚动到 Agent 区（该板块收起时先展开，否则没有可滚动目标）
  useEffect(() => {
    if (agentFocusTick === 0) return;
    if (detailCollapsedRef.current.includes("agent")) {
      const next = nextDetailCollapsed(detailCollapsedRef.current, "agent", false);
      applyDetailCollapsed(next);
      persistDetailCollapsed(next);
    }
    // 等展开后的内容提交渲染再定位
    const frame = requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        agentSectionRef.current?.scrollIntoView({ block: "start" });
      });
    });
    return () => cancelAnimationFrame(frame);
  }, [agentFocusTick, selectedTaskId]);

  // 卸载清理防抖
  useEffect(() => {
    return () => {
      if (agentRunsRefetchRef.current !== null) {
        window.clearTimeout(agentRunsRefetchRef.current);
      }
    };
  }, []);

  function applyTaskAgent(
    prev: TaskList[],
    listFile: string,
    taskId: string,
    enabled: boolean,
    paths: string[],
    links: string[],
  ): TaskList[] {
    return prev.map((list) =>
      list.file !== listFile
        ? list
        : {
            ...list,
            tasks: list.tasks.map((task) =>
              task.id !== taskId
                ? task
                : { ...task, agent: { enabled, paths, links } },
            ),
          },
    );
  }

  // 启用前置：需本机执行开关 + 已配置 Key + 有效结果根；缺条件则引导到 Agent 配置
  async function ensureAgentReady(): Promise<boolean> {
    try {
      const cfg = await loadAgentConfig();
      if (cfg.execution_enabled && cfg.key_present && cfg.result_root) {
        return true;
      }
      setAgentGateNotice(
        "启用 Agent 协助前，请先在「Agent 配置」中开启本机执行、配置 API Key 与结果主目录。",
      );
      setAgentConfigOpen(true);
      return false;
    } catch (e) {
      setActionError(String(e));
      return false;
    }
  }

  async function persistTaskIntent(
    entry: ViewTask,
    enabled: boolean,
    paths: string[],
    links: string[],
  ) {
    try {
      await setAgentTaskIntent(
        entry.listFile,
        entry.task.id,
        enabled,
        paths,
        links,
      );
      setLists((prev) =>
        applyTaskAgent(prev, entry.listFile, entry.task.id, enabled, paths, links),
      );
    } catch (e) {
      setActionError(String(e));
    }
  }

  async function toggleTaskAgent(entry: ViewTask, enabled: boolean) {
    if (enabled) {
      const ready = await ensureAgentReady();
      if (!ready) return;
    }
    await persistTaskIntent(
      entry,
      enabled,
      entry.task.agent?.paths ?? [],
      entry.task.agent?.links ?? [],
    );
  }

  async function addTaskPaths(entry: ViewTask, directories: boolean) {
    try {
      const picked = await pickAgentPaths(directories);
      if (!picked || picked.length === 0) return;
      setMaterialsError(null);
      const next = (entry.task.agent?.paths ?? []).slice();
      for (const p of picked) if (!next.includes(p)) next.push(p);
      await persistTaskIntent(
        entry,
        !!entry.task.agent?.enabled,
        next,
        entry.task.agent?.links ?? [],
      );
    } catch (e) {
      // 就地展示：错误与「关联材料」板块同屏，避免用户以为选完文件没反应
      setMaterialsError(String(e));
    }
  }

  async function removeTaskPath(entry: ViewTask, path: string) {
    const next = (entry.task.agent?.paths ?? []).filter((p) => p !== path);
    await persistTaskIntent(
      entry,
      !!entry.task.agent?.enabled,
      next,
      entry.task.agent?.links ?? [],
    );
  }

  async function addTaskLink(entry: ViewTask, url: string) {
    const value = normalizeMaterialLink(url);
    const current = entry.task.agent?.links ?? [];
    const message = materialLinkError(value, [
      ...(entry.task.agent?.paths ?? []),
      ...current,
    ]);
    if (message) {
      setActionError(message);
      return;
    }
    await persistTaskIntent(
      entry,
      !!entry.task.agent?.enabled,
      entry.task.agent?.paths ?? [],
      [...current, value],
    );
  }

  async function removeTaskLink(entry: ViewTask, url: string) {
    const next = (entry.task.agent?.links ?? []).filter((l) => l !== url);
    await persistTaskIntent(
      entry,
      !!entry.task.agent?.enabled,
      entry.task.agent?.paths ?? [],
      next,
    );
  }

  function openMaterial(target: string) {
    openExternal(target).catch((e) => {
      setActionError(`无法打开：${target}（${String(e)}）`);
    });
  }

  async function startTaskAgent(entry: ViewTask) {
    const key = taskKey(entry.listFile, entry.task.id);
    if (agentBusyRef.current.has(key)) return;
    markAgentBusy(key, true);
    try {
      const res = await startAgentTask(entry.listFile, entry.task.id, true);
      setAgentStates((prev) => {
        const next = new Map(prev);
        next.set(key, {
          runId: res.runId,
          state: "running",
          attempt: prev.get(key)?.attempt ?? 1,
          errorClass: null,
          stopReason: null,
          resultDir: prev.get(key)?.resultDir ?? null,
        });
        return next;
      });
      await refreshAgentStatus();
      if (selectedAgentKeyRef.current === key) scheduleReloadSelectedRuns(key);
    } catch (e) {
      setActionError(String(e));
    } finally {
      markAgentBusy(key, false);
    }
  }

  async function stopTaskAgent(entry: ViewTask) {
    const key = taskKey(entry.listFile, entry.task.id);
    if (agentBusyRef.current.has(key)) return;
    markAgentBusy(key, true);
    try {
      await stopAgentTask(entry.listFile, entry.task.id);
      await refreshAgentStatus();
      if (selectedAgentKeyRef.current === key) scheduleReloadSelectedRuns(key);
    } catch (e) {
      setActionError(String(e));
    } finally {
      markAgentBusy(key, false);
    }
  }

  async function openAgentResult(runId: string) {
    try {
      await openAgentResultDir(runId);
    } catch (e) {
      setActionError(String(e));
    }
  }

  async function loadMoreAgentRuns() {
    const task = selectedTask;
    const listFile = selectedTaskListFile;
    if (!task || !listFile || agentRunsLoading) return;
    const offset = agentRuns.length;
    setAgentRunsLoading(true);
    try {
      const page = await listAgentRuns(listFile, task.id, offset, AGENT_RUNS_PAGE);
      setAgentRuns((prev) => mergeRuns(prev, page));
      setAgentRunsHasMore(hasMoreRuns(page, AGENT_RUNS_PAGE));
    } catch (e) {
      setActionError(String(e));
    } finally {
      setAgentRunsLoading(false);
    }
  }

  function openTaskAgent(entry: ViewTask) {
    openTask(entry.task.id);
    setAgentFocusTick((t) => t + 1);
  }

  function openTaskMenu(event: MouseEvent, entry: ViewTask) {
    event.preventDefault();
    event.stopPropagation();
    setMenu(null);
    setCompletedGroupMenu(null);
    setTaskMenu({
      listFile: entry.listFile,
      taskId: entry.task.id,
      x: event.clientX,
      y: event.clientY,
    });
  }

  function buildTaskMenuItems(entry: ViewTask): MenuItem[] {
    const { task, listFile } = entry;
    const key = taskKey(listFile, task.id);
    const live = agentStates.get(key);
    const enabled = !!task.agent?.enabled;
    const model = agentMenuModel({
      agentEnabled: enabled,
      hasRun: !!live,
      isActive: isAgentActive(live?.state),
      latestResultDir: live?.resultDir ?? null,
    });
    const isDone = task.status === "done";
    const isArchived = typeof task.archived === "string";
    // 「移动到清单」目标：除任务所在清单外的全部真实清单（侧栏顺序）；无目标则整项不显示
    const moveTargets = moveTargetLists(lists, listFile);
    const moveItem: MenuItem[] = moveTargets.length
      ? [
          {
            key: "move",
            label: "移动到清单",
            children: moveTargets.map((target) => ({
              key: `move-${target.file}`,
              label: target.name,
              onSelect: () => void moveTask(listFile, task.id, target.file),
            })),
          },
        ]
      : [];

    // 分隔线项（仅视觉分区，不参与聚焦 / 悬停 / 选择）
    const sep = (key: string): MenuItem => ({ key, separator: true });

    // 已归档：只剩「打开详情 / 取消归档 / 删除」，不可完成、不可移动、无 Agent
    if (isArchived) {
      return [
        {
          key: "open",
          label: "打开详情",
          onSelect: () => openTask(task.id),
        },
        sep("sep-open"),
        {
          key: "unarchive",
          label: "取消归档",
          onSelect: () => void unarchiveTasks(listFile, [task.id]),
        },
        sep("sep-archive"),
        {
          key: "delete",
          label: "删除",
          danger: true,
          onSelect: () => removeTask(listFile, task),
        },
      ];
    }

    // 「归档」只对已完成任务出现；未归档才可归档
    const archiveItem: MenuItem[] = isDone
      ? [
          {
            key: "archive",
            label: "归档",
            onSelect: () => void archiveTasks(listFile, [task.id]),
          },
        ]
      : [];

    return [
      // 一区：查看 / 编辑
      {
        key: "open",
        label: "打开详情",
        onSelect: () => openTask(task.id),
      },
      {
        key: "edit",
        label: "编辑",
        onSelect: () => startEditTask(entry),
      },
      sep("sep-view"),
      // 二区：状态流转（完成 / 移动 / 归档）
      {
        key: "toggle-done",
        label: isDone ? "取消完成" : "完成",
        onSelect: () => toggle(listFile, task, !isDone),
      },
      ...moveItem,
      ...archiveItem,
      sep("sep-state"),
      // 三区：Agent
      {
        key: "agent",
        label: "Agent",
        children: [
          {
            key: "agent-toggle",
            label: model.toggleLabel,
            onSelect: () => void toggleTaskAgent(entry, !enabled),
          },
          {
            key: "agent-add-file",
            label: "添加关联文件",
            disabled: !model.addFileEnabled,
            onSelect: () => void addTaskPaths(entry, false),
          },
          {
            key: "agent-add-dir",
            label: "添加关联目录",
            disabled: !model.addDirEnabled,
            onSelect: () => void addTaskPaths(entry, true),
          },
          {
            key: "agent-start",
            label: model.startLabel,
            disabled: !model.startEnabled,
            onSelect: () => void startTaskAgent(entry),
          },
          {
            key: "agent-stop",
            label: "停止",
            disabled: !model.stopEnabled,
            onSelect: () => void stopTaskAgent(entry),
          },
          {
            key: "agent-result",
            label: "查看结果",
            disabled: !model.viewResultEnabled,
            onSelect: () => {
              const l = agentStatesRef.current.get(key);
              if (l?.runId) void openAgentResult(l.runId);
            },
          },
        ],
      },
      sep("sep-agent"),
      // 四区：删除（独立成区，恒在最后）
      {
        key: "delete",
        label: "删除",
        danger: true,
        onSelect: () => removeTask(listFile, task),
      },
    ];
  }

  // 启动：先确定主题（首帧前 setTheme）并读取功能栏顺序，再加载清单；主题未定前不画亮色内容
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const {
        theme: pref,
        appBarOrder: order,
        completedCollapsed: collapsed,
        completedShowArchived: showArchivedPref,
        trashCollapsed: trashCollapsedPref,
        detailCollapsed: detailCollapsedPref,
        minimizeToTray: trayPref,
      } = await loadSettings();
      if (cancelled) return;
      setTheme(pref);
      setMinimizeToTray(trayPref);
      setAppBarSaved(order);
      setAppBarOrder(resolveAppBarOrder(order));
      const initialCollapsed = collapsed ?? [];
      completedCollapsedRef.current = initialCollapsed;
      completedCollapsedSavedRef.current = initialCollapsed;
      setCompletedCollapsed(initialCollapsed);
      completedShowArchivedRef.current = showArchivedPref;
      completedShowArchivedSavedRef.current = showArchivedPref;
      setCompletedShowArchived(showArchivedPref);
      // 回收站折叠：null = 全部展开
      const initialTrashCollapsed = trashCollapsedPref ?? [];
      trashCollapsedRef.current = initialTrashCollapsed;
      trashCollapsedSavedRef.current = initialTrashCollapsed;
      setTrashCollapsed(initialTrashCollapsed);
      // 详情子板块：null = 默认（仅「任务属性」收起）；空数组 = 全部展开
      const initialDetail = detailCollapsedPref ?? [...DEFAULT_DETAIL_COLLAPSED];
      detailCollapsedRef.current = initialDetail;
      detailCollapsedSavedRef.current = initialDetail;
      setDetailCollapsed(initialDetail);
      try {
        await applyThemePref(pref);
      } catch {
        // 权限 / runtime 未就绪：静默降级，界面仍可用
      }
      if (cancelled) return;
      await refresh();
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 「跟随系统」：订阅系统主题变化并同步 color-scheme（带 guard 防回环）
  const lastSystemThemeRef = useRef<string | null>(null);
  useEffect(() => {
    if (theme !== "system") return;
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      try {
        unlisten = await win.onThemeChanged((event) => {
          if (cancelled) return;
          const next = event.payload;
          if (next === lastSystemThemeRef.current) return;
          lastSystemThemeRef.current = next;
          document.documentElement.style.colorScheme = next;
        });
      } catch {
        // 订阅失败忽略：CSS 的 prefers-color-scheme 仍会跟随
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [theme]);

  // 卸载时清掉待写的防抖
  useEffect(() => {
    return () => {
      if (saveTimerRef.current !== null) window.clearTimeout(saveTimerRef.current);
    };
  }, []);

  // 挂载时用真实状态校正置顶（默认应为未置顶），并记下未置顶时的默认快照
  useEffect(() => {
    const win = getCurrentWindow();
    let cancelled = false;
    (async () => {
      try {
        const alwaysOnTop = await win.isAlwaysOnTop();
        if (cancelled) return;
        setPinned(alwaysOnTop);
        if (!alwaysOnTop) {
          const [minimizable, resizable, maximizable] = await Promise.all([
            win.isMinimizable(),
            win.isResizable(),
            win.isMaximizable(),
          ]);
          if (cancelled) return;
          pinSnapshotRef.current = { minimizable, resizable, maximizable };
        }
      } catch {
        // 读取失败保持默认（未置顶）
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // 选中清单：虚拟视图（已完成 / 已删除）保持，真实清单仍存在则保持，否则回落
  useEffect(() => {
    setSelectedListFile((prev) => {
      if (prev === COMPLETED_ID || prev === TRASH_ID) return prev;
      if (prev && lists.some((l) => l.file === prev)) return prev;
      return lists[0]?.file ?? null;
    });
  }, [lists]);

  // 进入回收站：重置倒计时基准，每 30 秒推进剩余时长；每 60 秒重读一次，让后端清理过期条目
  useEffect(() => {
    if (!isTrashView) return;
    setTrashNow(new Date());
    const tick = window.setInterval(
      () => setTrashNow(new Date()),
      TRASH_TICK_MS,
    );
    // 定时重读：后端会顺手清理超过 24h 的条目；切走后清除
    const sweep = window.setInterval(
      () => void refreshRef.current(),
      TRASH_SWEEP_MS,
    );
    return () => {
      window.clearInterval(tick);
      window.clearInterval(sweep);
    };
  }, [isTrashView]);

  // 选中任务：在当前视图内仍存在则保持，否则清空（清空前 flush 待写改动）
  useEffect(() => {
    const ids = isTrashView
      ? trashTasksVisible.map((t) => t.trash_id)
      : isCompletedView
        ? completedTasks.map((e) => e.task.id)
        : selectedList?.tasks.map((t) => t.id) ?? [];
    setSelectedTaskId((prev) => {
      if (prev && ids.includes(prev)) return prev;
      if (prev) flushPendingSave();
      return null;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lists, selectedListFile, isTrashView, trashTasksVisible]);

  // 没有选中任务时关闭抽屉
  useEffect(() => {
    if (!selectedTaskId) setDrawerOpen(false);
  }, [selectedTaskId]);

  // 选中项切换时，把详情草稿同步为任务当前值（刷新时不覆盖正在编辑的草稿）
  useEffect(() => {
    const title = selectedEntry?.task.title ?? "";
    const notes = selectedEntry?.task.notes ?? "";
    editTitleRef.current = title;
    editNotesRef.current = notes;
    setEditTitle(title);
    setEditNotes(notes);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedTaskId, selectedListFile]);

  // 统一的操作执行器：防重复（busy）+ 失败提示 + 成功后刷新
  async function run(action: () => Promise<unknown>, onSuccess?: () => void) {
    if (busy) return;
    setBusy(true);
    setActionError(null);
    try {
      await action();
      await refresh();
      onSuccess?.();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(false);
    }
  }

  // 排序专用：无论成败都重新加载，失败即回滚到磁盘真实状态
  async function persistOrder(action: () => Promise<unknown>) {
    if (busy) return;
    setBusy(true);
    setActionError(null);
    try {
      await action();
    } catch (e) {
      setActionError(String(e));
    } finally {
      await refresh();
      setBusy(false);
    }
  }

  function markDragStart() {
    suppressClickRef.current = true;
  }

  function markDragEnd() {
    // 松手后的 click 在该微任务之后才派发，用 setTimeout 保证抑制生效
    window.setTimeout(() => {
      suppressClickRef.current = false;
    }, 0);
  }

  // ---- 跨清单转移（右键子菜单 + 拖到清单条） / 拖到「已完成」勾选完成 ----

  // 统一入口：移到目标清单末尾；成功后刷新；失败沿用现有错误提示。
  // 关闭正打开在详情面板的该任务，避免残留指向源清单的编辑（先落盘待写改动）。
  async function moveTask(
    sourceListFile: string,
    taskId: string,
    toListFile: string,
  ) {
    if (busy || !taskId || !toListFile || toListFile === sourceListFile) return;
    if (selectedTaskIdRef.current === taskId) {
      flushPendingSave();
      setDrawerOpen(false);
      setSelectedTaskId(null);
    }
    setBusy(true);
    setActionError(null);
    try {
      await invoke("move_task", {
        listFile: sourceListFile,
        taskId,
        toListFile,
      });
      await refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(false);
    }
  }

  // 读取清单条中真实清单项的视口矩形（固定「已完成」项无 data-list-file，天然排除）
  function listNavRects(): Array<RectLike & { file: string }> {
    const nodes = document.querySelectorAll<HTMLElement>(
      ".list-nav-item[data-list-file]",
    );
    const rects: Array<RectLike & { file: string }> = [];
    nodes.forEach((el) => {
      const file = el.dataset.listFile;
      if (!file) return;
      const rect = el.getBoundingClientRect();
      rects.push({
        file,
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
      });
    });
    return rects;
  }

  // 由激活事件取出按下时的视口坐标（鼠标 / 触屏）；键盘拖拽无坐标 → null
  function pointerFromActivator(
    src: ActivatorPointerEvent,
  ): { x: number; y: number } | null {
    if (typeof src.clientX === "number" && typeof src.clientY === "number") {
      return { x: src.clientX, y: src.clientY };
    }
    if (src.touches && src.touches.length > 0) {
      return { x: src.touches[0].clientX, y: src.touches[0].clientY };
    }
    return null;
  }

  // 由拖拽起点事件 + 累计位移推算当前指针视口坐标；键盘拖拽无坐标 → null
  function dragPointer(event: DragMoveEvent): { x: number; y: number } | null {
    const start = pointerFromActivator(
      event.activatorEvent as ActivatorPointerEvent,
    );
    if (!start) return null;
    return { x: start.x + event.delta.x, y: start.y + event.delta.y };
  }

  function applyDragDropTarget(file: string | null) {
    if (dragDropTargetRef.current === file) return;
    dragDropTargetRef.current = file;
    setDragDropTarget(file);
  }

  function applyDragOutsideSource(next: boolean) {
    if (dragOutsideSourceRef.current === next) return;
    dragOutsideSourceRef.current = next;
    setDragOutsideSource(next);
  }

  function applyDragOverCompleted(next: boolean) {
    if (dragOverCompletedRef.current === next) return;
    dragOverCompletedRef.current = next;
    setDragOverCompleted(next);
  }

  // 拖拽过程中：清单条命中 → 高亮目标；离开源任务滚动区 → 浮层收缩；
  // 命中「已完成」固定项 → 高亮并作为勾选完成落点（仅普通清单视图）
  function handleTaskDragMove(event: DragMoveEvent) {
    if (isCompletedView || !selectedListFile) return;
    const pointer = dragPointer(event);
    const rects = listNavRects();
    applyDragDropTarget(resolveDropTarget(pointer, rects, selectedListFile));
    const scrollRect = taskScrollRef.current?.getBoundingClientRect();
    applyDragOutsideSource(
      pointer !== null &&
        scrollRect != null &&
        !isPointInRect(pointer, scrollRect, 4),
    );
    const footerRect = completedFooterRef.current?.getBoundingClientRect();
    applyDragOverCompleted(
      pointer !== null &&
        footerRect != null &&
        isPointInRect(pointer, footerRect),
    );
  }

  function handleTaskDragStart(event: DragStartEvent) {
    markDragStart();
    dragMoveHandledRef.current = false;
    applyDragOutsideSource(false);
    applyDragOverCompleted(false);
    // 不读 event.active.rect.current.initial：dnd-kit 在 onStart 里先派发该事件、
    // 之后才写入初始矩形，同步读取会拿到上一轮（或 null）。直接量当前行更稳。
    const target = event.activatorEvent.target;
    const card =
      target instanceof Element
        ? target.closest<HTMLElement>(".task-card")
        : null;
    const rect = card?.getBoundingClientRect();
    if (!rect) {
      setDragMetrics(null);
      return;
    }
    const start = pointerFromActivator(
      event.activatorEvent as ActivatorPointerEvent,
    );
    setDragMetrics({
      grabX: start ? start.x - rect.left : rect.width / 2,
      grabY: start ? start.y - rect.top : rect.height / 2,
      width: rect.width,
      height: rect.height,
    });
  }

  // 拖拽结束：左侧「已完成」→ 勾选完成；清单条 → 跨清单移动；否则保持既有同清单排序
  function handleTaskDragEnd(activeId: string | null) {
    markDragEnd();
    const target = dragDropTargetRef.current;
    const completed = dragOverCompletedRef.current;
    applyDragDropTarget(null);
    applyDragOutsideSource(false);
    applyDragOverCompleted(false);
    setDragMetrics(null);
    if (completed && activeId && selectedListFile && !isCompletedView) {
      // 落点已被本次拖拽消费，压掉随后的同清单排序
      dragMoveHandledRef.current = true;
      const entry = entryById.get(activeId);
      if (entry && entry.task.status !== "done") {
        toggle(entry.listFile, entry.task, true);
      }
      return;
    }
    if (
      target &&
      activeId &&
      selectedListFile &&
      !isCompletedView &&
      target !== selectedListFile
    ) {
      dragMoveHandledRef.current = true;
      void moveTask(selectedListFile, activeId, target);
    }
  }

  // ---- 窗口置顶（pin） ----

  async function syncPinState() {
    try {
      setPinned(await getCurrentWindow().isAlwaysOnTop());
    } catch {
      // 读取失败保持现状
    }
  }

  async function pinWindow() {
    if (pinBusy || pinned) return;
    setPinBusy(true);
    setActionError(null);
    const win = getCurrentWindow();
    const applied: Array<() => Promise<void>> = [];
    try {
      // 1. 先读取并保存「置顶前」快照（不硬编码）
      const snapshot = {
        minimizable: await win.isMinimizable(),
        resizable: await win.isResizable(),
        maximizable: await win.isMaximizable(),
      };
      // 2. 逐条应用；每成功一步即登记回滚动作
      await win.setAlwaysOnTop(true);
      applied.push(() => win.setAlwaysOnTop(false));
      await win.setMinimizable(false);
      applied.push(() => win.setMinimizable(snapshot.minimizable));
      await win.setResizable(false);
      applied.push(() => win.setResizable(snapshot.resizable));
      await win.setMaximizable(false);
      applied.push(() => win.setMaximizable(snapshot.maximizable));
      pinSnapshotRef.current = snapshot;
    } catch (e) {
      // 回滚本次已应用的改动（能回滚多少回滚多少）
      for (const undo of applied.reverse()) {
        try {
          await undo();
        } catch {
          /* 回滚尽力而为 */
        }
      }
      setActionError(String(e));
    } finally {
      setPinBusy(false);
      // 以真实状态校正按钮
      await syncPinState();
    }
  }

  async function unpinWindow() {
    if (pinBusy || !pinned) return;
    setPinBusy(true);
    setActionError(null);
    const win = getCurrentWindow();
    const snapshot = pinSnapshotRef.current;
    const applied: Array<() => Promise<void>> = [];
    try {
      await win.setAlwaysOnTop(false);
      applied.push(() => win.setAlwaysOnTop(true));
      if (snapshot) {
        await win.setMinimizable(snapshot.minimizable);
        applied.push(() => win.setMinimizable(false));
        await win.setResizable(snapshot.resizable);
        applied.push(() => win.setResizable(false));
        await win.setMaximizable(snapshot.maximizable);
        applied.push(() => win.setMaximizable(false));
        pinSnapshotRef.current = null;
      }
    } catch (e) {
      for (const undo of applied.reverse()) {
        try {
          await undo();
        } catch {
          /* 回滚尽力而为 */
        }
      }
      setActionError(String(e));
    } finally {
      setPinBusy(false);
      await syncPinState();
    }
  }

  function togglePin() {
    if (pinBusy) return;
    if (pinned) void unpinWindow();
    else void pinWindow();
  }

  // ---- 应用功能栏（App Bar）接线 ----

  // 刷新入口：只重扫重读清单（不保存草稿、不重读设置、不新增监听）
  async function manualRefresh() {
    if (refreshing || busy) return;
    setRefreshing(true);
    try {
      await refresh();
    } finally {
      setRefreshing(false);
    }
  }

  function activateAppBar(id: string) {
    if (id === "refresh") void manualRefresh();
    else if (id === "pin") togglePin();
    else if (id === "note")
      void openNoteWindow(theme).catch((e) => setActionError(String(e)));
    else if (id === "settings") setSettingsOpen(true);
    else if (id === "agent") setAgentConfigOpen(true);
  }

  function isAppBarDisabled(id: string) {
    if (id === "refresh") return busy || refreshing;
    if (id === "pin") return pinBusy;
    return false;
  }

  function isAppBarPressed(id: string): boolean | undefined {
    return id === "pin" ? pinned : undefined;
  }

  // 重排可见入口：合并未知槽位 → 立即落盘；失败回滚到最后成功顺序并报错
  function reorderAppBar(nextVisible: string[]) {
    if (appBarSavingRef.current) return;
    appBarSavingRef.current = true;
    const prev = appBarOrder;
    const merged = mergeAppBarOrder(appBarOrder, nextVisible);
    setAppBarError(null);
    setAppBarOrder(merged);
    setAppBarSaving(true);
    void (async () => {
      try {
        await invoke("save_app_bar_order", { order: merged });
        setAppBarSaved(merged);
      } catch (e) {
        setAppBarOrder(prev);
        setAppBarError(String(e));
      } finally {
        appBarSavingRef.current = false;
        setAppBarSaving(false);
      }
    })();
  }

  // 恢复默认：清空历史顺序（含未知入口），落盘 null
  function resetAppBar() {
    if (appBarSavingRef.current) return;
    appBarSavingRef.current = true;
    const prev = appBarOrder;
    setAppBarError(null);
    setAppBarOrder([...DEFAULT_APP_BAR_ORDER]);
    setAppBarSaving(true);
    void (async () => {
      try {
        await invoke("save_app_bar_order", { order: null });
        setAppBarSaved(null);
      } catch (e) {
        setAppBarOrder(prev);
        setAppBarError(String(e));
      } finally {
        appBarSavingRef.current = false;
        setAppBarSaving(false);
      }
    })();
  }

  // ---- 「已完成」分组折叠接线 ----

  // 应用折叠集合并同步 state 与 ref
  function applyCompletedCollapsed(next: string[]) {
    completedCollapsedRef.current = next;
    setCompletedCollapsed(next);
  }

  // 串行保存当前完整集合；失败报错，并在期间无更新改动时回滚到最后一次成功值
  function persistCompletedCollapsed(next: string[]) {
    const snapshot = next.slice();
    const payload = snapshot.length > 0 ? snapshot : null;
    completedSaveChainRef.current = completedSaveChainRef.current.then(
      async () => {
        setCompletedCollapsedError(null);
        try {
          await invoke("save_completed_collapsed", { collapsed: payload });
          completedCollapsedSavedRef.current = snapshot;
          setCompletedCollapsedError(null);
        } catch (e) {
          setCompletedCollapsedError(String(e));
          // 仅当没有更新的改动时才回滚，避免覆盖用户后续操作
          if (sameCollapsed(completedCollapsedRef.current, snapshot)) {
            applyCompletedCollapsed(completedCollapsedSavedRef.current.slice());
          }
        }
      },
    );
  }

  // 切换某个来源分组的折叠态：立即生效，随后落盘完整集合
  function toggleCompletedGroup(file: string) {
    const current = activeCollapsedFiles(completedCollapsedRef.current, lists);
    const collapsedNow = current.includes(file);
    const next = nextCollapsed(current, file, !collapsedNow) ?? [];
    applyCompletedCollapsed(next);
    persistCompletedCollapsed(next);
  }

  // ---- 「已删除」分组折叠接线（与已完成同款；偏好键 trash_collapsed）----

  function applyTrashCollapsed(next: string[]) {
    trashCollapsedRef.current = next;
    setTrashCollapsed(next);
  }

  function persistTrashCollapsed(next: string[]) {
    const snapshot = next.slice();
    const payload = snapshot.length > 0 ? snapshot : null;
    trashSaveChainRef.current = trashSaveChainRef.current.then(async () => {
      setTrashCollapsedError(null);
      try {
        await invoke("save_trash_collapsed", { collapsed: payload });
        trashCollapsedSavedRef.current = snapshot;
        setTrashCollapsedError(null);
      } catch (e) {
        setTrashCollapsedError(String(e));
        // 仅当没有更新的改动时才回滚，避免覆盖用户后续操作
        if (sameCollapsed(trashCollapsedRef.current, snapshot)) {
          applyTrashCollapsed(trashCollapsedSavedRef.current.slice());
        }
      }
    });
  }

  // 回收站分组不按 `lists` 过滤：原清单可能已不存在，那些组同样要能折叠
  function toggleTrashGroup(file: string) {
    const current = trashCollapsedRef.current;
    const collapsedNow = current.includes(file);
    const next = nextCollapsed(current, file, !collapsedNow) ?? [];
    applyTrashCollapsed(next);
    persistTrashCollapsed(next);
  }

  // 切换「显示归档」：立即生效，随后落盘；失败且期间无新改动则回滚到最后成功值
  function changeCompletedShowArchived(next: boolean) {
    completedShowArchivedRef.current = next;
    setCompletedShowArchived(next);
    setCompletedShowArchivedError(null);
    showArchivedSaveChainRef.current = showArchivedSaveChainRef.current.then(
      async () => {
        try {
          await invoke("save_completed_show_archived", { show: next });
          completedShowArchivedSavedRef.current = next;
        } catch (e) {
          setCompletedShowArchivedError(String(e));
          if (completedShowArchivedRef.current === next) {
            const saved = completedShowArchivedSavedRef.current;
            completedShowArchivedRef.current = saved;
            setCompletedShowArchived(saved);
          }
        }
      },
    );
  }

  // ---- 归档 / 取消归档（FR-APP-36）----

  // 归档后 5 秒内可撤销；单槽位：新的归档直接覆盖旧的提示
  function showArchiveToast(listFile: string, taskIds: string[]) {
    if (archiveToastTimerRef.current !== null) {
      window.clearTimeout(archiveToastTimerRef.current);
    }
    setArchiveToast({ listFile, taskIds });
    archiveToastTimerRef.current = window.setTimeout(() => {
      archiveToastTimerRef.current = null;
      setArchiveToast(null);
    }, 5000);
  }

  function dismissArchiveToast() {
    if (archiveToastTimerRef.current !== null) {
      window.clearTimeout(archiveToastTimerRef.current);
      archiveToastTimerRef.current = null;
    }
    setArchiveToast(null);
  }

  // 归档：成功后按既有路径重读清单，再给一次撤销机会
  async function archiveTasks(listFile: string, taskIds: string[]) {
    if (taskIds.length === 0) return;
    setActionError(null);
    try {
      await invoke("archive_tasks", { listFile, taskIds });
      await refresh();
      showArchiveToast(listFile, taskIds);
    } catch (e) {
      setActionError(String(e));
    }
  }

  async function unarchiveTasks(listFile: string, taskIds: string[]) {
    if (taskIds.length === 0) return;
    setActionError(null);
    try {
      await invoke("unarchive_tasks", { listFile, taskIds });
      await refresh();
    } catch (e) {
      setActionError(String(e));
    }
  }

  // 撤销上一次列表级 / 单条归档
  async function undoArchive() {
    const toast = archiveToast;
    dismissArchiveToast();
    if (!toast) return;
    await unarchiveTasks(toast.listFile, toast.taskIds);
  }

  // 清理撤销提示的计时器
  useEffect(() => () => {
    if (archiveToastTimerRef.current !== null) {
      window.clearTimeout(archiveToastTimerRef.current);
      archiveToastTimerRef.current = null;
    }
  }, []);

  // ---- 任务详情子板块折叠接线 ----

  // 应用折叠集合并同步 state 与 ref
  function applyDetailCollapsed(next: string[]) {
    detailCollapsedRef.current = next;
    setDetailCollapsed(next);
  }

  // 串行保存当前完整集合；失败报错，并在期间无更新改动时回滚到最后一次成功值
  function persistDetailCollapsed(next: string[]) {
    // 与「已完成」分组不同：空数组是合法落盘值（全部展开），不能折叠成 null
    const snapshot = next.slice();
    detailSaveChainRef.current = detailSaveChainRef.current.then(async () => {
      setDetailCollapsedError(null);
      try {
        await invoke("save_detail_collapsed", { collapsed: snapshot });
        detailCollapsedSavedRef.current = snapshot;
        setDetailCollapsedError(null);
      } catch (e) {
        setDetailCollapsedError(String(e));
        // 仅当没有更新的改动时才回滚，避免覆盖用户后续操作
        if (sameDetailCollapsed(detailCollapsedRef.current, snapshot)) {
          applyDetailCollapsed(detailCollapsedSavedRef.current.slice());
        }
      }
    });
  }

  // 切换某个子板块的折叠态：立即生效，随后落盘完整集合
  function toggleDetailSection(key: DetailSectionKey) {
    const current = detailCollapsedRef.current;
    const collapsedNow = current.includes(key);
    const next = nextDetailCollapsed(current, key, !collapsedNow);
    applyDetailCollapsed(next);
    persistDetailCollapsed(next);
  }

  // 切换主题：立即生效；失败记入 actionError（不弹窗）
  function changeTheme(next: ThemePref) {
    setTheme(next);
    void (async () => {
      setActionError(null);
      try {
        await applyThemePref(next);
        // 尽力广播给便签窗口；失败静默，绝不影响主题切换与保存逻辑。
        try {
          await emit("theme-pref-changed", next);
        } catch {
          // 忽略广播失败
        }
      } catch (e) {
        setActionError(String(e));
      }
      try {
        await invoke("save_settings", { theme: next });
      } catch (e) {
        setActionError(String(e));
      }
    })();
  }

  // 切换「关闭时最小化到托盘」：乐观更新（开启＝null 恢复默认，关闭＝false）；失败记入 actionError（不回滚）
  function changeMinimizeToTray(next: boolean) {
    setMinimizeToTray(next ? null : false);
    void (async () => {
      setActionError(null);
      try {
        await invoke("save_minimize_to_tray", { enabled: next });
      } catch (e) {
        setActionError(String(e));
      }
    })();
  }

  function selectList(file: string) {
    if (suppressClickRef.current) return;
    if (file === selectedListFile) return;
    flushPendingSave();
    setSelectedListFile(file);
    setSelectedTaskId(null);
    setDrawerOpen(false);
  }

  function openTask(taskId: string, viaKeyboard = false) {
    if (suppressClickRef.current) return;
    if (!taskId) return;
    if (taskId !== selectedTaskId) {
      // 仅在「首次打开该任务」时记录打开方式：指针 → 关闭时清焦点环；键盘 → 保留。
      // 重复激活（如指针点击后再按 Enter，click.detail===0）不改写最初来源，
      // 否则「指针 → 回车 → Esc」会漏掉 closeDetail 的清焦点分支。
      openedByKeyboardRef.current = viaKeyboard;
      flushPendingSave();
    }
    setSelectedTaskId(taskId);
    setDrawerOpen(true);
  }

  function submitNewList() {
    const name = newListName.trim();
    if (!name) return;
    void run(async () => {
      await invoke("create_list", { name });
      setNewListName("");
    });
  }

  // 新建任务落在清单首位；把任务列表滚回顶部，让刚加的那条在视野内
  function scrollTaskListToTop() {
    const el = taskScrollRef.current;
    if (!el) return;
    const reduceMotion =
      typeof window.matchMedia === "function" &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    el.scrollTo({ top: 0, behavior: reduceMotion ? "auto" : "smooth" });
  }

  function submitNewTask(listFile: string) {
    const title = (newTaskTitles[listFile] ?? "").trim();
    if (!title) return;
    let created = false;
    void run(async () => {
      await invoke("create_task", { listFile, title });
      setNewTaskTitles((prev) => ({ ...prev, [listFile]: "" }));
      created = true;
    }).then(() => {
      // refresh 完成后再滚，否则内容还是旧的
      if (created) scrollTaskListToTop();
    });
  }

  // ---- 详情即时保存 ----

  // 输入停止 ~400ms 后落盘
  function scheduleSave() {
    if (saveTimerRef.current !== null) window.clearTimeout(saveTimerRef.current);
    saveTimerRef.current = window.setTimeout(() => {
      saveTimerRef.current = null;
      void persistEdit();
    }, 400);
  }

  // 立即落盘待写的防抖改动（失焦 / Esc / Enter / 切换任务前调用）
  function flushPendingSave() {
    if (saveTimerRef.current === null) return;
    window.clearTimeout(saveTimerRef.current);
    saveTimerRef.current = null;
    void persistEdit();
  }

  async function persistEdit() {
    const task = selectedTask;
    const listFile = selectedTaskListFile;
    if (!task || !listFile) return;
    const title = editTitleRef.current.trim();
    if (!title) return; // 空标题不落盘，保留磁盘原值
    const notes = editNotesRef.current;
    if (title === (task.title ?? "").trim() && notes === (task.notes ?? "")) return;
    try {
      await invoke("update_task", { listFile, taskId: task.id, title, notes });
      lastSaveErrorRef.current = null;
      await refresh();
    } catch (e) {
      reportSaveError(String(e));
    }
  }

  // 同一错误只提示一次，避免连续输入时刷屏
  function reportSaveError(message: string) {
    if (lastSaveErrorRef.current === message) return;
    lastSaveErrorRef.current = message;
    setActionError(message);
  }

  // 关闭详情：先 flush；空标题未落盘则回填磁盘原值；丢弃未提交的子任务草稿
  function closeDetail() {
    flushPendingSave();
    if (editTitleRef.current.trim() === "") backfillFromDisk();
    setSubtaskDraft("");
    subtaskEditHandledRef.current = true;
    setEditingSubtaskId(null);
    setSubtaskEditDraft("");
    // 焦点归还：指针打开的详情，关闭时清掉行按钮焦点（避免 Esc 触发 :focus-visible 焦点环）；
    // 键盘打开的保留焦点与焦点环。禁止全局 outline:none。
    if (!openedByKeyboardRef.current) {
      const active = document.activeElement;
      if (
        active instanceof HTMLElement &&
        (active.classList.contains("task-open") ||
          active.classList.contains("trash-row-open"))
      ) {
        active.blur();
      }
    }
    setDrawerOpen(false);
    if (!isNarrow) setSelectedTaskId(null);
  }

  function backfillFromDisk() {
    const title = selectedEntry?.task.title ?? "";
    const notes = selectedEntry?.task.notes ?? "";
    editTitleRef.current = title;
    editNotesRef.current = notes;
    setEditTitle(title);
    setEditNotes(notes);
  }

  function onEditTitleChange(value: string) {
    editTitleRef.current = value;
    setEditTitle(value);
    scheduleSave();
  }

  function onEditNotesChange(value: string) {
    editNotesRef.current = value;
    setEditNotes(value);
    scheduleSave();
  }

  // ---- 子任务：增 / 删 / 勾选（即时写盘，无防抖）----

  // 子任务改动同时落到列表里的对应 task（行尾计数的数据源），保持单一 state 源
  function applyTaskSubtasks(
    listFile: string,
    taskId: string,
    update: (subs: Subtask[]) => Subtask[],
  ) {
    setLists((prev) =>
      prev.map((list) =>
        list.file === listFile
          ? {
              ...list,
              tasks: list.tasks.map((t) =>
                t.id === taskId
                  ? { ...t, subtasks: update(t.subtasks ?? []) }
                  : t,
              ),
            }
          : list,
      ),
    );
  }

  // 任务行内改名成功后就地更新列表里的对应 task（保持单一 state 源）
  function applyTaskTitle(listFile: string, taskId: string, title: string) {
    setLists((prev) =>
      prev.map((list) =>
        list.file === listFile
          ? {
              ...list,
              tasks: list.tasks.map((t) =>
                t.id === taskId ? { ...t, title } : t,
              ),
            }
          : list,
      ),
    );
  }

  // 子任务错误同样去重/节流，避免连点刷屏；与标题/备注通道互不干扰
  function reportSubtaskError(message: string) {
    if (lastSubtaskErrorRef.current === message) return;
    lastSubtaskErrorRef.current = message;
    setActionError(message);
  }

  function markSubtaskBusy(id: string, busy: boolean) {
    const set = subtaskBusyRef.current;
    if (busy) set.add(id);
    else set.delete(id);
    setSubtaskBusyIds(new Set(set));
  }

  async function addSubtask() {
    const title = subtaskDraft.trim();
    if (!title) return;
    if (!selectedTask || !selectedTaskListFile) return;
    if (addSubtaskPendingRef.current) return;
    addSubtaskPendingRef.current = true;
    const listFile = selectedTaskListFile;
    const taskId = selectedTask.id;
    try {
      const created = await invoke<Subtask>("add_subtask", {
        listFile,
        taskId,
        title,
      });
      // 成功后追加到本地镜像，保持输入框打开并聚焦（连续录入）
      setSubtasks((prev) => [...prev, created]);
      applyTaskSubtasks(listFile, taskId, (prev) => [...prev, created]);
      setSubtaskDraft("");
      lastSubtaskErrorRef.current = null;
      subtaskInputRef.current?.focus();
    } catch (e) {
      reportSubtaskError(String(e));
    } finally {
      addSubtaskPendingRef.current = false;
    }
  }

  async function toggleSubtask(sub: Subtask) {
    if (!selectedTask || !selectedTaskListFile) return;
    if (subtaskBusyRef.current.has(sub.id)) return;
    const status = sub.status === "done" ? "open" : "done";
    const listFile = selectedTaskListFile;
    const taskId = selectedTask.id;
    markSubtaskBusy(sub.id, true);
    try {
      await invoke("set_subtask_status", {
        listFile,
        taskId,
        subtaskId: sub.id,
        status,
      });
      // 不做乐观更新：成功后再改本地状态
      setSubtasks((prev) =>
        prev.map((s) => (s.id === sub.id ? { ...s, status } : s)),
      );
      applyTaskSubtasks(listFile, taskId, (prev) =>
        prev.map((s) => (s.id === sub.id ? { ...s, status } : s)),
      );
      lastSubtaskErrorRef.current = null;
    } catch (e) {
      reportSubtaskError(String(e));
    } finally {
      markSubtaskBusy(sub.id, false);
    }
  }

  async function removeSubtask(sub: Subtask) {
    if (!selectedTask || !selectedTaskListFile) return;
    if (subtaskBusyRef.current.has(sub.id)) return;
    const listFile = selectedTaskListFile;
    const taskId = selectedTask.id;
    markSubtaskBusy(sub.id, true);
    try {
      await invoke("remove_subtask", { listFile, taskId, subtaskId: sub.id });
      setSubtasks((prev) => prev.filter((s) => s.id !== sub.id));
      applyTaskSubtasks(listFile, taskId, (prev) =>
        prev.filter((s) => s.id !== sub.id),
      );
      lastSubtaskErrorRef.current = null;
    } catch (e) {
      reportSubtaskError(String(e));
    } finally {
      markSubtaskBusy(sub.id, false);
    }
  }

  // ---- 子任务改名（双击文字进入编辑态；即时写盘，无防抖）----

  // 进入编辑态：预填原值，聚焦 + 全选由 effect 完成
  function startEditSubtask(sub: Subtask) {
    if (subtaskBusyRef.current.has(sub.id)) return; // 在途请求：忽略双击
    subtaskEditHandledRef.current = false;
    setEditingSubtaskId(sub.id);
    setSubtaskEditDraft(sub.title ?? "");
  }

  // 退出编辑态（不改本地值）
  function finishSubtaskEdit() {
    subtaskEditHandledRef.current = true;
    setEditingSubtaskId(null);
    setSubtaskEditDraft("");
  }

  // 提交改名：空 / 未改动 → 视为取消；否则落盘后更新本地
  async function commitSubtaskEdit() {
    if (subtaskEditHandledRef.current) return;
    const id = editingSubtaskId;
    if (!id) return;
    const title = subtaskEditDraft.trim();
    const current = subtasks.find((s) => s.id === id);
    // 空标题或与磁盘值一致：取消 / 无操作，不调命令、不删除
    if (!title || !current || current.title === title) {
      finishSubtaskEdit();
      return;
    }
    if (subtaskBusyRef.current.has(id)) return; // 在途：不重复提交
    if (!selectedTask || !selectedTaskListFile) {
      finishSubtaskEdit();
      return;
    }
    const listFile = selectedTaskListFile;
    const taskId = selectedTask.id;
    subtaskEditHandledRef.current = true; // 先占位，避免失焦再次提交
    markSubtaskBusy(id, true);
    try {
      await invoke("set_subtask_title", {
        listFile,
        taskId,
        subtaskId: id,
        title,
      });
      setSubtasks((prev) =>
        prev.map((s) => (s.id === id ? { ...s, title } : s)),
      );
      applyTaskSubtasks(listFile, taskId, (prev) =>
        prev.map((s) => (s.id === id ? { ...s, title } : s)),
      );
      lastSubtaskErrorRef.current = null;
    } catch (e) {
      reportSubtaskError(String(e)); // 失败：退出编辑态、保留原值
    } finally {
      markSubtaskBusy(id, false);
      // 若期间已切到别的子任务编辑，别把对方的编辑态关掉
      if (editingSubtaskIdRef.current === id) {
        setEditingSubtaskId(null);
        setSubtaskEditDraft("");
      }
    }
  }

  // 取消改名（Esc）：还原、不调命令
  function cancelSubtaskEdit() {
    finishSubtaskEdit();
  }

  // ---- 任务内容行内编辑（双击内容区 / 右键「编辑」；即时写盘，无防抖）----

  // 进入编辑态：预填原值；已在编辑同一任务 → 仅聚焦现有输入
  function startEditTask(entry: ViewTask) {
    const { task, listFile } = entry;
    if (!task.id) return;
    if (editingTaskIdRef.current === task.id) {
      taskEditRef.current?.focus();
      return;
    }
    taskEditHandledRef.current = false;
    editingTaskListFileRef.current = listFile;
    setEditingTaskId(task.id);
    setTaskEditDraft(task.title ?? "");
  }

  // 退出编辑态（不改本地值）
  function finishTaskEdit() {
    taskEditHandledRef.current = true;
    setEditingTaskId(null);
    setTaskEditDraft("");
    editingTaskListFileRef.current = null;
  }

  // 提交改名：空 / 未改动 → 视为取消；否则落盘后乐观更新本地
  async function commitTaskEdit() {
    if (taskEditHandledRef.current) return;
    const id = editingTaskId;
    const listFile = editingTaskListFileRef.current;
    if (!id || !listFile) {
      finishTaskEdit();
      return;
    }
    const title = taskEditDraft.trim();
    const task = lists
      .find((l) => l.file === listFile)
      ?.tasks.find((t) => t.id === id);
    // 空标题 → 还原退出；与当前 title 一致 → 无操作退出（不调命令）
    if (!task || !title || (task.title ?? "").trim() === title) {
      if (editingTaskIdRef.current === id) finishTaskEdit();
      return;
    }
    // 详情是否正显示同一任务（提交前捕获，成功后同步草稿，避免旧值回写）
    const detailShowsSame =
      selectedTaskIdRef.current === id && selectedTaskListFile === listFile;
    taskEditHandledRef.current = true; // 先占位，避免失焦再次提交
    try {
      await invoke("update_task", {
        listFile,
        taskId: id,
        title,
        notes: task.notes ?? "",
      });
      applyTaskTitle(listFile, id, title);
      if (detailShowsSame) {
        // 应用时再核验：期间切走任务 / 用户已改详情草稿 → 不同步
        if (
          selectedTaskIdRef.current === id &&
          editTitleRef.current.trim() === (task.title ?? "").trim()
        ) {
          editTitleRef.current = title;
          setEditTitle(title);
        }
      }
      lastSaveErrorRef.current = null;
    } catch (e) {
      reportSaveError(String(e)); // 失败：退出编辑态、保留原值
    } finally {
      // 若期间已切到别的任务编辑，别把对方的编辑态关掉
      if (editingTaskIdRef.current === id) finishTaskEdit();
    }
  }

  // 取消改名（Esc）：还原、不调命令；紧接的失焦不再提交（handled ref）
  function cancelTaskEdit() {
    finishTaskEdit();
  }

  // 详情激活时的 Esc：关闭详情（只在有选中任务时绑定，不劫持全局）
  const closeDetailRef = useRef<() => void>(() => {});
  closeDetailRef.current = closeDetail;
  useEffect(() => {
    if (!selectedTaskId) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.isComposing) return;
      event.preventDefault();
      closeDetailRef.current();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [selectedTaskId]);

  function removeTask(listFile: string, task: Task) {
    // 直接删除（行内图标 + 悬停显形 + 位置固定防误删，不做 web 确认框）
    // 后端为软删除：条目进入「已删除」，24 小时内可恢复
    void run(
      () => invoke("delete_task", { listFile, taskId: task.id }),
      showDeleteToast,
    );
  }

  // 删除任务后的轻量提示（5 秒自隐；与归档撤销提示互不影响）
  function showDeleteToast() {
    if (deleteToastTimerRef.current !== null) {
      window.clearTimeout(deleteToastTimerRef.current);
    }
    setDeleteToast(true);
    deleteToastTimerRef.current = window.setTimeout(() => {
      deleteToastTimerRef.current = null;
      setDeleteToast(false);
    }, 5000);
  }

  function dismissDeleteToast() {
    if (deleteToastTimerRef.current !== null) {
      window.clearTimeout(deleteToastTimerRef.current);
      deleteToastTimerRef.current = null;
    }
    setDeleteToast(false);
  }

  useEffect(() => () => {
    if (deleteToastTimerRef.current !== null) {
      window.clearTimeout(deleteToastTimerRef.current);
      deleteToastTimerRef.current = null;
    }
  }, []);

  // ---- 回收站：恢复任务 / 清单 ----

  function markRestoring(trashId: string, busy: boolean) {
    const set = restoringIdsRef.current;
    if (busy) set.add(trashId);
    else set.delete(trashId);
    setRestoringIds(new Set(set));
  }

  // 恢复任务：优先回原清单；原清单已不存在时由调用方先打开目标选择模态
  async function restoreTrashTask(trashId: string, targetListFile?: string) {
    if (restoringIdsRef.current.has(trashId)) return;
    markRestoring(trashId, true);
    setActionError(null);
    try {
      await invoke(
        "restore_task",
        targetListFile ? { trashId, targetListFile } : { trashId },
      );
      setRestoreTarget(null);
      await refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      markRestoring(trashId, false);
    }
  }

  // 点「恢复」：原清单仍在 → 直接恢复；否则打开「选择恢复到的清单」
  function requestRestoreTask(item: TrashTaskRef) {
    if (restoringIdsRef.current.has(item.trash_id)) return;
    if (lists.some((l) => l.file === item.list_file)) {
      void restoreTrashTask(item.trash_id, item.list_file);
      return;
    }
    setRestoreTargetFile(lists[0]?.file ?? "");
    setRestoreTarget({
      trashId: item.trash_id,
      title: item.task.title || "(无标题)",
    });
  }

  function confirmRestoreTarget() {
    const target = restoreTarget;
    if (!target || !restoreTargetFile) return;
    void restoreTrashTask(target.trashId, restoreTargetFile);
  }

  // ---- 回收站：右键彻底删除 ----

  function markPurging(trashId: string, busy: boolean) {
    const set = purgingIdsRef.current;
    if (busy) set.add(trashId);
    else set.delete(trashId);
    setPurgingIds(new Set(set));
  }

  function openTrashTaskMenu(event: MouseEvent, item: TrashTaskRef) {
    event.preventDefault();
    event.stopPropagation();
    setTrashMenu({
      trashId: item.trash_id,
      title: item.task.title || "(无标题)",
      kind: "task",
      x: event.clientX,
      y: event.clientY,
    });
  }

  // 已删除清单行右键：与任务行同一个菜单状态，只是确认文案带上清单内任务数
  function openTrashListMenu(event: MouseEvent, item: TrashListRef) {
    event.preventDefault();
    event.stopPropagation();
    setTrashMenu({
      trashId: item.trash_id,
      title: item.name || "(未命名清单)",
      kind: "list",
      taskCount: item.task_count,
      x: event.clientX,
      y: event.clientY,
    });
  }

  // 确认后立即真删除（不等 24 小时）；成功后刷新视图并关闭确认模态
  async function purgeTrashEntry(trashId: string) {
    if (purgingIdsRef.current.has(trashId)) return;
    markPurging(trashId, true);
    setActionError(null);
    try {
      await invoke("purge_trash_entry", { trashId });
      setPurgeTarget(null);
      await refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      markPurging(trashId, false);
    }
  }

  function confirmPurgeTrashEntry() {
    if (!purgeTarget) return;
    void purgeTrashEntry(purgeTarget.trashId);
  }

  // 恢复清单：后端返回新文件名；成功后刷新并选中该清单
  async function restoreTrashList(item: TrashListRef) {
    if (restoringIdsRef.current.has(item.trash_id)) return;
    markRestoring(item.trash_id, true);
    setActionError(null);
    try {
      const file = await invoke<string>("restore_list", {
        trashId: item.trash_id,
      });
      await refresh();
      if (file) {
        setSelectedTaskId(null);
        setDrawerOpen(false);
        setSelectedListFile(file);
      }
    } catch (e) {
      setActionError(String(e));
    } finally {
      markRestoring(item.trash_id, false);
    }
  }

  function openListMenu(event: MouseEvent, list: TaskList) {
    event.preventDefault();
    event.stopPropagation();
    setCompletedGroupMenu(null);
    setMenu({ file: list.file, x: event.clientX, y: event.clientY });
  }

  // 「已完成」组头右键：批量归档入口（左键行为不变，仍是折叠 / 展开）
  function openCompletedGroupMenu(event: MouseEvent, listFile: string) {
    event.preventDefault();
    event.stopPropagation();
    setMenu(null);
    setCompletedGroupMenu({
      listFile,
      x: event.clientX,
      y: event.clientY,
    });
  }

  function startRenameList(list: TaskList) {
    setEditingListFile(list.file);
    setEditListName(list.name);
  }

  function cancelRenameList() {
    setEditingListFile(null);
  }

  function saveRenameList(listFile: string) {
    const name = editListName.trim();
    if (!name) {
      setActionError("清单名称不能为空");
      return;
    }
    void run(async () => {
      await invoke("rename_list", { listFile, newName: name });
      setEditingListFile(null);
    });
  }

  function removeList(list: TaskList) {
    // 打开自定义确认模态（不再用 window.confirm）；用户确认后才真正删除
    setPendingDeleteList(list);
  }

  function confirmRemoveList() {
    const list = pendingDeleteList;
    setPendingDeleteList(null);
    if (!list) return;
    void run(() => invoke("delete_list", { listFile: list.file }));
  }

  function applyStatus(
    prev: TaskList[],
    listFile: string,
    taskId: string,
    done: boolean,
  ): TaskList[] {
    return prev.map((list) =>
      list.file !== listFile
        ? list
        : {
            ...list,
            tasks: list.tasks.map((task) =>
              task.id !== taskId
                ? task
                : {
                    ...task,
                    status: done ? "done" : "open",
                    completed: done ? task.completed : null,
                  },
            ),
          },
    );
  }

  // 勾选 / 取消：先播放移除动画，再从当前视图消失，随后持久化
  function toggle(listFile: string, task: Task, done: boolean) {
    setLeaving((prev) => {
      const next = new Map(prev);
      next.set(task.id, done);
      return next;
    });
    const delay = reduceMotion ? 0 : 200;
    window.setTimeout(async () => {
      setLists((prev) => applyStatus(prev, listFile, task.id, done));
      setLeaving((prev) => {
        const next = new Map(prev);
        next.delete(task.id);
        return next;
      });
      try {
        await invoke("set_task_status", { listFile, taskId: task.id, done });
        await refresh();
      } catch (e) {
        setError(String(e));
        // 失败回滚
        setLists((prev) => applyStatus(prev, listFile, task.id, !done));
      }
    }, delay);
  }

  // ---- 排序持久化 ----

  function reorderTasksLocal(
    listFile: string,
    taskId: string,
    beforeTaskId: string | null,
  ) {
    setLists((prev) =>
      prev.map((list) => {
        if (list.file !== listFile) return list;
        const tasks = [...list.tasks];
        const from = tasks.findIndex((t) => t.id === taskId);
        if (from < 0) return list;
        const [moved] = tasks.splice(from, 1);
        let insertAt = beforeTaskId
          ? tasks.findIndex((t) => t.id === beforeTaskId)
          : tasks.length;
        if (insertAt < 0) insertAt = tasks.length;
        tasks.splice(insertAt, 0, moved);
        return { ...list, tasks };
      }),
    );
  }

  function reorderListsLocal(listFile: string, beforeListFile: string | null) {
    setLists((prev) => {
      const arr = [...prev];
      const from = arr.findIndex((l) => l.file === listFile);
      if (from < 0) return prev;
      const [moved] = arr.splice(from, 1);
      let insertAt = beforeListFile
        ? arr.findIndex((l) => l.file === beforeListFile)
        : arr.length;
      if (insertAt < 0) insertAt = arr.length;
      arr.splice(insertAt, 0, moved);
      return arr;
    });
  }

  // 依据「可见（未完成）任务」的相邻关系算出 before_task_id
  function handleTaskReorder(activeId: string, overId: string) {
    // 本次拖拽已作为「跨清单转移」处理 → 吞掉随后触发的同清单排序
    if (dragMoveHandledRef.current) {
      dragMoveHandledRef.current = false;
      return;
    }
    if (busy || !selectedList) return;
    const from = taskIds.indexOf(activeId);
    const to = taskIds.indexOf(overId);
    if (from < 0 || to < 0 || from === to) return;
    const next = arrayMove(taskIds, from, to);
    const pos = next.indexOf(activeId);
    const beforeTaskId = next[pos + 1] ?? null;
    reorderTasksLocal(selectedList.file, activeId, beforeTaskId);
    void persistOrder(() =>
      invoke("reorder_task", {
        listFile: selectedList.file,
        taskId: activeId,
        beforeTaskId,
      }),
    );
  }

  function handleListReorder(activeId: string, overId: string) {
    if (busy) return;
    const from = listIds.indexOf(activeId);
    const to = listIds.indexOf(overId);
    if (from < 0 || to < 0 || from === to) return;
    const next = arrayMove(listIds, from, to);
    const pos = next.indexOf(activeId);
    const beforeListFile = next[pos + 1] ?? null;
    reorderListsLocal(activeId, beforeListFile);
    void persistOrder(() =>
      invoke("reorder_list", { listFile: activeId, beforeListFile }),
    );
  }

  // ---- 行渲染 ----

  function renderTaskCard(
    entry: ViewTask,
    activatorRef?: (element: HTMLElement | null) => void,
    hideSource = false,
  ) {
    const { task } = entry;
    const isDone = leaving.get(task.id) ?? task.status === "done";
    // 分组视图的组头已承载来源，行内不再重复「来自 X」
    const showSource = !hideSource && !!entry.listName;
    const subs = task.subtasks ?? [];
    const total = subs.length;
    const doneCount = subs.filter((s) => s.status === "done").length;
    const agentKey = taskKey(entry.listFile, task.id);
    const agentLive = agentStates.get(agentKey);
    const isArchived = typeof task.archived === "string";
    const badgeState = rowBadgeState(
      !!task.agent?.enabled,
      agentLive?.state,
      undefined,
    );
    return (
      <div className="task-row">
        {/* 归档项不可取消完成：不渲染完成复选框 */}
        {!isArchived && (
          <input
            type="checkbox"
            className="task-check"
            checked={isDone}
            disabled={!task.id}
            aria-label={task.title || "任务"}
            onClick={(e) => e.stopPropagation()}
            onPointerDown={(e) => e.stopPropagation()}
            onChange={(e) => toggle(entry.listFile, task, e.currentTarget.checked)}
          />
        )}
        {editingTaskId === task.id ? (
          <input
            ref={taskEditRef}
            className="task-edit"
            value={taskEditDraft}
            placeholder="任务…"
            aria-label="编辑任务"
            onChange={(e) => setTaskEditDraft(e.currentTarget.value)}
            onKeyDown={(e) => {
              // IME 组合态一律不触发
              if (e.nativeEvent.isComposing || e.nativeEvent.keyCode === 229)
                return;
              if (e.key === "Enter") {
                e.preventDefault();
                e.stopPropagation();
                void commitTaskEdit();
              } else if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                cancelTaskEdit();
              }
            }}
            onBlur={() => void commitTaskEdit()}
          />
        ) : (
          <button
            type="button"
            className="task-open"
            ref={activatorRef}
            onClick={(e) => openTask(task.id, e.detail === 0)}
            onDoubleClick={() => {
              if (!isNarrow && !suppressClickRef.current) startEditTask(entry);
            }}
            disabled={!task.id}
            title="查看详情"
          >
            <span className="task-title-line">
              <span className={`task-title${isDone ? " done" : ""}`}>
                {task.title || "(无标题)"}
              </span>
              {isArchived && (
                <span className="task-archived-badge">已归档</span>
              )}
            </span>
            {(task.due || showSource) && (
              <span className="task-meta-row">
                {showSource && (
                  <span className="meta">来自 {entry.listName}</span>
                )}
                {task.due && <span className="meta">截止 {task.due}</span>}
              </span>
            )}
          </button>
        )}
        <div className="task-actions">
          {/* 优先级仅作辅助提示：纯展示、不可聚焦、不拦截点击与拖拽 */}
          {task.priority && (
            <span className="task-priority">
              优先级 {PRIORITY_LABEL[task.priority] ?? task.priority}
            </span>
          )}
          {badgeState && (
            <button
              type="button"
              className={`task-agent-badge agent-tone-${
                agentStateMeta(badgeState).tone
              }`}
              onClick={() => openTaskAgent(entry)}
              onPointerDown={(e) => e.stopPropagation()}
              aria-label={agentStateTitle(
                badgeState,
                agentLive?.errorClass,
                agentLive?.stopReason,
              )}
              title={agentStateTitle(
                badgeState,
                agentLive?.errorClass,
                agentLive?.stopReason,
              )}
            >
              <AgentRobotGlyph />
            </button>
          )}
          <div className="task-actions-swap">
            {total > 0 && (
              <span
                className="task-subtask-count"
                title={`已完成 ${doneCount} / 共 ${total} 子任务`}
              >
                {doneCount}/{total}
              </span>
            )}
            <button
              type="button"
              className="row-action danger"
              onClick={(e) => {
                e.stopPropagation();
                removeTask(entry.listFile, task);
              }}
              onPointerDown={(e) => e.stopPropagation()}
              disabled={busy || !task.id}
              aria-label="删除任务"
              title="删除任务"
            >
              <svg
                className="trash-icon"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true"
              >
                <path d="M3 6h18" />
                <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
                <path d="M10 11v6" />
                <path d="M14 11v6" />
              </svg>
            </button>
          </div>
        </div>
      </div>
    );
  }

  function taskRowClass(entry: ViewTask) {
    const { task } = entry;
    const isDone = leaving.get(task.id) ?? task.status === "done";
    const priorityClass =
      task.priority === "high"
        ? " is-priority-high"
        : task.priority === "medium"
          ? " is-priority-medium"
          : "";
    return `task-card${isDone ? " is-done" : ""}${
      typeof task.archived === "string" ? " is-archived" : ""
    }${priorityClass}${task.id === selectedTaskId ? " is-selected" : ""}${
      leaving.has(task.id) ? " is-leaving" : ""
    }`;
  }

  function renderOverlayTask(id: string) {
    const entry = entryById.get(id);
    if (!entry) return null;
    const isDone = entry.task.status === "done";
    const initial =
      Array.from((entry.task.title || "").trim())[0] ?? "·";
    return (
      <>
        <div className="drag-card task-drag">
          <span className={`task-title${isDone ? " done" : ""}`}>
            {entry.task.title || "(无标题)"}
          </span>
        </div>
        {/* 拖到非源清单时卡片收缩，露出这枚首字图标 */}
        <span
          className={`drag-icon${isDone ? " is-done" : ""}`}
          aria-hidden="true"
        >
          {initial}
        </span>
      </>
    );
  }

  function renderListRow(list: TaskList) {
    if (editingListFile === list.file) {
      return (
        <div className="list-nav-edit" onPointerDown={(e) => e.stopPropagation()}>
          <input
            className="edit-list-name"
            autoFocus
            value={editListName}
            placeholder="清单名"
            onChange={(e) => setEditListName(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") saveRenameList(list.file);
              else if (e.key === "Escape") cancelRenameList();
            }}
          />
          <div className="list-actions">
            <button
              type="button"
              className="btn-primary"
              onClick={() => saveRenameList(list.file)}
              disabled={busy}
            >
              保存
            </button>
            <button type="button" onClick={cancelRenameList} disabled={busy}>
              取消
            </button>
          </div>
        </div>
      );
    }
    const count = list.tasks.filter((task) => task.status !== "done").length;
    return (
      <button
        type="button"
        className="list-nav-select"
        onClick={() => selectList(list.file)}
        title={list.name}
      >
        <span className="list-nav-name">{list.name}</span>
        <span className="list-nav-count">{count}</span>
      </button>
    );
  }

  function renderOverlayList(id: string) {
    const list = listById.get(id);
    if (!list) return null;
    return <div className="drag-card list-drag">{list.name}</div>;
  }

  // 任务详情内容（宽屏侧栏与窄屏抽屉共用）
  function saveMetadata(patch: { priority?: "high" | "medium" | "low" | null; due?: string | null; tags?: string[]; reminder?: false | null }): Promise<boolean> {
    if (!selectedTask || !selectedTaskListFile) return Promise.resolve(false);
    const listFile = selectedTaskListFile;
    const taskId = selectedTask.id;
    const operation = metadataQueueRef.current.then(async () => {
      try {
        await invoke("update_task_metadata", { listFile, taskId, patch });
        await refresh();
        return true;
      } catch (e) { setError(String(e)); return false; }
    });
    // operation handles its own failures; chain remains resolved and later patches still run.
    metadataQueueRef.current = operation;
    return operation;
  }

  /**
   * 回收站详情（只读）：回收站任务不在任何可写清单里，
   * 复用普通详情会把编辑写回错误文件，因此单独渲染一套只读板块。
   */
  function renderTrashDetail() {
    const entry = selectedTrashEntry;
    if (!entry) {
      return <p className="hint">选择任务查看详情</p>;
    }
    const task = entry.task;
    const isCollapsed = (key: DetailSectionKey) => collapsedSectionSet.has(key);
    const subs = task.subtasks ?? [];
    const tags = task.tags ?? [];
    const priority = task.priority
      ? PRIORITY_LABEL[task.priority] ?? task.priority
      : "—";
    const reminder = !task.due
      ? "—"
      : task.reminder === false
        ? "到期不提醒"
        : "到期提醒";
    return (
      <div className="detail detail--trash">
        <DetailSection
          sectionKey="title"
          title="标题"
          collapsed={isCollapsed("title")}
          onToggle={() => toggleDetailSection("title")}
        >
          <p className="trash-detail-title">{task.title || "(无标题)"}</p>
          <p className="trash-detail-note">已删除任务；恢复后可编辑</p>
        </DetailSection>

        <DetailSection
          sectionKey="metadata"
          title="任务属性"
          boxed
          collapsed={isCollapsed("metadata")}
          onToggle={() => toggleDetailSection("metadata")}
        >
          <dl className="detail-meta">
            <div className="detail-row">
              <dt>优先级</dt>
              <dd>{priority}</dd>
            </div>
            <div className="detail-row">
              <dt>截止日期</dt>
              <dd>{task.due || "—"}</dd>
            </div>
            <div className="detail-row">
              <dt>标签</dt>
              <dd className="trash-detail-tags">
                {tags.length > 0
                  ? tags.map((tag) => (
                      <span className="tag-chip" key={tag}>
                        {tag}
                      </span>
                    ))
                  : "—"}
              </dd>
            </div>
            <div className="detail-row">
              <dt>提醒</dt>
              <dd>{reminder}</dd>
            </div>
          </dl>
        </DetailSection>

        <DetailSection
          sectionKey="subtasks"
          title={`子任务 · ${subs.length}`}
          collapsed={isCollapsed("subtasks")}
          onToggle={() => toggleDetailSection("subtasks")}
        >
          {subs.length > 0 ? (
            <ul className="subtask-list">
              {subs.map((sub) => {
                const done = sub.status === "done";
                return (
                  <li
                    key={sub.id}
                    className={`subtask${done ? " done" : ""}`}
                  >
                    <span
                      className="trash-subtask-mark"
                      aria-hidden="true"
                    >
                      {done ? "✓" : "○"}
                    </span>
                    <span className="subtask-title">
                      {sub.title || "(无标题)"}
                    </span>
                  </li>
                );
              })}
            </ul>
          ) : (
            <p className="trash-detail-empty">无子任务</p>
          )}
        </DetailSection>

        <DetailSection
          sectionKey="notes"
          title="备注"
          collapsed={isCollapsed("notes")}
          onToggle={() => toggleDetailSection("notes")}
        >
          <p className="trash-detail-notes">{task.notes || "—"}</p>
        </DetailSection>

        <DetailSection
          sectionKey="meta"
          title="其他信息"
          boxed
          collapsed={isCollapsed("meta")}
          onToggle={() => toggleDetailSection("meta")}
        >
          <dl className="detail-meta">
            <div className="detail-row">
              <dt>状态</dt>
              <dd>{task.status === "done" ? "已完成" : "未完成"}</dd>
            </div>
            <div className="detail-row">
              <dt>完成时间</dt>
              <dd>{task.completed || "—"}</dd>
            </div>
            <div className="detail-row">
              <dt>删除时间</dt>
              <dd>{entry.deleted || "—"}</dd>
            </div>
            <div className="detail-row">
              <dt>来源清单</dt>
              <dd>{entry.list_name || entry.list_file || "—"}</dd>
            </div>
            <div className="detail-row">
              <dt>ID</dt>
              <dd className="mono">{task.id || "—"}</dd>
            </div>
          </dl>
        </DetailSection>
      </div>
    );
  }

  function renderDetail() {
    if (isTrashView) return renderTrashDetail();
    if (!selectedTask || !selectedTaskListFile) {
      return <p className="hint">选择任务查看详情</p>;
    }
    const isCollapsed = (key: DetailSectionKey) => collapsedSectionSet.has(key);
    return (
      <div className="detail">
        <DetailSection
          sectionKey="title"
          title="标题"
          collapsed={isCollapsed("title")}
          onToggle={() => toggleDetailSection("title")}
        >
          <input
            className="edit-title"
            aria-labelledby="detail-title-head"
            value={editTitle}
            placeholder="标题"
            onChange={(e) => onEditTitleChange(e.currentTarget.value)}
            onBlur={flushPendingSave}
            onKeyDown={(e) => {
              // Enter = 保存并关闭（IME 组合态不触发）
              if (
                e.key === "Enter" &&
                !e.nativeEvent.isComposing &&
                e.nativeEvent.keyCode !== 229
              ) {
                e.preventDefault();
                closeDetailRef.current();
              }
            }}
          />
        </DetailSection>

        <DetailSection
          sectionKey="metadata"
          title="任务属性"
          hint="随改随存"
          boxed
          collapsed={isCollapsed("metadata")}
          onToggle={() => toggleDetailSection("metadata")}
        >
          <div className="detail-attr-grid">
            <label className="field"><span className="field-label">优先级</span>
              <select aria-label="优先级" value={selectedTask.priority ?? ""} onChange={(e) => void saveMetadata({ priority: (e.currentTarget.value || null) as "high" | "medium" | "low" | null })}>
                <option value="">不设置</option><option value="high">高</option><option value="medium">中</option><option value="low">低</option>
              </select>
            </label>
            <DueEditor value={selectedTask.due} onSave={(due) => void saveMetadata({ due, ...(!due ? { reminder: null } : {}) })} />
            {selectedTask.due && <label className="reminder-toggle"><input type="checkbox" checked={selectedTask.reminder === false} onChange={(e) => void saveMetadata({ reminder: e.currentTarget.checked ? false : null })} /><span>不再提醒</span></label>}
            <TagEditor key={selectedTask.id} tags={selectedTask.tags ?? []} suggestions={[...new Set(lists.flatMap((list) => list.tasks.flatMap((task) => task.tags ?? [])))]} onSave={(tags) => saveMetadata({ tags })} />
          </div>
        </DetailSection>

        <DetailSection
          sectionKey="subtasks"
          title={`子任务 · ${subtasks.length}`}
          collapsed={isCollapsed("subtasks")}
          onToggle={() => toggleDetailSection("subtasks")}
        >
          {subtasks.length > 0 && (
            <ul className="subtask-list">
              {subtasks.map((sub) => {
                const done = sub.status === "done";
                const pending = subtaskBusyIds.has(sub.id);
                const editing = editingSubtaskId === sub.id;
                return (
                  <li
                    key={sub.id}
                    className={`subtask${done ? " done" : ""}${
                      pending ? " is-pending" : ""
                    }${editing ? " editing" : ""}`}
                  >
                    {editing ? (
                      <input
                        ref={subtaskEditRef}
                        className="subtask-edit"
                        value={subtaskEditDraft}
                        placeholder="子任务…"
                        onChange={(e) =>
                          setSubtaskEditDraft(e.currentTarget.value)
                        }
                        onKeyDown={(e) => {
                          // IME 组合态一律不触发
                          if (
                            e.nativeEvent.isComposing ||
                            e.nativeEvent.keyCode === 229
                          )
                            return;
                          if (e.key === "Enter") {
                            e.preventDefault();
                            e.stopPropagation();
                            void commitSubtaskEdit();
                          } else if (e.key === "Escape") {
                            e.preventDefault();
                            e.stopPropagation();
                            cancelSubtaskEdit();
                          }
                        }}
                        onBlur={() => void commitSubtaskEdit()}
                      />
                    ) : (
                      <>
                        <button
                          type="button"
                          className="subtask-mark"
                          onClick={() => void toggleSubtask(sub)}
                          disabled={pending}
                          aria-pressed={done}
                          aria-label={done ? "标记为未完成" : "标记为已完成"}
                          title={done ? "标记为未完成" : "标记为已完成"}
                        >
                          {done ? "✓" : "○"}
                        </button>
                        <span
                          className="subtask-title"
                          onDoubleClick={() => startEditSubtask(sub)}
                          title="双击改名"
                        >
                          {sub.title || "(无标题)"}
                        </span>
                        <button
                          type="button"
                          className="subtask-remove"
                          onClick={() => void removeSubtask(sub)}
                          disabled={pending}
                          aria-label="删除子任务"
                          title="删除子任务"
                        >
                          ×
                        </button>
                      </>
                    )}
                  </li>
                );
              })}
            </ul>
          )}

          <input
            ref={subtaskInputRef}
            className="subtask-input"
            value={subtaskDraft}
            placeholder="新增子任务…"
            aria-label="新增子任务"
            onChange={(e) => setSubtaskDraft(e.currentTarget.value)}
            onKeyDown={(e) => {
              // IME 组合态一律不触发
              if (
                e.nativeEvent.isComposing ||
                e.nativeEvent.keyCode === 229
              )
                return;
              if (e.key === "Enter") {
                // 隔离：不得冒泡触发详情面板的全局行为
                e.preventDefault();
                e.stopPropagation();
                void addSubtask();
              } else if (e.key === "Escape") {
                // 仅清空草稿，输入框常存且保持焦点，不关详情
                e.preventDefault();
                e.stopPropagation();
                setSubtaskDraft("");
              }
            }}
            onBlur={() => {
              /* 失焦不落盘、保留草稿 */
            }}
          />
        </DetailSection>

        <DetailSection
          sectionKey="notes"
          title="备注"
          collapsed={isCollapsed("notes")}
          onToggle={() => toggleDetailSection("notes")}
        >
          <textarea
            className="edit-notes"
            aria-labelledby="detail-notes-head"
            value={editNotes}
            placeholder="备注"
            rows={3}
            onChange={(e) => onEditNotesChange(e.currentTarget.value)}
            onBlur={flushPendingSave}
            onKeyDown={(e) => {
              // Ctrl/Cmd+Enter = 保存并关闭；普通 Enter 保留换行
              if (
                e.key === "Enter" &&
                (e.ctrlKey || e.metaKey) &&
                !e.nativeEvent.isComposing &&
                e.nativeEvent.keyCode !== 229
              ) {
                e.preventDefault();
                closeDetailRef.current();
              }
            }}
          />
        </DetailSection>

        <DetailSection
          sectionKey="meta"
          title="其他信息"
          boxed
          collapsed={isCollapsed("meta")}
          onToggle={() => toggleDetailSection("meta")}
        >
          <dl className="detail-meta">
            <div className="detail-row">
              <dt>状态</dt>
              <dd>{selectedTask.status === "done" ? "已完成" : "未完成"}</dd>
            </div>
            <div className="detail-row">
              <dt>完成时间</dt>
              <dd>{selectedTask.completed || "—"}</dd>
            </div>
            <div className="detail-row">
              <dt>ID</dt>
              <dd className="mono">{selectedTask.id || "—"}</dd>
            </div>
          </dl>
        </DetailSection>

        <DetailSection
          sectionKey="materials"
          title="关联材料"
          titleHint="本地路径与链接都会提供给 Agent 只读访问（链接仅支持 http(s) 抓取）"
          collapsed={isCollapsed("materials")}
          onToggle={() => toggleDetailSection("materials")}
        >
          <MaterialsPanel
            key={selectedTask.id}
            paths={selectedTask.agent?.paths ?? []}
            links={selectedTask.agent?.links ?? []}
            addError={materialsError}
            onAddPaths={(directories) =>
              void addTaskPaths(selectedEntry!, directories)
            }
            onRemovePath={(path) => void removeTaskPath(selectedEntry!, path)}
            onAddLink={(url) => void addTaskLink(selectedEntry!, url)}
            onRemoveLink={(url) => void removeTaskLink(selectedEntry!, url)}
            onOpen={openMaterial}
          />
        </DetailSection>

        <DetailSection
          sectionKey="agent"
          title="Agent 协助"
          boxed
          collapsed={isCollapsed("agent")}
          onToggle={() => toggleDetailSection("agent")}
        >
          <AgentTaskPanel
            enabled={!!selectedTask.agent?.enabled}
            liveState={selectedAgentLive?.state ?? null}
            liveErrorClass={selectedAgentLive?.errorClass ?? null}
            liveStopReason={selectedAgentLive?.stopReason ?? null}
            isActive={isAgentActive(selectedAgentLive?.state)}
            busy={selectedAgentKey !== null && agentBusyKeys.has(selectedAgentKey)}
            runs={agentRuns}
            runsLoading={agentRunsLoading}
            runsHasMore={agentRunsHasMore}
            sectionRef={agentSectionRef}
            onToggleEnabled={(enabled) =>
              void toggleTaskAgent(selectedEntry!, enabled)
            }
            onStart={() => void startTaskAgent(selectedEntry!)}
            onStop={() => void stopTaskAgent(selectedEntry!)}
            onOpenResult={(runId) => void openAgentResult(runId)}
            onLoadMore={() => void loadMoreAgentRuns()}
          />
        </DetailSection>
      </div>
    );
  }

  const menuList = menu ? listById.get(menu.file) ?? null : null;
  const menuItems: MenuItem[] = menuList
    ? [
        {
          key: "rename",
          label: "重命名",
          onSelect: () => startRenameList(menuList),
        },
        { key: "sep-rename", separator: true },
        {
          key: "delete",
          label: "删除",
          danger: true,
          onSelect: () => removeList(menuList),
        },
      ]
    : [];

  // 「已完成」视图：来源清单组头的右键菜单（唯一项＝批量归档该清单的已完成任务）
  const completedGroupMenuTarget = completedGroupMenu
    ? completedBucketedGroups.find(
        (g) => g.listFile === completedGroupMenu.listFile,
      ) ?? null
    : null;
  // 取该组内「已完成且未归档」的任务 id（组内只有可见项，归档项须再过滤）
  const completedGroupArchivableIds = completedGroupMenuTarget
    ? completedGroupMenuTarget.buckets
        .flatMap((bucket) => bucket.tasks)
        .filter((entry) => typeof entry.task.archived !== "string")
        .map((entry) => entry.task.id)
    : [];
  const completedGroupMenuItems: MenuItem[] = completedGroupMenuTarget
    ? [
        {
          key: "archive-group-done",
          label: "归档该清单的已完成任务",
          disabled: completedGroupArchivableIds.length === 0,
          onSelect: () =>
            void archiveTasks(
              completedGroupMenuTarget.listFile,
              completedGroupArchivableIds,
            ),
        },
      ]
    : [];

  const taskMenuEntry = taskMenu
    ? entryById.get(taskMenu.taskId) ?? null
    : null;
  const taskMenuItems: MenuItem[] = taskMenuEntry
    ? buildTaskMenuItems(taskMenuEntry)
    : [];

  // 「已完成」固定项（非真实清单，不可拖拽 / 右键；任务拖拽到此＝勾选完成）
  const completedFooter = (
    <li
      ref={completedFooterRef}
      className={`list-nav-item is-fixed${isCompletedView ? " is-active" : ""}${
        dragOverCompleted ? " is-drop-target" : ""
      }`}
      onContextMenu={(e) => e.preventDefault()}
    >
      <button
        type="button"
        className="list-nav-select"
        onClick={() => {
          // 拖拽落到此处时不要顺带切换视图
          if (suppressClickRef.current) return;
          selectList(COMPLETED_ID);
        }}
        title="已完成"
      >
        <span className="list-nav-name">
          <span className="list-nav-mark" aria-hidden="true">
            ✓
          </span>
          已完成
        </span>
        <span className="list-nav-count">{completedVisibleCount}</span>
      </button>
    </li>
  );

  // 「已删除」固定项（非真实清单，不可拖拽 / 右键）
  const trashFooter = (
    <li
      className={`list-nav-item is-fixed${isTrashView ? " is-active" : ""}`}
      onContextMenu={(e) => e.preventDefault()}
    >
      <button
        type="button"
        className="list-nav-select"
        onClick={() => {
          if (suppressClickRef.current) return;
          selectList(TRASH_ID);
        }}
        title="已删除"
      >
        <span className="list-nav-name">
          <span className="list-nav-mark" aria-hidden="true">
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M3 6h18" />
              <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
              <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
              <path d="M10 11v6" />
              <path d="M14 11v6" />
            </svg>
          </span>
          已删除
        </span>
        <span className="list-nav-count">{trashCount}</span>
      </button>
    </li>
  );

  // 收缩态浮层的几何变量：以抓取点为原点，按源行实际宽高换算非等比比例
  const dragOverlayStyle: CSSProperties | undefined = dragMetrics
    ? ({
        "--drag-grab-x": `${dragMetrics.grabX}px`,
        "--drag-grab-y": `${dragMetrics.grabY}px`,
        "--drag-scale-x": String(
          dragMetrics.width > 0 ? 32 / dragMetrics.width : 1,
        ),
        "--drag-scale-y": String(
          dragMetrics.height > 0 ? 32 / dragMetrics.height : 1,
        ),
      } as CSSProperties)
    : undefined;

  if (loading) {
    // 主题确定前不画亮色内容，避免深色用户闪白
    return (
      <main className="app">
        <div className="app-boot" role="status" aria-live="polite">
          <span className="boot-spinner" aria-hidden="true" />
          <p className="hint">加载中…</p>
        </div>
      </main>
    );
  }

  return (
    <main className={`app${isNarrow ? "" : " app--wide"}`}>
      {!isNarrow && (
        <AppBar
          orientation="vertical"
          entries={appBarEntries}
          onActivate={activateAppBar}
          isDisabled={isAppBarDisabled}
          isPressed={isAppBarPressed}
          reorderDisabled={appBarSaving}
          onReorder={reorderAppBar}
        />
      )}
      <div className="app-main">
        <header className="header">
          <h1>TodoList</h1>
          {isNarrow && (
            <AppBar
              orientation="horizontal"
              entries={appBarEntries}
              onActivate={activateAppBar}
              isDisabled={isAppBarDisabled}
              isPressed={isAppBarPressed}
              reorderDisabled={appBarSaving}
              onReorder={reorderAppBar}
            />
          )}
        </header>

        {warnings.length > 0 && (
          <p className="warning">部分清单加载失败：{warnings.join("；")}</p>
        )}

        {actionError && <p className="error">操作失败：{actionError}</p>}

        {appBarError && !settingsOpen && (
          <p className="error">功能栏排序保存失败：{appBarError}</p>
        )}

        {completedCollapsedError && (
          <p className="error">
            已完成分组折叠保存失败：{completedCollapsedError}
          </p>
        )}

        {completedShowArchivedError && (
          <p className="error">
            显示归档偏好保存失败：{completedShowArchivedError}
          </p>
        )}

        {trashCollapsedError && (
          <p className="error">
            回收站分组折叠保存失败：{trashCollapsedError}
          </p>
        )}

        {detailCollapsedError && (
          <p className="error">
            详情板块折叠保存失败：{detailCollapsedError}
          </p>
        )}

        {error && (
          <p className="error">
            出错了：{error}
            <button type="button" onClick={refresh}>
              重试
            </button>
          </p>
        )}

        <div className="layout">
          <div className="main-col">
            {/* ------------------------------------- 清单条（宽窄一致，位于任务列表上方） */}
            <section className="side-block lists-block">
              <SortableList
                className="list-nav"
                ids={listIds}
                axis="horizontal"
                rowClassName={(id) =>
                  `list-nav-item${id === selectedListFile ? " is-active" : ""}${
                    editingListFile === id ? " is-editing" : ""
                  }${dragDropTarget === id ? " is-drop-target" : ""}`
                }
                itemDisabled={(id) => editingListFile === id}
                rowDataAttributes={(id) => ({ "data-list-file": id })}
                renderItem={(id) => {
                  const list = listById.get(id);
                  return list ? renderListRow(list) : null;
                }}
                renderOverlay={renderOverlayList}
                footer={
                  <>
                    {completedFooter}
                    {trashFooter}
                  </>
                }
                onReorder={handleListReorder}
                onDragStart={markDragStart}
                onDragEnd={markDragEnd}
                onItemContextMenu={(id, event) => {
                  const list = listById.get(id);
                  if (list) openListMenu(event, list);
                }}
              />

              <div className="add-row">
                <input
                  value={newListName}
                  placeholder="新建清单…"
                  onChange={(e) => setNewListName(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") submitNewList();
                  }}
                />
                <button
                  type="button"
                  className="btn-primary add-btn"
                  onClick={submitNewList}
                  disabled={busy}
                  aria-label="新建清单"
                  title="新建清单"
                >
                  +
                </button>
              </div>
            </section>

            {/* ---------------------------------------------- 主区：当前清单 */}
            <section className="main-pane">
              {isTrashView ? (
                <>
                  <div className="pane-head pane-head--trash">
                    <div className="pane-head-title">
                      <h2>已删除</h2>
                      <p className="pane-note">24 小时后自动永久删除</p>
                    </div>
                    <span className="pane-count">{trashCount} 项</span>
                  </div>
                  <div className="task-scroll">
                    {trashError ? (
                      <p className="error">回收站读取失败：{trashError}</p>
                    ) : trashCount === 0 ? (
                      <p className="hint">回收站为空。</p>
                    ) : (
                      <div className="trash-groups">
                        {trashListsVisible.length > 0 && (
                          <section className="trash-section">
                            <h3 className="trash-section-title">已删除的清单</h3>
                            <ul className="trash-list">
                              {trashListsVisible.map((item) => {
                                const restoring = restoringIds.has(
                                  item.trash_id,
                                );
                                return (
                                  <li
                                    className="trash-row"
                                    key={item.trash_id}
                                    onContextMenu={(e) =>
                                      openTrashListMenu(e, item)
                                    }
                                  >
                                    <div className="trash-row-main">
                                      <span className="trash-row-title">
                                        {item.name}
                                      </span>
                                      <span className="trash-row-meta">
                                        {item.task_count} 个任务
                                      </span>
                                    </div>
                                    <div className="trash-row-side">
                                      <span className="trash-remaining">
                                        {formatTrashRemaining(
                                          item.deleted,
                                          trashNow,
                                        )}
                                      </span>
                                      <button
                                        type="button"
                                        className="trash-restore"
                                        onClick={() =>
                                          void restoreTrashList(item)
                                        }
                                        disabled={restoring}
                                        aria-busy={restoring}
                                        aria-label={`恢复清单「${item.name}」`}
                                      >
                                        {restoring ? "恢复中…" : "恢复"}
                                      </button>
                                    </div>
                                  </li>
                                );
                              })}
                            </ul>
                          </section>
                        )}
                        {trashTaskGroups.length > 0 && (
                          <section className="trash-section">
                            <h3 className="trash-section-title">
                              已删除的任务
                            </h3>
                            <div className="trash-task-groups">
                              {trashTaskGroups.map((group) => {
                                const collapsed = collapsedTrashSet.has(
                                  group.listFile,
                                );
                                return (
                                  <section
                                    className="trash-group"
                                    key={group.listFile}
                                  >
                                    <button
                                      type="button"
                                      className="trash-group-head"
                                      aria-expanded={!collapsed}
                                      onClick={() =>
                                        toggleTrashGroup(group.listFile)
                                      }
                                      title={
                                        collapsed
                                          ? `展开「${group.listName}」`
                                          : `折叠「${group.listName}」`
                                      }
                                    >
                                      <span
                                        className="completed-group-chevron"
                                        aria-hidden="true"
                                      >
                                        <svg
                                          viewBox="0 0 24 24"
                                          fill="none"
                                          stroke="currentColor"
                                          strokeWidth="2"
                                          strokeLinecap="round"
                                          strokeLinejoin="round"
                                        >
                                          <path d="M6 9l6 6 6-6" />
                                        </svg>
                                      </span>
                                      <span className="trash-group-name">
                                        {group.listName}
                                      </span>
                                      <span className="trash-group-count">
                                        {group.tasks.length}
                                      </span>
                                    </button>
                                    {!collapsed && (
                                      <div className="trash-group-body">
                                        <ul className="trash-list">
                                          {group.tasks.map((item) => {
                                            const restoring = restoringIds.has(
                                              item.trash_id,
                                            );
                                            const purging = purgingIds.has(
                                              item.trash_id,
                                            );
                                            const isDone =
                                              item.task.status === "done";
                                            const label =
                                              item.task.title || "(无标题)";
                                            return (
                                              <li
                                                className={`trash-row${
                                                  isDone ? " is-done" : ""
                                                }${
                                                  selectedTaskId ===
                                                  item.trash_id
                                                    ? " is-selected"
                                                    : ""
                                                }`}
                                                key={item.trash_id}
                                                onContextMenu={(e) =>
                                                  openTrashTaskMenu(e, item)
                                                }
                                              >
                                                <button
                                                  type="button"
                                                  className="trash-row-open"
                                                  onClick={(e) =>
                                                    openTask(
                                                      item.trash_id,
                                                      e.detail === 0,
                                                    )
                                                  }
                                                  title="查看详情"
                                                >
                                                  <span className="trash-row-main">
                                                    <span className="trash-row-title">
                                                      {label}
                                                    </span>
                                                    {isDone && (
                                                      <span className="trash-done-badge">
                                                        已完成
                                                      </span>
                                                    )}
                                                  </span>
                                                </button>
                                                <div className="trash-row-side">
                                                  <span className="trash-remaining">
                                                    {formatTrashRemaining(
                                                      item.deleted,
                                                      trashNow,
                                                    )}
                                                  </span>
                                                  <button
                                                    type="button"
                                                    className="trash-restore"
                                                    onClick={(e) => {
                                                      e.stopPropagation();
                                                      requestRestoreTask(item);
                                                    }}
                                                    disabled={
                                                      restoring || purging
                                                    }
                                                    aria-busy={restoring}
                                                    aria-label={`恢复任务「${label}」`}
                                                  >
                                                    {restoring
                                                      ? "恢复中…"
                                                      : "恢复"}
                                                  </button>
                                                </div>
                                              </li>
                                            );
                                          })}
                                        </ul>
                                      </div>
                                    )}
                                  </section>
                                );
                              })}
                            </div>
                          </section>
                        )}
                      </div>
                    )}
                  </div>
                </>
              ) : isCompletedView ? (
                <>
                  <div className="pane-head">
                    <h2>已完成</h2>
                    <div className="pane-head-actions">
                      <label className="reminder-toggle">
                        <input
                          type="checkbox"
                          checked={completedShowArchived}
                          onChange={(e) =>
                            changeCompletedShowArchived(e.currentTarget.checked)
                          }
                        />
                        <span>显示归档</span>
                      </label>
                      <span className="pane-count">
                        {completedVisibleCount} 项
                      </span>
                    </div>
                  </div>
                  <div className="task-scroll">
                    {completedBucketedGroups.length === 0 ? (
                      <p className="hint">暂无已完成任务。</p>
                    ) : (
                      <div className="completed-groups">
                        {completedBucketedGroups.map((group) => {
                          const collapsed = collapsedCompletedSet.has(
                            group.listFile,
                          );
                          return (
                            <section
                              className="completed-group"
                              key={group.listFile}
                            >
                              <button
                                type="button"
                                className="completed-group-head"
                                aria-expanded={!collapsed}
                                onClick={() =>
                                  toggleCompletedGroup(group.listFile)
                                }
                                onContextMenu={(e) =>
                                  openCompletedGroupMenu(e, group.listFile)
                                }
                                title={
                                  collapsed
                                    ? `展开「${group.listName}」`
                                    : `折叠「${group.listName}」`
                                }
                              >
                                <span
                                  className="completed-group-chevron"
                                  aria-hidden="true"
                                >
                                  <svg
                                    viewBox="0 0 24 24"
                                    fill="none"
                                    stroke="currentColor"
                                    strokeWidth="2"
                                    strokeLinecap="round"
                                    strokeLinejoin="round"
                                  >
                                    <path d="M6 9l6 6 6-6" />
                                  </svg>
                                </span>
                                <span className="completed-group-name">
                                  {group.listName}
                                </span>
                                <span className="completed-group-count">
                                  {group.count}
                                </span>
                              </button>
                              {!collapsed && (
                                <div className="completed-group-body">
                                  {group.buckets.map((bucket) => (
                                    <section
                                      className="completed-bucket"
                                      key={bucket.bucket}
                                    >
                                      <h3 className="completed-bucket-label">
                                        {bucket.label}
                                      </h3>
                                      <SortableList
                                        sortable={false}
                                        className="task-list"
                                        ids={bucket.tasks.map(
                                          (e) => e.task.id,
                                        )}
                                        rowClassName={(id) =>
                                          taskRowClass(entryById.get(id)!)
                                        }
                                        renderItem={(id) =>
                                          renderTaskCard(
                                            entryById.get(id)!,
                                            undefined,
                                            true,
                                          )
                                        }
                                        onItemContextMenu={(id, event) => {
                                          const entry = entryById.get(id);
                                          if (entry) openTaskMenu(event, entry);
                                        }}
                                      />
                                    </section>
                                  ))}
                                </div>
                              )}
                            </section>
                          );
                        })}
                      </div>
                    )}
                  </div>
                </>
              ) : !selectedList ? (
                <p className="hint pane-empty">暂无清单。</p>
              ) : (
                <>
                  <div className="pane-head">
                    <h2>{selectedList.name}</h2>
                    <span className="pane-count">{viewTasks.length} 项</span>
                  </div>
                  <div className="task-scroll" ref={taskScrollRef}>
                    {viewTasks.length === 0 ? (
                      <p className="hint">暂无任务。</p>
                    ) : (
                      <SortableList
                        className="task-list"
                        ids={taskIds}
                        axis="vertical"
                        rowClassName={(id) => taskRowClass(entryById.get(id)!)}
                        itemDisabled={(id) => editingTaskId === id}
                        renderItem={(id, activatorRef) =>
                          renderTaskCard(entryById.get(id)!, activatorRef)
                        }
                        renderOverlay={renderOverlayTask}
                        overlayCollapsed={dragOutsideSource}
                        overlayStyle={dragOverlayStyle}
                        onReorder={handleTaskReorder}
                        onDragStart={handleTaskDragStart}
                        onDragMove={handleTaskDragMove}
                        onDragEnd={handleTaskDragEnd}
                        onItemContextMenu={(id, event) => {
                          const entry = entryById.get(id);
                          if (entry) openTaskMenu(event, entry);
                        }}
                      />
                    )}
                  </div>
                </>
              )}

              {/* 左栏底部：新增 todo（「已完成」/「已删除」视图隐藏） */}
              {!isCompletedView && !isTrashView && (
                <div className="add-row">
                  <input
                    value={
                      selectedList ? newTaskTitles[selectedList.file] ?? "" : ""
                    }
                    placeholder="新增任务…"
                    disabled={!selectedList}
                    onChange={(e) => {
                      if (!selectedList) return;
                      const value = e.currentTarget.value;
                      setNewTaskTitles((prev) => ({
                        ...prev,
                        [selectedList.file]: value,
                      }));
                    }}
                    onKeyDown={(e) => {
                      // IME 组合态期间回车不提交（与详情标题框一致）
                      if (
                        e.key === "Enter" &&
                        selectedList &&
                        !e.nativeEvent.isComposing &&
                        e.nativeEvent.keyCode !== 229
                      )
                        submitNewTask(selectedList.file);
                    }}
                  />
                  <button
                    type="button"
                    className="btn-primary add-btn"
                    onClick={() =>
                      selectedList && submitNewTask(selectedList.file)
                    }
                    disabled={busy || !selectedList}
                    aria-label="新增任务"
                    title="新增任务"
                  >
                    +
                  </button>
                </div>
              )}
            </section>
          </div>

          {/* ---------------------------------------------- 右列：任务详情 */}
          <aside className="sidebar">
            {/* 任务详情：宽屏为侧栏块，窄屏为右侧抽屉 */}
            <div className={`detail-shell${drawerOpen ? " is-open" : ""}`}>
              <div
                className="drawer-backdrop"
                onClick={closeDetail}
                aria-hidden="true"
              />
              <aside className="detail-panel" aria-label="任务详情">
                <div className="side-head">
                  <h2>任务详情</h2>
                  <button
                    type="button"
                    className="drawer-close"
                    onClick={closeDetail}
                    aria-label="关闭"
                    title="关闭"
                  >
                    ×
                  </button>
                </div>
                {renderDetail()}
              </aside>
            </div>
          </aside>
        </div>
      </div>

      {menu && menuList && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems}
          onClose={() => setMenu(null)}
        />
      )}

      {completedGroupMenu && completedGroupMenuTarget && (
        <ContextMenu
          x={completedGroupMenu.x}
          y={completedGroupMenu.y}
          items={completedGroupMenuItems}
          onClose={() => setCompletedGroupMenu(null)}
        />
      )}

      {taskMenu && taskMenuEntry && (
        <ContextMenu
          x={taskMenu.x}
          y={taskMenu.y}
          items={taskMenuItems}
          onClose={() => setTaskMenu(null)}
        />
      )}

      {trashMenu && (
        <ContextMenu
          x={trashMenu.x}
          y={trashMenu.y}
          items={[
            {
              key: "purge",
              label: "彻底删除",
              danger: true,
              disabled: purgingIds.has(trashMenu.trashId),
              onSelect: () =>
                setPurgeTarget({
                  trashId: trashMenu.trashId,
                  title: trashMenu.title,
                  kind: trashMenu.kind,
                  taskCount: trashMenu.taskCount,
                }),
            },
          ]}
          onClose={() => setTrashMenu(null)}
        />
      )}

      {settingsOpen && (
        <SettingsModal
          theme={theme}
          onThemeChange={changeTheme}
          minimizeToTray={minimizeToTray}
          onMinimizeToTrayChange={changeMinimizeToTray}
          onClose={() => setSettingsOpen(false)}
          appBarEntries={appBarEntries}
          onAppBarReorder={reorderAppBar}
          onAppBarReset={resetAppBar}
          appBarBusy={appBarSaving}
          appBarError={appBarError}
          canResetAppBar={hasSavedAppBarOrder(appBarSaved)}
        />
      )}

      {agentConfigOpen && (
        <AgentConfigModal
          onClose={() => {
            setAgentConfigOpen(false);
            setAgentGateNotice(null);
          }}
          notice={agentGateNotice}
        />
      )}

      {pendingDeleteList && (
        <ConfirmModal
          title="删除清单"
          message={`「${pendingDeleteList.name}」及其中的任务将移入「已删除」，24 小时内可恢复；到期后永久删除。`}
          confirmLabel="删除"
          onConfirm={confirmRemoveList}
          onCancel={() => setPendingDeleteList(null)}
        />
      )}

      {restoreTarget && (
        <RestoreTaskModal
          title={restoreTarget.title}
          lists={lists}
          value={restoreTargetFile}
          busy={restoringIds.has(restoreTarget.trashId)}
          onChange={setRestoreTargetFile}
          onConfirm={confirmRestoreTarget}
          onCancel={() => setRestoreTarget(null)}
        />
      )}

      {purgeTarget && (
        <ConfirmModal
          title="彻底删除"
          message={
            purgeTarget.kind === "list"
              ? `「${purgeTarget.title}」及其中的 ${
                  purgeTarget.taskCount ?? 0
                } 个任务将被永久删除，无法恢复。`
              : `「${purgeTarget.title}」将被永久删除，无法恢复。`
          }
          confirmLabel="彻底删除"
          onConfirm={confirmPurgeTrashEntry}
          onCancel={() => setPurgeTarget(null)}
        />
      )}

      {exitNoteCount !== null && (
        <ConfirmModal
          title="退出将关闭全部便签"
          message={`当前有 ${exitNoteCount} 张便签未关闭；退出应用将一并关闭它们，便签内容不会保存。`}
          confirmLabel="退出"
          onConfirm={() => void invoke("exit_app")}
          onCancel={() => setExitNoteCount(null)}
        />
      )}

      {deleteToast && (
        <div
          className={`toast toast--delete${
            archiveToast ? " toast--stacked" : ""
          }`}
          role="status"
          aria-live="polite"
        >
          <span className="toast-text">
            已移入「已删除」，24 小时内可恢复
          </span>
          <button
            type="button"
            className="toast-action"
            onClick={() => {
              dismissDeleteToast();
              selectList(TRASH_ID);
            }}
          >
            查看
          </button>
        </div>
      )}

      {archiveToast && (
        <div className="toast" role="status" aria-live="polite">
          <span className="toast-text">
            已归档 {archiveToast.taskIds.length} 项
          </span>
          <button
            type="button"
            className="toast-action"
            onClick={() => void undoArchive()}
          >
            撤销
          </button>
        </div>
      )}
    </main>
  );
}

const MODAL_FOCUSABLE =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * 「选择恢复到的清单」模态（FR-APP-37）：
 * 仅当被恢复任务的原清单已不存在时打开；无清单时禁用确认。
 */
function RestoreTaskModal({
  title,
  lists,
  value,
  busy,
  onChange,
  onConfirm,
  onCancel,
}: {
  title: string;
  lists: TaskList[];
  value: string;
  busy: boolean;
  onChange: (file: string) => void;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef(onCancel);
  cancelRef.current = onCancel;

  useEffect(() => {
    const panel = panelRef.current;
    const select = panel?.querySelector<HTMLElement>("select");
    (select ?? panel)?.focus();
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        cancelRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const panel = panelRef.current;
      if (!panel) return;
      const focusables = Array.from(
        panel.querySelectorAll<HTMLElement>(MODAL_FOCUSABLE),
      );
      if (focusables.length === 0) {
        event.preventDefault();
        panel.focus();
        return;
      }
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      const active = document.activeElement;
      if (event.shiftKey) {
        if (active === first || !panel.contains(active)) {
          event.preventDefault();
          last.focus();
        }
      } else if (active === last || !panel.contains(active)) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);

  const canConfirm = lists.length > 0 && !busy;

  return (
    <div className="modal-backdrop" onPointerDown={onCancel}>
      <div
        ref={panelRef}
        className="modal-card"
        role="dialog"
        aria-modal="true"
        aria-label="选择恢复到的清单"
        tabIndex={-1}
        onPointerDown={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>选择恢复到的清单</h2>
        </div>
        <div className="confirm-body">
          <p className="confirm-message">
            原清单已不存在，请选择「{title}」恢复到的清单。
          </p>
          {lists.length === 0 ? (
            <p className="hint">没有可恢复到的清单</p>
          ) : (
            <label className="restore-target-field">
              <span className="field-label">恢复到</span>
              <select
                className="restore-target-select"
                value={value}
                disabled={busy}
                onChange={(e) => onChange(e.currentTarget.value)}
              >
                {lists.map((list) => (
                  <option key={list.file} value={list.file}>
                    {list.name}
                  </option>
                ))}
              </select>
            </label>
          )}
          <div className="confirm-actions">
            <button type="button" onClick={onCancel}>
              取消
            </button>
            <button
              type="button"
              className="btn-primary"
              disabled={!canConfirm}
              onClick={onConfirm}
            >
              {busy ? "恢复中…" : "恢复"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function DueEditor({ value, onSave }: { value: string | null; onSave: (value: string | null) => void }) {
  const [date, setDate] = useState(value?.slice(0, 10) ?? "");
  const [time, setTime] = useState(value && value.length > 10 ? value.slice(11, 16) : "");
  useEffect(() => { setDate(value?.slice(0, 10) ?? ""); setTime(value && value.length > 10 ? value.slice(11, 16) : ""); }, [value]);
  const save = (d: string, t: string) => onSave(d ? `${d}${t ? `T${t}` : ""}` : null);
  return <div className="field"><span className="field-label">截止日期</span><div className="due-controls">
    <input aria-label="截止日期" type="date" value={date} onChange={(e) => { setDate(e.currentTarget.value); save(e.currentTarget.value, time); }} />
    <input aria-label="截止时间（可选）" type="time" value={time} disabled={!date} onChange={(e) => setTime(e.currentTarget.value)} onBlur={() => save(date, time)} />
    {date && <button type="button" className="clear-due" onClick={() => { setDate(""); setTime(""); onSave(null); }}>清除</button>}
  </div></div>;
}

function TagEditor({ tags, suggestions, onSave }: { tags: string[]; suggestions: string[]; onSave: (tags: string[]) => Promise<boolean> }) {
  const [draft, setDraft] = useState("");
  const [current, setCurrent] = useState(tags);
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const committedRef = useRef(tags);
  const persist = async (next: string[]) => { if (savingRef.current) return; savingRef.current = true; setSaving(true); setCurrent(next); try { const succeeded = await onSave(next); if (succeeded) committedRef.current = next; else setCurrent(committedRef.current); } catch { setCurrent(committedRef.current); } finally { savingRef.current = false; setSaving(false); } };
  const add = (raw = draft) => { const tag = raw.trim(); if (tag && !current.includes(tag)) void persist([...current, tag]); setDraft(""); };
  return <div className="field"><label className="field-label" htmlFor="task-tag-input">标签</label><div className="tag-editor">
    {current.map((tag) => <span className="tag-chip" key={tag}>{tag}<button type="button" disabled={saving} aria-label={`移除标签 ${tag}`} onClick={() => void persist(current.filter((item) => item !== tag))}>×</button></span>)}
    <input id="task-tag-input" disabled={saving} list="task-tag-suggestions" value={draft} placeholder="输入标签…" onChange={(e) => setDraft(e.currentTarget.value)} onKeyDown={(e) => { if (e.key === "Enter" || e.key === ",") { e.preventDefault(); add(); } }} onBlur={() => { if (draft.trim()) add(); }} />
    <datalist id="task-tag-suggestions">{suggestions.map((tag) => <option key={tag} value={tag} />)}</datalist>
  </div></div>;
}

/** 行尾 Agent 机器人小图标（与既有内联图标同风格）。 */
function AgentRobotGlyph() {
  return (
    <svg
      className="task-agent-icon"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <rect x="4" y="8" width="16" height="11" rx="3" />
      <path d="M12 8V5.2" />
      <circle cx="12" cy="4" r="1.1" />
      <path d="M9 12v2" />
      <path d="M15 12v2" />
      <path d="M9.5 16.5h5" />
      <path d="M4 12.5H2.6" />
      <path d="M20 12.5h1.4" />
    </svg>
  );
}

export default App;
