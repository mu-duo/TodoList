# 任务行按回车进入异常状态（宽屏丢控件 / 窄屏叠在详情上）

> Status: doing ｜ 责任: agent ｜ 创建: 2026-10-10 ｜ 关联: 子任务编辑与勾选（同批发现）；**待诊断**

## 现象（用户报告）
选中任务后**按回车**，任务进入"未知状态"：

- **宽屏**：该任务的**勾选框 / 删除按钮丢失**，且**无法再选中其它任务**；**再按一次回车**才能退出该状态。
- **窄屏**：该任务的视图会**叠在任务详情之上**。

首次复现环境：`pnpm tauri dev`（宽 1100px / 窄 ≤760px）。

## 待诊断（CDP 真机取证）
- Enter 前后 `document.activeElement` 的变化；
- 该行与相邻行的 `classList` 及关键 computed style（hover / focus-within / 选中态类）；
- 是否存在**覆盖层**导致点击落空（`document.elementFromPoint` 命中的是谁）；
- Enter 进出该状态时**变化的那一对量**（哪个 state / class 变了一次又变回）；
- 窄屏下详情抽屉与任务行的 `z-index` / `position` / stacking 与媒体查询分支。

## 完成判据
- **定案（用户 2026-10-10）**：任务行按回车 = **打开详情**（与单击一致）；需配套设计焦点与 `Esc` 返回，**不得**产生"再按一次回车才能退出"的状态。
- 选中任务后按回车：**不进入**"未知状态"；宽屏不丢勾选框 / 删除按钮、可正常选中其它任务；窄屏任务行不叠在详情上。
- 若"回车"本就应有某个动作，其语义**明确且可预期**（需说明清楚），不得出现"只能靠再按一次回车退出"的状态。

## 步骤
- [x] 记录（用户报告 + 落条目）
- [x] 诊断（CDP 真机取证 → 根因）
- [x] 修复（最小改动）
- [x] 验证（宽 / 窄双屏 + 回归）
- [x] 回顾（文档同步）

## Log
- 2026-10-10 用户报告，落条目并派 CDP 诊断。
- 2026-10-10 **诊断（CDP 真机）根因**：
  - **RC1（主因，极高置信）**：`SortableList.tsx:71-74` 注册了 dnd-kit `KeyboardSensor`（默认激活键 **Enter / Space**），且 `{...listeners}` 铺在**整行 `<li>`**（`SortableList.tsx:172-179`）。点行后焦点在 `.task-open` 按钮上，回车冒泡到 `<li>` → **触发键盘拖拽**：`dragging 0→1→0`、浮层 `null→存在→null` —— 这正是"再按一次回车才能退出"的那对状态。
  - 「勾选框 / 删除键丢失」＝**拖拽浮层遮挡**（`.drag-card.task-drag` 不透明卡片，包裹层 `position:fixed;z-index:999` 盖住该行），**元素未从 DOM 移除**（`.task-check` 仍 `display:block`；`.row-action` 本就是 hover / `:focus-within` 才显形）。
  - 「无法选中其它任务」＝ `markDragStart()` 置 `suppressClickRef=true`（`App.tsx:368-370`），`openTask` / `selectList` 在抑制期直接 return（`App.tsx:487-496`），仅 `markDragEnd` 复位。
  - **RC2（窄屏叠层）**：浮层包裹 `z-index:999` > 窄屏抽屉 `.detail-shell` `z-index:50`（`App.css:1288-1294`），两者同为 `fixed` 且中间无层叠上下文 → 浮层压在抽屉上。
  - **衍生隐患**：拖拽结束后焦点被移到 `.task-check`，此时回车会切换完成态。
- 2026-10-10 **修复方向**（按定案"回车＝打开详情"）：键盘拖拽激活键收敛为 **Space**（Enter 归还给按钮原生点击 → 打开详情），指针拖拽仍整行可拖；补一套**有文档的 z-index 层级**（内容 < 拖拽浮层 < 窄屏抽屉 < 设置模态）；顺带处理拖拽结束后的焦点归属。
- 2026-10-10 **修复交付**：`keyboardCodes` 收敛为 `Space`（`SortableList.tsx:81-90`）；拖拽结束焦点归还 `.task-open`（`setActivatorNodeRef` + `renderItem` 可选第二参），不再落到 `.task-check`；新增 `src/layers.ts` 层级常量 + CSS `--z-*`（内容 < 菜单 100 < 拖拽浮层 300 < 窄屏抽屉 400 < 设置模态 500），`DragOverlay zIndex={300}` 覆盖 dnd-kit 默认 999。
- 2026-10-10 **CDP 复验 PASS（10/10）**：回车打开详情且**不再触发拖拽**（无 `.drag-overlay`/`.is-dragging`）、勾选框可见、可正常切换选中；窄屏浮层 300 < 抽屉 400 且抽屉区域 `elementFromPoint` 命中的是抽屉自身；指针拖拽排序回归通过；数据逐字节还原。
- 2026-10-10 用户验收通过（含其后同源的"指针点击 → 回车"焦点环修复）；**条目归档 -done**。
