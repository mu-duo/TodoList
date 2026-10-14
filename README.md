# TodoList

Windows 桌面待办工具：本地优先，**纯文本文件即数据**（YAML），对 agent 友好。数据可被外部程序、git、网盘直接读写，应用不设隐藏数据库。

技术栈：**Tauri 2 + Rust**（数据 / 文件逻辑）+ **React + TypeScript**（薄界面）。

## 快速开始

前置：Rust（stable）、Node.js、pnpm；Windows 10/11（WebView2）。本机已在 Rust 1.99 / Node 24 / pnpm 12 上验证。

```powershell
pnpm install
pnpm tauri dev      # 首次编译较慢
```

数据默认位于 `文档\TodoList\lists\`：首次启动自动创建，并生成带 2 条示例任务的「收集箱」清单。

## 下载与运行

发布包提供两种形态，按需选择：

- **安装包**：`TodoList_<版本>_x64-setup.exe`，双击安装后从开始菜单启动。
- **免安装 exe**：`TodoList_<版本>_x64.exe`，双击即用、无需安装。

同一版本附带对应 `.sha256` 校验文件（内容为 `<小写哈希>  <文件名>`）。

- **运行前提**：Windows 10/11 x64；需要 WebView2 运行时（Windows 11 及较新的 Windows 10 通常已内置）。
- **数据位置**：`文档\TodoList`（即 `%USERPROFILE%\Documents\TodoList`），为人类可读的 YAML 纯文本文件。
- **卸载不删数据**：卸载程序只移除程序文件，`文档\TodoList` 会保留，请自行备份或删除。
- **首次运行**：会自动创建数据目录并生成带示例任务的「收集箱」清单。

发布步骤见 [`docs/RELEASE.md`](docs/RELEASE.md)。

## 升级

- **直接双击新版安装包即可升级**（`TodoList_<版本>_x64-setup.exe`），无需先手动卸载旧版。
- 维护页默认选「**先卸载再安装**」（Uninstall before installing）：先运行旧版卸载器再安装新版；也可改选「**Do not uninstall**」，即原地覆盖安装。两种方式都能成功升级。
- 旧版卸载器页面若勾选「**删除应用数据**」（Delete the application data），会删除 `%APPDATA%\com.lftan.todolist`（含主题偏好 `settings.yaml`）；**任务数据在 `文档\TodoList`，不受影响**。
- **不支持降级**：安装程序已禁止用旧版本安装包覆盖版本更高的已安装版本。

## 当前功能

- 清单：新建 / 重命名 / 删除 / 上移 / 下移
- 任务：新建 / 编辑（标题、备注）/ 完成与取消 / 删除
- 子任务：在详情面板内增 / 删 / 勾选 / 双击改名（一层，独立完成状态）；任务行尾显示 `已完成/总数`，悬停换成删除按钮
- 设置：齿轮按钮 → 主题（亮 / 暗 / 跟随系统）与应用功能栏顺序（拖拽 / 上移 / 下移 / 恢复默认），偏好本地持久化
- 应用功能栏：宽屏为最左图标竖栏、窄屏为顶部图标横排；图标悬浮显示名称（无常驻文字），可直接拖动排序（鼠标拖动 / 触屏长按），窄屏放不下时收进「更多」菜单
- Agent 协助（可选）：功能栏 Agent 配置入口 + 任务右键「Agent ▶」子菜单；只读关联材料、按夜窗自动执行、结果落本地目录（需自备所选 Provider 的 Key；内置 9 预设 + 自定义端点，详见下文）
- 读取容错：单个清单文件损坏时跳过并提示，不影响其它清单
- 写回安全：原子写（临时文件 + rename）；未知字段原样保留
- 手动刷新：外部修改文件后点功能栏的「刷新」图标即可看到变化（不覆盖正在编辑的详情草稿）

规划中（未实现）：外部变更自动重载（watch）、搜索 / 过滤、拖拽排序与跨清单移动、优先级 / 截止 / 标签、提醒 / 重复任务、托盘 / 全局快捷键 / 自启。详见 [`docs/pdr/10-app/04-current.md`](docs/pdr/10-app/04-current.md)。

## 使用

1. 页头输入框：新建清单
2. 各清单底部输入框：新建任务（回车提交，追加到清单末尾）
3. 勾选任务 = 完成 / 取消完成（即时写回文件）；任务行右侧**垃圾桶图标** = 删除（**直接删，无确认**）
4. 清单头可「重命名 / ↑ / ↓」（排序写入 `order`）；清单删除在右键菜单里，会弹**自定义确认框**
5. 外部改了文件 → 点功能栏的「刷新」图标
6. 任务行按**回车** = 打开详情；详情内子任务可增 / 删 / 勾选，**双击**子任务文字可改名
7. 功能栏图标可拖动排序（宽屏竖栏与窄屏顶部共用同一顺序）；也可在「设置」内拖拽 / 上移 / 下移，或点「恢复默认」

## Agent 协助（可选，需自备所选 Provider 的 Key）

Agent 协助让应用读取任务关联的本地材料、联网研究并生成报告。**默认不启用、不自动调用。**

- **配置**：功能栏 Agent 配置入口（或在设置内）选择 **Provider**（内置 9 预设：OpenAI / DeepSeek / Anthropic / OpenCode Go / OpenRouter / Gemini / Qwen / Zhipu / Kimi，或**自定义端点**），填写 API Key（**本机系统保护存储、按 provider 分槽、掩罩显示、不可回显**）、可选结果输出目录；模型可从**动态拉取的模型列表**中选择，或自由输入（拉取失败 / 不支持时回退输入）；首次启用需**显式同意**联网。
- **自定义端点**：可选协议（Anthropic 兼容 / OpenAI 兼容）、自填 base URL、Key 可空（本地 / 自建端点可不带认证头）；非 https 端点且带 Key 时 UI 轻提示（不阻断）。
- **关联材料**：任务右键「Agent ▶」→ 添加关联文件 / 目录（可为任意文件或目录；**关联不等于授权**，Agent 只读、自行选择读取，不执行命令、不改源文件）。
- **启用**：同菜单「启用协助」；启用后任务行尾显示机器人徽标。
- **手动执行**：菜单「立即执行 / 重新执行」（**绕过夜间时间窗**）；「停止」后**本夜不再自动重跑**，需手动重新执行。
- **夜间自动**：仅应用运行期间，本地 00:00–09:00 自动尝试；每夜每任务最多 3 次尝试（含首次）；并发 1；单次运行最长 30 分钟。
- **查看结果**：详情 Agent 区可看状态与运行历史；菜单「查看结果」打开结果目录（`report.md` + `sources.json`）。

**联网与费用**：Agent 联网走**所选 Provider** 的官方预设端点或用户**自定义端点**，**发送的内容与产生的费用由用户自行承担**；应用**默认不自动联网、不自动付费**。**搜索能力按 Provider 声明降级**（仅声明支持者启用联网搜索，其余 UI 标注）。Agent 功能**已实现并通过隔离复验（假 provider、零真实网络）**；**真实联网仍未实测**，请谨慎使用。

## 输出产物

**数据（产物本体）** —— `文档\TodoList\lists\<清单名>.yaml`，每个清单一文件，schema v1：

```yaml
schema: 1
name: 收集箱
order: 10
tasks:
  - id: 01M4D1X3MY03GPWX9RB8BBKEA1
    title: 示例任务
    status: open            # open | done
    created: 2026-10-08T14:06
```

完整字段契约见 [`docs/pdr/10-app/02-interfaces.md`](docs/pdr/10-app/02-interfaces.md)。

**设置** —— `%APPDATA%\com.lftan.todolist\settings.yaml`（主题偏好、功能栏顺序 `app_bar.order` 与 Agent 配置 `agent`；与任务数据分家，见 [ADR-0006](docs/adr/0006-设置存储-应用配置目录.md)）。Agent 的 API Key 单独存 `%APPDATA%\com.lftan.todolist\agent.credentials`（**DPAPI 加密、按 provider 分槽、不回界面**；旧单槽自动迁移）；**运行记录 / 锁**在 `%APPDATA%\com.lftan.todolist\agent\`；**结果**写入 Agent 配置里指定的**结果主目录**（`{result_root}\{task_id}\{时间戳}_{run_id}\` 下 `report.md` + `sources.json`）。

**构建产物** —— 前端 `dist/`；Rust `src-tauri/target/`；发布打包用 `pnpm tauri build`。

提示：v1 无内置备份；建议将数据目录纳入 git / 网盘。

## 面向外部 agent（文件即接口）

- 全部任务数据都在 `lists/*.yaml`，可直接读取与编辑；应用无隐藏数据库。
- 定位任务用稳定 `id`（ULID，26 位）；新增任务建议自带 `id`（缺 id 的任务界面暂不能勾选）。
- 不认识的字段请保留——应用写回时同样会原样保留未知字段。
- 时间用本地时间：`YYYY-MM-DD` 或 `YYYY-MM-DDTHH:MM`。
- 外部修改后，应用当前需手动「刷新」才可见（watch 自动重载规划中）。

## 开发

```powershell
pnpm install
pnpm tauri dev            # 开发运行
pnpm build                # 前端构建（tsc + vite）
cd src-tauri
cargo test                # 测试（117 个）
cargo clippy --all-targets
cargo check
```

结构：`src/` 前端（React 薄层）；`src-tauri/src/storage.rs` 为存储核心（读取 → YAML Value 级补丁 → 原子写回）。

## 文档地图

- [`docs/pdr/10-app/`](docs/pdr/10-app/) — PDR：规格与文件契约（先读 `01-spec` / `02-interfaces`）
- [`docs/adr/`](docs/adr/) — 关键决策记录（本地优先 / 存储格式 / 写入安全 / 技术栈 / 边界）
- [`docs/issues/`](docs/issues/) — 工作条目（完成项归档于 `history/`）
- [`CONTEXT.md`](CONTEXT.md) — 术语表；[`AGENTS.md`](AGENTS.md) — 协作约定

## 已知限制

- 同一任务被双方（应用 / 外部）同时修改时，以磁盘后写为准（不做字段级合并，见 [ADR-0003](docs/adr/0003-外部写入与写入安全.md)）。
- 无撤销；删除仅确认一次；清单排序会重写目录内全部清单文件（YAML 排版会被规范化）。
