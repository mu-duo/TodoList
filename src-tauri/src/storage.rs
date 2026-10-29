//! 存储层：数据目录解析、清单读取、任务状态回写（Value 级补丁 + 原子写）。
//!
//! 所有函数都以「基础目录」为参数，便于在测试中注入临时目录，
//! 命令层再通过 [`data_dir`] 传入真实数据目录。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Deserializer, Serialize};
use yaml_serde::{Mapping, Value};

/// 进程级写锁：串行化所有「读取-修改-写回」任务 / 清单文件的操作。
///
/// 存在的理由：通知线程上的「不再提醒」回写（`disable_task_reminder` →
/// `update_task_metadata_in`）可能与前端命令的写操作（元数据 / 状态 / 子任务 / 移动 /
/// 排序 / 新建 / 删除等）并发；同一清单文件的读-改-写若交错会造成静默丢失更新
/// （lost update）。本锁完全包含在每个存储操作内部：进入操作体即获取、返回前释放，
/// 不与运行时的 `load_lists_lock` 嵌套或交叉持有（锁序平凡）。纯读取（`load_lists_from`
/// 等）不做加锁，依赖 `atomic_write` 的临时文件 + rename 保证读到完整内容。
static TASK_FILE_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// 获取进程级写锁；中毒时取回内部值（仍有 `()` 可用，等价于未中毒）。
fn task_file_write_guard() -> MutexGuard<'static, ()> {
    TASK_FILE_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub const LISTS_DIR: &str = "lists";
pub const EXAMPLE_LIST_FILE: &str = "收集箱.yaml";

/// 设置文件名（位于应用配置目录 `%APPDATA%\com.lftan.todolist`）。
pub const SETTINGS_FILE: &str = "settings.yaml";
/// 应用标识（同时用作配置目录名）。
pub const APP_IDENTIFIER: &str = "com.lftan.todolist";
/// 默认主题。
pub const DEFAULT_THEME: &str = "system";

/// 应用设置（主题 + 应用功能栏入口顺序）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub theme: String,
    /// 应用功能栏入口顺序（本机偏好）；缺失 / 空 / 非法 → `None`（前端用默认顺序）。
    #[serde(default)]
    pub app_bar_order: Option<Vec<String>>,
    /// 「已完成」视图折叠的清单文件名集合（本机偏好）；缺失 / 空 / 非法 → `None`（默认全部展开）。
    #[serde(default)]
    pub completed_collapsed: Option<Vec<String>>,
    /// 「已完成」视图是否显示归档任务（本机偏好）；缺失 / 非布尔 → `false`（默认关闭）。
    #[serde(default)]
    pub completed_show_archived: bool,
    /// 任务详情折叠的子板块 key 集合（本机偏好）；缺失 / 非法 → `None`（默认仅 `metadata` 收起）。
    /// 与 `completed_collapsed` 不同，显式空数组是有效值（全部展开）。
    #[serde(default)]
    pub detail_collapsed: Option<Vec<String>>,
    /// 「关闭时最小化到托盘」（本机偏好）；缺失 / 非法 → `None`（默认开启）。
    #[serde(default)]
    pub minimize_to_tray: Option<bool>,
}

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
    /// 归档时间（`FR-APP-36`）；缺失 / 非字符串 / 空 → 未归档。
    pub archived: Option<String>,
    pub priority: Option<String>,
    pub due: Option<String>,
    /// 是否关闭提醒；`Some(false)` = 不再提醒，`None` = 默认开启。
    /// 契约：仅序列化为 `null | false`，绝不出现 `true`（`true` / 缺省均归一化为 `None`）。
    pub reminder: Option<bool>,
    pub tags: Option<Vec<String>>,
    pub notes: Option<String>,
    pub subtasks: Option<Vec<SubtaskView>>,
    /// Agent 任务意图（`agent: { enabled, paths, links }`）；缺失 / 非法 → `None`。
    /// `links` 仅供 UI 点击打开，不参与沙箱、不传给模型。
    pub agent: Option<crate::agent::TaskAgent>,
}

/// Distinguishes an omitted patch property from an explicit null.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum FieldPatch<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for FieldPatch<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PatchVisitor<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for PatchVisitor<T> {
            type Value = FieldPatch<T>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a value or null")
            }

            fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(FieldPatch::Null)
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(FieldPatch::Null)
            }

            fn visit_some<D2: Deserializer<'de>>(self, deserializer: D2) -> Result<Self::Value, D2::Error> {
                T::deserialize(deserializer).map(FieldPatch::Value)
            }
        }

        deserializer.deserialize_option(PatchVisitor(std::marker::PhantomData))
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskMetadataPatch {
    #[serde(default)]
    pub priority: FieldPatch<String>,
    #[serde(default)]
    pub due: FieldPatch<String>,
    #[serde(default)]
    pub tags: FieldPatch<Vec<String>>,
    /// 省略 = 不变；`false` = 写 `reminder: false`；`null` = 删键；`true` = 非法。
    #[serde(default)]
    pub reminder: FieldPatch<bool>,
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

/// 系统配置目录下的应用设置目录：`%APPDATA%\com.lftan.todolist`。
///
/// 仅解析路径，不创建目录（由 [`save_settings_in`] 按需创建）。
pub fn settings_dir() -> Result<PathBuf, String> {
    let config = dirs::config_dir().ok_or_else(|| "无法定位系统配置目录".to_string())?;
    Ok(config.join(APP_IDENTIFIER))
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

/// 归一化主题值：仅接受 `system` / `light` / `dark`，其余回退 `system`。
fn normalize_theme(theme: &str) -> String {
    match theme {
        "light" | "dark" => theme.to_string(),
        _ => DEFAULT_THEME.to_string(),
    }
}

/// 剥离前导 UTF-8 BOM（`\u{FEFF}`）；避免解析器误判文档边界（误导性「多文档」错误）。
/// 供 settings 与 agent 配置读取共用。
pub(crate) fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{FEFF}').unwrap_or(text)
}

/// 读取设置目录（`%APPDATA%\com.lftan.todolist`）下的 `settings.yaml`。
///
/// 文件不存在、读取 / 解析失败、或 `theme` 值非法时均回退默认 `{ theme: "system" }`，
/// 且**不会**创建文件、不会 panic。`app_bar.order` 缺失 / 空 / 非法时 `app_bar_order` 为
/// `None`；合法字符串 id 去重（首次保留）并保留未知 id 原顺序，不自动补已知 id。
/// `completed.collapsed` 同理由 [`parse_completed_collapsed`] 严格解析；`completed.show_archived`
/// 缺失 / 非布尔 → `false`；`detail.collapsed` 由 [`parse_detail_collapsed`] 解析（显式空数组合法）。
/// 读取前剥离前导 BOM。
pub fn load_settings_from(base: &Path) -> Result<Settings, String> {
    let fallback = || Settings {
        theme: DEFAULT_THEME.to_string(),
        app_bar_order: None,
        completed_collapsed: None,
        completed_show_archived: false,
        detail_collapsed: None,
        minimize_to_tray: None,
    };
    let path = base.join(SETTINGS_FILE);
    if !path.is_file() {
        return Ok(fallback());
    }
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return Ok(fallback()),
    };
    let value: Value = match yaml_serde::from_str(strip_bom(&text)) {
        Ok(value) => value,
        Err(_) => return Ok(fallback()),
    };
    let theme = value
        .get("theme")
        .and_then(Value::as_str)
        .map(normalize_theme)
        .unwrap_or_else(|| DEFAULT_THEME.to_string());
    Ok(Settings {
        theme,
        app_bar_order: parse_app_bar_order(&value),
        completed_collapsed: parse_completed_collapsed(&value),
        completed_show_archived: parse_completed_show_archived(&value),
        detail_collapsed: parse_detail_collapsed(&value),
        // 缺失 / 非布尔 → None（默认开启）；字面 false → Some(false)（已关闭）。
        minimize_to_tray: value.get("minimize_to_tray").and_then(Value::as_bool),
    })
}

/// 从设置 Value 解析 `app_bar.order`：缺失 / 非对象 / 非数组 / 含非字符串或空白元素 /
/// 去重后为空 → `None`；否则按首次出现去重并保留原始顺序（含未知 id，不自动补已知 id）。
fn parse_app_bar_order(value: &Value) -> Option<Vec<String>> {
    let seq = value.get("app_bar")?.get("order")?.as_sequence()?;
    let mut order: Vec<String> = Vec::with_capacity(seq.len());
    for item in seq {
        let id = item.as_str()?;
        if id.trim().is_empty() {
            return None;
        }
        if !order.iter().any(|existing| existing == id) {
            order.push(id.to_string());
        }
    }
    if order.is_empty() {
        None
    } else {
        Some(order)
    }
}

/// 从设置 Value 解析 `completed.collapsed`：缺失 / 非对象 / 非数组 / 含非字符串或空白元素 /
/// 去重后为空 → `None`；否则按首次出现去重并保留原始顺序。
fn parse_completed_collapsed(value: &Value) -> Option<Vec<String>> {
    let seq = value.get("completed")?.get("collapsed")?.as_sequence()?;
    let mut collapsed: Vec<String> = Vec::with_capacity(seq.len());
    for item in seq {
        let file = item.as_str()?;
        if file.trim().is_empty() {
            return None;
        }
        if !collapsed.iter().any(|existing| existing == file) {
            collapsed.push(file.to_string());
        }
    }
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

/// 从设置 Value 解析 `completed.show_archived`：缺失 / `completed` 非对象 / 非布尔 → `false`
/// （默认不显示归档）；仅字面 `true` → `true`。
fn parse_completed_show_archived(value: &Value) -> bool {
    value
        .get("completed")
        .and_then(|completed| completed.get("show_archived"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// 从设置 Value 解析 `detail.collapsed`：缺失 / 非对象 / 非数组 / 含非字符串或空白元素 → `None`；
/// 否则按首次出现去重并保留原始顺序。**与 `completed.collapsed` 不同：显式空数组是有效值**
/// （`Some(vec![])` = 全部展开），不会被折叠成 `None`。
fn parse_detail_collapsed(value: &Value) -> Option<Vec<String>> {
    let seq = value.get("detail")?.get("collapsed")?.as_sequence()?;
    let mut collapsed: Vec<String> = Vec::with_capacity(seq.len());
    for item in seq {
        let section = item.as_str()?;
        if section.trim().is_empty() {
            return None;
        }
        if !collapsed.iter().any(|existing| existing == section) {
            collapsed.push(section.to_string());
        }
    }
    Some(collapsed)
}

/// 写入 `theme` 到设置目录（`%APPDATA%\com.lftan.todolist`）下的 `settings.yaml`。
///
/// 沿用现有做法：Value 级补丁（保留未知字段）+ 原子写 + 中文错误消息；
/// 文件不存在则创建，已存在则仅改 `theme`。非法主题值归一化为 `system`。
pub fn save_settings_in(base: &Path, theme: &str) -> Result<(), String> {
    let theme = normalize_theme(theme);
    fs::create_dir_all(base).map_err(|e| format!("创建配置目录失败：{e}"))?;
    let path = base.join(SETTINGS_FILE);

    let mut value = if path.is_file() {
        let text = fs::read_to_string(&path).map_err(|e| format!("读取设置文件失败：{e}"))?;
        yaml_serde::from_str(&text).map_err(|e| format!("解析设置文件失败：{e}"))?
    } else {
        Value::Mapping(Mapping::new())
    };

    let map = value
        .as_mapping_mut()
        .ok_or_else(|| "设置文件格式无效：期望对象".to_string())?;
    map.insert(key("theme"), Value::String(theme));
    write_list(&path, &value)
}

/// 写入 / 清除 `app_bar.order`（`None` 或空数组 → 删除 `order` 以恢复默认）。
///
/// 沿用现有做法：重读最新设置文件 → Value 级补丁**仅改 `app_bar.order`** → 原子写，
/// 保留 `theme`、`app_bar` 其它字段及所有未知字段（不做整份 [`Settings`] 覆盖）。
/// 空白 id → 中文错误；已有 `app_bar` 若非对象 → 中文错误（不覆盖未知标量）。
pub fn save_app_bar_order_in(base: &Path, order: Option<&[String]>) -> Result<(), String> {
    let ids = order.filter(|ids| !ids.is_empty());
    if let Some(ids) = ids {
        for id in ids {
            if id.trim().is_empty() {
                return Err("应用功能栏入口 id 不能为空".to_string());
            }
        }
    }

    fs::create_dir_all(base).map_err(|e| format!("创建配置目录失败：{e}"))?;
    let path = base.join(SETTINGS_FILE);

    let mut value = if path.is_file() {
        let text = fs::read_to_string(&path).map_err(|e| format!("读取设置文件失败：{e}"))?;
        yaml_serde::from_str(&text).map_err(|e| format!("解析设置文件失败：{e}"))?
    } else {
        Value::Mapping(Mapping::new())
    };

    let root = value
        .as_mapping_mut()
        .ok_or_else(|| "设置文件格式无效：期望对象".to_string())?;

    match ids {
        Some(ids) => {
            let app_bar = match root.get_mut("app_bar") {
                Some(existing) => existing
                    .as_mapping_mut()
                    .ok_or_else(|| "设置文件中的 app_bar 不是对象，无法写入".to_string())?,
                None => {
                    root.insert(key("app_bar"), Value::Mapping(Mapping::new()));
                    root.get_mut("app_bar")
                        .and_then(Value::as_mapping_mut)
                        .expect("刚插入的 app_bar 应为对象")
                }
            };
            let seq = ids.iter().map(|id| Value::String(id.clone())).collect();
            app_bar.insert(key("order"), Value::Sequence(seq));
        }
        None => {
            if let Some(existing) = root.get_mut("app_bar") {
                let app_bar = existing
                    .as_mapping_mut()
                    .ok_or_else(|| "设置文件中的 app_bar 不是对象，无法写入".to_string())?;
                app_bar.remove("order");
            }
        }
    }

    write_list(&path, &value)
}

/// 写入 / 清除 `completed.collapsed`（`None` 或空数组 → 删除 `collapsed` 以恢复默认：全部展开）。
///
/// 沿用现有做法：重读最新设置文件 → Value 级补丁**仅改 `completed.collapsed`** → 原子写，
/// 保留 `theme`、`app_bar`、`completed` 其它字段及所有未知字段（不做整份 [`Settings`] 覆盖）。
/// 空白文件名 → 中文错误；已有 `completed` 若非对象 → 中文错误（不覆盖未知标量）。
pub fn save_completed_collapsed_in(
    base: &Path,
    collapsed: Option<&[String]>,
) -> Result<(), String> {
    let files = collapsed.filter(|files| !files.is_empty());
    if let Some(files) = files {
        for file in files {
            if file.trim().is_empty() {
                return Err("已完成清单折叠键不能为空".to_string());
            }
        }
    }

    fs::create_dir_all(base).map_err(|e| format!("创建配置目录失败：{e}"))?;
    let path = base.join(SETTINGS_FILE);

    let mut value = if path.is_file() {
        let text = fs::read_to_string(&path).map_err(|e| format!("读取设置文件失败：{e}"))?;
        yaml_serde::from_str(&text).map_err(|e| format!("解析设置文件失败：{e}"))?
    } else {
        Value::Mapping(Mapping::new())
    };

    let root = value
        .as_mapping_mut()
        .ok_or_else(|| "设置文件格式无效：期望对象".to_string())?;

    match files {
        Some(files) => {
            let completed = match root.get_mut("completed") {
                Some(existing) => existing
                    .as_mapping_mut()
                    .ok_or_else(|| "设置文件中的 completed 不是对象，无法写入".to_string())?,
                None => {
                    root.insert(key("completed"), Value::Mapping(Mapping::new()));
                    root.get_mut("completed")
                        .and_then(Value::as_mapping_mut)
                        .expect("刚插入的 completed 应为对象")
                }
            };
            let seq = files
                .iter()
                .map(|file| Value::String(file.clone()))
                .collect();
            completed.insert(key("collapsed"), Value::Sequence(seq));
        }
        None => {
            if let Some(existing) = root.get_mut("completed") {
                let completed = existing
                    .as_mapping_mut()
                    .ok_or_else(|| "设置文件中的 completed 不是对象，无法写入".to_string())?;
                completed.remove("collapsed");
            }
        }
    }

    write_list(&path, &value)
}

/// 写入 / 清除 `completed.show_archived`（`false` → 删除该键以恢复默认：不显示归档任务）。
///
/// 沿用现有做法：重读最新设置文件 → Value 级补丁**仅改 `completed.show_archived`** → 原子写，
/// 保留 `theme`、`app_bar`、`completed.collapsed`、`detail` 及所有未知字段（不做整份 [`Settings`] 覆盖）。
/// 已有 `completed` 若非对象 → 中文错误（不覆盖未知标量）。
pub fn save_completed_show_archived_in(base: &Path, show: bool) -> Result<(), String> {
    fs::create_dir_all(base).map_err(|e| format!("创建配置目录失败：{e}"))?;
    let path = base.join(SETTINGS_FILE);

    let mut value = if path.is_file() {
        let text = fs::read_to_string(&path).map_err(|e| format!("读取设置文件失败：{e}"))?;
        yaml_serde::from_str(&text).map_err(|e| format!("解析设置文件失败：{e}"))?
    } else {
        Value::Mapping(Mapping::new())
    };

    let root = value
        .as_mapping_mut()
        .ok_or_else(|| "设置文件格式无效：期望对象".to_string())?;

    if show {
        let completed = match root.get_mut("completed") {
            Some(existing) => existing
                .as_mapping_mut()
                .ok_or_else(|| "设置文件中的 completed 不是对象，无法写入".to_string())?,
            None => {
                root.insert(key("completed"), Value::Mapping(Mapping::new()));
                root.get_mut("completed")
                    .and_then(Value::as_mapping_mut)
                    .expect("刚插入的 completed 应为对象")
            }
        };
        completed.insert(key("show_archived"), Value::Bool(true));
    } else if let Some(existing) = root.get_mut("completed") {
        let completed = existing
            .as_mapping_mut()
            .ok_or_else(|| "设置文件中的 completed 不是对象，无法写入".to_string())?;
        completed.remove("show_archived");
    }

    write_list(&path, &value)
}

/// 写入 / 清除 `detail.collapsed`（`None` → 删除该字段；**空数组会写盘**，表示全部展开）。
///
/// 沿用现有做法：重读最新设置文件 → Value 级补丁**仅改 `detail.collapsed`** → 原子写，
/// 保留 `theme`、`app_bar`、`completed`、`detail` 其它字段及所有未知字段（不做整份 [`Settings`] 覆盖）。
/// 与 [`save_completed_collapsed_in`] 的语义差异：`Some(&[])` 写入空序列并保留 key（不删 key），
/// 以便前端区分「显式全部展开」与「默认（仅 `metadata` 收起）」。
/// 空白 key → 中文错误；已有 `detail` 若非对象 → 中文错误（不覆盖未知标量）。
pub fn save_detail_collapsed_in(
    base: &Path,
    collapsed: Option<&[String]>,
) -> Result<(), String> {
    if let Some(collapsed) = collapsed {
        for section in collapsed {
            if section.trim().is_empty() {
                return Err("任务详情折叠键不能为空".to_string());
            }
        }
    }

    fs::create_dir_all(base).map_err(|e| format!("创建配置目录失败：{e}"))?;
    let path = base.join(SETTINGS_FILE);

    let mut value = if path.is_file() {
        let text = fs::read_to_string(&path).map_err(|e| format!("读取设置文件失败：{e}"))?;
        yaml_serde::from_str(&text).map_err(|e| format!("解析设置文件失败：{e}"))?
    } else {
        Value::Mapping(Mapping::new())
    };

    let root = value
        .as_mapping_mut()
        .ok_or_else(|| "设置文件格式无效：期望对象".to_string())?;

    match collapsed {
        Some(collapsed) => {
            let detail = match root.get_mut("detail") {
                Some(existing) => existing
                    .as_mapping_mut()
                    .ok_or_else(|| "设置文件中的 detail 不是对象，无法写入".to_string())?,
                None => {
                    root.insert(key("detail"), Value::Mapping(Mapping::new()));
                    root.get_mut("detail")
                        .and_then(Value::as_mapping_mut)
                        .expect("刚插入的 detail 应为对象")
                }
            };
            let seq = collapsed
                .iter()
                .map(|section| Value::String(section.clone()))
                .collect();
            detail.insert(key("collapsed"), Value::Sequence(seq));
        }
        None => {
            let mut remove_detail = false;
            if let Some(existing) = root.get_mut("detail") {
                let detail = existing
                    .as_mapping_mut()
                    .ok_or_else(|| "设置文件中的 detail 不是对象，无法写入".to_string())?;
                detail.remove("collapsed");
                remove_detail = detail.is_empty();
            }
            if remove_detail {
                root.remove("detail");
            }
        }
    }

    write_list(&path, &value)
}

/// 写入 / 清除 `minimize_to_tray`（`true` → 删除键以恢复默认：关闭时最小化到托盘）。
///
/// 沿用现有做法：重读最新设置文件 → Value 级补丁**仅改 `minimize_to_tray`** → 原子写，
/// 保留 `theme`、`app_bar`、`completed` 及所有未知字段（不做整份 [`Settings`] 覆盖）。
/// `false` → 写 `minimize_to_tray: false`（关闭）；`true` → 删除键（恢复默认开启）。
pub fn save_minimize_to_tray_in(base: &Path, enabled: bool) -> Result<(), String> {
    fs::create_dir_all(base).map_err(|e| format!("创建配置目录失败：{e}"))?;
    let path = base.join(SETTINGS_FILE);

    let mut value = if path.is_file() {
        let text = fs::read_to_string(&path).map_err(|e| format!("读取设置文件失败：{e}"))?;
        yaml_serde::from_str(&text).map_err(|e| format!("解析设置文件失败：{e}"))?
    } else {
        Value::Mapping(Mapping::new())
    };

    let root = value
        .as_mapping_mut()
        .ok_or_else(|| "设置文件格式无效：期望对象".to_string())?;

    if enabled {
        root.remove("minimize_to_tray");
    } else {
        root.insert(key("minimize_to_tray"), Value::Bool(false));
    }

    write_list(&path, &value)
}

/// 读取 `lists/*.yaml` 全部清单；排序：文件内 order 升序，其次按名称。
///
/// 单个文件读取 / 解析失败会被跳过并记入 [`LoadResult::warnings`]；
/// 仅 `lists/` 目录整体不可读时返回硬错误。
pub fn load_lists_from(base: &Path) -> Result<LoadResult, String> {
    let lists_dir = base.join(LISTS_DIR);
    let entries = fs::read_dir(&lists_dir).map_err(|e| format!("读取清单目录失败：{e}"))?;

    let mut paths: Vec<PathBuf> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if is_yaml(&path) {
                    paths.push(path);
                }
            }
            Err(error) => warnings.push(format!("遍历清单目录时读取条目失败：{error}")),
        }
    }
    paths.sort();

    let mut loaded: Vec<(i64, ListView)> = Vec::with_capacity(paths.len());
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
        if value.as_mapping().is_none() {
            warnings.push(format!("清单「{file}」根节点必须是 mapping，已跳过"));
            continue;
        }
        let Some(task_values) = value.get("tasks").and_then(Value::as_sequence) else {
            warnings.push(format!("清单「{file}」缺少有效的 tasks sequence，已跳过"));
            continue;
        };
        let valid_task_count = task_values
            .iter()
            .filter(|task| task.as_mapping().is_some())
            .count();
        if valid_task_count != task_values.len() {
            warnings.push(format!(
                "清单「{file}」包含非 mapping 任务项，部分任务已跳过"
            ));
        }
        let order = value
            .get("order")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MAX);
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
pub(crate) fn list_path(base: &Path, list_file: &str) -> Result<PathBuf, String> {
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
pub(crate) fn key(name: &str) -> Value {
    Value::String(name.to_string())
}

/// 读取并解析清单文件为 Value。
pub(crate) fn read_list_value(path: &Path, list_file: &str) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("读取清单「{list_file}」失败：{e}"))?;
    yaml_serde::from_str(&text).map_err(|e| format!("解析清单「{list_file}」失败：{e}"))
}

/// 将 Value 序列化后原子写回。
pub(crate) fn write_list(path: &Path, value: &Value) -> Result<(), String> {
    let out = yaml_serde::to_string(value).map_err(|e| format!("序列化失败：{e}"))?;
    atomic_write(path, out.as_bytes())
}

/// 按 id 定位任务对象，未找到 / 非对象时报错。
pub(crate) fn find_task_mut<'a>(
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

/// 按任务 id 仅修改 `status`（及 `completed`；回到 `open` 时同时移除 `archived`），
/// Value 级补丁后原子写回。
pub fn set_task_status_in(
    base: &Path,
    list_file: &str,
    task_id: &str,
    done: bool,
) -> Result<(), String> {
    let _guard = task_file_write_guard();
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
        // 未完成任务不可能处于归档态（FR-APP-36）；取消完成时一并取消归档。
        map.remove("archived");
    }

    write_list(&path, &value)
}

/// 批量写入 / 删除任务的 `archived` 键：一次重读 + 一次原子写（Value 级补丁）。
///
/// `archive = true`：仅对 `status == "done"` 且当前无归档时间的任务写入 `now_local()`；
/// 不存在的 id、非 done、已有非空 `archived` 的任务静默跳过（不覆盖原归档时间）。
/// `archive = false`：删除匹配任务的 `archived` 键；缺失该键的任务同样静默跳过。
/// 未知字段与未命中任务原样保留。
fn patch_task_archive(
    base: &Path,
    list_file: &str,
    task_ids: &[String],
    archive: bool,
) -> Result<(), String> {
    if task_ids.is_empty() {
        return Ok(());
    }

    let _guard = task_file_write_guard();
    let path = list_path(base, list_file)?;
    let mut value = read_list_value(&path, list_file)?;

    let tasks = value
        .get_mut("tasks")
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| format!("清单「{list_file}」缺少 tasks 数组"))?;

    for task in tasks.iter_mut() {
        let matched = task
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| task_ids.iter().any(|target| target == id));
        if !matched {
            continue;
        }
        let Some(map) = task.as_mapping_mut() else {
            continue;
        };

        if archive {
            if map.get("status").and_then(Value::as_str) != Some("done") {
                continue;
            }
            let already_archived = map
                .get("archived")
                .and_then(Value::as_str)
                .is_some_and(|archived| !archived.trim().is_empty());
            if already_archived {
                continue;
            }
            map.insert(key("archived"), Value::String(now_local()));
        } else {
            map.remove("archived");
        }
    }

    write_list(&path, &value)
}

/// 批量归档同一清单内的任务：对每个 `status == done` 的任务写入 `archived = 本地时间`。
///
/// 跳过不存在的 id 与非 done 任务（不报错）；已有非空 `archived` 的任务不覆盖。
/// 一次重读 + 一次原子写，未知字段与其它任务原样保留。
pub fn archive_tasks_in(base: &Path, list_file: &str, task_ids: &[String]) -> Result<(), String> {
    patch_task_archive(base, list_file, task_ids, true)
}

/// 批量取消归档同一清单内的任务：删除匹配任务的 `archived` 键。
///
/// 不存在的 id、缺失该键的任务静默跳过；一次重读 + 一次原子写。
pub fn unarchive_tasks_in(base: &Path, list_file: &str, task_ids: &[String]) -> Result<(), String> {
    patch_task_archive(base, list_file, task_ids, false)
}

/// 若 `value` 的 `tasks` 为数组则把 `task` 插到**首位**，否则报错。
fn prepend_task(value: &mut Value, list_file: &str, task: Mapping) -> Result<(), String> {
    let tasks = value
        .get_mut("tasks")
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| format!("清单「{list_file}」缺少 tasks 数组"))?;
    tasks.insert(0, Value::Mapping(task));
    Ok(())
}

/// 向清单**顶部**插入新任务（自动 ULID `id` 与 `created`），Value 级补丁后原子写回。
pub fn create_task_in(base: &Path, list_file: &str, title: &str) -> Result<(), String> {
    let _guard = task_file_write_guard();
    let title = title.trim();
    if title.is_empty() {
        return Err("任务标题不能为空".to_string());
    }

    let path = list_path(base, list_file)?;
    let mut value = read_list_value(&path, list_file)?;
    prepend_task(&mut value, list_file, new_task_mapping(title))?;
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
    let _guard = task_file_write_guard();
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

/// Value-level metadata patch that preserves all unrelated task/list fields.
pub fn update_task_metadata_in(
    base: &Path,
    list_file: &str,
    task_id: &str,
    patch: TaskMetadataPatch,
) -> Result<(), String> {
    let _guard = task_file_write_guard();
    let path = list_path(base, list_file)?;
    let mut value = read_list_value(&path, list_file)?;
    let map = find_task_mut(&mut value, list_file, task_id)?;

    let priority = match patch.priority {
        FieldPatch::Missing => None,
        FieldPatch::Null => Some(None),
        FieldPatch::Value(value) => {
            if !matches!(value.as_str(), "high" | "medium" | "low") {
                return Err("优先级仅支持 high、medium、low".to_string());
            }
            Some(Some(value))
        }
    };
    let due = match patch.due {
        FieldPatch::Missing => None,
        FieldPatch::Null => Some(None),
        FieldPatch::Value(value) => {
            validate_due(&value)?;
            Some(Some(value))
        }
    };
    let reminder = match patch.reminder {
        FieldPatch::Missing => None,
        FieldPatch::Null => Some(None),
        FieldPatch::Value(false) => Some(Some(false)),
        FieldPatch::Value(true) => {
            return Err("提醒默认开启，reminder 仅支持 false 或 null".to_string());
        }
    };

    let resulting_due = match &due {
        Some(value) => value.as_deref(),
        None => map.get("due").and_then(Value::as_str),
    };
    if reminder == Some(Some(false)) && resulting_due.is_none() {
        return Err("没有截止日期时不能设置不再提醒".to_string());
    }

    if let Some(priority) = priority {
        set_optional_string(map, "priority", priority);
    }
    if let Some(due) = due {
        let cleared = due.is_none();
        set_optional_string(map, "due", due);
        if cleared {
            map.remove("reminder");
        }
    }
    match patch.tags {
        FieldPatch::Missing => {}
        FieldPatch::Null => {
            map.remove("tags");
        }
        FieldPatch::Value(tags) if tags.is_empty() => {
            map.remove("tags");
        }
        FieldPatch::Value(tags) => {
            map.insert(
                key("tags"),
                Value::Sequence(tags.into_iter().map(Value::String).collect()),
            );
        }
    }
    if let Some(reminder) = reminder {
        match reminder {
            Some(false) => {
                map.insert(key("reminder"), Value::Bool(false));
            }
            None => {
                map.remove("reminder");
            }
            // 上面的 match 已拒绝 `true`，此处不可达。
            Some(true) => unreachable!(),
        }
    }
    // 旧版 `reminder_offset_days` 不再写入：任意保存都清理遗留键。
    map.remove("reminder_offset_days");
    write_list(&path, &value)
}

fn set_optional_string(map: &mut Mapping, field: &str, value: Option<String>) {
    match value {
        Some(value) => {
            map.insert(key(field), Value::String(value));
        }
        None => {
            map.remove(field);
        }
    }
}

fn validate_due(value: &str) -> Result<(), String> {
    let valid = if value.len() == 10 {
        chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map(|date| date.format("%Y-%m-%d").to_string() == value)
            .unwrap_or(false)
    } else if value.len() == 16 {
        chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
            .map(|date| date.format("%Y-%m-%dT%H:%M").to_string() == value)
            .unwrap_or(false)
    } else {
        false
    };
    if valid {
        Ok(())
    } else {
        Err("截止日期格式必须为 YYYY-MM-DD 或 YYYY-MM-DDTHH:MM".to_string())
    }
}

/// 从清单的 `tasks` 数组中移除指定任务。
pub fn delete_task_in(base: &Path, list_file: &str, task_id: &str) -> Result<(), String> {
    let _guard = task_file_write_guard();
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

/// 把任务从一个清单转移到另一个清单末尾（Value 级移动，id 与全部字段原样保留）。
///
/// 校验：源 ≠ 目标；源清单存在且含该 `task_id`；目标清单存在且**不含**同 id 任务
/// （任一不满足即报错，且不写任何文件）。
///
/// 写序：先向目标清单 `tasks` 末尾追加并原子写，成功后再从源清单移除并原子写。
/// 若源移除写失败：用此前读到的目标原 Value 尝试原子回滚目标——回滚成功返回错误
/// （说明移动未生效）；回滚也失败返回明确错误（两处可能同 id 双份，绝不静默）。
///
/// 已知限制：Agent 运行中的任务转移无特殊保护（不在本切片解决）；拖拽 UI 由前端负责。
pub fn move_task_in(
    base: &Path,
    list_file: &str,
    task_id: &str,
    to_list_file: &str,
) -> Result<(), String> {
    let _guard = task_file_write_guard();
    if list_file == to_list_file {
        return Err("源清单与目标清单相同，无法移动".to_string());
    }

    let from_path = list_path(base, list_file)?;
    let to_path = list_path(base, to_list_file)?;

    // 读取源清单并取出目标任务（此时尚未写盘）。
    let mut from_value = read_list_value(&from_path, list_file)?;
    let from_tasks = from_value
        .get_mut("tasks")
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| format!("清单「{list_file}」缺少 tasks 数组"))?;
    let index = from_tasks
        .iter()
        .position(|t| t.get("id").and_then(Value::as_str) == Some(task_id))
        .ok_or_else(|| format!("清单「{list_file}」中未找到任务 {task_id}"))?;
    let task = from_tasks.remove(index);

    // 读取目标清单并校验存在性与同 id 冲突；保留原值用于回滚。
    if !to_path.is_file() {
        return Err(format!("清单文件不存在：{to_list_file}"));
    }
    let mut to_value = read_list_value(&to_path, to_list_file)?;
    let to_original = to_value.clone();
    {
        let to_tasks = to_value
            .get_mut("tasks")
            .and_then(Value::as_sequence_mut)
            .ok_or_else(|| format!("清单「{to_list_file}」缺少 tasks 数组"))?;
        if to_tasks
            .iter()
            .any(|t| t.get("id").and_then(Value::as_str) == Some(task_id))
        {
            return Err(format!("目标清单「{to_list_file}」已存在任务 {task_id}"));
        }
        to_tasks.push(task);
    }

    // 先写目标（追加），再写源（移除）。
    write_list(&to_path, &to_value)?;

    if let Err(source_error) = write_list(&from_path, &from_value) {
        // 源移除失败：尝试把目标回滚为原内容。
        return match write_list(&to_path, &to_original) {
            Ok(()) => Err(format!(
                "移动任务失败（源清单「{list_file}」写回失败，目标已回滚，移动未生效）：{source_error}"
            )),
            Err(rollback_error) => Err(format!(
                "移动任务失败且状态不一致：源清单「{list_file}」写回失败（{source_error}），\
                 回滚目标清单「{to_list_file}」也失败（{rollback_error}）；\
                 任务 {task_id} 可能同时存在于两个清单，请手动检查"
            )),
        };
    }
    Ok(())
}

/// 向指定任务的 `subtasks` 末尾追加一条子任务，返回其视图。
///
/// 标题 `trim` 后为空 → 错误；任务缺失 → 错误；`subtasks` 缺失则新建数组，
/// 存在但非数组 → 错误（不静默覆盖）。Value 级补丁 + 原子写，保留未知字段。
pub fn add_subtask_in(
    base: &Path,
    list_file: &str,
    task_id: &str,
    title: &str,
) -> Result<SubtaskView, String> {
    let _guard = task_file_write_guard();
    let title = title.trim();
    if title.is_empty() {
        return Err("子任务标题不能为空".to_string());
    }

    let path = list_path(base, list_file)?;
    let mut value = read_list_value(&path, list_file)?;
    let map = find_task_mut(&mut value, list_file, task_id)?;

    let subtasks = match map.get_mut("subtasks") {
        Some(existing) => existing
            .as_sequence_mut()
            .ok_or_else(|| format!("任务 {task_id} 的 subtasks 不是数组"))?,
        None => {
            map.insert(key("subtasks"), Value::Sequence(Vec::new()));
            map.get_mut("subtasks")
                .and_then(Value::as_sequence_mut)
                .expect("刚插入的 subtasks 应为数组")
        }
    };

    let subtask = SubtaskView {
        id: ulid::Ulid::generate().to_string(),
        title: title.to_string(),
        status: "open".to_string(),
    };
    let mut item = Mapping::new();
    item.insert(key("id"), Value::String(subtask.id.clone()));
    item.insert(key("title"), Value::String(subtask.title.clone()));
    item.insert(key("status"), Value::String(subtask.status.clone()));
    subtasks.push(Value::Mapping(item));

    write_list(&path, &value)?;
    Ok(subtask)
}

/// 从指定任务移除一条子任务；未知 task / subtask id → 错误。
pub fn remove_subtask_in(
    base: &Path,
    list_file: &str,
    task_id: &str,
    subtask_id: &str,
) -> Result<(), String> {
    let _guard = task_file_write_guard();
    let path = list_path(base, list_file)?;
    let mut value = read_list_value(&path, list_file)?;
    let map = find_task_mut(&mut value, list_file, task_id)?;

    let subtasks = map
        .get_mut("subtasks")
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| format!("任务 {task_id} 没有子任务"))?;
    let index = subtasks
        .iter()
        .position(|s| s.get("id").and_then(Value::as_str) == Some(subtask_id))
        .ok_or_else(|| format!("任务 {task_id} 中未找到子任务 {subtask_id}"))?;
    subtasks.remove(index);

    write_list(&path, &value)
}

/// 切换指定子任务的 `status`（仅 `open` | `done`，其余 → 错误）。
///
/// 未知 task / subtask id → 错误；Value 级补丁 + 原子写，保留未知字段。
pub fn set_subtask_status_in(
    base: &Path,
    list_file: &str,
    task_id: &str,
    subtask_id: &str,
    status: &str,
) -> Result<(), String> {
    let _guard = task_file_write_guard();
    let status = match status {
        "open" | "done" => status,
        other => return Err(format!("非法的子任务状态：{other}")),
    };

    let path = list_path(base, list_file)?;
    let mut value = read_list_value(&path, list_file)?;
    let map = find_task_mut(&mut value, list_file, task_id)?;

    let subtasks = map
        .get_mut("subtasks")
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| format!("任务 {task_id} 没有子任务"))?;
    let subtask = subtasks
        .iter_mut()
        .find(|s| s.get("id").and_then(Value::as_str) == Some(subtask_id))
        .ok_or_else(|| format!("任务 {task_id} 中未找到子任务 {subtask_id}"))?;
    let sm = subtask
        .as_mapping_mut()
        .ok_or_else(|| format!("子任务 {subtask_id} 不是对象"))?;
    sm.insert(key("status"), Value::String(status.to_string()));

    write_list(&path, &value)
}

/// 仅修改指定子任务的 `title`；`trim` 后为空 → 错误。
///
/// 未知 task / subtask id → 错误；Value 级补丁 + 原子写，保留未知字段。
pub fn set_subtask_title_in(
    base: &Path,
    list_file: &str,
    task_id: &str,
    subtask_id: &str,
    title: &str,
) -> Result<(), String> {
    let _guard = task_file_write_guard();
    let title = title.trim();
    if title.is_empty() {
        return Err("子任务标题不能为空".to_string());
    }

    let path = list_path(base, list_file)?;
    let mut value = read_list_value(&path, list_file)?;
    let map = find_task_mut(&mut value, list_file, task_id)?;

    let subtasks = map
        .get_mut("subtasks")
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| format!("任务 {task_id} 没有子任务"))?;
    let subtask = subtasks
        .iter_mut()
        .find(|s| s.get("id").and_then(Value::as_str) == Some(subtask_id))
        .ok_or_else(|| format!("任务 {task_id} 中未找到子任务 {subtask_id}"))?;
    let sm = subtask
        .as_mapping_mut()
        .ok_or_else(|| format!("子任务 {subtask_id} 不是对象"))?;
    sm.insert(key("title"), Value::String(title.to_string()));

    write_list(&path, &value)
}

/// 在同清单内把任务移到 `before_task_id` 之前（`None` → 末尾），仅调整数组顺序。
///
/// 目标为自身或已在目标位置时视为成功且不改动文件；任务或清单不存在 → 错误。
pub fn reorder_task_in(
    base: &Path,
    list_file: &str,
    task_id: &str,
    before_task_id: Option<&str>,
) -> Result<(), String> {
    let _guard = task_file_write_guard();
    let path = list_path(base, list_file)?;
    let mut value = read_list_value(&path, list_file)?;
    let tasks = value
        .get_mut("tasks")
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| format!("清单「{list_file}」缺少 tasks 数组"))?;

    let current = tasks
        .iter()
        .position(|t| t.get("id").and_then(Value::as_str) == Some(task_id))
        .ok_or_else(|| format!("清单「{list_file}」中未找到任务 {task_id}"))?;

    let target = match before_task_id {
        Some(before) => {
            if before == task_id {
                return Ok(());
            }
            tasks
                .iter()
                .position(|t| t.get("id").and_then(Value::as_str) == Some(before))
                .ok_or_else(|| format!("清单「{list_file}」中未找到任务 {before}"))?
        }
        None => tasks.len(),
    };

    // 已在目标位置（或移到自己之前）→ 视为成功且不改动文件
    if target == current || target == current + 1 {
        return Ok(());
    }

    let task = tasks.remove(current);
    let insert_at = if current < target { target - 1 } else { target };
    tasks.insert(insert_at, task);
    write_list(&path, &value)
}

/// 新建清单文件，返回新清单的文件标识（`lists/` 下的文件名）。
///
/// 文件名由清单名规整化（见 [`sanitize_file_stem`]）并对重名追加 `-2`/`-3`…；
/// `order` 取目录内其余清单最大 order + 10。
pub fn create_list_in(base: &Path, name: &str) -> Result<String, String> {
    let _guard = task_file_write_guard();
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
    let _guard = task_file_write_guard();
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
    let _guard = task_file_write_guard();
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
    let _guard = task_file_write_guard();
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

/// 把清单移到 `before_list_file` 之前（`None` → 末尾），沿用整体重编号写回。
///
/// 目标为自身或已在目标位置时视为成功且不改动文件；清单不存在 → 错误。
pub fn reorder_list_in(
    base: &Path,
    list_file: &str,
    before_list_file: Option<&str>,
) -> Result<(), String> {
    let _guard = task_file_write_guard();
    let lists_dir = base.join(LISTS_DIR);
    let mut ordered = read_ordered_lists(&lists_dir)?;
    let current = ordered
        .iter()
        .position(|(file, _, _)| file == list_file)
        .ok_or_else(|| format!("清单文件不存在：{list_file}"))?;

    let target = match before_list_file {
        Some(before) => {
            if before == list_file {
                return Ok(());
            }
            ordered
                .iter()
                .position(|(file, _, _)| file == before)
                .ok_or_else(|| format!("清单文件不存在：{before}"))?
        }
        None => ordered.len(),
    };

    // 已在目标位置（或移到自己之前）→ 视为成功且不改动文件
    if target == current || target == current + 1 {
        return Ok(());
    }

    let item = ordered.remove(current);
    let insert_at = if current < target { target - 1 } else { target };
    ordered.insert(insert_at, item);

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

/// Windows 保留设备名判断（忽略大小写；容忍带扩展名 / 结尾空格的输入）。
///
/// 供 `sanitize_file_stem` 与 `agent::sandbox::guard_path_string` 共用，消除双份实现。
pub(crate) fn is_reserved_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let bytes = stem.as_bytes();
    bytes.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
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
        archived: get_str(m, "archived").filter(|value| !value.trim().is_empty()),
        priority: get_str(m, "priority"),
        due: get_str(m, "due"),
        // 仅字面 `false` = 关；`true` / 缺省 = 默认开（归一化为 None）。
        reminder: matches!(m.get("reminder").and_then(Value::as_bool), Some(false))
            .then_some(false),
        tags,
        notes: get_str(m, "notes"),
        subtasks,
        agent: crate::agent::intent::parse_task_agent(m),
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
pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
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

    fn task_by_id<'a>(value: &'a Value, id: &str) -> &'a Value {
        value
            .get("tasks")
            .and_then(Value::as_sequence)
            .unwrap()
            .iter()
            .find(|task| task.get("id").and_then(Value::as_str) == Some(id))
            .unwrap()
    }

    fn order_of(base: &Path, file: &str) -> i64 {
        read_value(&base.join(LISTS_DIR).join(file))
            .get("order")
            .and_then(Value::as_i64)
            .unwrap()
    }

    #[test]
    fn list_loader_warns_on_invalid_tasks_shape_and_keeps_other_lists() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(base, "missing-tasks.yaml", "name: Missing tasks\n");
        write_list(base, "object-tasks.yaml", "name: Object tasks\ntasks: {}\n");
        write_list(
            base,
            "non-mapping-task.yaml",
            "name: Partial task list\ntasks:\n  - not-a-task-mapping\n  - id: valid-task\n    title: Valid task\n    status: open\n",
        );
        write_list(base, "empty-valid.yaml", "name: Valid empty\ntasks: []\n");

        let loaded = load_lists_from(base).unwrap();
        assert!(loaded
            .warnings
            .iter()
            .any(|warning| warning.contains("missing-tasks.yaml")));
        assert!(loaded
            .warnings
            .iter()
            .any(|warning| warning.contains("object-tasks.yaml")));
        assert!(loaded
            .warnings
            .iter()
            .any(|warning| warning.contains("non-mapping-task.yaml")));
        assert_eq!(loaded.lists.len(), 2);
        let partial = loaded
            .lists
            .iter()
            .find(|list| list.file == "non-mapping-task.yaml")
            .unwrap();
        assert_eq!(partial.tasks.len(), 1);
        assert_eq!(partial.tasks[0].id, "valid-task");
        let valid_empty = loaded
            .lists
            .iter()
            .find(|list| list.file == "empty-valid.yaml")
            .unwrap();
        assert!(valid_empty.tasks.is_empty());
    }

    #[test]
    fn list_loader_warns_and_skips_non_mapping_root_but_loads_valid_neighbor() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(base, "scalar-root.yaml", "not a mapping\n");
        write_list(base, "valid-neighbor.yaml", "name: Neighbor\ntasks: []\n");

        let loaded = load_lists_from(base).unwrap();
        assert_eq!(loaded.lists.len(), 1);
        assert_eq!(loaded.lists[0].file, "valid-neighbor.yaml");
        assert!(loaded
            .warnings
            .iter()
            .any(|warning| warning.contains("scalar-root.yaml")));
    }

    fn metadata_patch(json: &str) -> TaskMetadataPatch {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn metadata_round_trip_uses_snake_disk_and_camel_frontend_keys() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\nx-top: keep\ntasks:\n  - id: T1\n    title: Keep title\n    status: open\n    notes: Keep notes\n    custom: keep task\n  - id: T2\n    title: Other task\n    status: done\n",
        );
        update_task_metadata_in(
            base,
            "L.yaml",
            "T1",
            metadata_patch(
                r#"{"priority":"high","due":"2026-10-13","tags":["a","b"],"reminder":false}"#,
            ),
        )
        .unwrap();

        let loaded = load_lists_from(base).unwrap();
        let task = &loaded.lists[0].tasks[0];
        assert_eq!(task.reminder, Some(false));
        let frontend = serde_json::to_value(task).unwrap();
        assert_eq!(
            frontend.get("reminder").and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert!(frontend.get("reminderOffsetDays").is_none());
        let disk_task = read_value(&path).get("tasks").and_then(Value::as_sequence).unwrap()[0].clone();
        assert_eq!(disk_task.get("reminder").and_then(Value::as_bool), Some(false));
        assert!(disk_task.get("reminderOffsetDays").is_none());
        assert!(disk_task.get("reminder_offset_days").is_none());

        update_task_metadata_in(
            base,
            "L.yaml",
            "T1",
            metadata_patch(r#"{"due":"2026-10-13T09:30"}"#),
        )
        .unwrap();
        assert_eq!(
            load_lists_from(base).unwrap().lists[0].tasks[0].due.as_deref(),
            Some("2026-10-13T09:30")
        );
        update_task_metadata_in(
            base,
            "L.yaml",
            "T1",
            metadata_patch(r#"{"priority":null,"due":null,"tags":[],"reminder":null}"#),
        )
        .unwrap();
        let value = read_value(&path);
        let task = &value.get("tasks").and_then(Value::as_sequence).unwrap()[0];
        for field in ["priority", "due", "tags", "reminder"] {
            assert!(task.get(field).is_none());
        }
        assert_eq!(task.get("title").and_then(Value::as_str), Some("Keep title"));
        assert_eq!(task.get("notes").and_then(Value::as_str), Some("Keep notes"));
        assert_eq!(task.get("custom").and_then(Value::as_str), Some("keep task"));
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        assert_eq!(value.get("tasks").and_then(Value::as_sequence).unwrap()[1].get("title").and_then(Value::as_str), Some("Other task"));
    }

    #[test]
    fn metadata_missing_fields_are_unchanged_and_invalid_patch_does_not_write() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "tasks:\n  - id: T1\n    title: T\n    priority: medium\n    due: '2026-10-13'\n    tags: [old]\n    reminder_offset_days: 1\n",
        );
        update_task_metadata_in(base, "L.yaml", "T1", metadata_patch("{}")).unwrap();
        update_task_metadata_in(base, "L.yaml", "T1", metadata_patch(r#"{"due":null}"#)).unwrap();
        let task = read_value(&path).get("tasks").and_then(Value::as_sequence).unwrap()[0].clone();
        assert_eq!(task.get("priority").and_then(Value::as_str), Some("medium"));
        assert!(task.get("reminder").is_none());
        assert!(task.get("reminder_offset_days").is_none());

        for patch in [
            r#"{"due":"2026-02-30"}"#,
            r#"{"priority":"urgent"}"#,
            r#"{"reminder":true}"#,
        ] {
            let before = fs::read(&path).unwrap();
            assert!(update_task_metadata_in(base, "L.yaml", "T1", metadata_patch(patch)).is_err());
            assert_eq!(fs::read(&path).unwrap(), before);
        }
    }

    #[test]
    fn reminder_patch_writes_omitted_unchanged_and_null_removes() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "tasks:\n  - id: T1\n    title: T\n    due: '2026-10-13'\n",
        );

        // 省略 → 不变（且不产生 reminder 键）
        update_task_metadata_in(base, "L.yaml", "T1", metadata_patch(r#"{"priority":"high"}"#))
            .unwrap();
        let task = read_value(&path).get("tasks").and_then(Value::as_sequence).unwrap()[0].clone();
        assert!(task.get("reminder").is_none());

        // false → 写键
        update_task_metadata_in(base, "L.yaml", "T1", metadata_patch(r#"{"reminder":false}"#))
            .unwrap();
        let task = read_value(&path).get("tasks").and_then(Value::as_sequence).unwrap()[0].clone();
        assert_eq!(task.get("reminder").and_then(Value::as_bool), Some(false));

        // null → 删键
        update_task_metadata_in(base, "L.yaml", "T1", metadata_patch(r#"{"reminder":null}"#))
            .unwrap();
        let task = read_value(&path).get("tasks").and_then(Value::as_sequence).unwrap()[0].clone();
        assert!(task.get("reminder").is_none());
    }

    #[test]
    fn reminder_false_without_due_is_rejected_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(base, "L.yaml", "tasks:\n  - id: T1\n    title: T\n");
        let before = fs::read(&path).unwrap();

        // 无 due 时设置 reminder:false → 报错且不写盘；同补丁清 due 也拒绝。
        assert!(update_task_metadata_in(base, "L.yaml", "T1", metadata_patch(r#"{"reminder":false}"#))
            .is_err());
        assert!(update_task_metadata_in(
            base,
            "L.yaml",
            "T1",
            metadata_patch(r#"{"due":null,"reminder":false}"#)
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn clearing_due_removes_reminder_and_legacy_key_cleaned_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "tasks:\n  - id: T1\n    title: T\n    due: '2026-10-13'\n    reminder: false\n    reminder_offset_days: 1\n",
        );

        // 任意保存（即使只改无关字段）都清理遗留 reminder_offset_days。
        update_task_metadata_in(base, "L.yaml", "T1", metadata_patch(r#"{"priority":"low"}"#))
            .unwrap();
        let task = read_value(&path).get("tasks").and_then(Value::as_sequence).unwrap()[0].clone();
        assert!(task.get("reminder_offset_days").is_none());
        assert_eq!(task.get("reminder").and_then(Value::as_bool), Some(false));

        // 清 due 同时清 reminder 键。
        update_task_metadata_in(base, "L.yaml", "T1", metadata_patch(r#"{"due":null}"#)).unwrap();
        let task = read_value(&path).get("tasks").and_then(Value::as_sequence).unwrap()[0].clone();
        assert!(task.get("due").is_none());
        assert!(task.get("reminder").is_none());
        assert!(task.get("reminder_offset_days").is_none());
    }

    #[test]
    fn task_view_normalizes_reminder_true_and_absent_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(
            base,
            "L.yaml",
            "tasks:\n  - id: off\n    title: off\n    due: '2026-10-13'\n    reminder: false\n  - id: on\n    title: on\n    due: '2026-10-13'\n    reminder: true\n  - id: absent\n    title: absent\n",
        );
        let loaded = load_lists_from(base).unwrap();
        let tasks = &loaded.lists[0].tasks;
        assert_eq!(tasks[0].reminder, Some(false), "字面 false 应为关");
        assert_eq!(tasks[1].reminder, None, "true 应归一化为默认开");
        assert_eq!(tasks[2].reminder, None, "缺失应归一化为默认开");
        let frontend = serde_json::to_value(&tasks[1]).unwrap();
        assert_eq!(frontend.get("reminder"), Some(&serde_json::Value::Null));
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
    fn set_task_status_open_removes_archived() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\ntasks:\n  - id: T1\n    title: T\n    status: done\n    completed: 2026-10-01T09:00\n    archived: 2026-10-02T09:00\n    x-task: keep\n",
        );

        set_task_status_in(base, "L.yaml", "T1", false).unwrap();

        let value = read_value(&path);
        let task = task_by_id(&value, "T1");
        assert_eq!(task.get("status").and_then(Value::as_str), Some("open"));
        assert!(task.get("completed").is_none(), "取消完成应移除 completed");
        assert!(
            task.get("archived").is_none(),
            "未完成任务不应保留 archived（取消完成即取消归档）"
        );
        assert_eq!(task.get("x-task").and_then(Value::as_str), Some("keep"));
    }

    #[test]
    fn archive_tasks_only_done_skips_unknown_and_preserves_existing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\nx-top: keep\ntasks:\n  - id: DONE1\n    title: 已完成\n    status: done\n    completed: 2026-10-01T09:00\n    x-task: keep1\n  - id: DONE2\n    title: 已归档\n    status: done\n    completed: 2026-10-02T09:00\n    archived: 2026-10-03T08:30\n  - id: OPEN1\n    title: 未完成\n    status: open\n  - id: NO_STATUS\n    title: 缺省状态\n  - id: DONE3\n    title: 也完成\n    status: done\n    completed: 2026-10-04T09:00\n",
        );

        let ids = vec![
            "DONE1".to_string(),
            "DONE2".to_string(),
            "OPEN1".to_string(),
            "NO_STATUS".to_string(),
            "MISSING".to_string(),
            "DONE3".to_string(),
        ];
        archive_tasks_in(base, "L.yaml", &ids).unwrap();

        let value = read_value(&path);
        let archived = task_by_id(&value, "DONE1")
            .get("archived")
            .and_then(Value::as_str)
            .expect("done 任务应写入 archived")
            .to_string();
        assert_eq!(archived.len(), 16, "archived 应为 YYYY-MM-DDTHH:MM：{archived}");
        assert!(archived.contains('T'));
        // 其它字段不受影响
        let done1 = task_by_id(&value, "DONE1");
        assert_eq!(done1.get("status").and_then(Value::as_str), Some("done"));
        assert_eq!(
            done1.get("completed").and_then(Value::as_str),
            Some("2026-10-01T09:00")
        );
        assert_eq!(
            done1.get("x-task").and_then(Value::as_str),
            Some("keep1")
        );

        // 已有归档时间：不覆盖
        assert_eq!(
            task_by_id(&value, "DONE2")
                .get("archived")
                .and_then(Value::as_str),
            Some("2026-10-03T08:30")
        );
        // 非 done / 缺省 status / 不存在 id：不写键
        assert!(task_by_id(&value, "OPEN1").get("archived").is_none());
        assert!(task_by_id(&value, "NO_STATUS").get("archived").is_none());
        // 其它 done 任务照常归档
        assert!(task_by_id(&value, "DONE3").get("archived").is_some());

        // 数组顺序与顶层未知字段保留
        let ids_after: Vec<&str> = value
            .get("tasks")
            .and_then(Value::as_sequence)
            .unwrap()
            .iter()
            .filter_map(|task| task.get("id").and_then(Value::as_str))
            .collect();
        assert_eq!(
            ids_after,
            vec!["DONE1", "DONE2", "OPEN1", "NO_STATUS", "DONE3"]
        );
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));

        // 读回视图：archived 解析为字符串，前置前端可用
        let lists = load_lists_from(base).unwrap().lists;
        let view = lists[0]
            .tasks
            .iter()
            .find(|task| task.id == "DONE1")
            .unwrap();
        assert!(view.archived.is_some());
    }

    #[test]
    fn unarchive_tasks_removes_key_and_skips_missing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\nx-top: keep\ntasks:\n  - id: DONE1\n    title: 已归档\n    status: done\n    completed: 2026-10-01T09:00\n    archived: 2026-10-02T09:00\n    x-task: keep1\n  - id: DONE2\n    title: 未归档\n    status: done\n    completed: 2026-10-02T09:00\n",
        );

        let ids = vec![
            "DONE1".to_string(),
            "DONE2".to_string(),
            "MISSING".to_string(),
        ];
        unarchive_tasks_in(base, "L.yaml", &ids).unwrap();

        let value = read_value(&path);
        let done1 = task_by_id(&value, "DONE1");
        assert!(done1.get("archived").is_none(), "应删除 archived");
        assert_eq!(done1.get("status").and_then(Value::as_str), Some("done"));
        assert_eq!(
            done1.get("completed").and_then(Value::as_str),
            Some("2026-10-01T09:00")
        );
        assert_eq!(
            done1.get("x-task").and_then(Value::as_str),
            Some("keep1")
        );
        // 未归档任务与未知 id 静默跳过
        assert_eq!(
            task_by_id(&value, "DONE2").get("status").and_then(Value::as_str),
            Some("done")
        );
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        assert!(load_lists_from(base).unwrap().lists[0].tasks[0]
            .archived
            .is_none());
    }

    #[test]
    fn archive_and_unarchive_error_on_invalid_list_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(base, "L.yaml", "schema: 1\nname: L\ntasks: notaseq\n");
        let before = fs::read(&path).unwrap();

        let ids = vec!["T1".to_string()];
        assert!(archive_tasks_in(base, "L.yaml", &ids).is_err());
        assert!(unarchive_tasks_in(base, "L.yaml", &ids).is_err());
        assert_eq!(fs::read(&path).unwrap(), before, "失败不应写盘");
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
    fn create_task_prepends_and_preserves_unknown_fields() {
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
        // 新任务在首位，title 已 trim
        let task = &tasks[0];
        assert_eq!(task.get("title").and_then(Value::as_str), Some("新任务"));
        assert_eq!(task.get("status").and_then(Value::as_str), Some("open"));
        assert!(task.get("created").and_then(Value::as_str).is_some());
        assert_eq!(
            task.get("id").and_then(Value::as_str).unwrap().len(),
            26,
            "新任务 id 应为 ULID"
        );
        // 原有任务顺序后移且内容不变
        assert_eq!(tasks[1].get("id").and_then(Value::as_str), Some("T1"));
        assert_eq!(tasks[1].get("x-task").and_then(Value::as_str), Some("keep2"));

        // 能被 load 读到
        let result = load_lists_from(base).unwrap();
        assert_eq!(result.lists[0].tasks.len(), 2);
        assert_eq!(result.lists[0].tasks[0].title, "新任务");
        assert_eq!(result.lists[0].tasks[0].status, "open");
        assert_eq!(result.lists[0].tasks[1].id, "T1");

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

    #[test]
    fn reorder_task_moves_within_list_and_preserves_fields() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\nx-top: keep\ntasks:\n  - id: T1\n    title: 一\n    status: open\n    x-task: keep1\n  - id: T2\n    title: 二\n    status: open\n  - id: T3\n    title: 三\n    status: open\n",
        );

        let ids = |path: &Path| -> Vec<String> {
            read_value(path)
                .get("tasks")
                .and_then(Value::as_sequence)
                .unwrap()
                .iter()
                .map(|t| t.get("id").and_then(Value::as_str).unwrap().to_string())
                .collect()
        };

        // 中间插入：T1 移到 T3 之前
        reorder_task_in(base, "L.yaml", "T1", Some("T3")).unwrap();
        assert_eq!(ids(&path), vec!["T2", "T1", "T3"]);
        // 未知字段与既有语义字段保留
        let value = read_value(&path);
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        let t1 = value
            .get("tasks")
            .and_then(Value::as_sequence)
            .unwrap()
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some("T1"))
            .unwrap();
        assert_eq!(t1.get("x-task").and_then(Value::as_str), Some("keep1"));
        assert_eq!(t1.get("status").and_then(Value::as_str), Some("open"));
        assert_eq!(t1.get("title").and_then(Value::as_str), Some("一"));

        // 重复调用幂等
        reorder_task_in(base, "L.yaml", "T1", Some("T3")).unwrap();
        assert_eq!(ids(&path), vec!["T2", "T1", "T3"]);

        // 移到末尾（None）
        reorder_task_in(base, "L.yaml", "T2", None).unwrap();
        assert_eq!(ids(&path), vec!["T1", "T3", "T2"]);
        // 已在末尾再移到末尾 → 无操作
        reorder_task_in(base, "L.yaml", "T2", None).unwrap();
        assert_eq!(ids(&path), vec!["T1", "T3", "T2"]);

        // 目标为自身 / 已在目标位置 → 无操作
        reorder_task_in(base, "L.yaml", "T3", Some("T3")).unwrap();
        reorder_task_in(base, "L.yaml", "T3", Some("T2")).unwrap();
        assert_eq!(ids(&path), vec!["T1", "T3", "T2"]);

        // 任务不存在 / 目标不存在 / 清单不存在 → 错误
        assert!(reorder_task_in(base, "L.yaml", "NOPE", Some("T3")).is_err());
        assert!(reorder_task_in(base, "L.yaml", "T1", Some("NOPE")).is_err());
        assert!(reorder_task_in(base, "nope.yaml", "T1", None).is_err());
    }

    #[test]
    fn reorder_list_moves_and_renumbers() {
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

        // 中间插入：c 移到 a 之前
        reorder_list_in(base, "c.yaml", Some("a.yaml")).unwrap();
        assert_eq!(names(base), vec!["c", "a", "b"]);
        assert_eq!(order_of(base, "c.yaml"), 10);
        assert_eq!(order_of(base, "a.yaml"), 20);
        assert_eq!(order_of(base, "b.yaml"), 30);
        // 未知字段保留
        assert_eq!(
            read_value(&base.join(LISTS_DIR).join("a.yaml"))
                .get("x-keep")
                .and_then(Value::as_i64),
            Some(1)
        );

        // 重复调用幂等
        reorder_list_in(base, "c.yaml", Some("a.yaml")).unwrap();
        assert_eq!(names(base), vec!["c", "a", "b"]);

        // 移到末尾（None）
        reorder_list_in(base, "c.yaml", None).unwrap();
        assert_eq!(names(base), vec!["a", "b", "c"]);
        assert_eq!(order_of(base, "c.yaml"), 30);

        // 目标为自身 / 已在目标位置 → 无操作
        reorder_list_in(base, "a.yaml", Some("a.yaml")).unwrap();
        reorder_list_in(base, "a.yaml", Some("b.yaml")).unwrap();
        assert_eq!(names(base), vec!["a", "b", "c"]);

        // 清单 / 目标不存在 → 错误
        assert!(reorder_list_in(base, "nope.yaml", Some("a.yaml")).is_err());
        assert!(reorder_list_in(base, "a.yaml", Some("nope.yaml")).is_err());
    }

    #[test]
    fn reorder_list_single_list_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        write_list(base, "only.yaml", "schema: 1\nname: only\norder: 10\ntasks: []\n");

        reorder_list_in(base, "only.yaml", None).unwrap();
        reorder_list_in(base, "only.yaml", Some("only.yaml")).unwrap();
        assert_eq!(load_lists_from(base).unwrap().lists[0].name, "only");
        assert_eq!(order_of(base, "only.yaml"), 10);
    }

    #[test]
    fn load_settings_default_when_missing_without_creating_file() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "system");
        assert!(
            !base.join(SETTINGS_FILE).exists(),
            "文件不存在时不应顺手创建 settings.yaml"
        );
    }

    #[test]
    fn save_settings_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        save_settings_in(base, "dark").unwrap();
        assert!(base.join(SETTINGS_FILE).is_file(), "应创建 settings.yaml");
        assert_eq!(load_settings_from(base).unwrap().theme, "dark");

        save_settings_in(base, "light").unwrap();
        assert_eq!(load_settings_from(base).unwrap().theme, "light");

        save_settings_in(base, "system").unwrap();
        assert_eq!(load_settings_from(base).unwrap().theme, "system");
    }

    #[test]
    fn save_settings_preserves_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        fs::write(
            base.join(SETTINGS_FILE),
            "theme: light\nx-custom: 保留我\n",
        )
        .unwrap();

        save_settings_in(base, "dark").unwrap();

        let value = read_value(&base.join(SETTINGS_FILE));
        assert_eq!(value.get("theme").and_then(Value::as_str), Some("dark"));
        assert_eq!(
            value.get("x-custom").and_then(Value::as_str),
            Some("保留我"),
            "未知键应保留"
        );
    }

    #[test]
    fn save_settings_normalizes_invalid_theme() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        save_settings_in(base, "purple").unwrap();
        assert_eq!(load_settings_from(base).unwrap().theme, "system");
        assert_eq!(
            read_value(&base.join(SETTINGS_FILE))
                .get("theme")
                .and_then(Value::as_str),
            Some("system"),
            "非法 theme 应归一化为 system"
        );
    }

    #[test]
    fn load_settings_falls_back_on_corrupt_or_invalid_value() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        // 损坏文件 → 默认
        fs::write(base.join(SETTINGS_FILE), "theme: [未闭合\n").unwrap();
        assert_eq!(load_settings_from(base).unwrap().theme, "system");

        // 合法 YAML 但 theme 值非法 → 默认
        fs::write(base.join(SETTINGS_FILE), "theme: blue\n").unwrap();
        assert_eq!(load_settings_from(base).unwrap().theme, "system");

        // 缺 theme 键 → 默认
        fs::write(base.join(SETTINGS_FILE), "x: 1\n").unwrap();
        assert_eq!(load_settings_from(base).unwrap().theme, "system");
    }

    #[test]
    fn load_settings_strips_leading_bom() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        // 带前导 BOM：theme / app_bar 应正常解析（不被误判为多文档 / 非对象）
        fs::write(
            base.join(SETTINGS_FILE),
            "\u{FEFF}theme: dark\napp_bar:\n  order: [refresh, settings]\n",
        )
        .unwrap();
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "dark");
        assert_eq!(
            settings.app_bar_order,
            Some(vec!["refresh".to_string(), "settings".to_string()])
        );
    }

    /// 取某任务下子任务的 `(id, title, status)` 列表。
    fn subtasks_of(base: &Path, task_id: &str) -> Vec<(String, String, String)> {
        let loaded = load_lists_from(base).unwrap();
        loaded.lists[0]
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .unwrap()
            .subtasks
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|s| (s.id, s.title, s.status))
            .collect()
    }

    #[test]
    fn add_subtask_appends_reads_back_and_preserves_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\nx-top: keep\ntasks:\n  - id: T1\n    title: 一\n    status: open\n    x-task: keep2\n  - id: T2\n    title: 二\n    status: open\n",
        );

        // 加一条并读回；title 已 trim，初始 status=open，id 为 ULID
        let first = add_subtask_in(base, "L.yaml", "T1", "  买菜  ").unwrap();
        assert_eq!(first.title, "买菜");
        assert_eq!(first.status, "open");
        assert_eq!(first.id.len(), 26, "子任务 id 应为 ULID");

        // 连加两条 → append 保序
        let second = add_subtask_in(base, "L.yaml", "T1", "做饭").unwrap();
        let subs = subtasks_of(base, "T1");
        assert_eq!(
            subs,
            vec![
                (first.id.clone(), "买菜".to_string(), "open".to_string()),
                (second.id.clone(), "做饭".to_string(), "open".to_string()),
            ]
        );

        // 未知字段与其它字段保留
        let value = read_value(&path);
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        let tasks = value.get("tasks").and_then(Value::as_sequence).unwrap();
        let t1 = tasks
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some("T1"))
            .unwrap();
        assert_eq!(t1.get("x-task").and_then(Value::as_str), Some("keep2"));
        assert_eq!(t1.get("title").and_then(Value::as_str), Some("一"));
        // 其它任务不被触碰（未新建 subtasks 字段）
        let t2 = tasks
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some("T2"))
            .unwrap();
        assert!(t2.get("subtasks").is_none(), "其它任务不应被写入 subtasks");
    }

    #[test]
    fn add_subtask_rejects_blank_title_and_unknown_task() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n",
        );

        assert!(add_subtask_in(base, "L.yaml", "T1", "").is_err(), "空标题应报错");
        assert!(add_subtask_in(base, "L.yaml", "T1", "   ").is_err(), "纯空白应报错");
        assert!(add_subtask_in(base, "L.yaml", "NOPE", "x").is_err(), "未知 task 应报错");
        // 失败均不写盘
        assert!(
            read_value(&path)
                .get("tasks")
                .and_then(Value::as_sequence)
                .unwrap()[0]
                .get("subtasks")
                .is_none()
        );
    }

    #[test]
    fn add_subtask_rejects_non_array_subtasks() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n    subtasks: 不是数组\n",
        );

        assert!(
            add_subtask_in(base, "L.yaml", "T1", "x").is_err(),
            "subtasks 存在但非数组应报错，不静默覆盖"
        );
        // 原值保留
        let value = read_value(&path);
        let t1 = &value.get("tasks").and_then(Value::as_sequence).unwrap()[0];
        assert_eq!(t1.get("subtasks").and_then(Value::as_str), Some("不是数组"));
    }

    #[test]
    fn remove_subtask_removes_only_target_and_errors_on_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\nx-top: keep\ntasks:\n  - id: T1\n    title: 一\n    status: open\n    x-task: keep2\n    subtasks:\n      - id: S1\n        title: 甲\n        status: open\n      - id: S2\n        title: 乙\n        status: open\n",
        );

        remove_subtask_in(base, "L.yaml", "T1", "S1").unwrap();
        assert_eq!(
            subtasks_of(base, "T1")
                .into_iter()
                .map(|(id, _, _)| id)
                .collect::<Vec<_>>(),
            vec!["S2".to_string()],
            "仅移除目标子任务"
        );

        // 未知 task / subtask → 错误
        assert!(remove_subtask_in(base, "L.yaml", "NOPE", "S2").is_err());
        assert!(remove_subtask_in(base, "L.yaml", "T1", "NOPE").is_err());

        // 未知字段保留
        let value = read_value(&path);
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        let t1 = &value.get("tasks").and_then(Value::as_sequence).unwrap()[0];
        assert_eq!(t1.get("x-task").and_then(Value::as_str), Some("keep2"));
    }

    #[test]
    fn set_subtask_status_validates_and_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n    subtasks:\n      - id: S1\n        title: 甲\n        status: open\n",
        );

        // open → done → open 往返
        set_subtask_status_in(base, "L.yaml", "T1", "S1", "done").unwrap();
        assert_eq!(subtasks_of(base, "T1")[0].2, "done");
        set_subtask_status_in(base, "L.yaml", "T1", "S1", "open").unwrap();
        assert_eq!(subtasks_of(base, "T1")[0].2, "open");

        // 非法 status → 报错且不写盘
        assert!(set_subtask_status_in(base, "L.yaml", "T1", "S1", "doing").is_err());
        assert_eq!(subtasks_of(base, "T1")[0].2, "open");
        // 未知 task / subtask → 错误
        assert!(set_subtask_status_in(base, "L.yaml", "NOPE", "S1", "done").is_err());
        assert!(set_subtask_status_in(base, "L.yaml", "T1", "NOPE", "done").is_err());

        // 未知字段保留
        let value = read_value(&path);
        let t1 = &value.get("tasks").and_then(Value::as_sequence).unwrap()[0];
        assert_eq!(t1.get("title").and_then(Value::as_str), Some("一"));
    }

    #[test]
    fn subtask_commands_work_on_done_task_and_preserve_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\nx-top: keep\ntasks:\n  - id: T1\n    title: 一\n    status: done\n    completed: 2026-10-01T09:00\n    x-task: keep2\n",
        );

        // 对已完成任务增 / 勾 / 删
        let added = add_subtask_in(base, "L.yaml", "T1", "步骤").unwrap();
        set_subtask_status_in(base, "L.yaml", "T1", &added.id, "done").unwrap();
        assert_eq!(subtasks_of(base, "T1")[0].2, "done");
        remove_subtask_in(base, "L.yaml", "T1", &added.id).unwrap();
        assert!(subtasks_of(base, "T1").is_empty());

        // 任务级 / 文件级未知字段与既有字段均保留；任务 status 未被联动
        let value = read_value(&path);
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        let t1 = &value.get("tasks").and_then(Value::as_sequence).unwrap()[0];
        assert_eq!(t1.get("status").and_then(Value::as_str), Some("done"));
        assert_eq!(
            t1.get("completed").and_then(Value::as_str),
            Some("2026-10-01T09:00")
        );
        assert_eq!(t1.get("x-task").and_then(Value::as_str), Some("keep2"));
    }

    #[test]
    fn set_subtask_title_renames_and_preserves_others() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\nx-top: keep\ntasks:\n  - id: T1\n    title: 一\n    status: open\n    x-task: keep2\n    subtasks:\n      - id: S1\n        title: 甲\n        status: open\n        x-sub: keepS\n      - id: S2\n        title: 乙\n        status: done\n",
        );

        // 改名生效且 trim
        set_subtask_title_in(base, "L.yaml", "T1", "S1", "  新甲  ").unwrap();
        let subs = subtasks_of(base, "T1");
        assert_eq!(subs[0], ("S1".to_string(), "新甲".to_string(), "open".to_string()));
        // 同任务其它子任务不受影响（title / status 不变）
        assert_eq!(subs[1], ("S2".to_string(), "乙".to_string(), "done".to_string()));

        // 未知字段与其它字段保留
        let value = read_value(&path);
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        let t1 = &value.get("tasks").and_then(Value::as_sequence).unwrap()[0];
        assert_eq!(t1.get("x-task").and_then(Value::as_str), Some("keep2"));
        let s1 = &t1.get("subtasks").and_then(Value::as_sequence).unwrap()[0];
        assert_eq!(s1.get("x-sub").and_then(Value::as_str), Some("keepS"));
        assert_eq!(s1.get("status").and_then(Value::as_str), Some("open"));
    }

    #[test]
    fn set_subtask_title_rejects_blank_and_unknown_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n    subtasks:\n      - id: S1\n        title: 甲\n        status: open\n",
        );
        let before = fs::read_to_string(&path).unwrap();

        // 空 / 纯空白标题报错且不写盘
        assert!(set_subtask_title_in(base, "L.yaml", "T1", "S1", "").is_err());
        assert!(set_subtask_title_in(base, "L.yaml", "T1", "S1", "   ").is_err());
        // 未知 task / subtask 报错
        assert!(set_subtask_title_in(base, "L.yaml", "NOPE", "S1", "x").is_err());
        assert!(set_subtask_title_in(base, "L.yaml", "T1", "NOPE", "x").is_err());

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before,
            "失败均不应改动文件"
        );
    }

    #[test]
    fn set_subtask_title_works_on_done_task() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = write_list(
            base,
            "L.yaml",
            "schema: 1\nname: L\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: done\n    completed: 2026-10-01T09:00\n    subtasks:\n      - id: S1\n        title: 甲\n        status: open\n",
        );

        set_subtask_title_in(base, "L.yaml", "T1", "S1", "改后").unwrap();
        assert_eq!(subtasks_of(base, "T1")[0].1, "改后");

        // 任务 status / completed 未被联动
        let value = read_value(&path);
        let t1 = &value.get("tasks").and_then(Value::as_sequence).unwrap()[0];
        assert_eq!(t1.get("status").and_then(Value::as_str), Some("done"));
        assert_eq!(
            t1.get("completed").and_then(Value::as_str),
            Some("2026-10-01T09:00")
        );
    }

    #[test]
    fn load_settings_defaults_include_no_app_bar_order() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "system");
        assert!(
            settings.app_bar_order.is_none(),
            "缺失文件时 app_bar_order 应为 None"
        );
        assert!(
            !base.join(SETTINGS_FILE).exists(),
            "读取不应创建 settings.yaml"
        );
    }

    #[test]
    fn load_settings_parses_app_bar_order_deduped_unknown_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        fs::write(
            base.join(SETTINGS_FILE),
            "theme: dark\napp_bar:\n  order: [refresh, refresh, custom1, pin, custom2, pin]\n  x-keep: 1\n",
        )
        .unwrap();

        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "dark");
        assert_eq!(
            settings.app_bar_order,
            Some(vec![
                "refresh".to_string(),
                "custom1".to_string(),
                "pin".to_string(),
                "custom2".to_string(),
            ]),
            "去重取首次、未知 id 原顺序保留、不自动补 settings"
        );
    }

    #[test]
    fn load_settings_app_bar_fallbacks_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let cases = [
            "theme: system\n",                         // app_bar 缺失
            "theme: system\napp_bar:\n  order: []\n",  // 空数组
            "theme: system\napp_bar: notamap\n",       // app_bar 非对象
            "theme: system\napp_bar:\n  order: notaseq\n", // order 非数组
            "theme: system\napp_bar:\n  order: [refresh, 42]\n", // 含非字符串
            "theme: system\napp_bar:\n  order: [refresh, '  ']\n", // 含空白
        ];
        for text in cases {
            fs::write(base.join(SETTINGS_FILE), text).unwrap();
            let settings = load_settings_from(base).unwrap();
            assert_eq!(settings.theme, "system", "case: {text:?}");
            assert!(
                settings.app_bar_order.is_none(),
                "缺失 / 空 / 非法应回退 None：{text:?}"
            );
        }
    }

    #[test]
    fn save_app_bar_order_writes_and_roundtrips_preserving_others() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(
            &path,
            "theme: dark\nx-top: keep\napp_bar:\n  x-custom: keep2\n",
        )
        .unwrap();

        let order = vec![
            "pin".to_string(),
            "custom-x".to_string(),
            "refresh".to_string(),
        ];
        save_app_bar_order_in(base, Some(&order)).unwrap();

        // 读回（含未知 id 原顺序保留）
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.app_bar_order, Some(order.clone()));

        // theme / app_bar 其它字段 / 顶层未知字段保留
        let value = read_value(&path);
        assert_eq!(value.get("theme").and_then(Value::as_str), Some("dark"));
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        let app_bar = value.get("app_bar").and_then(Value::as_mapping).unwrap();
        assert_eq!(app_bar.get("x-custom").and_then(Value::as_str), Some("keep2"));
        assert_eq!(
            app_bar
                .get("order")
                .and_then(Value::as_sequence)
                .map(|seq| seq.len()),
            Some(3)
        );
    }

    #[test]
    fn save_app_bar_order_restore_default_removes_order_only() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(
            &path,
            "theme: light\nx-top: keep\napp_bar:\n  order: [refresh, ghost]\n  x-custom: keep2\n",
        )
        .unwrap();

        // 空数组 → 删除 order（连同未知 id 历史），保留 app_bar 其它字段
        save_app_bar_order_in(base, Some(&[])).unwrap();
        let app_bar = read_value(&path);
        let app_bar = app_bar.get("app_bar").and_then(Value::as_mapping).unwrap();
        assert!(app_bar.get("order").is_none(), "空数组应删除 order");
        assert_eq!(app_bar.get("x-custom").and_then(Value::as_str), Some("keep2"));
        let value = read_value(&path);
        assert_eq!(value.get("theme").and_then(Value::as_str), Some("light"));
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        assert!(load_settings_from(base).unwrap().app_bar_order.is_none());

        // None → 幂等
        save_app_bar_order_in(base, None).unwrap();
        assert!(read_value(&path)
            .get("app_bar")
            .and_then(Value::as_mapping)
            .unwrap()
            .get("order")
            .is_none());
    }

    #[test]
    fn save_app_bar_order_creates_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        assert!(!base.join(SETTINGS_FILE).exists());

        save_app_bar_order_in(base, Some(&["refresh".to_string()])).unwrap();

        assert!(base.join(SETTINGS_FILE).is_file(), "应创建 settings.yaml");
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "system");
        assert_eq!(settings.app_bar_order, Some(vec!["refresh".to_string()]));
    }

    #[test]
    fn save_app_bar_order_rejects_blank_id_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(&path, "theme: dark\n").unwrap();
        let before = fs::read_to_string(&path).unwrap();

        assert!(
            save_app_bar_order_in(base, Some(&["refresh".to_string(), "   ".to_string()])).is_err(),
            "空白 id 应报错"
        );
        assert!(save_app_bar_order_in(base, Some(&["".to_string()])).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before, "失败不应写盘");
    }

    #[test]
    fn save_app_bar_order_rejects_malformed_app_bar() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(&path, "theme: dark\napp_bar: 未知标量\n").unwrap();
        let before = fs::read_to_string(&path).unwrap();

        assert!(
            save_app_bar_order_in(base, Some(&["refresh".to_string()])).is_err(),
            "app_bar 非对象应报错"
        );
        assert!(
            save_app_bar_order_in(base, None).is_err(),
            "恢复默认也不应覆盖未知标量"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), before, "不应覆盖未知标量");
    }

    #[test]
    fn save_app_bar_order_errors_on_non_mapping_root() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(&path, "- not\n- a map\n").unwrap();
        let before = fs::read_to_string(&path).unwrap();

        assert!(save_app_bar_order_in(base, Some(&["refresh".to_string()])).is_err());
        assert!(save_app_bar_order_in(base, None).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before, "保存失败不应写盘");
    }

    #[test]
    fn theme_and_app_bar_order_do_not_clobber_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        // 先写顺序，再改主题 → 顺序保留
        save_app_bar_order_in(base, Some(&["pin".to_string()])).unwrap();
        save_settings_in(base, "dark").unwrap();
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.app_bar_order, Some(vec!["pin".to_string()]));

        // 先改主题，再写顺序 → 主题保留
        save_settings_in(base, "light").unwrap();
        save_app_bar_order_in(base, Some(&["refresh".to_string()])).unwrap();
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "light");
        assert_eq!(settings.app_bar_order, Some(vec!["refresh".to_string()]));
    }

    /// 读取某个清单文件 `tasks` 数组中的任务 id 序列。
    fn task_ids(path: &Path) -> Vec<String> {
        read_value(path)
            .get("tasks")
            .and_then(Value::as_sequence)
            .unwrap()
            .iter()
            .map(|t| t.get("id").and_then(Value::as_str).unwrap().to_string())
            .collect()
    }

    #[test]
    fn move_task_appends_to_target_end_and_removes_from_source() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let from = write_list(
            base,
            "A.yaml",
            "schema: 1\nname: A\norder: 1\nx-top: keepA\ntasks:\n  - id: T1\n    title: 一\n    status: open\n    x-task: keep1\n    subtasks:\n      - id: S1\n        title: 甲\n        status: open\n  - id: T2\n    title: 二\n    status: open\n  - id: T3\n    title: 三\n    status: done\n    completed: 2026-10-01T09:00\n",
        );
        let to = write_list(
            base,
            "B.yaml",
            "schema: 1\nname: B\norder: 2\nx-top: keepB\ntasks:\n  - id: T9\n    title: 九\n    status: open\n",
        );

        move_task_in(base, "A.yaml", "T1", "B.yaml").unwrap();

        // 源：移除 T1，其余任务顺序不变，顶层未知字段保留
        assert_eq!(task_ids(&from), vec!["T2", "T3"]);
        assert_eq!(
            read_value(&from).get("x-top").and_then(Value::as_str),
            Some("keepA")
        );

        // 目标：原有任务在前，移入的任务追加到末尾
        assert_eq!(task_ids(&to), vec!["T9", "T1"]);
        let to_value = read_value(&to);
        let moved = &to_value.get("tasks").and_then(Value::as_sequence).unwrap()[1];
        assert_eq!(moved.get("id").and_then(Value::as_str), Some("T1"));
        assert_eq!(moved.get("title").and_then(Value::as_str), Some("一"));
        assert_eq!(moved.get("status").and_then(Value::as_str), Some("open"));
        assert_eq!(moved.get("x-task").and_then(Value::as_str), Some("keep1"));
        let subs = moved.get("subtasks").and_then(Value::as_sequence).unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].get("id").and_then(Value::as_str), Some("S1"));
        assert_eq!(to_value.get("x-top").and_then(Value::as_str), Some("keepB"));
    }

    #[test]
    fn move_task_rejects_missing_source_task_same_list_and_missing_target() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let from = write_list(
            base,
            "A.yaml",
            "schema: 1\nname: A\norder: 1\ntasks:\n  - id: T1\n    title: 一\n    status: open\n",
        );
        let to = write_list(
            base,
            "B.yaml",
            "schema: 1\nname: B\norder: 2\ntasks:\n  - id: T9\n    title: 九\n    status: open\n",
        );
        let from_before = fs::read(&from).unwrap();
        let to_before = fs::read(&to).unwrap();

        assert!(
            move_task_in(base, "A.yaml", "NOPE", "B.yaml").is_err(),
            "源清单无此 task 应报错"
        );
        assert!(
            move_task_in(base, "A.yaml", "T1", "A.yaml").is_err(),
            "源 = 目标应报错"
        );
        assert!(
            move_task_in(base, "A.yaml", "T1", "missing.yaml").is_err(),
            "目标文件不存在应报错"
        );

        // 失败均不写盘
        assert_eq!(fs::read(&from).unwrap(), from_before);
        assert_eq!(fs::read(&to).unwrap(), to_before);
    }

    #[test]
    fn move_task_rejects_duplicate_id_in_target_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let from = write_list(
            base,
            "A.yaml",
            "schema: 1\nname: A\norder: 1\ntasks:\n  - id: T1\n    title: 源一\n    status: open\n  - id: T2\n    title: 源二\n    status: open\n",
        );
        let to = write_list(
            base,
            "B.yaml",
            "schema: 1\nname: B\norder: 2\ntasks:\n  - id: T1\n    title: 目标已有一\n    status: open\n",
        );
        let from_before = fs::read(&from).unwrap();
        let to_before = fs::read(&to).unwrap();

        assert!(
            move_task_in(base, "A.yaml", "T1", "B.yaml").is_err(),
            "目标已存在同 id 应报错"
        );

        // 两个文件都未被修改
        assert_eq!(fs::read(&from).unwrap(), from_before);
        assert_eq!(fs::read(&to).unwrap(), to_before);
        assert_eq!(task_ids(&from), vec!["T1", "T2"]);
        assert_eq!(task_ids(&to), vec!["T1"]);
    }

    #[test]
    fn move_task_round_trip_preserves_content() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let from = write_list(
            base,
            "A.yaml",
            "schema: 1\nname: A\norder: 1\nx-top: keepA\ntasks:\n  - id: T1\n    title: 一\n    status: open\n    x-task: keep1\n    subtasks:\n      - id: S1\n        title: 甲\n        status: open\n  - id: T2\n    title: 二\n    status: open\n",
        );
        let to = write_list(
            base,
            "B.yaml",
            "schema: 1\nname: B\norder: 2\ntasks:\n  - id: T9\n    title: 九\n    status: open\n",
        );

        // A → B
        move_task_in(base, "A.yaml", "T1", "B.yaml").unwrap();
        assert_eq!(task_ids(&from), vec!["T2"]);
        assert_eq!(task_ids(&to), vec!["T9", "T1"]);

        // B → A（移回）
        move_task_in(base, "B.yaml", "T1", "A.yaml").unwrap();
        assert_eq!(task_ids(&from), vec!["T2", "T1"], "移回后追加在源清单末尾");
        assert_eq!(task_ids(&to), vec!["T9"], "目标恢复为原状");

        // 内容完整：id、未知字段、子任务均保留
        let value = read_value(&from);
        let moved = value
            .get("tasks")
            .and_then(Value::as_sequence)
            .unwrap()
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some("T1"))
            .unwrap()
            .clone();
        assert_eq!(moved.get("title").and_then(Value::as_str), Some("一"));
        assert_eq!(moved.get("status").and_then(Value::as_str), Some("open"));
        assert_eq!(moved.get("x-task").and_then(Value::as_str), Some("keep1"));
        let subtask = &moved.get("subtasks").and_then(Value::as_sequence).unwrap()[0];
        assert_eq!(subtask.get("id").and_then(Value::as_str), Some("S1"));
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keepA"));
    }

    #[test]
    fn load_settings_defaults_include_no_completed_collapsed() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "system");
        assert!(
            settings.completed_collapsed.is_none(),
            "缺失文件时 completed_collapsed 应为 None"
        );
        assert!(
            !base.join(SETTINGS_FILE).exists(),
            "读取不应创建 settings.yaml"
        );
    }

    #[test]
    fn load_settings_parses_completed_collapsed_deduped_unknown_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        fs::write(
            base.join(SETTINGS_FILE),
            "theme: dark\ncompleted:\n  collapsed: [工作.yaml, 工作.yaml, 家务.yaml, 工作.yaml]\n  x-keep: 1\n",
        )
        .unwrap();

        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "dark");
        assert_eq!(
            settings.completed_collapsed,
            Some(vec!["工作.yaml".to_string(), "家务.yaml".to_string()]),
            "去重取首次、保序"
        );
    }

    #[test]
    fn load_settings_completed_collapsed_fallbacks_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let cases = [
            "theme: system\n",                                             // completed 缺失
            "theme: system\ncompleted:\n  collapsed: []\n",                // 空数组
            "theme: system\ncompleted: notamap\n",                         // completed 非对象
            "theme: system\ncompleted:\n  collapsed: notaseq\n",           // collapsed 非数组
            "theme: system\ncompleted:\n  collapsed: [工作.yaml, 42]\n",   // 含非字符串
            "theme: system\ncompleted:\n  collapsed: [工作.yaml, '  ']\n", // 含空白
        ];
        for text in cases {
            fs::write(base.join(SETTINGS_FILE), text).unwrap();
            let settings = load_settings_from(base).unwrap();
            assert_eq!(settings.theme, "system", "case: {text:?}");
            assert!(
                settings.completed_collapsed.is_none(),
                "缺失 / 空 / 非法应回退 None：{text:?}"
            );
        }
    }

    #[test]
    fn save_completed_collapsed_writes_and_roundtrips_preserving_others() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(
            &path,
            "theme: dark\nx-top: keep\ncompleted:\n  x-custom: keep2\n",
        )
        .unwrap();

        let collapsed = vec!["工作.yaml".to_string(), "家务.yaml".to_string()];
        save_completed_collapsed_in(base, Some(&collapsed)).unwrap();

        // 读回
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.completed_collapsed, Some(collapsed.clone()));

        // theme / completed 其它字段 / 顶层未知字段保留
        let value = read_value(&path);
        assert_eq!(value.get("theme").and_then(Value::as_str), Some("dark"));
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        let completed = value.get("completed").and_then(Value::as_mapping).unwrap();
        assert_eq!(
            completed.get("x-custom").and_then(Value::as_str),
            Some("keep2")
        );
        assert_eq!(
            completed
                .get("collapsed")
                .and_then(Value::as_sequence)
                .map(|seq| seq.len()),
            Some(2)
        );
    }

    #[test]
    fn save_completed_collapsed_restore_default_removes_collapsed_only() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(
            &path,
            "theme: light\nx-top: keep\ncompleted:\n  collapsed: [工作.yaml, 已删除.yaml]\n  x-custom: keep2\n",
        )
        .unwrap();

        // 空数组 → 删除 collapsed（连同失效历史），保留 completed 其它字段
        save_completed_collapsed_in(base, Some(&[])).unwrap();
        let value = read_value(&path);
        let completed = value.get("completed").and_then(Value::as_mapping).unwrap();
        assert!(
            completed.get("collapsed").is_none(),
            "空数组应删除 collapsed"
        );
        assert_eq!(
            completed.get("x-custom").and_then(Value::as_str),
            Some("keep2")
        );
        assert_eq!(value.get("theme").and_then(Value::as_str), Some("light"));
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        assert!(load_settings_from(base)
            .unwrap()
            .completed_collapsed
            .is_none());

        // None → 幂等
        save_completed_collapsed_in(base, None).unwrap();
        assert!(read_value(&path)
            .get("completed")
            .and_then(Value::as_mapping)
            .unwrap()
            .get("collapsed")
            .is_none());
    }

    #[test]
    fn save_completed_collapsed_creates_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        assert!(!base.join(SETTINGS_FILE).exists());

        save_completed_collapsed_in(base, Some(&["工作.yaml".to_string()])).unwrap();

        assert!(base.join(SETTINGS_FILE).is_file(), "应创建 settings.yaml");
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "system");
        assert_eq!(
            settings.completed_collapsed,
            Some(vec!["工作.yaml".to_string()])
        );
    }

    #[test]
    fn save_completed_collapsed_rejects_blank_file_name_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(&path, "theme: dark\n").unwrap();
        let before = fs::read_to_string(&path).unwrap();

        assert!(
            save_completed_collapsed_in(base, Some(&["工作.yaml".to_string(), "   ".to_string()]))
                .is_err(),
            "空白文件名应报错"
        );
        assert!(save_completed_collapsed_in(base, Some(&["".to_string()])).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before, "失败不应写盘");
    }

    #[test]
    fn save_completed_collapsed_rejects_malformed_completed() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(&path, "theme: dark\ncompleted: 未知标量\n").unwrap();
        let before = fs::read_to_string(&path).unwrap();

        assert!(
            save_completed_collapsed_in(base, Some(&["工作.yaml".to_string()])).is_err(),
            "completed 非对象应报错"
        );
        assert!(
            save_completed_collapsed_in(base, None).is_err(),
            "恢复默认也不应覆盖未知标量"
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before,
            "不应覆盖未知标量"
        );
    }

    #[test]
    fn save_completed_collapsed_errors_on_non_mapping_root() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(&path, "- not\n- a map\n").unwrap();
        let before = fs::read_to_string(&path).unwrap();

        assert!(save_completed_collapsed_in(base, Some(&["工作.yaml".to_string()])).is_err());
        assert!(save_completed_collapsed_in(base, None).is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before,
            "保存失败不应写盘"
        );
    }

    #[test]
    fn load_settings_completed_show_archived_defaults_false() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        // 缺失文件
        assert!(
            !load_settings_from(base).unwrap().completed_show_archived,
            "缺失文件时默认关闭"
        );

        let cases = [
            "theme: dark\n",                                            // completed 缺失
            "theme: dark\ncompleted:\n  collapsed: [工作.yaml]\n",      // show_archived 缺失
            "theme: dark\ncompleted:\n  show_archived: maybe\n",        // 非布尔
            "theme: dark\ncompleted:\n  show_archived: 1\n",            // 整数
            "theme: dark\ncompleted:\n  show_archived: null\n",         // null
            "theme: dark\ncompleted: notamap\n",                        // completed 非对象
        ];
        for text in cases {
            fs::write(base.join(SETTINGS_FILE), text).unwrap();
            assert!(
                !load_settings_from(base).unwrap().completed_show_archived,
                "缺失 / 非布尔应回退 false：{text:?}"
            );
        }
    }

    #[test]
    fn save_completed_show_archived_roundtrips_true_and_false_preserving_others() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(
            &path,
            "theme: dark\nx-top: keep\ncompleted:\n  collapsed: [工作.yaml, 家务.yaml]\n  x-custom: keep2\n",
        )
        .unwrap();

        // true → 写 true，保留 collapsed 与未知字段
        save_completed_show_archived_in(base, true).unwrap();
        let settings = load_settings_from(base).unwrap();
        assert!(settings.completed_show_archived);
        assert_eq!(
            settings.completed_collapsed,
            Some(vec!["工作.yaml".to_string(), "家务.yaml".to_string()])
        );

        let value = read_value(&path);
        assert_eq!(value.get("theme").and_then(Value::as_str), Some("dark"));
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        let completed = value.get("completed").and_then(Value::as_mapping).unwrap();
        assert_eq!(
            completed.get("show_archived").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            completed.get("x-custom").and_then(Value::as_str),
            Some("keep2")
        );
        assert_eq!(
            completed
                .get("collapsed")
                .and_then(Value::as_sequence)
                .map(|seq| seq.len()),
            Some(2)
        );

        // false → 删除键，collapsed / 未知字段仍保留
        save_completed_show_archived_in(base, false).unwrap();
        let value = read_value(&path);
        let completed = value.get("completed").and_then(Value::as_mapping).unwrap();
        assert!(
            completed.get("show_archived").is_none(),
            "false 应删除 completed.show_archived"
        );
        assert_eq!(
            completed.get("x-custom").and_then(Value::as_str),
            Some("keep2")
        );
        assert_eq!(
            completed
                .get("collapsed")
                .and_then(Value::as_sequence)
                .map(|seq| seq.len()),
            Some(2)
        );
        assert_eq!(value.get("theme").and_then(Value::as_str), Some("dark"));
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        assert!(!load_settings_from(base).unwrap().completed_show_archived);
    }

    #[test]
    fn save_completed_show_archived_creates_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        assert!(!base.join(SETTINGS_FILE).exists());

        save_completed_show_archived_in(base, true).unwrap();

        assert!(base.join(SETTINGS_FILE).is_file(), "应创建 settings.yaml");
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "system");
        assert!(settings.completed_show_archived);

        // false 在没有 completed 时也不应报错
        save_completed_show_archived_in(base, false).unwrap();
        assert!(!load_settings_from(base).unwrap().completed_show_archived);
    }

    #[test]
    fn save_completed_show_archived_rejects_malformed_completed_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(&path, "theme: dark\ncompleted: 未知标量\n").unwrap();
        let before = fs::read_to_string(&path).unwrap();

        assert!(
            save_completed_show_archived_in(base, true).is_err(),
            "completed 非对象应报错"
        );
        assert!(
            save_completed_show_archived_in(base, false).is_err(),
            "关闭也不应覆盖未知标量"
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before,
            "失败不应写盘"
        );
    }

    #[test]
    fn save_completed_show_archived_errors_on_non_mapping_root() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(&path, "- not\n- a map\n").unwrap();
        let before = fs::read_to_string(&path).unwrap();

        assert!(save_completed_show_archived_in(base, true).is_err());
        assert!(save_completed_show_archived_in(base, false).is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            before,
            "保存失败不应写盘"
        );
    }

    #[test]
    fn theme_app_bar_and_completed_do_not_clobber_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        // 写入折叠集合，再改主题 / 顺序 → 折叠集合保留
        save_completed_collapsed_in(base, Some(&["工作.yaml".to_string()])).unwrap();
        save_settings_in(base, "dark").unwrap();
        save_app_bar_order_in(base, Some(&["pin".to_string()])).unwrap();
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.app_bar_order, Some(vec!["pin".to_string()]));
        assert_eq!(
            settings.completed_collapsed,
            Some(vec!["工作.yaml".to_string()])
        );

        // 改主题 / 顺序后折叠集合仍在
        save_settings_in(base, "light").unwrap();
        save_app_bar_order_in(base, Some(&["refresh".to_string()])).unwrap();
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "light");
        assert_eq!(settings.app_bar_order, Some(vec!["refresh".to_string()]));
        assert_eq!(
            settings.completed_collapsed,
            Some(vec!["工作.yaml".to_string()])
        );

        // 清除折叠集合不影响 theme / app_bar
        save_completed_collapsed_in(base, None).unwrap();
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "light");
        assert_eq!(settings.app_bar_order, Some(vec!["refresh".to_string()]));
        assert!(settings.completed_collapsed.is_none());
    }

    #[test]
    fn load_settings_parses_detail_collapsed_and_distinguishes_empty_array() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);

        // 缺 key / 非法 → None
        let invalid = [
            "theme: system\n",
            "theme: system\ndetail: notamap\n",
            "theme: system\ndetail:\n  collapsed: notaseq\n",
            "theme: system\ndetail:\n  collapsed: [metadata, 42]\n",
            "theme: system\ndetail:\n  collapsed: [metadata, '  ']\n",
        ];
        for text in invalid {
            fs::write(&path, text).unwrap();
            assert!(
                load_settings_from(base).unwrap().detail_collapsed.is_none(),
                "缺失 / 非法应回退 None：{text:?}"
            );
        }

        // 去重取首次、保序
        fs::write(
            &path,
            "theme: system\ndetail:\n  collapsed: [metadata, metadata, notes]\n  x-keep: 1\n",
        )
        .unwrap();
        assert_eq!(
            load_settings_from(base).unwrap().detail_collapsed,
            Some(vec!["metadata".to_string(), "notes".to_string()])
        );

        // 显式空数组是有效值，不能折叠成 None
        fs::write(&path, "theme: system\ndetail:\n  collapsed: []\n").unwrap();
        assert_eq!(
            load_settings_from(base).unwrap().detail_collapsed,
            Some(Vec::new()),
            "显式空数组应读回 Some(vec![])"
        );
    }

    #[test]
    fn save_detail_collapsed_writes_and_roundtrips_preserving_others() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(
            &path,
            "theme: dark\nx-top: keep\ncompleted:\n  collapsed: [工作.yaml]\n  x-custom: keep2\ndetail:\n  x-detail: keep3\n",
        )
        .unwrap();

        let collapsed = vec!["metadata".to_string(), "notes".to_string()];
        save_detail_collapsed_in(base, Some(&collapsed)).unwrap();

        // 读回
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.detail_collapsed, Some(collapsed.clone()));

        // theme / completed / detail 其它字段 / 顶层未知字段保留
        let value = read_value(&path);
        assert_eq!(value.get("theme").and_then(Value::as_str), Some("dark"));
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        let completed = value.get("completed").and_then(Value::as_mapping).unwrap();
        assert_eq!(
            completed.get("x-custom").and_then(Value::as_str),
            Some("keep2")
        );
        assert_eq!(
            completed
                .get("collapsed")
                .and_then(Value::as_sequence)
                .map(|seq| seq.len()),
            Some(1),
            "completed.collapsed 不应被破坏"
        );
        let detail = value.get("detail").and_then(Value::as_mapping).unwrap();
        assert_eq!(
            detail.get("x-detail").and_then(Value::as_str),
            Some("keep3")
        );
        assert_eq!(
            detail
                .get("collapsed")
                .and_then(Value::as_sequence)
                .map(|seq| seq.iter().filter_map(Value::as_str).collect::<Vec<_>>()),
            Some(vec!["metadata", "notes"])
        );
    }

    #[test]
    fn save_detail_collapsed_writes_empty_array_keeping_key() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(&path, "theme: light\ndetail:\n  collapsed: [metadata]\n").unwrap();

        save_detail_collapsed_in(base, Some(&[])).unwrap();

        // 空数组必须落盘且保留 key（与 completed.collapsed 的差异）
        let value = read_value(&path);
        let detail = value.get("detail").and_then(Value::as_mapping).unwrap();
        assert!(
            detail.get("collapsed").is_some(),
            "空数组应保留 collapsed key"
        );
        assert_eq!(
            detail
                .get("collapsed")
                .and_then(Value::as_sequence)
                .map(Vec::len),
            Some(0)
        );
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.detail_collapsed, Some(Vec::new()));
    }

    #[test]
    fn save_detail_collapsed_none_removes_key_and_empty_detail() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(&path, "theme: light\ndetail:\n  collapsed: [metadata]\n").unwrap();

        save_detail_collapsed_in(base, None).unwrap();
        let value = read_value(&path);
        assert!(
            value.get("detail").is_none(),
            "detail 因此变空时应一并删除"
        );
        assert!(load_settings_from(base).unwrap().detail_collapsed.is_none());

        // detail 仍有其它字段时只删 collapsed
        fs::write(
            &path,
            "theme: light\ndetail:\n  collapsed: [notes]\n  x-keep: 1\n",
        )
        .unwrap();
        save_detail_collapsed_in(base, None).unwrap();
        let value = read_value(&path);
        let detail = value.get("detail").and_then(Value::as_mapping).unwrap();
        assert!(detail.get("collapsed").is_none());
        assert_eq!(detail.get("x-keep").and_then(Value::as_i64), Some(1));
        assert!(load_settings_from(base).unwrap().detail_collapsed.is_none());
    }

    #[test]
    fn save_detail_collapsed_rejects_blank_key_and_malformed_detail() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);

        // 空白 key → 报错且不写盘
        fs::write(&path, "theme: dark\n").unwrap();
        let before = fs::read_to_string(&path).unwrap();
        assert!(save_detail_collapsed_in(base, Some(&["metadata".to_string(), "  ".to_string()]))
            .is_err());
        assert!(save_detail_collapsed_in(base, Some(&["".to_string()])).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before, "失败不应写盘");

        // detail 非对象 → 写入与删除都报错、不覆盖未知标量
        fs::write(&path, "theme: dark\ndetail: 未知标量\n").unwrap();
        let before = fs::read_to_string(&path).unwrap();
        assert!(save_detail_collapsed_in(base, Some(&["metadata".to_string()])).is_err());
        assert!(save_detail_collapsed_in(base, None).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn save_detail_collapsed_creates_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        assert!(!base.join(SETTINGS_FILE).exists());

        save_detail_collapsed_in(base, Some(&["metadata".to_string()])).unwrap();

        assert!(base.join(SETTINGS_FILE).is_file(), "应创建 settings.yaml");
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "system");
        assert_eq!(
            settings.detail_collapsed,
            Some(vec!["metadata".to_string()])
        );
    }

    #[test]
    fn load_settings_default_minimize_to_tray_is_none_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        let settings = load_settings_from(base).unwrap();
        assert!(
            settings.minimize_to_tray.is_none(),
            "缺失文件时 minimize_to_tray 应为 None（默认开启）"
        );
        assert!(
            !base.join(SETTINGS_FILE).exists(),
            "读取不应创建 settings.yaml"
        );

        // 缺失键 / 非布尔值 → None
        for text in ["theme: system\n", "theme: system\nminimize_to_tray: maybe\n"] {
            fs::write(base.join(SETTINGS_FILE), text).unwrap();
            assert!(
                load_settings_from(base).unwrap().minimize_to_tray.is_none(),
                "缺失 / 非法应回退 None：{text:?}"
            );
        }
    }

    #[test]
    fn save_minimize_to_tray_false_roundtrips_preserving_others() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(&path, "theme: dark\nx-top: keep\napp_bar:\n  order: [pin]\n").unwrap();

        save_minimize_to_tray_in(base, false).unwrap();

        // 读回 false
        assert_eq!(load_settings_from(base).unwrap().minimize_to_tray, Some(false));
        let value = read_value(&path);
        assert_eq!(
            value.get("minimize_to_tray").and_then(Value::as_bool),
            Some(false)
        );
        // 其它设置与未知字段保留
        assert_eq!(value.get("theme").and_then(Value::as_str), Some("dark"));
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        assert_eq!(
            value
                .get("app_bar")
                .and_then(|app_bar| app_bar.get("order"))
                .and_then(Value::as_sequence)
                .map(|seq| seq.len()),
            Some(1)
        );
    }

    #[test]
    fn save_minimize_to_tray_true_removes_key_and_preserves_others() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let path = base.join(SETTINGS_FILE);
        fs::write(
            &path,
            "theme: light\nx-top: keep\nminimize_to_tray: false\ncompleted:\n  x-custom: keep2\n",
        )
        .unwrap();

        save_minimize_to_tray_in(base, true).unwrap();

        let value = read_value(&path);
        assert!(
            value.get("minimize_to_tray").is_none(),
            "开启应删除键以恢复默认"
        );
        assert_eq!(value.get("theme").and_then(Value::as_str), Some("light"));
        assert_eq!(value.get("x-top").and_then(Value::as_str), Some("keep"));
        assert_eq!(
            value
                .get("completed")
                .and_then(|completed| completed.get("x-custom"))
                .and_then(Value::as_str),
            Some("keep2")
        );
        assert!(
            load_settings_from(base).unwrap().minimize_to_tray.is_none(),
            "删键后读回 None（默认开启）"
        );
    }

    #[test]
    fn save_minimize_to_tray_creates_file_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        assert!(!base.join(SETTINGS_FILE).exists());

        save_minimize_to_tray_in(base, false).unwrap();

        assert!(base.join(SETTINGS_FILE).is_file(), "应创建 settings.yaml");
        let settings = load_settings_from(base).unwrap();
        assert_eq!(settings.theme, "system");
        assert_eq!(settings.minimize_to_tray, Some(false));
    }
}
