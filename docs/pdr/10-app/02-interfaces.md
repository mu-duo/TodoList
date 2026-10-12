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

## Log

- 2026-10-10 子任务**定形**（FR-APP-03）：`subtasks` 元素固定为 `{ id, title, status }`（`status ∈ open / done`，缺省 `open`）；**一层、元素不可嵌套**（深度恒为 1）；数组顺序 = 显示顺序；增 / 删 / 勾选 / **改名**经**按 id 的原子命令**写入（避免整数组覆盖冲掉外部并发改动）。schema 形状本身未变（与 §2 示例一致）。
- 2026-10-12 新增 §5「应用功能栏入口顺序」（FR-APP-24）：**设计待实现**契约——`app_bar.order` 存 `settings.yaml`、宽窄共用、默认 `[refresh, pin, settings]`、未知 id 隐藏保留槽位、恢复默认清历史、放下即原子写且失败回滚。未实现字段 / 命令，未改数据目录契约。
- 2026-10-12 **落地 §5**：`load_settings` 增 `app_bar_order: string[] | null`（取自 `app_bar.order`）；新增 `save_app_bar_order({ order })`——Value 级补丁**仅改 `app_bar.order`**，`null` / 空数组删除该字段恢复默认，保留 `theme` 与全部未知字段，空白 id / `app_bar` 非对象报中文错误。契约与 01-spec §8 一致，**未改数据目录 schema**。
