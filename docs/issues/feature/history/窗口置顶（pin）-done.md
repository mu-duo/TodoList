# 窗口置顶（pin）

> Status: doing ｜ 责任: agent ｜ 创建: 2026-10-09 ｜ 关联: grilling 五轮定案；新增 capability 权限（能力/契约层面的权限清单变化）

## 共识（grilling 定案）
- **按钮**：header 右侧、「刷新」旁；**图钉图标** + `aria-pressed` + 激活态强调色；`title` 在「置顶 / 取消置顶」间切换。状态指示与退出**只用这个按钮**（不加标题标记、不加兜底解除键）。
- **置顶** = `setAlwaysOnTop(true)` + `setMinimizable(false)` + `setResizable(false)` + `setMaximizable(false)`；取消时按**置顶前的快照**逐条还原（不硬编码 `true`）。
- **不做**：(b) 位置锁定 / 位置记忆、(d) 不可关闭、(e) 额外工作、快捷键、跨虚拟桌面可见、覆盖独占全屏。
- **不持久化**：重启即普通窗口（这也是"锁死"场景下的系统级出路）。
- **失败**：回滚已应用改动 + 按钮状态复原为真实状态 → 走既有 `actionError` 提示条，不弹窗。

## 完成判据
- header「刷新」旁有图钉按钮；点击置顶、再点取消；激活态可读（`aria-pressed` + 强调色）。
- 置顶期间：窗口独占置顶、标题栏最小化按钮置灰、不可缩放 / 最大化。
- 取消置顶：逐条还原置顶前状态 + 取消置顶层级。
- 权限：`src-tauri/capabilities/default.json` 追加 `core:window:allow-set-always-on-top`、`allow-set-minimizable`、`allow-set-maximizable`、`allow-set-resizable`；**不注册 `onCloseRequested`、不加 `allow-destroy`**。
- 任一步失败可见（`actionError`）且不残留半置顶状态。

## 已知边界（记录，不修）
- (a) 为**软保证**：标题栏最小化按钮置灰，但 `Win+↓` / `Win+M` / 显示桌面 / Alt+Space 菜单等 OS 级路径不保证被拦。
- always-on-top **盖不住独占全屏应用**（Windows 平台限制）；**跨虚拟桌面钉住** Windows 无原生支持（`setVisibleOnAllWorkspaces` 在 Windows 为 no-op）。
- 置顶期间不可缩放 / 最大化——要调尺寸先取消置顶。

## 步骤
- [x] 规划（grilling 五轮定案）
- [x] 骨架（权限 + 按钮与状态）
- [x] 细节（快照 / 还原、激活态、失败回滚）
- [x] 异常（权限拒绝、部分 API 失败、连击竞态）
- [x] 回顾（文档同步）

## Log
- 2026-10-09 grilling：用户提出置顶需求 → 五轮收敛（语义范围 / 持久化 / 按钮形态 / 状态指示 / (b) 取舍）。
- 2026-10-09 @librarian 核证（`tauri 2.12.1` / `@tauri-apps/api 2.12.1`）：`core:default` **不含任何 setter**（须逐条加）；`setClosable` 确实存在；Windows 无原生位置锁（官方 FR #7836 未实现，`onMoved` 回写会甩动）；`onCloseRequested` 缺 `allow-destroy` 会导致注 X 关不掉；`setMinimizable(false)` 仅保证按钮置灰。
- 2026-10-09 用户确认最终方案；开工。
- 2026-10-09 实现交付（@designer）：header「刷新」右侧图钉按钮（内联 SVG、`aria-pressed`、`title` 切换、激活态 `--accent` 强调）；置顶先读快照（`isMinimizable/isResizable/isMaximizable`）再依次 `setAlwaysOnTop(true)` + 三项 `false`，取消时 `setAlwaysOnTop(false)` + **按快照逐条还原**；每步登记回滚动作，失败即逆序回滚 + `syncPinState()` 校正真实状态 + 既有 `actionError` 提示；`pinBusy` 防连击；挂载时 `isAlwaysOnTop()` 校正初始状态。权限追加 4 条 setter（未加 `allow-destroy`、未注册 `onCloseRequested`）。证据：`pnpm build` 通过、capability 结构核对、`pnpm tauri dev` 冒烟通过且**无 ACL/权限告警**（Rust 侧 ACL 校验通过）。
- 2026-10-09 已知限制（实现层，另见「已知边界」）：快照仅存内存，前端重载后丢失，兜底退化为「只解除置顶、三项不还原」（刻意不硬编码 `true`）；若窗口在挂载前已被外部置顶，取消时同样退化为仅解除置顶。
- 2026-10-09 用户验收通过（含肉眼层级复核）；回顾：`docs/pdr/10-app/04-current.md` 已同步（能力 + 已知边界 + 权限提示）；不新增 ADR；条目归档 -done。
