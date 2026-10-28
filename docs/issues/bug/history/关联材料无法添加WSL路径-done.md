# 关联材料无法添加 WSL 路径

> Status: done（用户实机验收通过 2026-10-28，随 **1.10.1** 发布）｜ 责任: agent ｜ 创建: 2026-10-28 ｜ 关联: `docs/product/10-app/02-interfaces.md` §2 `agent.paths`、`docs/product/10-app/03-nfr.md` 只读授权边界

## 背景与目标

用户在任务详情的「关联材料」里点「添加文件 / 添加目录」，原生选择器能弹出、也能选中文件，但选完之后材料列表没有任何变化。选中对象位于 WSL（资源管理器给出 `\\wsl.localhost\<发行版>\...` 或 `\\wsl$\<发行版>\...`）。

## 已确认事实（2026-10-28 诊断）

- 失败链：`MaterialsPanel` 三个按钮 → `App.tsx:884-899 addTaskPaths` → `invoke("agent_pick_paths")`（picker 正常返回 UNC 字符串）→ `agent_set_task_intent` → `intent.rs:105-112` 逐条 `sandbox::guard_source_root` → `sandbox.rs:29-31` 对所有以 `\\` / `//` 开头的路径一律返回「拒绝 UNC / 设备路径」。
- 放大效应：`intent.rs:105-112` 先校验全部路径再写入，因此**一条** WSL 路径会让整批（含正常的 `C:\` 路径）全部丢弃，YAML 一字未改。
- 非回归：该守卫自 Agent 功能首发即存在，从未接受过 UNC；1.10.0 的两个提交未触及 picker / 校验 / 材料板。
- 读时还有三道同款校验，只放宽写时不解决问题：`sandbox.rs:423`（候选路径）、`:425`（解析后的最终路径）、`runner.rs:241`（运行前复查）、`commands.rs:805`（自动排队）。
- 包含性比较 `sandbox.rs:114-124 is_under_root` 是逐组件比较、不依赖盘符，UNC 根包含 UNC 子路径在结构上成立；`normalize_final_path` 已把 `\\?\UNC\server\share` 还原成 `\\server\share`。
- 另有一处已知边界（本次不改）：`file://///wsl.localhost/...` 形式的链接在 `net.rs:112-122` 会被剥掉前导斜杠退化成相对路径，因而仍被拒。材料路径本身不受影响。

## 已对齐决策（2026-10-28 grilling）

- **放行全部 UNC**：`\\server\share\...`、`\\wsl.localhost\<发行版>\...`、`\\wsl$\<发行版>\...` 都接受为材料路径。
- **仍拒绝** `\\?\` / `\\.\` 设备命名空间前缀；共享根（`\\server\share`、`\\wsl.localhost\<发行版>`）与盘符根一样拒绝（避免授权整棵共享）。
- 保留现有其它拒绝项：`..`、结尾点/空格、保留设备名、敏感组件（`.ssh`、`.env*` 等）、ADS 冒号。
- WSL 内部路径（`/home/...`、`/mnt/c/...`）拒绝，并提示「Windows 无法访问这类路径，请从资源管理器的 WSL 位置选择」。
- 批量保持**全有或全无**，但错误信息**点名具体路径**。
- 前端：错误就地显示在材料板（三个添加按钮下方），不再只靠页面顶部横幅。

## 完成判据

- 选中 WSL 文件 / 目录后材料行正常出现，`agent.paths` 写入原始 UNC 字符串；重启后仍在，点击可打开。
- 同一批里混入一条非法路径 → 整批不写入，材料板就地报错并包含该路径原文。
- `\\?\C:\x`、`/\\.\x`、`//server/share/x`、`\\server`（缺共享名）、`\\server\share`（共享根）、`/mnt/c/x` 均被拒且提示可操作。
- `\\server\share\.ssh\id_rsa` 仍被拒（敏感组件在 UNC 下同样生效）。
- 读路径一致：Agent 对 UNC 材料执行 `fs_list` / `fs_read` 不再报 `UnsafePath`（`resolve_allowed` 与运行前复查放行）。
- 验证：Rust 全量测试、`cargo clippy -D warnings`、`pnpm build`、`completedGroups` 断言、`git diff --check`。

## 结果

- 根因：`sandbox.rs` 的 `guard_path_string` 把所有 `\\` 开头路径一律判为「UNC / 设备路径」，而 WSL 在资源管理器中的入口正是 UNC；又因「先全量校验再写入」，一条 WSL 路径会让整批材料（含正常盘符路径）全部丢弃。
- 修复：UNC 形状判定（设备命名空间 / `//` / server+share 必填 → 再走原有组件检查），共享根 / 盘符根仍拒；`normalize_final_path` 的 `\\?\UNC\` 判定改为大小写不敏感；写前校验、读时 `resolve_allowed`、运行前复查共用同一护栏。
- 错误：`路径「{p}」不可用：{原因}` 点名路径，全有或全无；前端改为在材料板就地显示。
- 验证：Rust 270/270（+3 转绿、+6 新增）、`clippy -D warnings`、`pnpm build`、`completedGroups` 26/26、`git diff --check`；用户实机用 WSL 文件 / 目录验收通过 2026-10-28。
- 遗留（已知边界，非本次范围）：`file://///wsl.localhost/...` 链接形式仍因前导斜杠被剥而拒绝。

## 步骤

- [x] 规划（反馈回路红灯 + 决策对齐）
- [x] 细节（校验四处共享同一判定 + 错误点名 + 材料板就地错误）
- [x] 异常（设备前缀 / 共享根 / 敏感组件 / 混批 / WSL 内部路径）
- [x] 回顾（SPEC / Current 与 issue 归档）

## Log

- 2026-10-28 建条目。诊断：红斑测试 `repro_wsl_unc_path_is_acceptance_gate` 等 3 条在 `intent.rs` 内新增后全红（`cargo test --lib repro_` → 0 passed / 3 failed），错误统一为 `拒绝 UNC / 设备路径`；原有 261 条不受影响。确认非回归。
- 2026-10-28 决策：放行全部 UNC、拒绝设备前缀与共享根、全有或全无 + 点名路径、错误就地显示。
- 2026-10-28 后端修复（@fixer）：`guard_path_string` 重写为形状判定（设备命名空间 / `//` / server+share 必填 → 再走原有组件检查），`guard_source_root` 与 `validate_result_root_in` 文案统一为「整个共享 / 盘符根目录」；`normalize_final_path` 的 `\\?\UNC\` 前缀判定改为 ASCII 大小写不敏感；`intent.rs` 错误包装为 `路径「{p}」不可用：{e}`。`resolve_allowed` / 运行前复查 / 自动排队三处无需改动（只调用同一护栏）。Rust 261 → **270 全绿**，clippy 干净。
- 2026-10-28 前端修复（@fixer）：新增 `materialsError`，`addTaskPaths` 失败改为就地提示（材料板按钮下方 `添加失败：…`），成功添加 / 切换任务时清除；全局 `actionError` 对其它操作不变。
- 2026-10-28 文档同步：`02-interfaces.md` §6.1 补齐 `paths` 校验规则与 UNC 示例、Log 追加；`03-nfr.md` 只读授权边界改写为「接受 UNC、拒绝设备命名空间 / 共享根 / WSL 内部路径」；`current/app.md` 关联材料条目补 UNC 与就地报错。验证：`cargo test --all-targets` 270/270、`cargo clippy -D warnings`、`pnpm build`、`completedGroups` 26/26、`git diff --check` 全绿。**待用户实机用 WSL 文件验收。**
