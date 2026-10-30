# 发布操作（Release）

> 目标形态：**免安装裸 exe ＋ NSIS 安装包**。产物不入库（`release/` 已在 `.gitignore`）。
> 下文命令用占位符 `x.y.z` / `<版本>` 表示版本号，替换成实际值即可。

## 前提

- Windows 10 / 11 **x64**；目标机器需有 **WebView2 Runtime**（Win11 预装；Win10 绝大多数已装）。
- 开发机：Node + pnpm、Rust 工具链、Tauri CLI（走 `pnpm tauri`）。
- **首次** `tauri build` **需要联网**：Tauri 会下载 NSIS 打包工具链（之后有缓存）。

## 一键发布（推荐）

```powershell
# 版本三处同步 → 构建 → 收集产物到 release/ → 生成 SHA256
./scripts/build-release.ps1 -Version x.y.z
```

脚本做的事：

1. 把 `src-tauri/tauri.conf.json`、`package.json`、`src-tauri/Cargo.toml` 的版本号统一为 `x.y.z`（并打印三处实际值核对）；
2. `pnpm tauri build`（NSIS 目标）；
3. 收集产物到仓库根 `release/`：
   - 安装器 → `TodoList_<版本>_x64-setup.exe`
   - 裸 exe → `TodoList_<版本>_x64.exe`
4. 各生成一份同名 `.sha256`。

只想同步版本号（不构建）：`./scripts/build-release.ps1 -Version x.y.z -SkipBuild`

## 手动步骤（等价）

```powershell
pnpm tauri build
# 裸 exe      ：src-tauri/target/release/todolist.exe      （Cargo 包名，非 productName）
# NSIS 安装器 ：src-tauri/target/release/bundle/nsis/TodoList_<版本>_x64-setup.exe
```

## 打 tag 与交付

> 顺序不能反：**先把该版本的改动与版本号提交，再打 tag**，否则 tag 指向的提交里没有该版本。

```powershell
git commit -m "chore: bump version to x.y.z"
git tag vx.y.z
git push origin master --tags      # 注意：只 push tag（--tags）不会推分支，master 也要推
```

然后在 GitHub 建 `vx.y.z` Release，上传 `release/` 里的**两个产物 + 两份 `.sha256`**。

## 用户侧校验

```powershell
Get-FileHash .\TodoList_<版本>_x64-setup.exe -Algorithm SHA256
# 与同名 .sha256 文件内容比对
```

## 已发布版本（实测）

| 版本 | 安装器 | 裸 exe | 说明 |
| :--- | :--- | :--- | :--- |
| 1.0.0 | 1.46 MB | 4.4 MB | 首个分发版本（NSIS + 裸 exe、应用图标、发布脚本） |
| 1.1.0 | 1.47 MB | 4.4 MB | 设置面板与主题（亮/暗/跟随系统）、详情即时保存与键盘操作 |
| 1.2.0 | 1.47 MB | 4.4 MB | 子任务（增 / 删 / 勾选 / 双击改名、行尾 `x/n` 计数）、回车＝打开详情、焦点与层叠修复 |
| 1.3.0 | 1.47 MB | 4.4 MB | 删除交互：任务行垃圾桶图标 + 直接删；清单删除改自定义确认模态 |
| 1.4.0 | 2.76 MB | 7.7 MB | Agent 协助（可选，需自备 DeepSeek Key）：配置与密钥（DPAPI 本机）、任务级启用与关联材料、只读沙箱 + 独立原生搜索、夜窗调度与运行记录/结果、任务右键 Agent 子菜单、行尾状态徽标、详情 Agent 区 |
| 1.9.0 | 3.01 MB | 8.48 MB | 详情面板统一为七板块并可折叠（任务属性默认收起）；关联材料独立板块、材料行可点击打开（文件定位 / 目录打开 / http(s)·file 走浏览器）；`url_read` 让 Agent 抓取授权链接（SSRF 逐跳防护、`file://` 走本地白名单、计入读取预算并留档）；清单条回归左栏（宽模式 = 窄模式 + 详情） |
| 1.9.1 | 3.01 MB | 8.48 MB | 关联材料：删常驻提示文案，说明改为标题悬浮可见；添加文件 / 目录 / 链接入口三等分铺满并强化可点反馈 |
| 1.10.0 | 3.01 MB | 8.49 MB | 「已完成」归档（任务字段 `archived`、组头右键批量归档、5 秒撤销提示、「显示归档」开关存 `settings.yaml`）与「今天 / 七天内 / 更早」时间分层；已完成视图层级重做；任务 / 清单右键菜单分区，「删除」独立成区；顺带修 `build-release.ps1` 被 pnpm stderr 横幅误判为构建失败的问题 |
| 1.10.1 | 3.01 MB | 8.50 MB | 修复「关联材料」添加失效：`agent.paths` 接受 UNC（含 WSL 的 `\\wsl.localhost\` / `\\wsl$\`），仍拒绝设备命名空间 / 共享根 / WSL 内部路径；批量添加保持全有或全无并点名出错路径，错误改为在材料板就地显示 |
| 1.10.2 | 3.01 MB | 8.50 MB | 新建任务改为插入清单**顶部**（`tasks[0]`），不再追加到末尾；新增后任务列表自动滚回顶部，刚加的那条立即在视野内（遵守 reduced-motion）；跨清单移动仍落目标清单末尾 |
| 1.11.0 | 3.03 MB | 8.55 MB | 删除回收站：任务 / 清单删除先移入 `lists/.trash.yaml`，24 小时内可单项恢复，到期自动真删除；侧栏「已删除」固定入口（按来源清单分组、可折叠、任务行只读详情、右键彻底删除、剩余时间 + 恢复）；新增 `trash.collapsed` 偏好与 `purge_trash_entry` 命令 |
| 1.11.1 | 3.03 MB | 8.55 MB | 修复「已删除」中清单行缺少右键「彻底删除」：清单行与任务行共用右键菜单，确认文案带清单内任务数，确认后立即永久删除 |
| 1.11.2 | 3.03 MB | 8.55 MB | 任务列表卡片按优先级描边（高红 / 中黄 1px；低与无优先级不加；完成 / 归档 / 选中态不叠加优先级色）；优先级文字移出标题下 meta 行，改在卡片右侧单行显示，卡片过窄时整块隐藏 |

## 升级（机器上已装旧版本时）

**直接双击新版安装包即可**，不要先手动卸载。

- 安装器会进"维护页"：**默认选中「Uninstall before installing」**（先跑旧卸载器再装新版）；也可选「**Do not uninstall**」＝**原地覆盖**。两者都能升级成功，推荐后者（更快、少一次卸载）。
- ⚠️ **旧卸载器那页有个「Delete the application data」勾选框**：勾了会 `RmDir /r` 掉 `%APPDATA%\com.lftan.todolist`，而**主题偏好**就存在那里 → 会丢（重设一次即可）。**任务数据在 `文档\TodoList`，不受影响。**
- 安装时若应用**正在运行**，安装器会**自动检测并提示关闭**（Restart Manager），取消则中止安装——内置行为，无需配置。
- ⚠️ **托盘态先退出**：「关闭时最小化到托盘」开启（默认）时，点窗口关闭只是**隐藏到托盘**、进程仍在运行。安装器提示应用未关闭时，请从**托盘图标菜单「退出」**真正退出（或先在设置中关闭该开关再关窗口），然后继续安装。
- **不支持降级**（`bundle.windows.allowDowngrades: false`）：用旧版包覆盖已装的新版会被拦。
- 跨版本务必保持 `identifier` / `productName` / `installMode` 不变，否则安装器检测不到旧安装，可能在机器上留下两份。

## 坑与说明

- **SmartScreen**：exe 未签名，首次运行可能提示「未知发布者」→ 点「更多信息」→「仍要运行」。要彻底消除需代码签名证书（本项目不做）。
- **缺 WebView2**：
  - 走**安装器**：会联网**自动安装**运行时（静默）；
  - 走**裸 exe**：WebView2 loader 会弹**原生英文**对话框（"Could not find the WebView2 Runtime…"）后拒绝启动 → 请改用安装器，或先装 Evergreen 运行时。
- **离线机器**：默认安装器需联网补 WebView2；若必须离线安装，把 `bundle.windows.webviewInstallMode` 改为 `"offlineInstaller"`（安装包 **+127MB**）。
- **不要用 `fixedRuntime`**：它**无法**让单 exe 自带运行时（仍要求 exe 旁存在 ~180MB 的运行时目录），只会让产物白白膨胀。
- **卸载不动用户数据**：数据在 `文档\TodoList`（安装目录之外），卸载**不会**删除；要清空请手动删目录。
- **首次运行**：数据目录不存在时会自动创建并生成「收集箱」与示例任务，属预期行为。
- **版本号三处必须一致**：不一致会导致安装器显示的版本与程序内版本不符——用脚本同步，别手改一处。
