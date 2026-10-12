import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { arrayMove } from "@dnd-kit/sortable";
import { SortableList } from "./components/SortableList";
import { ContextMenu, type MenuItem } from "./components/ContextMenu";
import { SettingsModal } from "./components/SettingsModal";
import { ConfirmModal } from "./components/ConfirmModal";
import { AppBar } from "./components/AppBar";
import { AgentConfigModal } from "./components/AgentConfigModal";
import { applyThemePref, type ThemePref } from "./theme";
import { loadSettings } from "./settings";
import {
  APP_BAR_ENTRIES,
  DEFAULT_APP_BAR_ORDER,
  hasSavedAppBarOrder,
  mergeAppBarOrder,
  resolveAppBarOrder,
  visibleAppBarIds,
  type AppBarEntryMeta,
} from "./appBar";
import { useMediaQuery, usePrefersReducedMotion } from "./hooks";
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
  notes: string | null;
  subtasks: Subtask[] | null;
  /** Phase 2 起由后端返回的任务级 Agent 意图（本 Phase 仅预定义字段，不接线 UI）。 */
  agent?: { enabled: boolean; paths: string[] } | null;
};

type TaskList = {
  name: string;
  file: string;
  tasks: Task[];
};

type LoadResult = {
  lists: TaskList[];
  warnings: string[];
};

/** 视图中的一条任务（含其来源清单）。 */
type ViewTask = { task: Task; listFile: string; listName?: string };

const PRIORITY_LABEL: Record<string, string> = {
  high: "高",
  medium: "中",
  low: "低",
};

/** 「已完成」虚拟清单的哨兵 id（非真实文件）。 */
const COMPLETED_ID = "__completed__";

function App() {
  const [lists, setLists] = useState<TaskList[]>([]);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // 窗口置顶（pin）
  const [pinned, setPinned] = useState(false);
  const [pinBusy, setPinBusy] = useState(false);

  // 设置面板（模态）与主题偏好
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [theme, setTheme] = useState<ThemePref>("system");
  // Agent 配置模态（Phase 1）
  const [agentConfigOpen, setAgentConfigOpen] = useState(false);

  // 待确认删除的清单（非空即显示确认模态）
  const [pendingDeleteList, setPendingDeleteList] = useState<TaskList | null>(
    null,
  );

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

  // 任务详情面板的编辑草稿
  const [editTitle, setEditTitle] = useState("");
  const [editNotes, setEditNotes] = useState("");

  // 详情即时保存：输入值镜像到 ref，供防抖与 flush 读取最新值
  const editTitleRef = useRef("");
  const editNotesRef = useRef("");
  const saveTimerRef = useRef<number | null>(null);
  // 同一错误去重/节流：记住上一次已提示的保存错误
  const lastSaveErrorRef = useRef<string | null>(null);

  // ---- 子任务编辑（独立即时写盘通道，无防抖）----
  // 本地镜像：增 / 删 / 勾选成功后按命令返回值就地更新，不做整表刷新
  const [subtasks, setSubtasks] = useState<Subtask[]>([]);
  // 就地录入框的开启态与草稿（失焦保留、关详情丢弃）
  const [subtaskDraftOpen, setSubtaskDraftOpen] = useState(false);
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

  const [editingListFile, setEditingListFile] = useState<string | null>(null);
  const [editListName, setEditListName] = useState("");

  // 拖拽结束后的「点击抑制」：避免松手时误触发选中
  const suppressClickRef = useRef(false);
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
  const selectedTask = selectedEntry?.task ?? null;
  const selectedTaskListFile = selectedEntry?.listFile ?? null;

  // 磁盘侧子任务的「指纹」：选中任务切换、或外部/刷新/本地改动子任务时重新同步本地镜像
  const selectedSubtasksKey = JSON.stringify(selectedTask?.subtasks ?? null);
  // 切换任务：重置录入草稿与改名编辑态（仅在换任务时，避免本地改动子任务时误清空）
  useEffect(() => {
    setSubtaskDraftOpen(false);
    setSubtaskDraft("");
    setEditingSubtaskId(null);
    setSubtaskEditDraft("");
    subtaskEditHandledRef.current = true;
  }, [selectedTaskId]);

  // 子任务数据变化（详情内增 / 删 / 勾 / 改名，或外部刷新）→ 同步本地镜像
  useEffect(() => {
    setSubtasks(selectedTask?.subtasks ?? []);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedTaskId, selectedSubtasksKey]);

  // 录入框出现即聚焦（含连续录入时保持聚焦）
  useEffect(() => {
    if (subtaskDraftOpen) subtaskInputRef.current?.focus();
  }, [subtaskDraftOpen]);

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
    try {
      const data = await invoke<LoadResult>("load_lists");
      setLists(data.lists);
      setWarnings(data.warnings);
      setError(null);
    } catch (e) {
      setError(String(e));
      // 硬失败时清空可能过期的告警
      setWarnings([]);
    } finally {
      setLoading(false);
    }
  }

  // 启动：先确定主题（首帧前 setTheme）并读取功能栏顺序，再加载清单；主题未定前不画亮色内容
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const { theme: pref, appBarOrder: order } = await loadSettings();
      if (cancelled) return;
      setTheme(pref);
      setAppBarSaved(order);
      setAppBarOrder(resolveAppBarOrder(order));
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

  // 选中清单：仍存在则保持，否则回落到第一个（无清单则为空）
  useEffect(() => {
    setSelectedListFile((prev) => {
      if (prev === COMPLETED_ID) return prev;
      if (prev && lists.some((l) => l.file === prev)) return prev;
      return lists[0]?.file ?? null;
    });
  }, [lists]);

  // 选中任务：在当前视图内仍存在则保持，否则清空（清空前 flush 待写改动）
  useEffect(() => {
    const ids = isCompletedView
      ? completedTasks.map((e) => e.task.id)
      : selectedList?.tasks.map((t) => t.id) ?? [];
    setSelectedTaskId((prev) => {
      if (prev && ids.includes(prev)) return prev;
      if (prev) flushPendingSave();
      return null;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lists, selectedListFile]);

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
  async function run(action: () => Promise<unknown>) {
    if (busy) return;
    setBusy(true);
    setActionError(null);
    try {
      await action();
      await refresh();
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

  // 切换主题：立即生效；失败记入 actionError（不弹窗）
  function changeTheme(next: ThemePref) {
    setTheme(next);
    void (async () => {
      setActionError(null);
      try {
        await applyThemePref(next);
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

  function submitNewTask(listFile: string) {
    const title = (newTaskTitles[listFile] ?? "").trim();
    if (!title) return;
    void run(async () => {
      await invoke("create_task", { listFile, title });
      setNewTaskTitles((prev) => ({ ...prev, [listFile]: "" }));
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
    setSubtaskDraftOpen(false);
    setSubtaskDraft("");
    subtaskEditHandledRef.current = true;
    setEditingSubtaskId(null);
    setSubtaskEditDraft("");
    // 焦点归还：指针打开的详情，关闭时清掉行按钮焦点（避免 Esc 触发 :focus-visible 焦点环）；
    // 键盘打开的保留焦点与焦点环。禁止全局 outline:none。
    if (!openedByKeyboardRef.current) {
      const active = document.activeElement;
      if (active instanceof HTMLElement && active.classList.contains("task-open")) {
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

  function openSubtaskDraft() {
    setSubtaskDraftOpen(true);
  }

  // 关闭录入框并丢弃未提交草稿（空回车 / Esc / 关详情）
  function discardSubtaskDraft() {
    setSubtaskDraftOpen(false);
    setSubtaskDraft("");
  }

  async function addSubtask() {
    const title = subtaskDraft.trim();
    if (!title) {
      discardSubtaskDraft();
      return;
    }
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
    void run(() => invoke("delete_task", { listFile, taskId: task.id }));
  }

  function openListMenu(event: MouseEvent, list: TaskList) {
    event.preventDefault();
    event.stopPropagation();
    setMenu({ file: list.file, x: event.clientX, y: event.clientY });
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
  ) {
    const { task } = entry;
    const isDone = leaving.get(task.id) ?? task.status === "done";
    const subs = task.subtasks ?? [];
    const total = subs.length;
    const doneCount = subs.filter((s) => s.status === "done").length;
    return (
      <div className="task-row">
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
        <button
          type="button"
          className="task-open"
          ref={activatorRef}
          onClick={(e) => openTask(task.id, e.detail === 0)}
          disabled={!task.id}
          title="查看详情"
        >
          <span className={`task-title${isDone ? " done" : ""}`}>
            {task.title || "(无标题)"}
          </span>
          {(task.priority || task.due || entry.listName) && (
            <span className="task-meta-row">
              {entry.listName && <span className="meta">来自 {entry.listName}</span>}
              {task.priority && (
                <span className="meta">
                  优先级 {PRIORITY_LABEL[task.priority] ?? task.priority}
                </span>
              )}
              {task.due && <span className="meta">截止 {task.due}</span>}
            </span>
          )}
        </button>
        <div className="task-actions">
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
    );
  }

  function taskRowClass(entry: ViewTask) {
    const { task } = entry;
    const isDone = leaving.get(task.id) ?? task.status === "done";
    return `task-card${isDone ? " is-done" : ""}${
      task.id === selectedTaskId ? " is-selected" : ""
    }${leaving.has(task.id) ? " is-leaving" : ""}`;
  }

  function renderOverlayTask(id: string) {
    const entry = entryById.get(id);
    if (!entry) return null;
    const isDone = entry.task.status === "done";
    return (
      <div className="drag-card task-drag">
        <span className={`task-title${isDone ? " done" : ""}`}>
          {entry.task.title || "(无标题)"}
        </span>
      </div>
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
  function renderDetail() {
    if (!selectedTask || !selectedTaskListFile) {
      return <p className="hint">选择任务查看详情</p>;
    }
    return (
      <div className="detail">
        <label className="field">
          <span className="field-label">标题</span>
          <input
            className="edit-title"
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
        </label>

        <div className="subtask-block">
          <div className="subtask-head">
            <span className="field-label">子任务 · {subtasks.length}</span>
            <button
              type="button"
              className="subtask-add"
              onClick={openSubtaskDraft}
              aria-label="添加子任务"
              title="添加子任务"
            >
              +
            </button>
          </div>

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

          {subtaskDraftOpen && (
            <input
              ref={subtaskInputRef}
              className="subtask-input"
              value={subtaskDraft}
              placeholder="子任务…"
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
                  e.preventDefault();
                  e.stopPropagation();
                  discardSubtaskDraft();
                }
              }}
              onBlur={() => {
                /* 失焦不落盘、保留草稿 */
              }}
            />
          )}
        </div>

        <label className="field">
          <span className="field-label">备注</span>
          <textarea
            className="edit-notes"
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
        </label>

        <dl className="detail-meta">
          <div className="detail-row">
            <dt>状态</dt>
            <dd>{selectedTask.status === "done" ? "已完成" : "未完成"}</dd>
          </div>
          <div className="detail-row">
            <dt>优先级</dt>
            <dd>
              {selectedTask.priority
                ? PRIORITY_LABEL[selectedTask.priority] ?? selectedTask.priority
                : "—"}
            </dd>
          </div>
          <div className="detail-row">
            <dt>截止</dt>
            <dd>{selectedTask.due || "—"}</dd>
          </div>
          <div className="detail-row">
            <dt>标签</dt>
            <dd>
              {selectedTask.tags && selectedTask.tags.length > 0
                ? selectedTask.tags.join("、")
                : "—"}
            </dd>
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
        {
          key: "delete",
          label: "删除",
          danger: true,
          onSelect: () => removeList(menuList),
        },
      ]
    : [];

  // 「已完成」固定项（非真实清单，不可拖拽 / 右键）
  const completedFooter = (
    <li
      className={`list-nav-item is-fixed${isCompletedView ? " is-active" : ""}`}
      onContextMenu={(e) => e.preventDefault()}
    >
      <button
        type="button"
        className="list-nav-select"
        onClick={() => selectList(COMPLETED_ID)}
        title="已完成"
      >
        <span className="list-nav-name">
          <span className="list-nav-mark" aria-hidden="true">
            ✓
          </span>
          已完成
        </span>
        <span className="list-nav-count">{completedTasks.length}</span>
      </button>
    </li>
  );

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

        {error && (
          <p className="error">
            出错了：{error}
            <button type="button" onClick={refresh}>
              重试
            </button>
          </p>
        )}

        <div className="layout">
          {/* ---------------------------------------------- 主区：当前清单 */}
          <section className="main-pane">
            {isCompletedView ? (
              <>
                <div className="pane-head">
                  <h2>已完成</h2>
                  <span className="pane-count">{completedTasks.length} 项</span>
                </div>
                <div className="task-scroll">
                  {completedTasks.length === 0 ? (
                    <p className="hint">暂无已完成任务。</p>
                  ) : (
                    <SortableList
                      sortable={false}
                      className="task-list"
                      ids={taskIds}
                      rowClassName={(id) =>
                        taskRowClass(entryById.get(id)!)
                      }
                      renderItem={(id) => renderTaskCard(entryById.get(id)!)}
                    />
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
                <div className="task-scroll">
                  {viewTasks.length === 0 ? (
                    <p className="hint">暂无任务。</p>
                  ) : (
                    <SortableList
                      className="task-list"
                      ids={taskIds}
                      axis="vertical"
                      rowClassName={(id) => taskRowClass(entryById.get(id)!)}
                      renderItem={(id, activatorRef) =>
                        renderTaskCard(entryById.get(id)!, activatorRef)
                      }
                      renderOverlay={renderOverlayTask}
                      onReorder={handleTaskReorder}
                      onDragStart={markDragStart}
                      onDragEnd={markDragEnd}
                    />
                  )}
                </div>
              </>
            )}

            {/* 左栏底部：新增 todo（「已完成」视图隐藏） */}
            {!isCompletedView && (
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

          {/* ---------------------------------------------- 右侧边栏 */}
          <aside className="sidebar">
            <section className="side-block lists-block">
              <div className="side-head">
                <h2>清单</h2>
                <span className="side-count">{lists.length}</span>
              </div>

              <SortableList
                className="list-nav"
                ids={listIds}
                axis={isNarrow ? "horizontal" : "vertical"}
                rowClassName={(id) =>
                  `list-nav-item${id === selectedListFile ? " is-active" : ""}${
                    editingListFile === id ? " is-editing" : ""
                  }`
                }
                itemDisabled={(id) => editingListFile === id}
                renderItem={(id) => {
                  const list = listById.get(id);
                  return list ? renderListRow(list) : null;
                }}
                renderOverlay={renderOverlayList}
                footer={completedFooter}
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

      {settingsOpen && (
        <SettingsModal
          theme={theme}
          onThemeChange={changeTheme}
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
        <AgentConfigModal onClose={() => setAgentConfigOpen(false)} />
      )}

      {pendingDeleteList && (
        <ConfirmModal
          title="删除清单"
          message={`「${pendingDeleteList.name}」及其中的任务将一并删除，且无法撤销。对应文件也会一并删除。`}
          confirmLabel="删除"
          onConfirm={confirmRemoveList}
          onCancel={() => setPendingDeleteList(null)}
        />
      )}
    </main>
  );
}

export default App;
