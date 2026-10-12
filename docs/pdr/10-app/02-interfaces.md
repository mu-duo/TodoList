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
- 非任务数据（设置等）放**应用配置目录**，不进数据目录；当前为 `settings.yaml`（Windows：`%APPDATA%\com.lftan.todolist\settings.yaml`；文件名、字段与回退语义见 ADR-0006）。

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
- **容错**：单个清单文件损坏 / 解析失败不影响其它清单加载（跳过并在界面提示，需自行修复该文件）。
- **备份**：v1 无内置备份；建议数据目录纳入 git / 网盘。

## 4. 版本与演进

- `schema` 标识格式版本；新增字段一律可选、向后兼容（配合未知字段宽容）。
- 不兼容变更：更新本文件 + 落 ADR + 提供迁移说明。

- 权威定义：本文件（落地后以 `src-tauri` 解析实现为准，语义不得偏离本文件）。
- 待决：提醒 / 重复字段（对应切片定稿）；清单文件名规整化细则。

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

## 6. Agent 协助（Phase 1 接口冻结，**实现中待核，未实测**）

> **状态（2026-10-12）：Phase 1 契约冻结，实现中待核**——命令与字段已按 architecture.md（含父会话修正）冻结，**尚在实现、未经核验**；**不得据此声称已实现或可用**。运行时（run）记录与结果 schema 属 **Phase 2，待实现**。**真实联网未验证**（见 §6.6）。

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
- 目录 / 文件选择（**Rust 侧 dialog 桥**，前端不获原始 FS 能力）：`agent_pick_result_root() -> string | null`、`agent_pick_paths({ directories: bool }) -> string[]`

### 6.5 运行记录与结果（**Phase 2，待实现，schema 待定**）

- 本机记录计划为 `%APPDATA%\com.lftan.todolist\agent\runs\{run_id}.json`（一次运行一文件＝事实源，temp+rename 原子写；列表＝扫描目录；损坏→跳过+告警）；**schema 待 Phase 2 落地**。
- 结果计划输出到 `{result_root}\{task_id}\{时间}_{run_id}\`（`report.md` + `sources.json`）；**字段待定**。
- 运行状态 / 重试 / 历史索引**仅存本机**，**不写入任务 YAML**。

### 6.6 联网与运行约束（规范；**未实测**）

- 端点 `POST https://api.deepseek.com/anthropic/v1/messages`；默认 `model=deepseek-flash`（可改，**不自动回退到其它模型**，不支持的模型明确报错）；HTTP **禁跟随重定向**（3xx＝失败）。
- 主循环仅用**客户端只读 fs 工具**；联网仅走**独立辅助 native search 请求**（`tools: [{ type: "web_search_20250305", name: "web_search", max_uses: 5 }]`），**不与客户端文件工具混在同一请求**；`max_uses` 是**单请求内**上限、**非运行总帽**。
- 2xx 但**无 `web_search_tool_result` 块＝失败**，**不得判为联网成功**；检索结果 / 引用为**不可信外部内容**，仅作数据、绝不执行。
- 运行**无全局模型 / 搜索调用次数帽**；保留 **30 分钟 deadline / 取消 / 读取限制**（ADR-0007：单次文本 ≤256KiB、运行累计 ≤5MiB、分页不可绕）。
- **联网未经真实调用验证**（仅 DeepSeek 原生 Anthropic 兼容端点有官方一方源码，字段存在未决项），**不声称可用或已验收**。

## Log

- 2026-10-10 子任务**定形**（FR-APP-03）：`subtasks` 元素固定为 `{ id, title, status }`（`status ∈ open / done`，缺省 `open`）；**一层、元素不可嵌套**（深度恒为 1）；数组顺序 = 显示顺序；增 / 删 / 勾选 / **改名**经**按 id 的原子命令**写入（避免整数组覆盖冲掉外部并发改动）。schema 形状本身未变（与 §2 示例一致）。
- 2026-10-12 新增 §5「应用功能栏入口顺序」（FR-APP-24）：**设计待实现**契约——`app_bar.order` 存 `settings.yaml`、宽窄共用、默认 `[refresh, pin, settings]`、未知 id 隐藏保留槽位、恢复默认清历史、放下即原子写且失败回滚。未实现字段 / 命令，未改数据目录契约。
- 2026-10-12 **落地 §5**：`load_settings` 增 `app_bar_order: string[] | null`（取自 `app_bar.order`）；新增 `save_app_bar_order({ order })`——Value 级补丁**仅改 `app_bar.order`**，`null` / 空数组删除该字段恢复默认，保留 `theme` 与全部未知字段，空白 id / `app_bar` 非对象报中文错误。契约与 01-spec §8 一致，**未改数据目录 schema**。
- 2026-10-12 新增 §6「Agent 协助：任务意图与本机运行记录」（**概念契约，待架构冻结**）：任务 YAML 记启用状态 / 关联路径（字段名待定），本机运行记录不进数据目录；成果输出主目录 / 任务 ULID / 时间（+ 运行 ID）、`report.md` 来源摘要、独立稳定 id 历史。**未改 schema、未新增命令**。
- 2026-10-12 **冻结 §6 Phase 1 接口（实现中待核）**：任务 YAML `agent.enabled/paths`；本机配置 `agent{model/result_root/execution_enabled/max_uses}`（execution 默认 false、false→true 需显式 `consentCloud`）；DPAPI 凭据独立本机、密钥不回 JS；命令 `agent_load_config` / `agent_save_config` / `agent_set_key`·`clear`·`present` / `agent_validate_result_root` / `agent_probe`(仅 `confirmPaid=true` 且本阶段无真实调用) / `agent_set_task_intent` / `TaskView.agent` / dialog 桥 `agent_pick_result_root`·`agent_pick_paths`；运行 / 结果 schema 属 **P2 待实现**。**未实现、未实测**。
