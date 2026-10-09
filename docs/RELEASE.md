# 发布操作（Release）

> 目标形态：**免安装裸 exe ＋ NSIS 安装包**。产物不入库（`release/` 已在 `.gitignore`）。
> 1.0.0 实测（2026-10-09）：安装器 **1.46MB**、裸 exe **4.4MB**；裸 exe 的实际文件名是 `todolist.exe`（Cargo 包名，**不是** productName）。

## 前提

- Windows 10 / 11 **x64**；目标机器需有 **WebView2 Runtime**（Win11 预装；Win10 绝大多数已装）。
- 开发机：Node + pnpm、Rust 工具链、Tauri CLI（走 `pnpm tauri`）。
- **首次** `tauri build` **需要联网**：Tauri 会下载 NSIS 打包工具链（之后有缓存）。

## 一键发布（推荐）

```powershell
# 版本三处同步 → 构建 → 收集产物到 release/ → 生成 SHA256
./scripts/build-release.ps1 -Version 1.0.0
```

脚本做的事：

1. 把 `src-tauri/tauri.conf.json`、`package.json`、`src-tauri/Cargo.toml` 的版本号统一为 `1.0.0`（并打印三处实际值核对）；
2. `pnpm tauri build`（NSIS 目标）；
3. 收集产物到仓库根 `release/`：
   - 安装器 → `TodoList_1.0.0_x64-setup.exe`
   - 裸 exe → `TodoList_1.0.0_x64.exe`
4. 各生成一份同名 `.sha256`。

只想同步版本号（不构建）：`./scripts/build-release.ps1 -Version 1.0.0 -SkipBuild`

## 手动步骤（等价）

```powershell
pnpm tauri build
# 裸 exe      ：src-tauri/target/release/todolist.exe      （Cargo 包名，非 productName）
# NSIS 安装器 ：src-tauri/target/release/bundle/nsis/TodoList_1.0.0_x64-setup.exe
```

## 打 tag 与交付

> 顺序不能反：**先提交 1.0.0 的改动，再打 tag**，否则 tag 指向的提交里没有 1.0.0。

```powershell
git add -A
git commit -m "release: v1.0.0"
git tag v1.0.0
git push origin main --tags
```

然后在 GitHub 建 `v1.0.0` Release，上传 `release/` 里的**两个产物 + 两份 `.sha256`**。

## 用户侧校验

```powershell
Get-FileHash .\TodoList_1.0.0_x64-setup.exe -Algorithm SHA256
# 与同名 .sha256 文件内容比对
```

## 升级（机器上已装旧版本时）

**直接双击新版安装包即可**，不要先手动卸载。

- 安装器会进"维护页"：**默认选中「Uninstall before installing」**（先跑旧卸载器再装新版）；也可选「**Do not uninstall**」＝**原地覆盖**。两者都能升级成功，推荐后者（更快、少一次卸载）。
- ⚠️ **旧卸载器那页有个「Delete the application data」勾选框**：勾了会 `RmDir /r` 掉 `%APPDATA%\com.lftan.todolist`，而**主题偏好**就存在那里 → 会丢（重设一次即可）。**任务数据在 `文档\TodoList`，不受影响。**
- 安装时若应用**正在运行**，安装器会**自动检测并提示关闭**（Restart Manager），取消则中止安装——内置行为，无需配置。
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
