> 模板来源：project-docs-workflow v5（模板演进后可重跑幂等 init 对齐；本文件可按项目实际修改）

# 10-app 导航（00-index）

本文件是模块的**导航与追溯**侧车，面向 agent：文档地图、依赖、接口提供方归属与能力 ↔ FR 追溯。模块叙事见[模块 PRD](./00-README.md)，上层导航见[产品架构](../02-architecture.md)与 [`docs/toc.yml`](../../toc.yml)。本文件不发布。

## 文档地图

| 文件 | 内容 |
| :--- | :--- |
| [00-README.md](00-README.md) | 模块 PRD（目标态叙事） |
| [01-spec.md](01-spec.md) | 功能规格（`FR-APP-NN` + 验收判据） |
| [02-interfaces.md](02-interfaces.md) | 对外契约：数据目录、清单文件 schema 与设置 / Agent 契约 |
| [03-nfr.md](03-nfr.md) | 非功能约束 |

## 依赖

| 依赖 | 用途 | 契约入口 |
| :--- | :--- | :--- |
| 无内部模块依赖 | — | — |

（无依赖则删行；契约入口只保留将真实存在的文件链接。）

## 接口归属

| 接口 | 提供方 | 消费方 |
| :--- | :--- | :--- |
| 数据目录与清单文件（schema v1） | 10-app | 使用者、外部 agent、git / 网盘 |
| 应用设置与 Agent 配置 / 凭据 | 10-app | 10-app 界面与 Agent 运行时 |

（对外接口由提供方模块拥有并在其 `02-interfaces.md` 定义；消费方只引用。）

## 能力 ↔ FR 追溯

| 能力（CAP） | FR | 备注 |
| :--- | :--- | :--- |
| 清单与任务 | `FR-APP-01` ~ `FR-APP-04`、`FR-APP-18` | 含跨清单移动与已完成视图 |
| 组织与时间 | `FR-APP-05` ~ `FR-APP-10` | 标签 / 优先级 / 截止 / 提醒 / 重复 / 逾期 |
| 检索与视图 | `FR-APP-11` ~ `FR-APP-13` | 仅视图过滤与排序 |
| 桌面集成与外观 | `FR-APP-14` ~ `FR-APP-17` | 托盘 / 快捷键 / 自启 / 主题 |
| 文件与外部协作 | `FR-APP-19` ~ `FR-APP-21` | 文件即事实源 |
| 应用功能栏 | `FR-APP-22` ~ `FR-APP-24` | 位置、行为与顺序 |
| Agent 协助 | `FR-APP-25` ~ `FR-APP-34` | 多 Provider、只读沙箱、调度与结果 |
| 便签 | `FR-APP-35` | 独立速记窗口 |

（CAP 编号可选；没有追溯需求时保留表头即可。）

## 相关文档

- 模块 PRD：[00-README.md](./00-README.md)
- 产品 PRD 封面：[../00-README.md](../00-README.md)
- 产品需求：[../01-requirements.md](../01-requirements.md)
- 架构与模块划分：[../02-architecture.md](../02-architecture.md)
- 发布目录源：[`docs/toc.yml`](../../toc.yml)
- 现状（Current）：[../../current/app.md](../../current/app.md)
