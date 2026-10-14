# 任务内容行内编辑（双击与右键菜单）

> Status: done ｜ 责任: agent ｜ 创建: 2026-10-14 ｜ 关联: PDR `docs/pdr/10-app/01-spec.md`（FR-APP-02 交互补充）、`04-current.md`；前序归档「关闭时最小化到托盘与单例唤醒-done」

背景：任务内容（`title`）目前只能在详情面板修改。新增**行内编辑**：宽屏双击任务内容、或任意宽度右键菜单「编辑」→ 行内输入框直接改名。2026-10-14 grilling 定案。

## 共识（2026-10-14 grilling，用户已确认）
1. **范围**：仅任务内容（`title`）；备注 / 截止 / 优先级 / 标签仍走详情；子任务「双击改名」不动。
2. **生效面**：普通清单行 +「已完成」行；**宽屏**＝双击 + 右键菜单「编辑」；**窄屏**＝仅右键菜单「编辑」（双击在窄屏不可行：单击即开抽屉、第二击被遮罩挡住，且详情本就可编辑）。
3. **触发区**：双击内容区（标题文字及周边行内空白）；勾选框 / 删除按钮 / 行尾徽标不触发。
4. **编辑交互**：进入时**光标置尾（不全选）**；Enter 提交、Esc 取消、**失焦提交**；空 / 纯空白 → **还原原值退出**；无改动 → 直接退出不写盘；IME 组合中不误提交。
5. **宽屏共存**：详情保持打开、与行内编辑同屏；实现上同步草稿，避免旧值回写覆盖新内容。
6. **右键菜单**：新增「编辑」项，位于「打开详情」之后、「完成」之前；普通行 + 已完成行、全宽度可用。
7. **可发现性**：不加 tooltip / cursor:text / 悬浮下划线（菜单项即入口）。
8. **批次**：并入 1.7.0（提交、暂不打 tag，等后续功能集成）。

## 冻结契约（实现依据；全部前端，无后端改动）
- **状态**：`editingTaskId: string | null`、`taskEditDraft`、`taskEditRef`、`taskEditHandledRef`（防双提交）——镜像子任务编辑（App.tsx:244-251）。
- **进入** `startEditTask(entry)`：编辑中重复调用 → 聚焦现有输入；草稿 = 当前 `title`。
  - 双击入口（`.task-open` 的 `onDoubleClick`）：守卫 `!isNarrow && !suppressClickRef.current`。
  - 菜单入口：无守卫（全宽度、普通 + 已完成行）。
- **提交** `commitTaskEdit`：`trim`；空 → 取消；与当前（lists 中）`title` 相同 → 直接退出；否则 `invoke("update_task", { listFile, taskId, title, notes: task.notes ?? null })`；
  - 成功 → 乐观本地更新（新增 `applyTaskTitle` 式 helper，镜像 `applyTaskSubtasks`，App.tsx:1537-1559）+ 若详情正显示同一任务 → 同步 `editTitle` / `editTitleRef`（并核对 `persistEdit` 的 no-op 判定基线，确保详情 flush / backfill / 再次编辑任何路径都不会把旧标题写回）；
  - 失败 → 镜像子任务错误处理（去重提示，App.tsx:1690 一带）。
- **取消** `cancelTaskEdit`：不写盘、退出；Esc-取消后紧接的 blur 不得再提交（镜像子任务 handled ref 机制）。
- **输入框**：编辑态在内容槽位替换 `.task-open` 按钮渲染 `<input class="task-edit">`（镜像 `.subtask-edit`，App.css:1168-1173，适配任务行排版）；`onKeyDown` Enter/Esc（IME 守卫 `!e.nativeEvent.isComposing && e.nativeEvent.keyCode !== 229`；**Esc 需 stopPropagation**，避免全局 Esc 关详情）；`onBlur` 提交。
- **聚焦效果**：`editingTaskId` 变化 → focus + `setSelectionRange(len, len)`（光标置尾；镜像 356-364 但**不**全选——与子任务的有意差异）。
- **菜单**：`buildTaskMenuItems`（App.tsx:805-870）在 `"open"` 之后插入 `{ key: "edit", label: "编辑", onSelect: () => startEditTask(entry) }`。
- **拖拽**：编辑中的行临时禁用 dnd（任务 `SortableList`（App.tsx:2553-2570）传 `itemDisabled={(id) => editingTaskId === id}`，机制同清单导航 2637）；拖拽刚结束的点击 / 双击不进入编辑（`suppressClickRef`，App.tsx:1058-1067）。
- 详情区 `openTask` / `openedByKeyboardRef`、行三区 stopPropagation 逻辑均不动。

## 完成判据
- 契约落地；`pnpm build` + 全部 `src/*.verify.cjs` 全绿；`git diff --check` 通过。
- 用户 dev 验收：宽屏双击进入编辑（光标置尾）、Enter / Esc / 失焦、空值还原、右键「编辑」（普通 + 已完成 + 窄屏）、详情同屏一致且无旧值回写、编辑中该行不可拖拽、拖拽后不误触发。
- 回顾：PDR（FR-APP-02 交互补充 / `04-current.md`）同步；归档。

## 步骤
- [x] 规划（grilling 定案 + 两路侦察）
- [x] 骨架 / 细节（单点功能，前端一路：`App.tsx` + `App.css`）
- [x] 异常（边界核对；测试；oracle 审阅）
- [x] 回顾（PDR 同步、验收、归档）

## 结果
- 宽屏双击内容区 / 右键菜单「编辑」（全宽度、普通 + 已完成行）进入行内编辑；光标置尾、Enter / Esc / 失焦、空值还原、详情同屏草稿同步、编辑中该行拖拽禁用。
- oracle 审阅：2 实缺陷 + 1 潜在风险已修（`notes` 契约 HIGH、详情同步应用时核验、编辑器归属守卫）；其余核证可接受。
- 验证：`pnpm build`、断言 13 / 15 / 16 / 14 全绿、`git diff --check` 通过。
- 用户 dev 验收通过（2026-10-14）；PDR（FR-APP-02 行 / 02-interfaces「任务保存命令」/ 04-current）已同步；并入 1.7.0 批次（未打 tag）。

## Log
- 2026-10-14 grilling 定案：10 项共识（窄屏双击不可行 → 菜单入口补位；空值还原；宽屏详情同屏共存并同步草稿；光标置尾不全选；「编辑」菜单项第二位；无 tooltip）。
- 2026-10-14 侦察（两路）：行交互面（单击恒开详情、行上无既有双击、拖拽监听在整行、子任务 / 清单改名为范本、已完成行同组件）；保存管线（`title`=内容字段、`update_task`、空标题后端拒绝、详情去抖 400ms + 失焦 flush、无 types.ts）。
- 2026-10-14 实现落地（前端一路）：行内编辑（状态 / 聚焦置尾 / 提交 / 取消 / 菜单项 / `itemDisabled` / `applyTaskTitle` / 详情草稿同步）+ `.task-edit` 样式；验证：`pnpm build`、断言全绿、`git diff --check` 通过。PDR 同步（01-spec FR-APP-02 行、02-interfaces「任务保存命令」、04-current 概览 / 批次 / 归档 / 结论）。oracle 审阅派发。
- 2026-10-14 oracle 审阅：2 实缺陷 + 1 潜在风险——①（HIGH）`notes: task.notes ?? null` 与 Rust `update_task` 的 `notes: String` 不匹配（无备注任务改名必失败）→ 改传 `""`；②（MEDIUM）详情草稿同步在 await 后无条件应用（切任务 / 新输入会被覆盖）→ 改为应用时再核验（同任务且草稿未改动才同步）；③（潜在）早期 `finishTaskEdit` 加归属守卫。其余（flush 微竞态 / 切换丢草稿 / 删除并发 / 布局细节）核证可接受。修正派发（待复核）。
- 2026-10-14 修正落地复核（三项）＋用户 dev 验收通过；issue 归档（-done）。
