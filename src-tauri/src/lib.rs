mod agent;
pub mod reminder;
mod reminder_runtime;
mod storage;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use agent::clock::SystemClock;
use agent::commands::AgentRuntime;
use tauri::menu::MenuBuilder;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_notification::{Action, ActionPerformed, ActionType, NotificationExt};

/// 读取数据目录中的全部清单（不存在时自动创建并生成示例清单）。
/// 单个文件失败不影响其余清单，失败信息通过 `warnings` 返回。
#[tauri::command]
async fn load_lists(
    reminder_runtime: State<'_, reminder_runtime::ReminderRuntime>,
) -> Result<storage::LoadResult, String> {
    reminder_runtime.load_lists().await
}

/// 勾选 / 取消勾选任务：以 Value 级补丁原子写回，保留未知字段。
#[tauri::command]
fn set_task_status(list_file: String, task_id: String, done: bool) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::set_task_status_in(&base, &list_file, &task_id, done)
}

/// 批量归档任务（仅 `done`；跳过不存在 / 非 done / 已归档项）。
#[tauri::command]
fn archive_tasks(list_file: String, task_ids: Vec<String>) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::archive_tasks_in(&base, &list_file, &task_ids)
}

/// 批量取消归档任务（删除 `archived` 键；不存在的 id 静默跳过）。
#[tauri::command]
fn unarchive_tasks(list_file: String, task_ids: Vec<String>) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::unarchive_tasks_in(&base, &list_file, &task_ids)
}

/// 向指定清单末尾追加新任务。
#[tauri::command]
fn create_task(list_file: String, title: String) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::create_task_in(&base, &list_file, &title)
}

/// 新建清单文件，返回新清单的文件标识。
#[tauri::command]
fn create_list(name: String) -> Result<String, String> {
    let base = storage::data_dir()?;
    storage::create_list_in(&base, &name)
}

/// 修改任务的标题 / 备注（空备注移除字段）。
#[tauri::command]
fn update_task(
    list_file: String,
    task_id: String,
    title: String,
    notes: String,
) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::update_task_in(&base, &list_file, &task_id, &title, &notes)
}

/// 更新任务详情元数据（缺省字段不修改，null 清除可选字段）。
#[tauri::command]
fn update_task_metadata(
    list_file: String,
    task_id: String,
    patch: storage::TaskMetadataPatch,
) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::update_task_metadata_in(&base, &list_file, &task_id, patch)
}

/// 删除指定任务。
#[tauri::command]
fn delete_task(list_file: String, task_id: String) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::delete_task_in(&base, &list_file, &task_id)
}

/// 向指定任务末尾追加一条子任务，返回新子任务视图。
#[tauri::command]
fn add_subtask(
    list_file: String,
    task_id: String,
    title: String,
) -> Result<storage::SubtaskView, String> {
    let base = storage::data_dir()?;
    storage::add_subtask_in(&base, &list_file, &task_id, &title)
}

/// 从指定任务移除一条子任务。
#[tauri::command]
fn remove_subtask(list_file: String, task_id: String, subtask_id: String) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::remove_subtask_in(&base, &list_file, &task_id, &subtask_id)
}

/// 切换指定子任务的状态（仅 `open` | `done`）。
#[tauri::command]
fn set_subtask_status(
    list_file: String,
    task_id: String,
    subtask_id: String,
    status: String,
) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::set_subtask_status_in(&base, &list_file, &task_id, &subtask_id, &status)
}

/// 修改指定子任务的标题（trim 后为空报错）。
#[tauri::command]
fn set_subtask_title(
    list_file: String,
    task_id: String,
    subtask_id: String,
    title: String,
) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::set_subtask_title_in(&base, &list_file, &task_id, &subtask_id, &title)
}

/// 重命名清单（更新 name 并按规整化名移动文件），返回新文件名。
#[tauri::command]
fn rename_list(list_file: String, new_name: String) -> Result<String, String> {
    let base = storage::data_dir()?;
    storage::rename_list_in(&base, &list_file, &new_name)
}

/// 删除清单文件。
#[tauri::command]
fn delete_list(list_file: String) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::delete_list_in(&base, &list_file)
}

/// 读取回收站（读取前先清理过期条目）。
#[tauri::command]
fn load_trash() -> Result<storage::TrashView, String> {
    let base = storage::data_dir()?;
    storage::load_trash_from(&base)
}

/// 恢复回收站中的任务（`target_list_file` 缺省 → 恢复到原清单）。
#[tauri::command]
fn restore_task(trash_id: String, target_list_file: Option<String>) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::restore_task_in(&base, &trash_id, target_list_file.as_deref())
}

/// 恢复回收站中的清单，返回实际写入的文件名。
#[tauri::command]
fn restore_list(trash_id: String) -> Result<String, String> {
    let base = storage::data_dir()?;
    storage::restore_list_in(&base, &trash_id)
}

/// 彻底删除回收站中的单条条目（立即真删除，不等待 24 小时）。
#[tauri::command]
fn purge_trash_entry(trash_id: String) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::purge_trash_entry_in(&base, &trash_id)
}

/// 上 / 下移清单（direction: "up" | "down"）。
#[tauri::command]
fn move_list(list_file: String, direction: String) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::move_list_in(&base, &list_file, &direction)
}

/// 同清单内把任务移到指定任务之前（`before_task_id` 为空 → 末尾）。
#[tauri::command]
fn reorder_task(
    list_file: String,
    task_id: String,
    before_task_id: Option<String>,
) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::reorder_task_in(&base, &list_file, &task_id, before_task_id.as_deref())
}

/// 把任务从一个清单转移到另一个清单末尾（id 与全部字段保留）。
#[tauri::command]
fn move_task(list_file: String, task_id: String, to_list_file: String) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::move_task_in(&base, &list_file, &task_id, &to_list_file)
}

/// 把清单移到指定清单之前（`before_list_file` 为空 → 末尾），整体重编号。
#[tauri::command]
fn reorder_list(list_file: String, before_list_file: Option<String>) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::reorder_list_in(&base, &list_file, before_list_file.as_deref())
}

/// 读取设置（文件不存在 / 解析失败 / 值非法 → 默认 `system`）。
#[tauri::command]
fn load_settings() -> Result<storage::Settings, String> {
    let base = storage::settings_dir()?;
    storage::load_settings_from(&base)
}

/// 保存主题设置（仅 `system` | `light` | `dark`，其余归一化为 `system`）。
#[tauri::command]
fn save_settings(theme: String) -> Result<(), String> {
    let base = storage::settings_dir()?;
    storage::save_settings_in(&base, &theme)
}

/// 保存应用功能栏入口顺序（`None` / 空数组 → 恢复默认，删除 `order`）。
#[tauri::command]
fn save_app_bar_order(order: Option<Vec<String>>) -> Result<(), String> {
    let base = storage::settings_dir()?;
    storage::save_app_bar_order_in(&base, order.as_deref())
}

/// 保存「已完成」视图折叠的清单文件名集合（`None` / 空数组 → 恢复默认，删除 `collapsed`）。
#[tauri::command]
fn save_completed_collapsed(collapsed: Option<Vec<String>>) -> Result<(), String> {
    let base = storage::settings_dir()?;
    storage::save_completed_collapsed_in(&base, collapsed.as_deref())
}

/// 保存「已删除」视图折叠的来源清单文件名集合（`None` / 空数组 → 恢复默认，删除 `collapsed`）。
#[tauri::command]
fn save_trash_collapsed(collapsed: Option<Vec<String>>) -> Result<(), String> {
    let base = storage::settings_dir()?;
    storage::save_trash_collapsed_in(&base, collapsed.as_deref())
}

/// 保存「已完成」视图是否显示归档任务（`false` → 删除 `completed.show_archived`，恢复默认关闭）。
#[tauri::command]
fn save_completed_show_archived(show: bool) -> Result<(), String> {
    let base = storage::settings_dir()?;
    storage::save_completed_show_archived_in(&base, show)
}

/// 保存任务详情折叠的子板块 key 集合（`None` → 删除 `detail.collapsed`；空数组照写，表示全部展开）。
#[tauri::command]
fn save_detail_collapsed(collapsed: Option<Vec<String>>) -> Result<(), String> {
    let base = storage::settings_dir()?;
    storage::save_detail_collapsed_in(&base, collapsed.as_deref())
}

/// 固定托盘图标 id。
const TRAY_ID: &str = "main";

/// 按当前设置读取「关闭时最小化到托盘」开关；读取失败按默认开启（`true`）。
fn minimize_to_tray_enabled() -> bool {
    match storage::settings_dir() {
        Ok(base) => match storage::load_settings_from(&base) {
            Ok(settings) => settings.minimize_to_tray.unwrap_or(true),
            Err(error) => {
                eprintln!("读取设置失败，按默认开启「关闭时最小化到托盘」：{error}");
                true
            }
        },
        Err(error) => {
            eprintln!("无法定位设置目录，按默认开启「关闭时最小化到托盘」：{error}");
            true
        }
    }
}

/// 恢复并聚焦主窗口：`is_minimized` 时 `unminimize`、`!is_visible` 时 `show`，最后 `set_focus`。
///
/// 找不到窗口或任一步失败只记日志，绝不 panic。供托盘左键 / 菜单「显示主窗口」/ 单例唤醒共用。
fn restore_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        eprintln!("无法恢复主窗口：未找到 main 窗口");
        return;
    };
    if let Ok(true) = window.is_minimized() {
        if let Err(error) = window.unminimize() {
            eprintln!("取消最小化主窗口失败：{error}");
        }
    }
    if let Ok(false) = window.is_visible() {
        if let Err(error) = window.show() {
            eprintln!("显示主窗口失败：{error}");
        }
    }
    if let Err(error) = window.set_focus() {
        eprintln!("聚焦主窗口失败：{error}");
    }
}

/// 统计当前打开的便签窗口数量（label 以 `note-` 开头）。
fn note_window_count(app: &AppHandle) -> usize {
    app.webview_windows()
        .keys()
        .filter(|label| label.starts_with("note-"))
        .count()
}

/// 托盘「退出」确认对话框的防连点开关：对话框打开期间再次点「退出」不重复弹。
static EXIT_CONFIRM_OPEN: AtomicBool = AtomicBool::new(false);

/// 按需创建固定 id 的托盘图标（已存在则不改动）。
///
/// 图标为默认窗口图标；右键菜单「显示主窗口」「退出」；左键单击恢复主窗口。
/// 失败只记日志，不影响调用方（启动 / 设置切换）。
fn ensure_tray(app: &AppHandle) {
    if app.tray_by_id(TRAY_ID).is_some() {
        return;
    }

    let menu = match MenuBuilder::new(app)
        .text("show", "显示主窗口")
        .text("quit", "退出")
        .build()
    {
        Ok(menu) => menu,
        Err(error) => {
            eprintln!("创建托盘菜单失败：{error}");
            return;
        }
    };

    // 兜底：仅当 `default_window_icon()` 返回 None 时用 1x1 全透明图（Image 未实现 Default）。
    // 该分支在 Windows 上实际不可达——应用始终带默认窗口图标；`icon()` 仅在转换失败时丢弃，
    // 一张合法的全透明图会被接受，结果是图标不可见。
    let icon = app
        .default_window_icon()
        .cloned()
        .unwrap_or_else(|| tauri::image::Image::new(&[0, 0, 0, 0], 1, 1));

    // 注意：菜单事件处理器由 builder 级 `on_menu_event` 一次性注册（见 `run`），
    // 此处**不得**再调 `TrayIconBuilder::on_menu_event`——它每次 build 都会向全局菜单
    // 监听表追加闭包，而 `remove_tray_by_id` 不会移除，导致开关循环后重复触发。
    let result = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip("TodoList")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                restore_main_window(tray.app_handle());
            }
        })
        .build(app);

    if let Err(error) = result {
        eprintln!("创建托盘图标失败：{error}");
    }
}

/// 保存「关闭时最小化到托盘」设置并即时同步托盘：开启按需创建、关闭移除。
#[tauri::command]
fn save_minimize_to_tray(app: AppHandle, enabled: bool) -> Result<(), String> {
    let base = storage::settings_dir()?;
    storage::save_minimize_to_tray_in(&base, enabled)?;
    if enabled {
        ensure_tray(&app);
    } else {
        app.remove_tray_by_id(TRAY_ID);
    }
    Ok(())
}

/// 退出应用：由前端在「有便签的退出确认」通过后调用。
#[tauri::command]
fn exit_app(app: AppHandle) {
    app.exit(0);
}

/// 处理提醒通知上的动作：`remind-off`（不再提醒）与 `tap`（前置窗口）。
///
/// 运行于插件的通知线程：只记日志、不向用户抛错；刷新提醒快照走
/// `ReminderRuntime::load_lists`（与 `load_lists` 命令同一条路径），并放到
/// async runtime 上执行，避免阻塞通知线程。
fn handle_reminder_action(app: &tauri::AppHandle, performed: &ActionPerformed) {
    match performed.action_id() {
        "remind-off" => {
            let Some(notification) = performed.notification() else {
                eprintln!("「不再提醒」动作缺少通知上下文，已忽略");
                return;
            };
            let extra = notification.extra();
            let task_id = extra.get("taskId").and_then(serde_json::Value::as_str);
            let list_file = extra.get("listFile").and_then(serde_json::Value::as_str);
            let (Some(task_id), Some(list_file)) = (task_id, list_file) else {
                eprintln!("「不再提醒」动作缺少 taskId/listFile，已忽略");
                return;
            };
            if let Err(error) = reminder_runtime::disable_task_reminder(list_file, task_id) {
                eprintln!("「不再提醒」回写失败：{error}");
                return;
            }
            if let Err(error) = app.emit(
                "reminder-disabled",
                serde_json::json!({ "listFile": list_file, "taskId": task_id }),
            ) {
                eprintln!("发送 reminder-disabled 事件失败：{error}");
            }
            let runtime = app
                .try_state::<reminder_runtime::ReminderRuntime>()
                .map(|state| state.inner().clone());
            if let Some(runtime) = runtime {
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = runtime.load_lists().await {
                        eprintln!("「不再提醒」后刷新提醒快照失败：{error}");
                    }
                });
            }
        }
        "tap" => {
            restore_main_window(app);
        }
        _ => {}
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例须最先注册：第二实例恢复并前置既有实例（隐藏 / 最小化状态均唤醒）。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            restore_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // 提醒动作须在任何通知显示前注册：action type 决定 toast 按钮，
            // on_action 处理器同时让插件认为存在监听者（否则不显示按钮）。
            let notification = app.notification();
            if let Err(error) = notification.register_action_types(vec![ActionType::builder(
                reminder_runtime::REMINDER_ACTION_TYPE_ID,
            )
            .actions(vec![Action::builder("remind-off", "不再提醒").build()])
            .build()])
            {
                eprintln!("注册提醒动作类型失败：{error}");
            }
            let action_handle = app.handle().clone();
            if let Err(error) = notification.on_action(move |performed| {
                handle_reminder_action(&action_handle, performed);
            }) {
                eprintln!("注册提醒动作处理器失败：{error}");
            }
            // 把主窗口标题设为「TodoList <版本>」；找不到窗口时静默忽略。
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title(&format!("TodoList {}", app.package_info().version));
            }
            let initial_snapshot = match storage::data_dir() {
                Ok(base) => reminder_runtime::startup_snapshot(&base),
                Err(error) => reminder_runtime::ReminderSnapshot {
                    tasks: Vec::new(),
                    complete: false,
                    warnings: vec![format!("启动时读取提醒快照失败：{error}")],
                },
            };
            let reminder_runtime = match storage::settings_dir() {
                Ok(settings_dir) => reminder_runtime::ReminderRuntime::start(
                    app.handle().clone(),
                    settings_dir,
                    initial_snapshot,
                ),
                Err(error) => {
                    eprintln!("无法启动提醒运行时：{error}");
                    reminder_runtime::ReminderRuntime::disabled()
                }
            };
            app.manage(reminder_runtime);
            // Agent 运行编排：独占 runner.lock + 并发 1 队列（应用运行期间）。
            if let (Ok(config_base), Ok(data_base)) = (storage::settings_dir(), storage::data_dir())
            {
                // 先取独占 runner.lock（构造 runtime），**再**恢复 running→interrupted：
                // 非持有锁的实例不得把在跑记录误标 interrupted。
                let runtime = Arc::new(AgentRuntime::new(
                    config_base,
                    data_base,
                    Arc::new(SystemClock::new()),
                ));
                let now = chrono::Local::now()
                    .naive_local()
                    .format("%Y-%m-%dT%H:%M")
                    .to_string();
                let _ = runtime.recover_interrupted(&now);
                app.manage(runtime.clone());
                runtime.spawn_worker(app.handle().clone());
            }
            // 启动时按设置创建托盘（默认开启）；读取失败按默认开启，绝不阻断启动。
            if minimize_to_tray_enabled() {
                ensure_tray(app.handle());
            }
            // 回收站后台清理：启动即清理一次，之后每 10 分钟一次；失败只记日志，绝不 panic。
            // 不调用 `ensure_data_dir`：避免后台线程在用户删光清单后偷偷重建示例清单。
            std::thread::spawn(|| loop {
                match storage::data_dir() {
                    Ok(base) => {
                        if let Err(error) = storage::purge_trash_in(&base) {
                            eprintln!("回收站清理失败：{error}");
                        }
                    }
                    Err(error) => eprintln!("回收站清理无法定位数据目录：{error}"),
                }
                std::thread::sleep(std::time::Duration::from_secs(600));
            });
            Ok(())
        })
        // 关闭拦截：托盘设置开启且托盘可用时点「关闭」隐藏到托盘、进程保活；
        // 否则若存在便签 → 拦截关闭并请求前端确认；否则不拦截（现状退出）。
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != "main" {
                    return;
                }
                // 托盘缺失时不隐藏（避免没有可用的恢复入口）；直接走下方便签 / 退出分支。
                if minimize_to_tray_enabled() && window.app_handle().tray_by_id(TRAY_ID).is_some() {
                    api.prevent_close();
                    if let Err(error) = window.hide() {
                        eprintln!("隐藏主窗口到托盘失败：{error}");
                    }
                    return;
                }
                // 关闭应用路径：存在便签则拦截关闭，请求前端确认（确认→exit_app；取消→一切保持）。
                let count = note_window_count(window.app_handle());
                if count > 0 {
                    api.prevent_close();
                    if let Err(error) = window.app_handle().emit(
                        "exit-requested-with-notes",
                        serde_json::json!({ "noteCount": count }),
                    ) {
                        eprintln!("广播退出确认事件失败：{error}");
                    }
                }
                // 无便签：不拦截，窗口关闭 → 进程随之退出（现状）。
            }
        })
        // 托盘菜单事件：一次性注册到全局菜单监听表（不随托盘重建重复注册）。
        // 「显示主窗口」恢复并前置主窗口；「退出」有便签时弹系统原生确认对话框（不恢复主窗、不广播事件）。
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => restore_main_window(app),
            "quit" => {
                let count = note_window_count(app);
                if count == 0 {
                    app.exit(0);
                } else if EXIT_CONFIRM_OPEN
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    // 非阻塞系统原生对话框；**不**绑定父窗口（主窗可能已隐藏到托盘）。
                    // 确认 → 退出应用；取消 → 无操作。两种结果都复位防连点开关。
                    let handle = app.clone();
                    app.dialog()
                        .message(format!(
                            "当前有 {count} 张便签未关闭；退出应用将一并关闭它们，便签内容不会保存。"
                        ))
                        .title("TodoList")
                        .buttons(MessageDialogButtons::OkCancelCustom(
                            "退出".to_string(),
                            "取消".to_string(),
                        ))
                        .show(move |confirmed| {
                            EXIT_CONFIRM_OPEN.store(false, Ordering::SeqCst);
                            if confirmed {
                                handle.exit(0);
                            }
                        });
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            load_lists,
            set_task_status,
            archive_tasks,
            unarchive_tasks,
            create_task,
            create_list,
            update_task,
            update_task_metadata,
            delete_task,
            load_trash,
            restore_task,
            restore_list,
            purge_trash_entry,
            add_subtask,
            remove_subtask,
            set_subtask_status,
            set_subtask_title,
            rename_list,
            delete_list,
            move_list,
            reorder_task,
            move_task,
            reorder_list,
            load_settings,
            save_settings,
            save_app_bar_order,
            save_completed_collapsed,
            save_trash_collapsed,
            save_completed_show_archived,
            save_detail_collapsed,
            save_minimize_to_tray,
            agent::commands::agent_load_config,
            agent::commands::agent_list_providers,
            agent::commands::agent_list_models,
            agent::commands::agent_save_config,
            agent::commands::agent_set_key,
            agent::commands::agent_clear_key,
            agent::commands::agent_key_present,
            agent::commands::agent_validate_result_root,
            agent::commands::agent_probe,
            agent::commands::agent_set_task_intent,
            agent::commands::agent_pick_result_root,
            agent::commands::agent_pick_paths,
            agent::commands::agent_start_task,
            agent::commands::agent_stop_task,
            agent::commands::agent_get_status,
            agent::commands::agent_list_runs,
            agent::commands::agent_open_result_dir,
            agent::commands::agent_open_external,
            exit_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
