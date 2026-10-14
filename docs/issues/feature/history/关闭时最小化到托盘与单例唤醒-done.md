# 关闭时最小化到托盘与单例唤醒

> Status: done ｜ 责任: agent ｜ 创建: 2026-10-14 ｜ 关联: PDR `docs/pdr/10-app/01-spec.md`（FR-APP-14 部分）、`02-interfaces.md`、`04-current.md`、`docs/RELEASE.md`；前序归档「子任务输入框常存-done」

背景：新增「关闭时最小化到托盘」（用户口径原为「最小化到托盘」，grilling 定为由**关闭按钮**触发）：设置开启（默认）时点「关闭」隐藏到托盘、进程保活；同步修单例唤醒（隐藏 / 最小化状态下重开 app 恢复并前置既有实例）。2026-10-14 grilling 定案。

## 已对齐共识（2026-10-14 grilling）
1. **触发（选 c）**：设置开启时**仅「关闭」按钮**进托盘——点 X 隐藏到托盘（任务栏同步消失）；**最小化始终回任务栏**；退出走托盘菜单「退出」（真退出）。
2. **托盘交互**：左键单击＝恢复并聚焦主窗口（`unminimize() → show() → set_focus()`，带 `is_minimized` / `is_visible` 守卫；已显示则前置）；右键菜单＝「显示主窗口」「退出」；不做「隐藏窗口」菜单项、不做首次隐藏提示。
3. **设置**：`settings.yaml` 新字段 `minimize_to_tray`（缺省＝开启）；设置面板开关「**关闭时最小化到托盘**」；开关关闭后：无托盘图标、关闭＝退出、最小化＝任务栏（完全回到现状）。
4. **单例唤醒（用户点名确认项）**：单例回调改为 `show() + unminimize() + set_focus()`——隐藏 / 最小化状态重开 app 都恢复并前置既有实例（现状仅 `set_focus()`、对隐藏窗口无效，为本次修正内容）。
5. **技术口径**：托盘逻辑全部 Rust 侧（无需新 capability）；`tray-icon` feature 打开；托盘图标仅设置开启时存在、运行中切换即时生效；系统关机 / 注销**不拦截**（tao `WM_ENDSESSION` 直接退出，不经 close 拦截）；**升级摩擦（已知）**：安装器 Restart Manager 提示关闭时，托盘态需先从托盘菜单「退出」再继续。
6. **范围与版本**：只做「关闭常驻（隐藏 / 显示 / 退出）+ 单例唤醒 + 设置开关」；FR-APP-14 的「快速查看 / 添加（收集箱）」不做、留后续（PDR 按**部分实现**标注）。并入进行中的 **1.7.0**（提交、暂不打 tag）。

### 冻结契约（实现依据）
- YAML：`minimize_to_tray`（bool，可选）；缺省 / 非法＝**默认开启**；关闭＝写 `false`；重新开启＝**删键**（恢复默认）。
- 命令：`save_minimize_to_tray({ enabled: bool })`（`true`＝删键恢复默认；`false`＝写 `minimize_to_tray: false`）；`load_settings` 的 `minimize_to_tray: bool | null`（`null`＝默认开启，`false`＝已关闭；沿用 Option 字段 + 恢复默认删键的既有风格）。
- 托盘：启动时**仅设置开启**才创建；toggle 即时生效（开→按需创建，关→`remove_tray_by_id`）；固定 id；图标＝默认窗口图标；tooltip「TodoList」；`show_menu_on_left_click(false)`；左键 `TrayIconEvent::Click{Left, Up}` → 恢复；菜单「显示主窗口」「退出」。
- 关闭：`CloseRequested` → 设置开启且非退出中 → `prevent_close()` + `hide()`；设置关闭 → 不拦截（现状退出）；设置读取失败 → 按默认开启处理。
- 单例：恢复顺序 `unminimize()`（若最小化）→ `show()`（若不可见）→ `set_focus()`；同步修正旧注释（「不唤醒 / 不托盘」）。
- Cargo：`tauri = { version = "2", features = ["tray-icon"] }`。
- 前端：`settings.ts` `minimizeToTray: boolean | null`（读 snake_case `minimize_to_tray`，非法 → null）；开关勾选态＝`minimizeToTray !== false`；保存走 `save_minimize_to_tray`（乐观更新 + 失败进 `setActionError`，对齐 `changeTheme` 惯例）。

## 完成判据
- 上述契约落地；`cargo test --all-targets` 全绿；`pnpm build` + 全部 `src/*.verify.cjs` 断言全绿；`git diff --check` 通过。
- 用户验收（dev）：关闭→托盘（图标出现、任务栏消失）、左键恢复并聚焦、菜单「显示主窗口」/「退出」、开关关闭回现状、单例唤醒（隐藏 / 最小化两态）、提醒 toast 点击恢复窗口、最小化仍回任务栏。
- 回顾：PDR（FR-APP-14 部分实现标注 / 设置字段 / 窗口生命周期语义）与 `RELEASE.md`（升级摩擦 + 托盘态退出步骤）同步。

## 步骤
- [x] 规划（grilling 定案 + 库研究核证）
- [x] 骨架（与细节合并推进：单点功能，后端 / 前端两路并行）
- [x] 细节（Rust：托盘 + 关闭拦截 + 单例 + 设置字段；前端：设置开关）
- [x] 异常（失败路径与边界核对；测试；oracle 审阅）
- [x] 回顾（PDR / RELEASE 同步、验收、归档）

## 结果
- 关闭时最小化到托盘（默认开、设置可关）+ 托盘菜单（「显示主窗口」「退出」）+ 单例唤醒（隐藏 / 最小化均恢复并前置）落地；托盘缺失守卫、提醒 tap 统一恢复函数。
- oracle 审阅无阻塞问题；修正：菜单监听器泄漏（改一次性注册）、前端错误提示清理、注释修正。
- 验证：`cargo test --all-targets` **191/191**、`pnpm build`、断言全绿、`git diff --check` 通过。
- 用户 dev 验收通过（2026-10-14）；PDR（FR-APP-14 部分实现 / 02-interfaces §5.2）与 `RELEASE.md` 已同步；并入 1.7.0 批次（未打 tag）；产物按最新代码重打。

## Log
- 2026-10-14 grilling 定案（触发=c、托盘交互、设置与文案、单例唤醒、范围与版本、升级摩擦）。
- 2026-10-14 库研究核证（tauri 2.12.1 源码）：无最小化事件（本方案不需要）；`CloseRequested` + `prevent_close` + `hide` 可行；关机 / 注销不经拦截（tao 直接退出）；托盘 API（`TrayIconBuilder` / `remove_tray_by_id`）与单例 `AllowSetForegroundWindow` 前置均可行。
- 2026-10-14 实现落地（后端 + 前端两路）：托盘 / 关闭拦截 / 单例唤醒 / 设置字段 + 设置面板开关。核对加固：关闭拦截加「托盘实际存在」守卫（防无恢复入口）；提醒 tap 复用统一恢复函数（覆盖最小化态）。验证：`cargo test --all-targets` **191/191**、`pnpm build`、断言全绿、`git diff --check` 通过。
- 2026-10-14 PDR / RELEASE 同步（01-spec FR-APP-14 部分实现 + 托盘语义段、02-interfaces §5.2、04-current 批次 / 归档 / 差距 / 结论、RELEASE 升级摩擦）；oracle 审阅派发。
- 2026-10-14 oracle 审阅完成：无阻塞问题；核证退出路径 / 关机不拦截 / 线程模型 / 存储补丁 / 前端契约均正确。修正：托盘菜单监听器随重建泄漏（改一次性注册）、前端切换失败提示清理（对齐 changeTheme）、图标回退注释修正；「托盘缺失不隐藏」守卫确认为有意设计（02-interfaces §5.2 已含该行为契约）。
- 2026-10-14 用户验收通过；issue 归档（-done）。
