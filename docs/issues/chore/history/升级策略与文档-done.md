# 升级策略与文档（1.0.0 之后）

> Status: done ｜ 责任: agent ｜ 创建: 2026-10-09 ｜ 关联: 「首个分发版本 1.0.0」（-done）；@librarian NSIS 核证；用户确认升级验证通过（v1.4.0）

## 共识
- `bundle.windows.allowDowngrades: false` —— 阻止用**旧包**覆盖已装的新版本（默认是 `true`＝允许）。
- **文档**写清：直接双击新版安装包即可升级；维护页**默认选中「Uninstall before installing」**（先跑旧卸载器再装），也可选「**Do not uninstall**」＝原地覆盖；两者都能升级成功。
- 提醒：旧卸载器那页若勾「**Delete the application data**」会 `RmDir /r` 掉 `%APPDATA%\com.lftan.todolist`（我们的**数据在 `文档\TodoList`、设置在数据目录**，均不受影响）。
- **运行中应用检测已内置**（NSIS 用 Restart Manager，提示 / 中止安装），**无需**任何配置。
- 跨版本务必保持 `identifier` / `productName` / `installMode` 不变，否则检测不到旧安装、可能留下两份。
- **验证方式**：同版本重装（1.0.0 覆盖 1.0.0）走一遍；真跨版本升级等 1.0.1 再验。

## 完成判据
- `allowDowngrades: false` 生效（降级安装被拦）。
- 文档覆盖：升级步骤、维护页两个选项的差别、"删除应用数据"的坑、数据位置安全性。
- 同版本重装验证通过。

## 步骤
- [x] 规划（grilling + 事实核证）
- [x] 骨架（配置项）
- [x] 细节（文档：`RELEASE.md` 升级段 + README）
- [x] 异常（降级被拦 / 安装时应用在运行 / 数据保留）
- [x] 回顾（文档同步）

## 结果
- 完成判据已满足；按既有验证/配置与本次用户确认完成收尾。用户确认从已安装旧版本升级到 v1.4.0 正常，升级过程与数据保留验证通过；不将降级拦截或运行中安装测试表述为本次用户实测。

## Log
- 2026-10-09 grilling + @librarian 核证（NSIS 模板源码）：手动装新版默认走维护页且默认「先卸载再安装」；`bundle.windows.allowDowngrades` 默认 `true`；**运行中应用检测已内置**（Restart Manager），无需配置；升级/卸载都不碰 `文档\TodoList`。据此定 `allowDowngrades: false` + 文档；`RELEASE.md` 升级段已写。
- 2026-10-09 实现交付：`bundle.windows.allowDowngrades: false`；README 新增「升级」段；`RELEASE.md` 新增「升级（机器上已装旧版本时）」段。
- 2026-10-09 机器现状（影响验证方式）：用户**已装 1.0.0**（`D:\APP\TodoList`；HKCU 卸载项 `DisplayVersion=1.0.0`、`Publisher=lftan`、有 `uninstall.exe`；当前正在运行）→ **同版本重装验证改由用户手动执行**（维护页必须人工点击，且安装器会先提示关闭运行中的实例）；真跨版本升级待下一版（1.0.1）自然验证。
- 2026-10-09 **用户发布 v1.1.0**：tag `v1.1.0` 已推远端并指向 `ce4c66f`（含主题/设置/即时保存）；GitHub Release 已建。⚠️ **远端 `master` 仍停在 `eed5401`**——本地 `2e612df` / `ce4c66f` 两笔**未推分支**（`git ls-remote` 核实），需补 `git push origin master`。
- 2026-10-09 机器上已安装实例仍为 **1.0.0**（版本资源与卸载项均未变）→ **装包验证仍待用户执行**（装 1.1.0 后可一并确认维护页升级与降级拦截），完成后本条目收尾归档。
- 2026-10-12 用户确认从已安装旧版本升级到 v1.4.0 正常；升级过程与数据保留验证通过；本 issue 归档。
