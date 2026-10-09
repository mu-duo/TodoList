import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { arrayMove } from "@dnd-kit/sortable";
import { SortableList } from "./components/SortableList";
import { ContextMenu, type MenuItem } from "./components/ContextMenu";
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

  const [editingListFile, setEditingListFile] = useState<string | null>(null);
  const [editListName, setEditListName] = useState("");

  // 拖拽结束后的「点击抑制」：避免松手时误触发选中
  const suppressClickRef = useRef(false);
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

  const listIds = useMemo(() => lists.map((l) => l.file), [lists]);
  const taskIds = useMemo(() => viewTasks.map((e) => e.task.id), [viewTasks]);
  const entryById = useMemo(
    () => new Map(viewTasks.map((e) => [e.task.id, e])),
    [viewTasks],
  );
  const listById = useMemo(() => new Map(lists.map((l) => [l.file, l])), [lists]);

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

  useEffect(() => {
    refresh();
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

  // 选中任务：在当前视图内仍存在则保持，否则清空
  useEffect(() => {
    const ids = isCompletedView
      ? completedTasks.map((e) => e.task.id)
      : selectedList?.tasks.map((t) => t.id) ?? [];
    setSelectedTaskId((prev) => (prev && ids.includes(prev) ? prev : null));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lists, selectedListFile]);

  // 没有选中任务时关闭抽屉
  useEffect(() => {
    if (!selectedTaskId) setDrawerOpen(false);
  }, [selectedTaskId]);

  // 选中项切换时，把详情草稿同步为任务当前值（刷新时不覆盖正在编辑的草稿）
  useEffect(() => {
    setEditTitle(selectedEntry?.task.title ?? "");
    setEditNotes(selectedEntry?.task.notes ?? "");
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

  function selectList(file: string) {
    if (suppressClickRef.current) return;
    if (file === selectedListFile) return;
    setSelectedListFile(file);
    setSelectedTaskId(null);
    setDrawerOpen(false);
  }

  function openTask(taskId: string) {
    if (suppressClickRef.current) return;
    if (!taskId) return;
    setSelectedTaskId(taskId);
    setDrawerOpen(true);
  }

  function closeDrawer() {
    setDrawerOpen(false);
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

  function cancelEditTask() {
    if (!selectedTask) return;
    setEditTitle(selectedTask.title);
    setEditNotes(selectedTask.notes ?? "");
  }

  function saveTask(listFile: string, taskId: string) {
    const title = editTitle.trim();
    if (!title) {
      setActionError("任务标题不能为空");
      return;
    }
    void run(async () => {
      await invoke("update_task", { listFile, taskId, title, notes: editNotes });
    });
  }

  function removeTask(listFile: string, task: Task) {
    if (!window.confirm(`删除任务「${task.title || "(无标题)"}」？`)) return;
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
    if (!window.confirm(`删除清单「${list.name}」？该操作会删除对应文件。`)) return;
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

  function renderTaskCard(entry: ViewTask) {
    const { task } = entry;
    const isDone = leaving.get(task.id) ?? task.status === "done";
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
          onClick={() => openTask(task.id)}
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
          <button
            type="button"
            className="row-action danger"
            onClick={(e) => {
              e.stopPropagation();
              removeTask(entry.listFile, task);
            }}
            onPointerDown={(e) => e.stopPropagation()}
            disabled={busy || !task.id}
          >
            删除
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
            onChange={(e) => setEditTitle(e.currentTarget.value)}
          />
        </label>
        <label className="field">
          <span className="field-label">备注</span>
          <textarea
            className="edit-notes"
            value={editNotes}
            placeholder="备注"
            rows={3}
            onChange={(e) => setEditNotes(e.currentTarget.value)}
          />
        </label>
        <div className="edit-actions">
          <button
            type="button"
            className="btn-primary"
            onClick={() => saveTask(selectedTaskListFile, selectedTask.id)}
            disabled={busy}
          >
            保存
          </button>
          <button type="button" onClick={cancelEditTask} disabled={busy}>
            取消
          </button>
        </div>

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
            <dt>子任务</dt>
            <dd>
              {selectedTask.subtasks && selectedTask.subtasks.length > 0 ? (
                <ul className="subtask-list">
                  {selectedTask.subtasks.map((sub) => (
                    <li
                      key={sub.id}
                      className={sub.status === "done" ? "subtask done" : "subtask"}
                    >
                      <span className="subtask-mark" aria-hidden="true">
                        {sub.status === "done" ? "✓" : "○"}
                      </span>
                      <span>{sub.title || "(无标题)"}</span>
                    </li>
                  ))}
                </ul>
              ) : (
                "—"
              )}
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

  return (
    <main className="app">
      <div className="app-main">
        <header className="header">
          <h1>TodoList</h1>
          <div className="header-actions">
            <button type="button" className="refresh" onClick={refresh}>
              刷新
            </button>
            <button
              type="button"
              className={`pin-btn${pinned ? " is-pinned" : ""}`}
              onClick={togglePin}
              disabled={pinBusy}
              aria-pressed={pinned}
              aria-label={pinned ? "取消置顶" : "置顶"}
              title={pinned ? "取消置顶" : "置顶"}
            >
              <svg
                className="pin-icon"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.8"
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true"
              >
                <path d="M12 17v5" />
                <path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" />
              </svg>
            </button>
          </div>
        </header>

        {warnings.length > 0 && (
          <p className="warning">部分清单加载失败：{warnings.join("；")}</p>
        )}

        {actionError && <p className="error">操作失败：{actionError}</p>}

        {loading && <p className="hint">加载中…</p>}
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
                      renderItem={(id) => renderTaskCard(entryById.get(id)!)}
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
                    if (e.key === "Enter" && selectedList)
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
                onClick={closeDrawer}
                aria-hidden="true"
              />
              <aside className="detail-panel" aria-label="任务详情">
                <div className="side-head">
                  <h2>任务详情</h2>
                  <button
                    type="button"
                    className="drawer-close"
                    onClick={closeDrawer}
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
    </main>
  );
}

export default App;
