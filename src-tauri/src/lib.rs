mod agent;
pub mod reminder;
mod reminder_runtime;
mod storage;

use std::sync::Arc;

use agent::clock::SystemClock;
use agent::commands::AgentRuntime;
use tauri::{Emitter, Manager, State};
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
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        _ => {}
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例须最先注册：第二实例聚焦既有窗口，不唤醒 / 不托盘。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_lists,
            set_task_status,
            create_task,
            create_list,
            update_task,
            update_task_metadata,
            delete_task,
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
            agent::commands::agent_load_config,
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
            agent::commands::agent_open_result_dir
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
