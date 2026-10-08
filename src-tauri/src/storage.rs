//! 存储层：数据目录解析、清单读取、任务状态回写（Value 级补丁 + 原子写）。
//!
//! 所有函数都以「基础目录」为参数，便于在测试中注入临时目录，
//! 命令层再通过 [`data_dir`] 传入真实数据目录。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use yaml_serde::{Mapping, Value};

pub const LISTS_DIR: &str = "lists";
pub const EXAMPLE_LIST_FILE: &str = "收集箱.yaml";

/// 加载结果：成功解析的清单 + 被跳过的文件告警。
#[derive(Debug, Clone, Serialize)]
pub struct LoadResult {
    pub lists: Vec<ListView>,
    /// 单个文件读取 / 解析失败的信息（不中断整体加载）。
    pub warnings: Vec<String>,
}

/// 前端展示用清单。
#[derive(Debug, Clone, Serialize)]
pub struct ListView {
    /// 清单显示名。
    pub name: String,
    /// 文件标识（`lists/` 下的文件名，用于回写定位）。
    pub file: String,
    /// 任务列表（按文件内数组顺序）。
    pub tasks: Vec<TaskView>,
}

/// 前端展示用任务（已解析的常用字段）。
#[derive(Debug, Clone, Serialize)]
pub struct TaskView {
    pub id: String,
    pub title: String,
    /// open | done
    pub status: String,
    pub completed: Option<String>,
    pub priority: Option<String>,
    pub due: Option<String>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
    pub subtasks: Option<Vec<SubtaskView>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubtaskView {
    pub id: String,
    pub title: String,
    pub status: String,
}

/// 系统「文档」目录下的 `TodoList` 数据目录。
pub fn data_dir() -> Result<PathBuf, String> {
    let docs = dirs::document_dir().ok_or_else(|| "无法定位系统文档目录".to_string())?;
    Ok(docs.join("TodoList"))
}

/// 确保数据目录与 `lists/` 存在；`lists/` 无 YAML 时生成示例清单。
pub fn ensure_data_dir(base: &Path) -> Result<(), String> {
    let lists = base.join(LISTS_DIR);
    fs::create_dir_all(&lists).map_err(|e| format!("创建数据目录失败：{e}"))?;

    let mut has_yaml = false;
    if let Ok(entries) = fs::read_dir(&lists) {
        for entry in entries.flatten() {
            if is_yaml(&entry.path()) {
                has_yaml = true;
                break;
            }
        }
    }

    if !has_yaml {
        let example = lists.join(EXAMPLE_LIST_FILE);
        if !example.exists() {
            fs::write(&example, example_content())
                .map_err(|e| format!("生成示例清单失败：{e}"))?;
        }
    }
    Ok(())
}

/// 读取 `lists/*.yaml` 全部清单；排序：文件内 order 升序，其次按名称。
///
/// 单个文件读取 / 解析失败会被跳过并记入 [`LoadResult::warnings`]；
/// 仅 `lists/` 目录整体不可读时返回硬错误。
pub fn load_lists_from(base: &Path) -> Result<LoadResult, String> {
    let lists_dir = base.join(LISTS_DIR);
    let entries = fs::read_dir(&lists_dir).map_err(|e| format!("读取清单目录失败：{e}"))?;

    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_yaml(p))
        .collect();
    paths.sort();

    let mut loaded: Vec<(i64, ListView)> = Vec::with_capacity(paths.len());
    let mut warnings: Vec<String> = Vec::new();
    for path in &paths {
        let file = file_name(path);
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                warnings.push(format!("读取清单「{file}」失败：{e}"));
                continue;
            }
        };
        let value: Value = match yaml_serde::from_str(&text) {
            Ok(value) => value,
            Err(e) => {
                warnings.push(format!("解析清单「{file}」失败：{e}"));
                continue;
            }
        };
        let order = value.get("order").and_then(Value::as_i64).unwrap_or(i64::MAX);
        loaded.push((order, value_to_list(&file, &value)));
    }

    loaded.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    Ok(LoadResult {
        lists: loaded.into_iter().map(|(_, v)| v).collect(),
        warnings,
    })
}

/// 校验清单文件名并返回其在 `lists/` 下的完整路径。
///
/// 拒绝空名、路径分隔符与 `..`；`create_*` 与 `set_*` 共用同一套规则。
fn list_path(base: &Path, list_file: &str) -> Result<PathBuf, String> {
    if list_file.is_empty()
        || list_file.contains('/')
        || list_file.contains('\\')
        || list_file.contains("..")
    {
        return Err(format!("非法的清单文件名：{list_file}"));
    }
    Ok(base.join(LISTS_DIR).join(list_file))
}

/// 构造 YAML 字符串键。
fn key(name: &str) -> Value {
    Value::String(name.to_string())
}

/// 读取并解析清单文件为 Value。
fn read_list_value(path: &Path, list_file: &str) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("读取清单「{list_file}」失败：{e}"))?;
    yaml_serde::from_str(&text).map_err(|e| format!("解析清单「{list_file}」失败：{e}"))
}

/// 将 Value 序列化后原子写回。
fn write_list(path: &Path, value: &Value) -> Result<(), String> {
    let out = yaml_serde::to_string(value).map_err(|e| format!("序列化失败：{e}"))?;
    atomic_write(path, out.as_bytes())
}

/// 按 id 定位任务对象，未找到 / 非对象时报错。
fn find_task_mut<'a>(
    value: &'a mut Value,
    list_file: &str,
    task_id: &str,
) -> Result<&'a mut Mapping, String> {
    let tasks = value
        .get_mut("tasks")
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| format!("清单「{list_file}」缺少 tasks 数组"))?;
    let task = tasks
        .iter_mut()
        .find(|t| t.get("id").and_then(Value::as_str) == Some(task_id))
        .ok_or_else(|| format!("清单「{list_file}」中未找到任务 {task_id}"))?;
    task.as_mapping_mut()
        .ok_or_else(|| format!("任务 {task_id} 不是对象"))
}

/// 按任务 id 仅修改 `status`（及 `completed`），Value 级补丁后原子写回。
pub fn set_task_status_in(
    base: &Path,
    list_file: &str,
    task_id: &str,
    done: bool,
) -> Result<(), String> {
    let path = list_path(base, list_file)?;
    if task_id.is_empty() {
        return Err("任务缺少 id，无法写回".to_string());
    }

    let mut value = read_list_value(&path, list_file)?;
    let map = find_task_mut(&mut value, list_file, task_id)?;

    map.insert(
        key("status"),
        Value::String(if done { "done" } else { "open" }.to_string()),
    );
    if done {
        map.insert(key("completed"), Value::String(now_local()));
    } else {
        map.remove("completed");
    }

    write_list(&path, &value)
}

/// 若 `value` 的 `tasks` 为数组则在其末尾追加 `task`，否则报错。
fn push_task(value: &mut Value, list_file: &str, task: Mapping) -> Result<(), String> {
    let tasks = value
        .get_mut("tasks")
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| format!("清单「{list_file}」缺少 tasks 数组"))?;
    tasks.push(Value::Mapping(task));
    Ok(())
}

/// 向清单末尾追加新任务（自动 ULID `id` 与 `created`），Value 级补丁后原子写回。
pub fn create_task_in(base: &Path, list_file: &str, title: &str) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("任务标题不能为空".to_string());
    }

    let path = list_path(base, list_file)?;
    let mut value = read_list_value(&path, list_file)?;
    push_task(&mut value, list_file, new_task_mapping(title))?;
    write_list(&path, &value)
}

fn new_task_mapping(title: &str) -> Mapping {
    let mut task = Mapping::new();
    task.insert(key("id"), Value::String(ulid::Ulid::generate().to_string()));
    task.insert(key("title"), Value::String(title.to_string()));
    task.insert(key("status"), Value::String("open".to_string()));
    task.insert(key("created"), Value::String(now_local()));
    task
}

/// 仅修改任务的 `title` / `notes`；备注 trim 后为空则移除 `notes` 字段。
pub fn update_task_in(
    base: &Path,
    list_file: &str,
    task_id: &str,
    title: &str,
    notes: &str,
) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("任务标题不能为空".to_string());
    }
    if task_id.is_empty() {
        return Err("任务缺少 id，无法写回".to_string());
    }

    let path = list_path(base, list_file)?;
    let mut value = read_list_value(&path, list_file)?;
    let map = find_task_mut(&mut value, list_file, task_id)?;

    map.insert(key("title"), Value::String(title.to_string()));
    let notes = notes.trim();
    if notes.is_empty() {
        map.remove("notes");
    } else {
        map.insert(key("notes"), Value::String(notes.to_string()));
    }

    write_list(&path, &value)
}

/// 从清单的 `tasks` 数组中移除指定任务。
pub fn delete_task_in(base: &Path, list_file: &str, task_id: &str) -> Result<(), String> {
    let path = list_path(base, list_file)?;
    let mut value = read_list_value(&path, list_file)?;
    let tasks = value
        .get_mut("tasks")
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| format!("清单「{list_file}」缺少 tasks 数组"))?;
    let index = tasks
        .iter()
        .position(|t| t.get("id").and_then(Value::as_str) == Some(task_id))
        .ok_or_else(|| format!("清单「{list_file}」中未找到任务 {task_id}"))?;
    tasks.remove(index);
    write_list(&path, &value)
}

/// 新建清单文件，返回新清单的文件标识（`lists/` 下的文件名）。
///
/// 文件名由清单名规整化（见 [`sanitize_file_stem`]）并对重名追加 `-2`/`-3`…；
/// `order` 取目录内其余清单最大 order + 10。
pub fn create_list_in(base: &Path, name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("清单名称不能为空".to_string());
    }

    let lists_dir = base.join(LISTS_DIR);
    fs::create_dir_all(&lists_dir).map_err(|e| format!("创建数据目录失败：{e}"))?;

    let order = next_order(&lists_dir)?;
    let file = unique_file_name(&lists_dir, name);
    let path = lists_dir.join(&file);

    let mut root = Mapping::new();
    root.insert(key("schema"), Value::Number(1.into()));
    root.insert(key("name"), Value::String(name.to_string()));
    root.insert(key("order"), Value::Number(order.into()));
    root.insert(key("tasks"), Value::Sequence(Vec::new()));

    write_list(&path, &Value::Mapping(root))?;
    Ok(file)
}

/// 重命名清单：更新文件内 `name`，按规整化名移动文件（冲突追加后缀），返回新文件名。
///
/// 目标文件名与现文件名相同则仅就地改写；不同则先写新文件再删除旧文件。
pub fn rename_list_in(base: &Path, list_file: &str, new_name: &str) -> Result<String, String> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err("清单名称不能为空".to_string());
    }

    let path = list_path(base, list_file)?;
    let mut value = read_list_value(&path, list_file)?;
    let root = value
        .as_mapping_mut()
        .ok_or_else(|| format!("清单「{list_file}」不是对象"))?;
    root.insert(key("name"), Value::String(new_name.to_string()));

    let lists_dir = base.join(LISTS_DIR);
    let stem = sanitize_file_stem(new_name);

    // 文件名未变（忽略大小写）→ 就地改写
    if format!("{stem}.yaml").eq_ignore_ascii_case(list_file) {
        write_list(&path, &value)?;
        return Ok(list_file.to_string());
    }

    let target_file = unique_file_name(&lists_dir, new_name);
    let target_path = lists_dir.join(&target_file);
    write_list(&target_path, &value)?;
    if let Err(e) = fs::remove_file(&path) {
        // 新文件已写入；回滚删除，避免留下重复清单
        let _ = fs::remove_file(&target_path);
        return Err(format!("重命名后删除旧文件失败：{e}"));
    }
    Ok(target_file)
}

/// 删除清单文件（不存在 → 错误）。
pub fn delete_list_in(base: &Path, list_file: &str) -> Result<(), String> {
    let path = list_path(base, list_file)?;
    if !path.is_file() {
        return Err(format!("清单文件不存在：{list_file}"));
    }
    fs::remove_file(&path).map_err(|e| format!("删除清单「{list_file}」失败：{e}"))
}

/// 按显示顺序上 / 下移清单：与相邻清单交换后按 `index * 10 + 10` 整体重编号写回。
///
/// 已在边界时为无操作。
pub fn move_list_in(base: &Path, list_file: &str, direction: &str) -> Result<(), String> {
    let step: isize = match direction {
        "up" => -1,
        "down" => 1,
        other => return Err(format!("未知的移动方向：{other}")),
    };

    let lists_dir = base.join(LISTS_DIR);
    let mut ordered = read_ordered_lists(&lists_dir)?;
    let position = ordered
        .iter()
        .position(|(file, _, _)| file == list_file)
        .ok_or_else(|| format!("清单文件不存在：{list_file}"))?;

    let target = position as isize + step;
    if target < 0 || target >= ordered.len() as isize {
        return Ok(());
    }
    ordered.swap(position, target as usize);

    for (index, (file, _, _)) in ordered.iter().enumerate() {
        let path = lists_dir.join(file);
        let mut value = match read_list_value(&path, file) {
            Ok(value) => value,
            Err(_) => continue, // 跳过无法解析的文件
        };
        if let Some(map) = value.as_mapping_mut() {
            map.insert(key("order"), Value::Number(((index as i64 + 1) * 10).into()));
        }
        write_list(&path, &value)?;
    }
    Ok(())
}

/// 读取全部清单的 `(文件名, order, name)`，按显示顺序（order 升序，其次名称）排序。
fn read_ordered_lists(lists_dir: &Path) -> Result<Vec<(String, i64, String)>, String> {
    let entries = fs::read_dir(lists_dir).map_err(|e| format!("读取清单目录失败：{e}"))?;
    let mut result: Vec<(String, i64, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_yaml(&path) {
            continue;
        }
        let file = file_name(&path);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => continue,
        };
        let value: Value = match yaml_serde::from_str(&text) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let order = value.get("order").and_then(Value::as_i64).unwrap_or(i64::MAX);
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                file.trim_end_matches(".yaml").trim_end_matches(".yml").to_string()
            });
        result.push((file, order, name));
    }
    result.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)));
    Ok(result)
}

/// 目录内其余清单的最大 `order` + 10；没有可用 order 时返回 10。
fn next_order(lists_dir: &Path) -> Result<i64, String> {
    let max = read_ordered_lists(lists_dir)?
        .into_iter()
        .map(|(_, order, _)| order)
        .filter(|order| *order != i64::MAX)
        .max();
    Ok(max.map(|m| m + 10).unwrap_or(10))
}

/// 规整化清单名得到不冲突的文件名（含 `.yaml`）。
fn unique_file_name(lists_dir: &Path, name: &str) -> String {
    let stem = sanitize_file_stem(name);
    let mut candidate = format!("{stem}.yaml");
    let mut n = 2;
    while lists_dir.join(&candidate).exists() {
        candidate = format!("{stem}-{n}.yaml");
        n += 1;
    }
    candidate
}

/// 规整化清单名得到可安全用作文件名的主干：
/// 1. Windows 非法字符 `\ / : * ? " < > |` 及控制字符 → `-`；
/// 2. 去除结尾的 `.` 与空格；
/// 3. Windows 保留名（CON / PRN / AUX / NUL / COM1-9 / LPT1-9，忽略大小写）→ 追加 `-`；
/// 4. 规整化后为空 → `未命名`。
fn sanitize_file_stem(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => '-',
            c => c,
        })
        .collect();

    let mut stem = replaced.trim_end_matches(['.', ' ']).to_string();

    if is_reserved_name(&stem) {
        stem.push('-');
    }
    if stem.is_empty() {
        stem = "未命名".to_string();
    }
    stem
}

/// Windows 保留设备名判断（忽略大小写）。
fn is_reserved_name(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let bytes = upper.as_bytes();
    bytes.len() == 4
        && (upper.starts_with("COM") || upper.starts_with("LPT"))
        && matches!(bytes[3], b'1'..=b'9')
}

fn value_to_list(file: &str, value: &Value) -> ListView {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or_else(|| file.trim_end_matches(".yaml").trim_end_matches(".yml").to_string());

    let tasks = value
        .get("tasks")
        .and_then(Value::as_sequence)
        .map(|seq| seq.iter().filter_map(mapping_to_task).collect())
        .unwrap_or_default();

    ListView {
        name,
        file: file.to_string(),
        tasks,
    }
}

fn mapping_to_task(value: &Value) -> Option<TaskView> {
    let m = value.as_mapping()?;
    let id = get_str(m, "id").unwrap_or_default();
    let title = get_str(m, "title").unwrap_or_default();
    let status = get_str(m, "status").unwrap_or_else(|| "open".to_string());

    let tags = m.get("tags").and_then(Value::as_sequence).map(|seq| {
        seq.iter()
            .filter_map(Value::as_str)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    });

    let subtasks = m.get("subtasks").and_then(Value::as_sequence).map(|seq| {
        seq.iter()
            .filter_map(|s| {
                let sm = s.as_mapping()?;
                Some(SubtaskView {
                    id: get_str(sm, "id").unwrap_or_default(),
                    title: get_str(sm, "title").unwrap_or_default(),
                    status: get_str(sm, "status").unwrap_or_else(|| "open".to_string()),
                })
            })
            .collect::<Vec<_>>()
    });

    Some(TaskView {
        id,
        title,
        status,
        completed: get_str(m, "completed"),
        priority: get_str(m, "priority"),
        due: get_str(m, "due"),
        tags,
        notes: get_str(m, "notes"),
        subtasks,
    })
}

fn get_str(m: &Mapping, key: &str) -> Option<String> {
    m.get(key).and_then(Value::as_str).map(|s| s.to_string())
}

fn is_yaml(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"),
        None => false,
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 同目录临时文件 + rename 覆盖，尽量避免半写状态。
fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or_else(|| "无效的写入路径".to_string())?;
    let name = file_name(path);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{name}.tmp-{}-{nanos}", std::process::id()));

    if let Err(e) = write_file_sync(&tmp, contents) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("写入临时文件失败：{e}"));
    }

    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("原子写入失败：{e}"));
    }
    Ok(())
}

/// 写入并 flush + sync 到磁盘。
fn write_file_sync(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut f = fs::File::create(path)?;
    f.write_all(contents)?;
    f.flush()?;
    f.sync_all()
}

/// 本地时间 `YYYY-MM-DDTHH:MM`。
fn now_local() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M").to_string()
}

fn example_content() -> String {
    let created = now_local();
    let id1 = ulid::Ulid::generate().to_string();
    let id2 = ulid::Ulid::generate().to_string();
    format!(
        r#"schema: 1
name: 收集箱
order: 10
tasks:
  - id: {id1}
    title: 欢迎使用 TodoList
    status: open
    created: {created}
  - id: {id2}
    title: 勾选此任务以写回文件
    status: open
    created: {created}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write_list(base: &Path, file: &str, text: &str) -> PathBuf {
        let lists = base.join(LISTS_DIR);
        fs::create_dir_all(&lists).unwrap();
        let path = lists.join(file);
        fs::write(&path, text).unwrap();
        path
    }

    fn read_value(path: &Path) -> Value {
        yaml_serde::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    fn order_of(base: &Path, file: &str) -> i64 {
        read_value(&base.join(LISTS_DIR).join(file))
            .get("order")
            .and_then(Value::as_i64)
            .unwrap()
    }

    #[test]
    fn load_patch_write_roundtrip_preserves_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        let original = r#"schema: 1
name: 测试清单
order: 5
x-custom-top: 保留我
tasks:
  - id: TASK1
    title: 任务一
    status: open
    created: 2026-10-08T10:00
    x-custom-task: 也要保留
  - id: TASK2
    title: 任务二
    status: done
    completed: 2026-10-07T09:00
"#;
        let path = write_list(base, "测试清单.yaml", original);

        // 读
        let result = load_lists_from(base).unwrap();
        assert!(result.warnings.is_empty(), "正常文件不应产生 warning");
        let lists = result.lists;
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].name, "测试清单");
        assert_eq!(lists[0].file, "测试清单.yaml");
        assert_eq!(lists[0].tasks.len(), 2);
        assert_eq!(lists[0].tasks[0].status, "open");

        // 改 + 写
        set_task_status_in(base, "测试清单.yaml", "TASK1", true).unwrap();

        // 重读（原始文本 → 保证文件可解析 + 未知字段保留）
        let text = fs::read_to_string(&path).unwrap();
        let value: Value = yaml_serde::from_str(&text).unwrap();

        assert_eq!(
            value.get("x-custom-top").and_then(Value::as_str),
            Some("保留我"),
            "顶层未知字段应保留"
        );
        assert_eq!(value.get("name").and_then(Value::as_str), Some("测试清单"));
        assert_eq!(value.get("schema").and_then(Value::as_i64), Some(1));

        let tasks = value.get("tasks").and_then(Value::as_sequence).unwrap();
        let t1 = tasks
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some("TASK1"))
            .unwrap();
        assert_eq!(t1.get("status").and_then(Value::as_str), Some("done"));
        assert!(
            t1.get("completed").and_then(Value::as_str).is_some(),
            "done 时应写入 completed"
        );
        assert_eq!(
            t1.get("x-custom-task").and_then(Value::as_str),
            Some("也要保留"),
            "任务级未知字段应保留"
        );
        assert_eq!(
            t1.get("created").and_then(Value::as_str),
            Some("2026-10-08T10:00"),
            "其它字段不变"
        );

        // 其它任务不受影响
        let t2 = tasks
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some("TASK2"))
            .unwrap();
        assert_eq!(t2.get("status").and_then(Value::as_str), Some("done"));
        assert_eq!(
            t2.get("completed").and_then(Value::as_str),
            Some("2026-10-07T09:00")
        );

        // 取消完成：status 回到 open 且移除 completed
        set_task_status_in(base, "测试清单.yaml", "TASK1", false).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let value: Value = yaml_serde::from_str(&text).unwrap();
        let tasks = value.get("tasks").and_then(Value::as_sequence).unwrap();
        let t1 = tasks
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some("TASK1"))
            .unwrap();
        assert_eq!(t1.get("status").and_then(Value::as_str), Some("open"));
        assert!(t1.get("completed").is_none(), "取消完成应移除 completed");
    }

    #[test]
    fn ensure_data_dir_creates_example_then_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        ensure_data_dir(base).unwrap();
        let lists = base.join(LISTS_DIR);
        assert!(lists.is_dir(), "应创建 lists/ 目录");
        let example = lists.join(EXAMPLE_LIST_FILE);
        assert!(example.is_file(), "应生成示例清单");

        // 生成的示例可被读取
        let loaded = load_lists_from(base).unwrap();
        assert_eq!(loaded.lists.len(), 1);
        assert_eq!(loaded.lists[0].name, "收集箱");
        assert_eq!(loaded.lists[0].tasks.len(), 2);

        // 再次调用不应报错、不应覆盖
        fs::write(&example, "schema: 1\nname: 已改\norder: 1\ntasks: []\n").unwrap();
        ensure_data_dir(base).unwrap();
        assert!(fs::read_to_string(&example).unwrap().contains("已改"));
    }

    #[test]
    fn bad_file_is_skipped_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "good.yaml",
            "schema: 1\nname: 好清单\norder: 1\ntasks:\n  - id: G1\n    title: 好任务\n    status: open\n",
        );
        // 未闭合的 flow sequence → 解析失败
        write_list(base, "bad.yaml", "schema: 1\nname: [未闭合\n");

        let result = load_lists_from(base).unwrap();
        assert_eq!(result.lists.len(), 1, "坏文件应被跳过，好文件正常加载");
        assert_eq!(result.lists[0].name, "好清单");
        assert_eq!(result.lists[0].tasks.len(), 1);
        assert_eq!(result.warnings.len(), 1, "坏文件应产生一条 warning");
        assert!(
            result.warnings[0].contains("bad.yaml"),
            "warning 应包含坏文件名，实际：{:?}",
            result.warnings
        );
    }

    #[test]
    fn create_task_appends_and_preserves_unknown_fields() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let original = r#"schema: 1
name: 清单
order: 5
x-top: keep
tasks:
  - id: T1
    title: 原有
    status: open
    x-task: keep2
"#;
        let path = write_list(base, "清单.yaml", original);

        create_task_in(base, "清单.yaml", "  新任务  ").unwrap();

        let value: Value = yaml_serde::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        // 未知字段保留
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        let tasks = value.get("tasks").and_then(Value::as_sequence).unwrap();
        assert_eq!(tasks.len(), 2);
        // 原有任务不变
        assert_eq!(tasks[0].get("id").and_then(Value::as_str), Some("T1"));
        assert_eq!(tasks[0].get("x-task").and_then(Value::as_str), Some("keep2"));
        // 新任务追加在末尾，title 已 trim
        let task = &tasks[1];
        assert_eq!(task.get("title").and_then(Value::as_str), Some("新任务"));
        assert_eq!(task.get("status").and_then(Value::as_str), Some("open"));
        assert!(task.get("created").and_then(Value::as_str).is_some());
        assert_eq!(
            task.get("id").and_then(Value::as_str).unwrap().len(),
            26,
            "新任务 id 应为 ULID"
        );

        // 能被 load 读到
        let result = load_lists_from(base).unwrap();
        assert_eq!(result.lists[0].tasks.len(), 2);
        assert_eq!(result.lists[0].tasks[1].title, "新任务");
        assert_eq!(result.lists[0].tasks[1].status, "open");

        // 空标题拒绝，且不改动文件
        assert!(create_task_in(base, "清单.yaml", "   ").is_err());
        let value: Value = yaml_serde::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            value.get("tasks").and_then(Value::as_sequence).unwrap().len(),
            2
        );
    }

    #[test]
    fn create_list_generates_file_order_and_unique_names() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let lists = base.join(LISTS_DIR);
        fs::create_dir_all(&lists).unwrap();

        let read = |file: &str| -> Value {
            yaml_serde::from_str(&fs::read_to_string(lists.join(file)).unwrap()).unwrap()
        };

        // 空目录 → order 10
        let f1 = create_list_in(base, " 工作 ").unwrap();
        assert_eq!(f1, "工作.yaml");
        let v1 = read(&f1);
        assert_eq!(v1.get("schema").and_then(Value::as_i64), Some(1));
        assert_eq!(v1.get("name").and_then(Value::as_str), Some("工作"));
        assert_eq!(v1.get("order").and_then(Value::as_i64), Some(10));
        assert!(v1.get("tasks").and_then(Value::as_sequence).unwrap().is_empty());

        // 重名 → -2 / -3
        assert_eq!(create_list_in(base, "工作").unwrap(), "工作-2.yaml");
        assert_eq!(create_list_in(base, "工作").unwrap(), "工作-3.yaml");

        // order = 目录内最大 + 10
        assert_eq!(read("工作-3.yaml").get("order").and_then(Value::as_i64), Some(30));

        // 非法字符替换为 -
        let f4 = create_list_in(base, "a/b:c*?\"<>|").unwrap();
        assert!(f4.ends_with(".yaml"));
        assert!(f4.starts_with("a-b-c"), "非法字符应替换为 -：{f4}");
        assert!(
            !f4.chars()
                .any(|c| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')),
            "文件名不应残留非法字符：{f4}"
        );

        // 控制字符替换
        assert_eq!(create_list_in(base, "x\ty").unwrap(), "x-y.yaml");

        // 空名拒绝
        assert!(create_list_in(base, "   ").is_err());
        assert!(create_list_in(base, "").is_err());
    }

    #[test]
    fn update_task_changes_title_notes_preserving_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\nx-top: keep\ntasks:\n  - id: T1\n    title: 旧\n    status: open\n    notes: 旧备注\n    x-task: keep\n  - id: T2\n    title: 二\n    status: open\n",
        );

        update_task_in(base, "L.yaml", "T1", "  新标题  ", "  新备注  ").unwrap();
        let value = read_value(&path);
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        let tasks = value.get("tasks").and_then(Value::as_sequence).unwrap();
        let t1 = tasks
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some("T1"))
            .unwrap();
        assert_eq!(t1.get("title").and_then(Value::as_str), Some("新标题"));
        assert_eq!(t1.get("notes").and_then(Value::as_str), Some("新备注"));
        assert_eq!(t1.get("status").and_then(Value::as_str), Some("open"));
        assert_eq!(t1.get("x-task").and_then(Value::as_str), Some("keep"));
        // 其它任务不变
        let t2 = tasks
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some("T2"))
            .unwrap();
        assert_eq!(t2.get("title").and_then(Value::as_str), Some("二"));

        // 清空备注 → 移除 notes 字段
        update_task_in(base, "L.yaml", "T1", "新标题", "   ").unwrap();
        let value = read_value(&path);
        let t1 = value
            .get("tasks")
            .and_then(Value::as_sequence)
            .unwrap()
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some("T1"))
            .unwrap()
            .clone();
        assert!(t1.get("notes").is_none(), "空备注应移除 notes");
        assert_eq!(t1.get("title").and_then(Value::as_str), Some("新标题"));

        // 空标题 / 未找到任务 → 错误
        assert!(update_task_in(base, "L.yaml", "T1", "  ", "x").is_err());
        assert!(update_task_in(base, "L.yaml", "NOPE", "t", "").is_err());
    }

    #[test]
    fn delete_task_removes_only_target() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\nx-top: k\ntasks:\n  - id: T1\n    title: 一\n    status: open\n  - id: T2\n    title: 二\n    status: open\n    x-task: keep\n",
        );

        delete_task_in(base, "L.yaml", "T1").unwrap();
        let value = read_value(&path);
        let tasks = value.get("tasks").and_then(Value::as_sequence).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].get("id").and_then(Value::as_str), Some("T2"));
        assert_eq!(tasks[0].get("x-task").and_then(Value::as_str), Some("keep"));
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("k"));

        assert!(delete_task_in(base, "L.yaml", "NOPE").is_err());
    }

    #[test]
    fn rename_list_moves_file_and_suffixes_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let lists = base.join(LISTS_DIR);
        write_list(base, "A.yaml", "schema: 1\nname: A\norder: 10\nx-keep: 1\ntasks: []\n");
        write_list(base, "B.yaml", "schema: 1\nname: B\norder: 20\ntasks: []\n");

        // A → C：文件移动、name 更新、未知字段保留
        let new_file = rename_list_in(base, "A.yaml", "C").unwrap();
        assert_eq!(new_file, "C.yaml");
        assert!(lists.join("C.yaml").is_file());
        assert!(!lists.join("A.yaml").exists());
        let value = read_value(&lists.join("C.yaml"));
        assert_eq!(value.get("name").and_then(Value::as_str), Some("C"));
        assert_eq!(value.get("x-keep").and_then(Value::as_i64), Some(1));
        assert_eq!(value.get("order").and_then(Value::as_i64), Some(10));

        // B → C：重名追加后缀
        let new_file = rename_list_in(base, "B.yaml", "C").unwrap();
        assert_eq!(new_file, "C-2.yaml");
        assert!(lists.join("C-2.yaml").is_file());
        assert!(!lists.join("B.yaml").exists());
        assert_eq!(
            read_value(&lists.join("C-2.yaml")).get("name").and_then(Value::as_str),
            Some("C")
        );

        // 同名 → 就地改写，文件名不变
        let new_file = rename_list_in(base, "C.yaml", "C").unwrap();
        assert_eq!(new_file, "C.yaml");
        assert!(lists.join("C.yaml").is_file());

        // 空名 / 文件不存在 → 错误
        assert!(rename_list_in(base, "C.yaml", "   ").is_err());
        assert!(rename_list_in(base, "nope.yaml", "X").is_err());
    }

    #[test]
    fn delete_list_removes_file() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(base, "gone.yaml", "schema: 1\nname: gone\norder: 1\ntasks: []\n");

        delete_list_in(base, "gone.yaml").unwrap();
        assert!(!base.join(LISTS_DIR).join("gone.yaml").exists());
        assert!(delete_list_in(base, "gone.yaml").is_err());
        assert!(delete_list_in(base, "../evil.yaml").is_err());
    }

    #[test]
    fn move_list_reorders_and_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(base, "a.yaml", "schema: 1\nname: a\norder: 10\nx-keep: 1\ntasks: []\n");
        write_list(base, "b.yaml", "schema: 1\nname: b\norder: 20\ntasks: []\n");
        write_list(base, "c.yaml", "schema: 1\nname: c\norder: 30\ntasks: []\n");

        let names = |base: &Path| -> Vec<String> {
            load_lists_from(base)
                .unwrap()
                .lists
                .into_iter()
                .map(|l| l.name)
                .collect()
        };

        move_list_in(base, "b.yaml", "up").unwrap();
        assert_eq!(order_of(base, "b.yaml"), 10);
        assert_eq!(order_of(base, "a.yaml"), 20);
        assert_eq!(order_of(base, "c.yaml"), 30);
        assert_eq!(names(base), vec!["b", "a", "c"]);
        // 未知字段保留
        assert_eq!(
            read_value(&base.join(LISTS_DIR).join("a.yaml"))
                .get("x-keep")
                .and_then(Value::as_i64),
            Some(1)
        );

        // 已在首位再上移 → 无操作
        move_list_in(base, "b.yaml", "up").unwrap();
        assert_eq!(names(base), vec!["b", "a", "c"]);

        // c 下移 → 无操作（已在末位）
        move_list_in(base, "c.yaml", "down").unwrap();
        assert_eq!(names(base), vec!["b", "a", "c"]);

        // a 下移 → 与 c 交换
        move_list_in(base, "a.yaml", "down").unwrap();
        assert_eq!(names(base), vec!["b", "c", "a"]);

        // 非法方向 / 文件不存在 → 错误
        assert!(move_list_in(base, "a.yaml", "sideways").is_err());
        assert!(move_list_in(base, "nope.yaml", "up").is_err());
    }

    #[test]
    fn sanitize_handles_reserved_trailing_and_empty() {
        assert_eq!(sanitize_file_stem("CON"), "CON-");
        assert_eq!(sanitize_file_stem("con"), "con-");
        assert_eq!(sanitize_file_stem("Prn"), "Prn-");
        assert_eq!(sanitize_file_stem("COM1"), "COM1-");
        assert_eq!(sanitize_file_stem("lpt9"), "lpt9-");
        assert_eq!(sanitize_file_stem("COM0"), "COM0");
        assert_eq!(sanitize_file_stem("LPT0"), "LPT0");
        assert_eq!(sanitize_file_stem("name. ."), "name");
        assert_eq!(sanitize_file_stem("..."), "未命名");
        assert_eq!(sanitize_file_stem("   "), "未命名");
    }

    #[test]
    fn create_list_edge_names_use_sanitizer() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        fs::create_dir_all(base.join(LISTS_DIR)).unwrap();

        assert_eq!(create_list_in(base, "CON").unwrap(), "CON-.yaml");
        assert_eq!(create_list_in(base, "...").unwrap(), "未命名.yaml");
        assert_eq!(
            create_list_in(base, "name. ").unwrap(),
            "name.yaml",
            "结尾点 / 空格应被去除"
        );
    }

    #[test]
    fn rejects_bad_file_name_and_missing_task() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "a.yaml",
            "schema: 1\nname: a\norder: 1\ntasks:\n  - id: X\n    title: x\n    status: open\n",
        );

        assert!(set_task_status_in(base, "../evil.yaml", "X", true).is_err());
        assert!(set_task_status_in(base, "a.yaml", "NOPE", true).is_err());
        assert!(create_task_in(base, "../evil.yaml", "x").is_err());
        assert!(create_task_in(base, "a.yaml", "   ").is_err());
    }
}
