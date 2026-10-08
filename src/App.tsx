import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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

const PRIORITY_LABEL: Record<string, string> = {
  high: "高",
  medium: "中",
  low: "低",
};

function App() {
  const [lists, setLists] = useState<TaskList[]>([]);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const [newListName, setNewListName] = useState("");
  const [newTaskTitles, setNewTaskTitles] = useState<Record<string, string>>({});

  // 当前选中的清单 / 任务
  const [selectedListFile, setSelectedListFile] = useState<string | null>(null);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  // 窄屏下任务详情以抽屉形式从右侧滑出
  const [drawerOpen, setDrawerOpen] = useState(false);

  // 任务详情面板的编辑草稿
  const [editTitle, setEditTitle] = useState("");
  const [editNotes, setEditNotes] = useState("");

  const [editingListFile, setEditingListFile] = useState<string | null>(null);
  const [editListName, setEditListName] = useState("");

  const selectedList = lists.find((l) => l.file === selectedListFile) ?? null;
  const selectedTask =
    selectedList?.tasks.find((t) => t.id === selectedTaskId) ?? null;

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

  // 选中清单：仍存在则保持，否则回落到第一个（无清单则为空）
  useEffect(() => {
    setSelectedListFile((prev) => {
      if (prev && lists.some((l) => l.file === prev)) return prev;
      return lists[0]?.file ?? null;
    });
  }, [lists]);

  // 选中任务：在当前清单内仍存在则保持，否则清空
  useEffect(() => {
    const list = lists.find((l) => l.file === selectedListFile);
    setSelectedTaskId((prev) =>
      prev && list?.tasks.some((t) => t.id === prev) ? prev : null,
    );
  }, [lists, selectedListFile]);

  // 没有选中任务时关闭抽屉
  useEffect(() => {
    if (!selectedTaskId) setDrawerOpen(false);
  }, [selectedTaskId]);

  // 选中项切换时，把详情草稿同步为任务当前值（刷新时不覆盖正在编辑的草稿）
  useEffect(() => {
    const task = lists
      .find((l) => l.file === selectedListFile)
      ?.tasks.find((t) => t.id === selectedTaskId);
    setEditTitle(task?.title ?? "");
    setEditNotes(task?.notes ?? "");
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

  function selectList(file: string) {
    if (file === selectedListFile) return;
    setSelectedListFile(file);
    setSelectedTaskId(null);
    setDrawerOpen(false);
  }

  function selectTask(taskId: string) {
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
      await invoke("update_task", {
        listFile,
        taskId,
        title,
        notes: editNotes,
      });
    });
  }

  function removeTask(listFile: string, task: Task) {
    if (!window.confirm(`删除任务「${task.title || "(无标题)"}」？`)) return;
    void run(() => invoke("delete_task", { listFile, taskId: task.id }));
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

  function moveList(listFile: string, direction: "up" | "down") {
    void run(() => invoke("move_list", { listFile, direction }));
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

  async function toggle(listFile: string, task: Task, done: boolean) {
    setLists((prev) => applyStatus(prev, listFile, task.id, done));
    try {
      await invoke("set_task_status", { listFile, taskId: task.id, done });
      await refresh();
    } catch (e) {
      setError(String(e));
      // 失败回滚勾选状态
      setLists((prev) => applyStatus(prev, listFile, task.id, !done));
    }
  }

  // 任务详情内容（宽屏侧栏与窄屏抽屉共用）
  function renderDetail() {
    if (!selectedTask || !selectedList) {
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
            onClick={() => saveTask(selectedList.file, selectedTask.id)}
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

  return (
    <main className="app">
      <div className="app-main">
      <header className="header">
        <h1>TodoList</h1>
        <div className="header-actions">
          <button type="button" className="refresh" onClick={refresh}>
            刷新
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
        {/* ------------------------------------------------- 主区：当前清单 */}
        <section className="main-pane">
          {!selectedList ? (
            <p className="hint pane-empty">暂无清单。</p>
          ) : (
            <>
              <div className="pane-head">
                <h2>{selectedList.name}</h2>
                <span className="pane-count">{selectedList.tasks.length} 项</span>
              </div>

              <div className="task-scroll">
              {selectedList.tasks.length === 0 ? (
                <p className="hint">暂无任务。</p>
              ) : (
                <ul className="task-list">
                  {selectedList.tasks.map((task, index) => {
                    const isDone = task.status === "done";
                    const isSelected = task.id === selectedTaskId;
                    return (
                      <li
                        key={task.id || `${selectedList.file}-${index}`}
                        className={`task-card${isDone ? " is-done" : ""}${
                          isSelected ? " is-selected" : ""
                        }`}
                        style={{ animationDelay: `${Math.min(index * 40, 240)}ms` }}
                        onClick={() => selectTask(task.id)}
                      >
                        <div className="task-row">
                          <input
                            type="checkbox"
                            className="task-check"
                            checked={isDone}
                            disabled={!task.id}
                            onChange={(e) =>
                              toggle(selectedList.file, task, e.currentTarget.checked)
                            }
                          />
                          <button
                            type="button"
                            className="task-open"
                            onClick={() => selectTask(task.id)}
                            disabled={!task.id}
                          >
                            <span className={`task-title${isDone ? " done" : ""}`}>
                              {task.title || "(无标题)"}
                            </span>
                          </button>
                          <div className="task-actions">
                            <button
                              type="button"
                              className="row-action danger"
                              onClick={(e) => {
                                e.stopPropagation();
                                removeTask(selectedList.file, task);
                              }}
                              disabled={busy || !task.id}
                            >
                              删除
                            </button>
                          </div>
                        </div>
                        {(task.priority || task.due) && (
                          <div className="task-meta-row">
                            {task.priority && (
                              <span className="meta">
                                优先级 {PRIORITY_LABEL[task.priority] ?? task.priority}
                              </span>
                            )}
                            {task.due && <span className="meta">截止 {task.due}</span>}
                          </div>
                        )}
                      </li>
                    );
                  })}
                </ul>
              )}
              </div>
            </>
          )}

          {/* 左栏底部：新增 todo（作用于当前清单） */}
          <div className="add-row">
            <input
              value={selectedList ? newTaskTitles[selectedList.file] ?? "" : ""}
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
              onClick={() => selectedList && submitNewTask(selectedList.file)}
              disabled={busy || !selectedList}
              aria-label="新增任务"
              title="新增任务"
            >
              +
            </button>
          </div>
        </section>

        {/* ------------------------------------------------------- 右侧边栏 */}
        <aside className="sidebar">
          {/* 清单条（宽屏：纵向可滑动 / 窄屏：横向可滑动） */}
          <section className="side-block lists-block">
            <div className="side-head">
              <h2>清单</h2>
              <span className="side-count">{lists.length}</span>
            </div>

            {lists.length === 0 ? (
              <p className="hint">暂无清单。</p>
            ) : (
              <ul className="list-nav">
                {lists.map((list) => {
                  const isActive = list.file === selectedListFile;
                  return (
                    <li
                      key={list.file}
                      className={`list-nav-item${isActive ? " is-active" : ""}`}
                    >
                      {editingListFile === list.file ? (
                        <div className="list-nav-edit">
                          <input
                            className="edit-list-name"
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
                            <button
                              type="button"
                              onClick={cancelRenameList}
                              disabled={busy}
                            >
                              取消
                            </button>
                          </div>
                        </div>
                      ) : (
                        <>
                          <button
                            type="button"
                            className="list-nav-select"
                            onClick={() => selectList(list.file)}
                          >
                            <span className="list-nav-name">{list.name}</span>
                          </button>
                          <span className="list-nav-count">{list.tasks.length}</span>
                          <div className="list-nav-actions">
                            <button
                              type="button"
                              className="row-action"
                              onClick={() => startRenameList(list)}
                              disabled={busy}
                            >
                              重命名
                            </button>
                            <button
                              type="button"
                              className="row-action danger"
                              onClick={() => removeList(list)}
                              disabled={busy}
                            >
                              删除
                            </button>
                            <button
                              type="button"
                              className="row-action"
                              onClick={() => moveList(list.file, "up")}
                              disabled={busy}
                            >
                              ↑
                            </button>
                            <button
                              type="button"
                              className="row-action"
                              onClick={() => moveList(list.file, "down")}
                              disabled={busy}
                            >
                              ↓
                            </button>
                          </div>
                        </>
                      )}
                    </li>
                  );
                })}
              </ul>
            )}

            {/* 清单条底部：新增清单（与「新增 todo」同款） */}
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
    </main>
  );
}

export default App;
