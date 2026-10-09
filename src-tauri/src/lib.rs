mod storage;

/// 读取数据目录中的全部清单（不存在时自动创建并生成示例清单）。
/// 单个文件失败不影响其余清单，失败信息通过 `warnings` 返回。
#[tauri::command]
fn load_lists() -> Result<storage::LoadResult, String> {
    let base = storage::data_dir()?;
    storage::ensure_data_dir(&base)?;
    storage::load_lists_from(&base)
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

/// 删除指定任务。
#[tauri::command]
fn delete_task(list_file: String, task_id: String) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::delete_task_in(&base, &list_file, &task_id)
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

/// 把清单移到指定清单之前（`before_list_file` 为空 → 末尾），整体重编号。
#[tauri::command]
fn reorder_list(list_file: String, before_list_file: Option<String>) -> Result<(), String> {
    let base = storage::data_dir()?;
    storage::reorder_list_in(&base, &list_file, before_list_file.as_deref())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_lists,
            set_task_status,
            create_task,
            create_list,
            update_task,
            delete_task,
            rename_list,
            delete_list,
            move_list,
            reorder_task,
            reorder_list
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
