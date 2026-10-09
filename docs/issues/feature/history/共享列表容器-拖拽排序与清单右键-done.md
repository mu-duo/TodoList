# 共享列表容器：拖拽排序 + 清单右键操作

> Status: doing ｜ 责任: agent ｜ 创建: 2026-10-09 ｜ 关联: PDR 10-app（UI 结构）；「已完成清单聚合渲染」条目共用该容器

## 完成判据

### 复用容器
- 「任务列表」与「清单条」采用同一套列表容器抽象（同一行模板 / 同一滚动与拖拽机制），不再各写一份。

### 拖拽排序
- 任务：列表内拖拽移动，落点有插入位指示，松手即持久化；
- 清单：清单条内拖拽移动（宽屏纵向 / 窄屏横向）；
- 拖拽与点击分离：位移超过阈值才算拖拽，否则按点击处理；
- 滚动区边缘自动滚动；`prefers-reduced-motion` 下无动画直接落位。

### 清单交互（右键）
- 清单项仅「点击 = 切换选中」；重命名 / 删除改为**右键菜单**；
- 右键菜单：重命名、删除（危险红）；点外部 / Esc 关闭；
- 悬停不再常驻显示操作按钮。

### 持久化
- 任务排序：新命令 `reorder_task`；
- 清单排序：新命令 `reorder_list`（既有 `move_list` 是 direction 式，不适合任意落点）。

## 接口（前后端约定）
- `reorder_task(list_file: string, task_id: string, before_task_id: string | null) -> Result<(), String>`
  - 把 `task_id` 移到 `before_task_id` 之前；`null` = 移到末尾；仅同清单内。
- `reorder_list(list_file: string, before_list_file: string | null) -> Result<(), String>`
  - 把清单移到 `before_list_file` 之前；`null` = 移到最后；顺序重编号沿用既有 `move_list_in` 的做法。
- 两者：原子写 + 未知字段保留；只调整顺序，不改任务字段语义。

## 步骤
- [x] 规划（容器抽象范围 / 拖拽交互 / 右键菜单）
- [x] 骨架（共享容器组件 + 两个排序命令）
- [x] 细节（拖拽指示与落点 / 右键菜单 / 点击与拖拽判定）
- [x] 异常（拖拽越界、空清单、窄屏横向拖拽、菜单贴边）
- [x] 回顾（文档同步）

## Log
- 2026-10-09 立条目（用户需求 1、2）。
- 2026-10-09 决策：拖拽实现采用 **dnd-kit**（用户选定，允许引入该前端依赖）。
- 2026-10-09 后端交付：`reorder_task` / `reorder_list` 落地（`cargo test` 16/16、clippy 干净）。
- 2026-10-09 前端交付（@designer，`@dnd-kit/core@6.3.1` + `sortable@10.0.0` + `utilities@3.2.2`）：新增 `src/components/SortableList.tsx`（共享排序容器：滚动 + 行渲染 + 拖拽 + 行内插槽 + 固定尾项 + 只读退化）、`src/components/ContextMenu.tsx`、`src/hooks.ts`；持久化走 `reorder_task` / `reorder_list`（先乐观重排，成败均 `load_lists()` 回滚）；点击/拖拽以 6px 位移阈值分离；清单操作移入右键菜单。证据：`pnpm build` 通过、结构核对、`pnpm tauri dev` 冒烟启动正常。
- 2026-10-09 已知限制：键盘排序未启用（未展开 dnd-kit `attributes`，建议后续加独立拖拽手柄）；触屏拖拽未设 `touch-action:none`，可能与列表滚动竞争；右键菜单无焦点陷阱 / 初始聚焦。
- 2026-10-09 用户验收通过；回顾：`docs/pdr/10-app/04-current.md` 已同步；ADR / CONTEXT / 02-interfaces 无需变更；条目归档 -done。
