# 10-app 模块

> 定位：TodoList 桌面应用本体——承载全部产品功能、数据目录与文件契约。

## 形态

Windows 桌面应用（Tauri 2：Rust 后端 + WebView 内 React 前端）；任务数据存于用户数据目录的 YAML 文件（详见 [02-interfaces.md](02-interfaces.md)）。单实例运行；外部 agent 通过文件接口读写，不属本模块。

## 范围与边界

| 交付（范围内） | 边界（不做） |
| :--- | :--- |
| 全部功能组（清单与任务 / 组织 / 时间 / 检索 / 桌面集成 / 历史） | 账号体系、内置云同步（多设备由用户经 git / 网盘自理） |
| 数据目录与清单文件契约（schema、写入与重载语义） | agent 侧能力（信息收集、自动完成等，属外部进程） |
| 桌面平台集成（托盘 / 快捷键 / 自启等，后续切片） | 多平台支持（仅 Windows）；v1 不做内置备份 |

## 依赖

无内部模块依赖。外部消费方 = 用户与外部 agent，契约入口：[02-interfaces.md](02-interfaces.md)。

## 文档地图

| 文件 | 内容 |
| :--- | :--- |
| [01-spec.md](01-spec.md) | 功能规格（FR-APP-NN + 验收判据） |
| [02-interfaces.md](02-interfaces.md) | 对外契约：数据目录与清单文件 schema |
| [03-nfr.md](03-nfr.md) | 非功能约束 |
| [04-current.md](04-current.md) | 现状快照与差距 |
