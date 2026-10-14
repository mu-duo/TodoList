# 10-app 对外契约（数据目录与清单文件）

> 定位：10-app 对外提供的接口与契约；本文件是 app 与外部使用者（用户、外部 agent、脚本）的共同依据，也是 01-spec 中 FR-APP-19~21 的契约细则。schema 变更须更新本文件；方向性决策同步落 ADR。

## 1. 数据目录

默认：系统「文档」目录下的 `TodoList`（首启可改用任意目录，含 git / 网盘目录）。

```
TodoList/
└── lists/
    ├── 收集箱.yaml
    └── 工作.yaml
```

- 文件名 = 清单名（规整化）；清单显示名以文件内 `name` 为准。
- 非任务数据（设置等）放**应用配置目录**，不进数据目录；当前包括 `settings.yaml` 与提醒账本 `reminder-ledger.json`（Windows：`%APPDATA%\com.lftan.todolist\`；设置文件名、字段与回退语义见 ADR-0006）。

## 2. 清单文件（schema v1）

```yaml
schema: 1
name: 工作
order: 10
tasks:
  - id: 01JAVX8ZQK3M2N4P5R6S7T8V9W
    title: 写周报
    status: open
    priority: high
    due: 2026-10-10
    reminder: false
    tags: [汇报, 周]
    created: 2026-10-08T09:30
    completed: 2026-10-08T11:00   # done 时存在
    notes: |
      多行备注；外部补充的信息也可放这里
    subtasks:
      - id: 01JAVX9A1B2C3D4E5F6G7H8J9K
        title: 收集数据
        status: open
```

### 清单级字段

| 字段 | 类型 | 必填 | 说明 |
| :--- | :--- | :--- | :--- |
| `schema` | int | ✓ | 格式版本，当前 `1` |
| `name` | string | ✓ | 清单显示名 |
| `order` | int |  | 侧栏排序（可选） |
| `tasks` | array | ✓（可为空数组） | 任务数组；数组顺序 = 显示顺序 |

### 任务字段

| 字段 | 类型 | 必填 | 说明 |
| :--- | :--- | :--- | :--- |
| `id` | string（ULID） | 外部新增可省 | 清单内稳定唯一；缺失时 app 载入补发 |
| `title` | string | ✓ | 标题 |
| `status` | `open` \| `done` | ✓（缺省 open） | 完成状态 |
| `completed` | 本地时间 | done 时 | 完成时间 |
| `priority` | `high` \| `medium` \| `low` |  | 优先级 |
| `due` | `YYYY-MM-DD` 或 `YYYY-MM-DDTHH:MM` |  | 截止（本地时间） |
| `reminder` | `false` |  | 可选；`false`＝该任务不再提醒；缺省（或 `true` / 非法值）按默认开启处理；无 due 时不可设为 `false`；旧 `reminder_offset_days` 不再写入，任务元数据保存时移除 |
| `tags` | string[] |  | 标签 |
| `notes` | string（多行） |  | 备注 |
| `subtasks` | array（一层，**元素不可嵌套**） |  | 子任务；元素为 `{ id, title, status }`（`status ∈ open / done`，缺省 `open`）；数组顺序 = 显示顺序；增 / 删 / 勾选 / 改名经**按 id 的命令**原子写入（不做整数组覆盖）；可扩展 |

## 3. 语义与写入约定

- **id 稳定**：重排 / 编辑不改 id；删除后不复用。
- **顺序语义**：显示顺序 = `tasks` 数组顺序；拖拽排序即重排数组；自动排序视图不落盘。
- **清单文件**：文件名 = 清单名规整化（含 Windows 保留名 / 结尾点空格处理）；重命名清单同步改名文件（冲突自动后缀）；删除清单即删除对应文件。
- **完成留档**：完成任务保留在原文件（带 `completed`），不迁出文件。
- **未知字段一律保留**：app 不认识的字段原样写回（agent 可自定义扩展字段）。
- **写入**：app 写回 = 重读最新文件 → Value 级补丁只改目标字段 → 原子写（同目录临时文件 + rename）。
- **外部变更**：文件变更触发 app 自动重载（目标：≤2s 防抖；骨架阶段暂以刷新替代）。
- **冲突**：同一任务被双方同时修改时，以磁盘后写为准（不做字段级合并，已知限制，见 ADR-0003）。
- **容错**：单个清单文件损坏 / 解析失败不影响其它清单展示（跳过并在界面提示，需自行修复该文件）。根节点必须为 mapping，必需字段 `tasks` 必须为 sequence；非法任务项会产生 warning。目录迭代或单文件读取错误同样告警。任何 warning 都表示提醒快照不完整：提醒全局 fail closed 暂停，不更新 / 清理 status checkpoint，不发送；修复后完整 `load_lists` 刷新恢复。硬读取错误会先等待 worker 确认暂停，再向 UI 返回错误。
- **备份**：v1 无内置备份；建议数据目录纳入 git / 网盘。

### 任务保存命令（详情即时保存与任务行内编辑共用）

- `update_task({ listFile, taskId, title, notes }) -> void`：`title` trim 后非空（空标题报「任务标题不能为空」，不写盘）；`notes` trim 后为空 → 删除 `notes` 键；保存为最新 YAML 上的目标字段 Value 级补丁并原子写（未知字段保留）。前端两条入口共用：详情面板（400ms 防抖 + 失焦 / Enter / Esc flush）与任务行内编辑（Enter / 失焦即时提交，Esc 取消，空 / 纯空白还原原值；2026-10-14，1.7.0 已验收）。

### 任务详情元数据命令

- `update_task_metadata({ listFile, taskId, patch }) -> void`：patch 使用 camelCase 可选字段 `priority?: "high" | "medium" | "low" | null`、`due?: string | null`、`tags?: string[]`、`reminder?: false | null`。字段缺省不修改，显式 `null` 清除（`reminder: null` 删除开关键＝恢复默认提醒）；`reminder: false` 写入关闭（无 due 时报错不写）；空 tags 清除。`due` 只接受本地 `YYYY-MM-DD` / `YYYY-MM-DDTHH:MM`，无时区转换。保存为最新 YAML 上的目标字段 Value 级补丁并原子写。
- `TaskView` 前端属性为 `reminder: false | null`；磁盘字段名 `reminder`（缺省开启 → `null`）。清除 due 同时清除提醒开关。旧 `reminder_offset_days` 视同开启、任务元数据保存时移除。

## 4. 版本与演进

- `schema` 标识格式版本；新增字段一律可选、向后兼容（配合未知字段宽容）。
- 不兼容变更：更新本文件 + 落 ADR + 提供迁移说明。

- 权威定义：本文件（落地后以 `src-tauri` 解析实现为准，语义不得偏离本文件）。
- 待决：重复任务字段；清单文件名规整化细则。

## 5. 应用功能栏入口顺序（不进数据目录）

> **状态（2026-10-12）：已实现**——经既有 settings 读写通道（ADR-0006：字段可选、**未知字段保留**、原子写、缺失 / 解析失败回退默认）落地，**不新增数据目录内容**。
> 应用功能栏本身的命名、位置与入口行为见 [01-spec.md](01-spec.md) §8；验证见 [04-current.md](04-current.md)。

应用功能栏（App Bar）入口的顺序是**本机偏好**，存现有应用配置 `settings.yaml`（**不写清单、不进数据目录**）。实际字段：

```yaml
theme: system                 # 现有
app_bar:
  order: [refresh, pin, settings]
```

**接口（Tauri 命令）**：

- `load_settings() -> { theme, app_bar_order }`：`app_bar_order` 为 `string[] | null`，取自 `app_bar.order`；缺失 / 空 / 非法 → `null`（= 默认）。
- `save_app_bar_order({ order: string[] | null })`：Value 级补丁**仅改 `app_bar.order`**（保留 `theme`、`app_bar` 其它字段与全部未知字段，不做整份覆盖）；`order` 为 `null` 或空数组 → **删除 `order`**（恢复默认）；空白 id、或已有 `app_bar` 非对象 → 中文错误、不写盘。

**顺序语义**：

- `app_bar.order`（string[]，可选）：入口稳定 id 的**有序**列表；宽屏竖栏与窄屏顶部**共用**同一顺序。
- **已知 id**：`refresh`、`pin`、`settings`（本轮）；`agent` 预留（本轮不实现、不占位）。
- **默认顺序**（字段缺失 / 为空）：`[refresh, pin, settings]`；未来新增入口在默认顺序末尾追加。
- **重复 id**：取第一次出现，忽略后续。
- **未知 id**：不渲染（隐藏），但**保留其在数组中的原始槽位**；仅允许重排可见入口。
- **新增未配置入口**：默认追加到用户顺序末尾；用户重排后写回。
- **恢复默认**：删除 `app_bar.order`（连同其中记录的未知 id 历史顺序一并清除）。
- **写入时机**：排序放下即原子写；写失败**回滚到最后一次成功顺序**并向用户报错。
- **不新增文件监听**；本字段与清单文件、`load_lists` / 刷新语义无关。

### 5.1 「已完成」视图分组折叠偏好（completed.collapsed）

> **状态（2026-10-13）：已实现并经用户验收通过**——经既有 settings 读写通道（ADR-0006：字段可选、**未知字段保留**、原子写、缺失 / 解析失败回退默认）落地，**不新增数据目录内容**。
> 「已完成」虚拟清单的分组规则与折叠交互见 [01-spec.md](01-spec.md) §6（FR-APP-18）；验证见 [04-current.md](04-current.md)。

「已完成」视图的分组折叠状态是**本机偏好**，存现有应用配置 `settings.yaml`（**不写清单、不进数据目录**）。实际字段：

```yaml
theme: system                 # 现有
app_bar:
  order: [refresh, pin, settings]
completed:
  collapsed: [工作.yaml, 收集箱.yaml]   # 键 = 清单文件名（含扩展名，与 list.file 一致）
```

- 缺失 `completed.collapsed` = **全部展开**（默认）。

**接口（Tauri 命令）**：

- `load_settings()` 增 `completed_collapsed: string[] | null`，取自 `completed.collapsed`；缺失 / 空 / 非法 → `null`（= 全部展开）。
- `save_completed_collapsed({ collapsed: string[] | null })`：Value 级补丁**仅改 `completed.collapsed`**（保留 `theme`、`app_bar`、`completed` 其它字段与全部未知字段，不做整份覆盖）；`collapsed` 为 `null` 或空数组 → **删除该字段**（恢复默认 = 全部展开）；空白文件名、或已有 `completed` 非对象 → 中文错误、不写盘。

**折叠语义**：

- `completed.collapsed`（string[]，可选）：**已折叠**来源清单的文件名集合；键 = 清单文件名（与侧栏 / 磁盘文件名一致）。
- 空集合 / 字段缺失 = 全部展开；不在集合中的组默认展开。
- **失效键**（清单重命名 / 删除后残留的旧文件名）：由 **UI 静默忽略**，对应组回退为**展开**，不报错、不主动清理。
- 该字段与 `theme`、`app_bar.order` **互不覆盖**（三者各自 Value 级补丁）。
- **不新增文件监听**；本字段与清单文件、`load_lists` / 刷新语义无关。

### 5.2 「关闭时最小化到托盘」偏好（minimize_to_tray）

> **状态（2026-10-14）：已实现（1.7.0，用户验收通过 2026-10-14）**——经既有 settings 读写通道（ADR-0006：字段可选、**未知字段保留**、原子写、缺失 / 解析失败回退默认）落地，**不新增数据目录内容**。
> 功能定位见 [01-spec.md](01-spec.md) §5（FR-APP-14 **部分实现**）；窗口生命周期与托盘交互行为契约见下；验证见 [04-current.md](04-current.md)。

「关闭时最小化到托盘」是**本机偏好**，存现有应用配置 `settings.yaml`（**不写清单、不进数据目录**）。实际字段：

```yaml
theme: system                 # 现有
minimize_to_tray: false       # 可选；缺省＝开启（默认）；false＝关闭
```

**接口（Tauri 命令）**：

- `load_settings()` 增 `minimize_to_tray: bool | null`，取自同名字段；缺失 / 非布尔 → `null`（= 默认开启）。
- `save_minimize_to_tray({ enabled: bool })`：Value 级补丁**仅改 `minimize_to_tray`**（保留 `theme`、`app_bar`、`completed` 及全部未知字段，不做整份覆盖）；`enabled: false` → 写 `minimize_to_tray: false`；`enabled: true` → **删除该键**（恢复默认开启）。

**行为契约（托盘逻辑全部 Rust 侧，不占前端能力面）**：

- **关闭拦截**：仅主窗口；设置开启（缺省）且**托盘实际存在**时，`CloseRequested` → `prevent_close()` + 隐藏到托盘（任务栏同步消失、进程保活）；设置关闭或托盘缺失时不拦截（按现状退出）。**最小化不受影响**（始终回任务栏）。
- **托盘**：启动时按设置创建（默认开启；失败只记日志）；`save_minimize_to_tray` 即时同步（开→按需创建，关→移除）。图标＝默认窗口图标，tooltip `TodoList`；左键单击＝恢复并聚焦主窗口（`unminimize → show → set_focus` 带守卫）；右键菜单＝「显示主窗口」「退出」（`app.exit(0)`，不经 `CloseRequested`，必为真退出）。
- **单例唤醒**：第二实例回调＝同一恢复逻辑（隐藏 / 最小化状态均恢复并前置；单例插件代递 `AllowSetForegroundWindow`）。
- **系统关机 / 注销不拦截**（tao 直接退出，不经本拦截）。
- **已知边界**：安装器 Restart Manager 提示关闭时，托盘态需先从托盘菜单「退出」再继续（见 `docs/RELEASE.md`）。

## 6. Agent 协助（**已实现（Phase1+Phase2），待验证**）

> **状态（2026-10-12）：已实现**（Phase 1 + Phase 2；后端 **117 个 Rust 测试**、前端纯逻辑断言、`pnpm build` 通过）——契约按 architecture.md（含父会话修正）落地；**已通过隔离集成复验（假 provider、零真实网络）并经用户验收通过；真实 DeepSeek 联网仍未实测**（见 §6.6）。

### 6.1 任务 YAML `agent` 形状（冻结）

```yaml
tasks:
  - id: <ULID>
    title: ...
    agent:
      enabled: true                       # 是否启用 Agent（缺省 = false）
      paths: ["D:\\a.txt", "D:\\proj"]    # 只存原始绝对路径，运行时解析
```

- 支持文件与目录、存**原路径**；**任意文件 / 目录均可关联**、由 Agent 自主选择读取；**关联 ≠ 授权**（授权为本机行为）。
- 未知字段宽容：沿用 §3，app 写回保留未知字段（`agent` 之外不动）。

### 6.2 本机全局配置（`settings.yaml`，非密钥）

```yaml
agent:
  model: deepseek-flash      # 默认；可改，**不自动回退**
  result_root: null          # 或绝对路径（结果根目录）
  execution_enabled: false   # 本机执行 opt-in，**默认 false**
  max_uses: 5                # 单请求内服务端搜索上限，**非运行总帽**
```

- 仅补丁 `agent` 下**已知字段**，保留 `agent` 其它 / 未知字段与 `theme`、`app_bar` 等。
- `execution_enabled` 由 false → true 必须经**显式云端同意**（见 §6.4）；默认 false。

### 6.3 凭据（**独立本机，绝不回 JS**）

- 密钥经 **Windows DPAPI**（`CryptProtectData` / `CryptUnprotectData`）加密，落 `%APPDATA%\com.lftan.todolist\agent.credentials`（**独立本机文件**）；仅内存解密、HTTP 全在 Rust；**绝不回传 JS / 写日志 / 错误串**。
- presence = 可解密且非空（仅返回布尔）。

### 6.4 Phase 1 命令契约（冻结；**不编造未列命令**）

- `agent_load_config() -> { model: string, result_root: string|null, execution_enabled: bool, max_uses: number, key_present: bool }`
- `agent_save_config({ config: { model?, result_root?, execution_enabled?, max_uses? }, consentCloud: bool }) -> void`：Value 级补丁仅改 `agent` 已知字段、保留未知；**`execution_enabled` false→true 当且仅当 `consentCloud === true`**，否则报错不写。
- `agent_set_key({ key: string }) -> void` ／ `agent_clear_key() -> void` ／ `agent_key_present() -> bool`：**密钥值永不回 JS**。
- `agent_validate_result_root({ path: string }) -> { ok: bool, message: string }`
- `agent_probe({ confirmPaid: bool }) -> { ok: bool, error_class?: string, message: string }`：**仅用户显式二次费用确认（`confirmPaid === true`）才可发起网络**；`confirmPaid` 为 false 时**不发网络**（fail closed）。**不提供自动 / 保存流 / 测试流**，**不做任何真实付费调用**。
- `agent_set_task_intent({ listFile: string, taskId: string, enabled: bool, paths: string[] }) -> void`
- `load_lists` 的 `TaskView` 增可选 `agent?: { enabled: bool, paths: string[] }`
- `load_lists` 的 `TaskView` 另暴露 `reminder: false | null`（YAML 键 `reminder`；缺省开启 → `null`）。
- 目录 / 文件选择（**Rust 侧 dialog 桥**，前端不获原始 FS 能力）：`agent_pick_result_root() -> string | null`、`agent_pick_paths({ directories: bool }) -> string[]`

### 6.5 运行记录、结果与运行命令 / 事件（**已实现**）

**运行命令（Phase 2，已实现；冻结名）**：

- `agent_start_task({ listFile, taskId, force }) -> { runId }`（手动立即 / 重跑，`force` 绕夜窗；**去重**：已在运行或排队则拒绝）
- `agent_stop_task({ listFile, taskId }) -> void`（停止；**本夜不再自动重排**，见 §6.6）
- `agent_get_status() -> { active: RunSummary | null, queue: RunRef[] }`
- `agent_list_runs({ listFile, taskId, offset, limit }) -> RunSummary[]`
- `agent_open_result_dir({ runId }) -> void`（路径**由记录推导**，JS 不能传任意路径）
- 事件 `agent://state`：`{ runId, listFile, taskId, state, attempt, errorClass?, stopReason?, startedAt, endedAt? }`（**载荷不含密钥**）

**运行记录与结果**：

- 本机记录：`%APPDATA%\com.lftan.todolist\agent\runs\{run_id}.json`（一次运行一文件＝事实源；temp+rename 原子写；列表＝扫描目录；损坏→跳过+告警）。
- 结果：`{result_root}\{task_id}\{时间戳}_{run_id}\` → `report.md` + `sources.json`（**原子写 + 写入前后目录复核**）。
- `RunSummary` 字段（snake_case）：`run_id, task_id, list_file, state, attempt, error_class?, stop_reason?, started_at, ended_at?, result_dir, partial, model, stats{ chat_calls, search_calls, server_search_requests?, bytes_read, steps }`。
  - `stop_reason ∈ { manual | disabled | completed | deleted }`：用于**区分**手动停止与停用 / 完成 / 删除导致的停止。
  - `stats.steps`：本 run 主循环**实际步数**；步数护栏见 §6.6。
- 运行状态 / 重试 / 历史索引**仅存本机**，**不写入任务 YAML**。

### 6.6 联网与运行约束（规范；**未实测**）

- 端点 `POST https://api.deepseek.com/anthropic/v1/messages`；默认 `model=deepseek-flash`（可改，**不自动回退到其它模型**，不支持的模型明确报错）；HTTP **禁跟随重定向**（3xx＝失败）。
- 主循环仅用**客户端只读 fs 工具**；联网仅走**独立辅助 native search 请求**（`tools: [{ type: "web_search_20250305", name: "web_search", max_uses: 5 }]`），**不与客户端文件工具混在同一请求**；`max_uses` 是**单请求内**上限、**非运行总帽**。
- 2xx 但**无 `web_search_tool_result` 块＝失败**，**不得判为联网成功**；检索结果 / 引用为**不可信外部内容**，仅作数据、绝不执行。
- 运行**无全局模型 / 搜索调用次数帽**；保留 **30 分钟 deadline / 取消 / 读取限制**（ADR-0007：单次文本 ≤256KiB、运行累计 ≤5MiB、分页不可绕）。
- **步数护栏**：每 run 主循环步数上限 **40**（`MAX_STEPS`）——**防失控护栏，不是调用次数帽、不计入预算 / 次数统计口径**；达上限 → `failed(StepLimit)` **不重试**；实际步数记入运行记录 `stats.steps`。
- **交付 / 判据警告**：**不得**用 `--features agent-test-provider`（或 `--all-features`）构建的产物**交付**，**也不得**据此判断联网可用——该构建返回**离线假来源**（`example.test`），**仅供测试**。
- **联网未经真实调用验证**（仅 DeepSeek 原生 Anthropic 兼容端点有官方一方源码，字段存在未决项），**不声称可用或已验收**。

## 7. 提醒运行时与发送尝试账本（1.6.0 已实现并本地验证）

- **计划**：due 为本地 `YYYY-MM-DD` 时刻取当地 09:00；`YYYY-MM-DDTHH:MM` 使用指定本地时刻。四档事件：`advance7` / `advance1`（due 前 7 / 1 个**本地日历日**同一时刻）、`advance1h`（前 1 小时墙钟）与 `due`；`reminder: false` 的任务不参与（缺省开启）。夏令时 gap 顺延到下一有效分钟；ambiguous 取 earliest。
- **快照与调度**：应用启动及 `load_lists` 手动刷新同步快照；load_lists 的读盘、提交快照与 worker ack 在同一异步互斥区内串行；无文件 watcher。仅 `status == open` 参与。单 worker timer 最多每 60 秒校时（已过事件立即复查）；catch-up 每 task 最多一条并 claim 本次全部已过档，文案：due 已过→「已到截止时间」，否则按真实剩余时长；future event 保留排程。done / delete 取消；已观察到 done→open 抑制全部已过档、只保留未来未试事件；首帧（新建 / 改 due / 升级 / 关闭后再开启）静默 claim 已过的提前档、首帧已过的到期档补一条。
- **事件去重**：ledger key = `task_id + canonical due + event type(advance7|advance1|advance1h|due)`，不含清单文件名（旧 `advance` 兼容读取、不再生成）；due 修改产生新 key，同 due 设回已尝试值仍去重。
- **账本结构与 checkpoint**：`%APPDATA%\com.lftan.todolist\reminder-ledger.json`（通过 `settings_dir()`），包含 `attempted` event keys、`observed: { task_id: canonical due }`（首帧标记）与 `last_status: { task_id: bool }`；缺少新字段的旧 JSON 兼容读取（旧 `last_status` 继续生效）。两 checkpoint 仅在完整快照且 task ID 唯一时更新；完整快照确认缺席时才移除；重复 ID 保留上次值。跨重启观察到 done→open 时，把该任务当前全部逾期未尝试 keys 与新状态一次原子持久化后再正常 reconcile；future keys 不 claim。损坏或读取失败 fail closed 并报告 warning。
- **不完整快照**：目录遍历 / 文件读取失败、解析错误、根节点非 mapping、缺少或非 sequence 的 `tasks`、任务项被跳过，均使快照 incomplete。可展示成功读取的清单，但提醒整体暂停；不 reconcile / send，不更新或清理 checkpoint，也不把部分任务集当作 active schedule。完整 `load_lists` 刷新成功后恢复。硬读取错误先暂停并等待 worker ack，再返回 UI 错误；刷新读盘至 worker ack 串行。
- **存储与发送顺序**：确认 plugin permission-state 为 Granted 后，对每 task 先将全部本次 overdue claim keys 原子持久化，再调用系统通知；写账本失败不 show，show 失败仍保留 attempt。此策略偏向 at-most-once，不承诺 exactly-once。任务 / 清单文件的读改写操作持进程级写锁（通知线程写回与命令写并发时防丢失更新）。
- **通知动作（Tier 1）**：通知注册 action type `due-reminder`（动作 `remind-off`＝「不再提醒」），`extra` 携带 `taskId` / `listFile`；点击动作写回 `reminder: false`（与元数据命令同路径）并发 Tauri 事件 `reminder-disabled { listFile, taskId }`，同时触发提醒运行时快照刷新；点通知本体＝显示并聚焦主窗口；失败静默记日志。Windows 限制：点击仅在通知停留屏幕时被接收（进通知中心 / 应用退出后点击无响应）。
- **平台限制**：Windows toast best effort；permission API 不能探测 Windows 设置 / 专注助手屏蔽；`show` 返回成功不代表系统实际展示或用户已送达。当前无 UI 状态面板；账本只表示发送尝试，不是送达回执。
- **离线状态限制**：无 watcher / 事件日志时，应用关闭期间完整发生的 open→done→open 不可识别；按产品确认，启动以最终 open 状态作普通 catch-up，不推断曾完成。

## Log

- 2026-10-10 子任务**定形**（FR-APP-03）：`subtasks` 元素固定为 `{ id, title, status }`（`status ∈ open / done`，缺省 `open`）；**一层、元素不可嵌套**（深度恒为 1）；数组顺序 = 显示顺序；增 / 删 / 勾选 / **改名**经**按 id 的原子命令**写入（避免整数组覆盖冲掉外部并发改动）。schema 形状本身未变（与 §2 示例一致）。
- 2026-10-12 新增 §5「应用功能栏入口顺序」（FR-APP-24）：**设计待实现**契约——`app_bar.order` 存 `settings.yaml`、宽窄共用、默认 `[refresh, pin, settings]`、未知 id 隐藏保留槽位、恢复默认清历史、放下即原子写且失败回滚。未实现字段 / 命令，未改数据目录契约。
- 2026-10-12 **落地 §5**：`load_settings` 增 `app_bar_order: string[] | null`（取自 `app_bar.order`）；新增 `save_app_bar_order({ order })`——Value 级补丁**仅改 `app_bar.order`**，`null` / 空数组删除该字段恢复默认，保留 `theme` 与全部未知字段，空白 id / `app_bar` 非对象报中文错误。契约与 01-spec §8 一致，**未改数据目录 schema**。
- 2026-10-12 新增 §6「Agent 协助：任务意图与本机运行记录」（**概念契约，待架构冻结**）：任务 YAML 记启用状态 / 关联路径（字段名待定），本机运行记录不进数据目录；成果输出主目录 / 任务 ULID / 时间（+ 运行 ID）、`report.md` 来源摘要、独立稳定 id 历史。**未改 schema、未新增命令**。
- 2026-10-12 **冻结 §6 Phase 1 接口（实现中待核）**：任务 YAML `agent.enabled/paths`；本机配置 `agent{model/result_root/execution_enabled/max_uses}`（execution 默认 false、false→true 需显式 `consentCloud`）；DPAPI 凭据独立本机、密钥不回 JS；命令 `agent_load_config` / `agent_save_config` / `agent_set_key`·`clear`·`present` / `agent_validate_result_root` / `agent_probe`(仅 `confirmPaid=true` 且本阶段无真实调用) / `agent_set_task_intent` / `TaskView.agent` / dialog 桥 `agent_pick_result_root`·`agent_pick_paths`；运行 / 结果 schema 属 **P2 待实现**。**未实现、未实测**。
- 2026-10-12 **Phase 3 文档同步（已实现待验证）**：§6 由「Phase 1 冻结 / P2 待实现」更新为**后端与前端已实现（Phase1+Phase2）待真机集成验证 / 未实测**；补 Phase 2 运行命令与 `agent://state` 事件（含 `stopReason`、无密钥）、`RunSummary`（含 `stop_reason`、`stats.steps`）、结果布局与本机记录路径；记**步数护栏 40（非次数帽）**。**未验收、未联网实测**。
- 2026-10-12 **Phase 3 收尾修正**：状态改「**已通过隔离集成复验（假 provider、零真实网络）；真实 DeepSeek 联网仍未实测；未经用户验收**」；`RunSummary` **移除后端不存在的可选 `message` 字段**；新增 **`agent-test-provider` 交付 / 判据警告**；测试数更正为 117。
- 2026-10-12 **用户验收通过**：§6 状态改「**已实现、隔离集成复验通过、用户验收通过；真实联网未实测**」；随 **1.4.0** 发布（构建与提交由父会话执行）。
- 2026-10-13 新增 §5.1「已完成」视图分组折叠偏好（FR-APP-18）：`load_settings` 增 `completed_collapsed: string[] | null`、新增 `save_completed_collapsed({ collapsed })`——Value 级补丁**仅改 `completed.collapsed`**，`null` / 空数组删除该字段恢复默认（全部展开），保留 `theme` / `app_bar` / 未知字段，空白文件名 / `completed` 非对象报中文错误不写盘；键 = 清单文件名、失效键由 UI 静默忽略。同批记 **`move_task(list_file, task_id, to_list_file)`**（FR-APP-04 任务跨清单转移）为**命令级移动**、**不新增字段 / 文件**；双文件写序为「先向目标追加、再自源移除 + 失败回滚」。**未改数据目录 schema**。
- 2026-10-13 **提醒改造（1.6.0）**：字段 `reminder_offset_days: 1`（提前 1 天 opt-in）改为 `reminder: false`（默认四档 opt-out）；`update_task_metadata` patch 改 `reminder?: false | null`（`false` 写入、`null` 删键）；`TaskView.reminder: false | null`。四档事件 `advance7/advance1/advance1h/due`（旧 `advance` 兼容读取）；ledger 增 `observed`（首帧标记），`last_status` 语义保持（done→open 抑制全部已过档）；首帧静默 claim 已过提前档、首帧到期档补一条；合并 catch-up 文案按 due 是否已过。通知注册 `due-reminder`/`remind-off` 动作 + `reminder-disabled` 事件；点击仅在通知停留屏幕时被接收（Tier 1）。**未改数据目录 schema**。
