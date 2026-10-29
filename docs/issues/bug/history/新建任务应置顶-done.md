# 新建任务应位于清单顶部

> Status: done（随 **1.10.2** 发布；待用户实机确认观感）｜ 责任: agent ｜ 创建: 2026-10-29 ｜ 关联: `docs/product/10-app/01-spec.md` FR-APP-02、`docs/product/10-app/02-interfaces.md` §3 顺序语义

## 背景与目标

在清单里新建的任务当前被追加到 `tasks` 数组末尾（`storage.rs:847 push_task`），因为显示顺序 = 数组顺序，新任务出现在清单最底部。期望：新建任务出现在该清单**最顶部**，刚记下的事项不需要滚动到底部找。

## 已确认事实

- 唯一写入点：`create_task_in`（`storage.rs:852`）→ `push_task`（`storage.rs:842`）→ `tasks.push(...)`；`push_task` 无其它调用方。
- 显示顺序 = `tasks` 数组顺序（`02-interfaces.md` §3），前端新建后只做 `invoke("create_task")` + 重载（`App.tsx:1930-1937`），无乐观插入，因此只需改后端插入位置。
- 跨清单移动（`move_task`）仍按既有契约追加到**目标清单末尾**，本项不改。
- 外部（agent / 手改）直接写文件不受影响。

## 完成判据

- 新建任务写入 `tasks[0]`，界面刷新后出现在清单顶端；原任务相对顺序不变。
- 取消完成（回到 `open`）仍回原数组位置，不因本项改变。
- 跨清单移动仍落在目标清单末尾。
- Rust 单测覆盖新顺序；`cargo test --all-targets`、`cargo clippy -D warnings`、`pnpm build` 全绿。

## 结果

- 新建任务写入 `tasks[0]`（`prepend_task`，`storage.rs:842-849`），界面刷新后位于清单顶端；原任务相对顺序与未知字段保留。
- 新增任务后任务列表滚回顶部（`App.tsx` `submitNewTask` + `scrollTaskListToTop`，`refresh` 完成后触发、遵守 `prefers-reduced-motion`）；输入框仍在左栏底部，布局不变。
- 跨清单移动仍追加到目标清单末尾；取消完成仍回原数组位置。
- 验证：Rust 270/270、`clippy -D warnings`、`pnpm build`、`completedGroups` 26/26、`git diff --check`。

## 步骤

- [x] 规划（定位唯一写入点与排序语义）
- [x] 细节（插入位置 + 测试）
- [x] 异常（空清单、外部字段保留、取消完成回位）
- [x] 回顾（文档同步）

## Log

- 2026-10-29 建条目；确认 `push_task` 仅 `create_task_in` 使用，改插入位置即可，前端无需改动。
- 2026-10-29 实现：`push_task` → `prepend_task`（`tasks.insert(0, …)`，`storage.rs:842-849`），`create_task_in` 改调用；测试改名 `create_task_prepends_and_preserves_unknown_fields`，断言新任务在 `tasks[0]`、原任务后移且未知字段保留、`load_lists` 读到同一顺序。
- 2026-10-29 文档同步：`01-spec` FR-APP-02 判据、`02-interfaces` §3 顺序语义（新建插首位 / 跨清单移动仍追加末尾）、`current/app` 清单与任务条目。
- 2026-10-29 验证：`cargo test --all-targets` 270/270、`cargo clippy -D warnings` 干净、`pnpm build`、`git diff --check` 全绿。**待用户实机确认排序与观感。**
- 2026-10-29 追加（用户选定）：新增任务后把任务列表滚回顶部（`App.tsx submitNewTask` + `scrollTaskListToTop`，`refresh` 完成后触发，遵守 `prefers-reduced-motion`）；输入框位置不变。注意：**顶部来自数据顺序**（`tasks[0]`），滚动只是让视口跟上，不是靠滚动制造顺序。
