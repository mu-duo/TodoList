//! 只读沙箱：路径字符串护栏 + **open-first** 句柄解析 + 读取分页 / 预算 + 结果根校验。
//!
//! 原则：绝不 `canonicalize(path)` 后再 open；先按字符串快速拒绝危险形状，再 `CreateFileW`
//! 打开（目录带 `FILE_FLAG_BACKUP_SEMANTICS`），用 `GetFinalPathNameByHandleW` 取**最终路径**。
//! 文件读取经 [`PathGuard`] 持有的**同一句柄**完成（[`PathGuard::read_page`]），调用方拿不到
//! 裸路径再重开，TOCTOU 窗口收敛为单次 open。非 Windows 平台缺少等价句柄能力，fail closed。

use std::path::{Path, PathBuf};

/// 单次文本读取分页上限：256KiB。
#[allow(dead_code)] // P2 runner：按页从句柄读取。
pub const FILE_PAGE_LIMIT: usize = 256 * 1024;
/// 目录单页条目上限。
#[allow(dead_code)] // P2 runner：目录分页。
pub const DIR_PAGE_LIMIT: usize = 200;
/// 单次运行累计读取上限：5MiB。
#[allow(dead_code)] // P2 runner：累计读取预算。
pub const READ_BUDGET_LIMIT: u64 = 5 * 1024 * 1024;

/// 路径字符串护栏：拒绝 UNC / 设备路径 / 相对路径 / ADS / 结尾点空格 / 保留名 / 敏感项 / `..`。
pub fn guard_path_string(path: &str) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("路径不能为空".to_string());
    }
    if path.trim_end() != path {
        return Err("路径不能以空白结尾".to_string());
    }
    let p = path;
    if p.starts_with("\\\\") || p.starts_with("//") {
        return Err("拒绝 UNC / 设备路径".to_string());
    }

    let parsed = Path::new(p);
    if !parsed.is_absolute() {
        return Err("路径必须为绝对路径".to_string());
    }

    // ADS：盘符后的额外 `:`（首个 `:` 仅允许出现在索引 1 且后跟分隔符）
    if let Some(pos) = p.find(':') {
        let bytes = p.as_bytes();
        let drive_ok = pos == 1
            && bytes.len() > 2
            && (bytes[2] == b'\\' || bytes[2] == b'/');
        if !drive_ok || p[pos + 1..].contains(':') {
            return Err("拒绝备用数据流(ADS)路径".to_string());
        }
    }

    for comp in parsed.components() {
        match comp {
            std::path::Component::ParentDir => {
                return Err("拒绝包含 .. 的路径".to_string());
            }
            std::path::Component::Normal(os) => {
                let name = os.to_string_lossy();
                if name.ends_with('.') || name.ends_with(' ') {
                    return Err(format!("路径组件不能以点或空格结尾：{name}"));
                }
                if is_reserved_name(&name) {
                    return Err(format!("拒绝保留设备名：{name}"));
                }
                if is_sensitive_component(&name) {
                    return Err(format!("拒绝敏感路径：{name}"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// 源授权根语义护栏：字符串护栏 + 拒绝盘符根。
///
/// 供 `intent` 存储前校验使用；**P2 runner 在启动处理每个已存根时应再次调用本函数**
/// （外部 YAML 可绕过命令直接写 `agent.enabled: true` + 危险根）。
pub fn guard_source_root(path: &str) -> Result<(), String> {
    guard_path_string(path)?;
    if Path::new(path).parent().is_none() {
        return Err("不能使用盘符根目录作为授权根".to_string());
    }
    Ok(())
}

/// Windows 保留设备名（忽略大小写与扩展名；实现与 `storage::is_reserved_name` 共用一份）。
pub fn is_reserved_name(name: &str) -> bool {
    crate::storage::is_reserved_name(name)
}

/// 敏感路径组件（默认阻止）：`.git` / `.ssh` / `.env*` / 密钥与凭据文件等。
pub fn is_sensitive_component(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        ".git" | ".ssh" | ".gnupg" | ".aws" | ".netrc" | ".npmrc" | ".pgpass"
    ) {
        return true;
    }
    if lower.starts_with(".env") {
        return true;
    }
    if lower.starts_with("id_rsa") || lower.starts_with("id_ed25519") {
        return true;
    }
    if lower.contains("credentials") {
        return true;
    }
    lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.ends_with(".pfx")
        || lower.ends_with(".p12")
}

/// `candidate` 是否位于 `root` 之下（按组件、忽略大小写；相等视为在内）。
pub fn is_under_root(candidate: &Path, root: &Path) -> bool {
    let c: Vec<String> = candidate
        .components()
        .map(|x| x.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    let r: Vec<String> = root
        .components()
        .map(|x| x.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    !r.is_empty() && c.len() >= r.len() && c[..r.len()] == r[..]
}

fn same_path_ci(a: &Path, b: &Path) -> bool {
    is_under_root(a, b) && is_under_root(b, a)
}

/// **open-first** 打开路径，返回持有 OS 句柄的文件对象与最终路径。
///
/// 句柄所有权移交 `std::fs::File`（drop 即 `CloseHandle`）；失败 fail closed。
#[cfg(windows)]
fn open_guard(path: &Path) -> Result<(std::fs::File, PathBuf), String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, GetFinalPathNameByHandleW, FILE_ATTRIBUTE_NORMAL, FILE_CREATION_DISPOSITION,
        FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, OPEN_EXISTING,
        GETFINALPATHNAMEBYHANDLE_FLAGS,
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let flags = FILE_FLAGS_AND_ATTRIBUTES(
        FILE_ATTRIBUTE_NORMAL.0 | FILE_FLAG_BACKUP_SEMANTICS.0,
    );

    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_READ,
            None,
            FILE_CREATION_DISPOSITION(OPEN_EXISTING.0),
            flags,
            None,
        )
    }
    .map_err(|_| "打开路径失败".to_string())?;
    let raw_handle = handle.0;

    let mut buf = vec![0u16; 32768];
    let len = unsafe {
        GetFinalPathNameByHandleW(handle, &mut buf, GETFINALPATHNAMEBYHANDLE_FLAGS(0))
    } as usize;
    if len == 0 || len > buf.len() {
        // 关闭句柄（drop File）后 fail closed
        drop(unsafe { std::fs::File::from_raw_handle(raw_handle) });
        return Err("解析最终路径失败".to_string());
    }
    let raw = String::from_utf16_lossy(&buf[..len]);
    let final_path = normalize_final_path(&raw);
    let file = unsafe { std::fs::File::from_raw_handle(raw_handle) };
    Ok((file, final_path))
}

/// 打开路径并取最终路径（句柄随即关闭）。仅用于一次性校验，不用于读取。
#[cfg(windows)]
pub fn open_readonly_final_path(path: &Path) -> Result<PathBuf, String> {
    open_guard(path).map(|(_, final_path)| final_path)
}

#[cfg(not(windows))]
pub fn open_readonly_final_path(_path: &Path) -> Result<PathBuf, String> {
    Err("句柄级路径校验仅支持 Windows".to_string())
}

/// 去掉 `\\?\` / `\\?\UNC\` 前缀，还原常规路径。
fn normalize_final_path(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        PathBuf::from(raw)
    }
}

/// 目录项（按句柄枚举）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryInfo {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// open-first 守卫：持有 OS 句柄期间 `final_path` 可信；文件读取走**同一句柄**。
#[allow(dead_code)] // P2 runner：读取阶段持有句柄，杜绝按路径重开。
pub struct PathGuard {
    file: std::fs::File,
    final_path: PathBuf,
    is_dir: bool,
}

#[allow(dead_code)] // 方法由 P2 runner 消费（读取 / 目录判断 / 最终路径）。
impl PathGuard {
    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    pub fn is_dir(&self) -> bool {
        self.is_dir
    }

    /// 从句柄读取一页（`offset` 起、至多 [`FILE_PAGE_LIMIT`]），先扣预算。
    pub fn read_page(&mut self, offset: u64, budget: &mut ReadBudget) -> Result<Vec<u8>, String> {
        use std::io::{Read, Seek, SeekFrom};
        if self.is_dir {
            return Err("不能读取目录句柄".to_string());
        }
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| format!("定位读取失败：{e}"))?;
        let mut buf = vec![0u8; FILE_PAGE_LIMIT];
        let mut filled = 0;
        while filled < buf.len() {
            let n = self
                .file
                .read(&mut buf[filled..])
                .map_err(|e| format!("读取失败：{e}"))?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        buf.truncate(filled);
        budget.reserve(filled as u64)?;
        Ok(buf)
    }

    /// **按句柄**枚举目录的一页（每页 ≤ [`DIR_PAGE_LIMIT`] 项），返回 `(items, has_more)`。
    ///
    /// 经 `GetFileInformationByHandleEx(FileIdBothDirectoryInfo)` 在**已持有**的目录句柄上枚举，
    /// 不按路径重开。
    #[cfg(windows)]
    pub fn list_dir_page(&mut self, page: usize) -> Result<(Vec<DirEntryInfo>, bool), String> {
        if !self.is_dir {
            return Err("不是目录".to_string());
        }
        list_dir_handle(self, page)
    }

    #[cfg(not(windows))]
    pub fn list_dir_page(&mut self, _page: usize) -> Result<(Vec<DirEntryInfo>, bool), String> {
        Err("目录枚举仅支持 Windows".to_string())
    }
}

/// 在已持有的目录句柄上枚举（Windows）。
#[cfg(windows)]
fn list_dir_handle(guard: &PathGuard, page: usize) -> Result<(Vec<DirEntryInfo>, bool), String> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandleEx, FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo,
    };

    const ERROR_MORE_DATA: i32 = 234;

    fn win32_code(e: &windows::core::Error) -> i32 {
        (e.code().0 as u32 & 0xFFFF) as i32
    }

    let handle = HANDLE(guard.file.as_raw_handle());
    let needed = (page + 1) * DIR_PAGE_LIMIT;
    let mut collected: Vec<DirEntryInfo> = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];

    // 句柄枚举是**有状态**的（游标随调用前进）：首次用 Restart 复位，保证每次调用可独立分页。
    let mut first = true;
    loop {
        let class = if first {
            FileIdBothDirectoryRestartInfo
        } else {
            FileIdBothDirectoryInfo
        };
        first = false;
        let res = unsafe {
            GetFileInformationByHandleEx(
                handle,
                class,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                buf.len() as u32,
            )
        };
        match res {
            Ok(()) => {
                parse_dir_buffer(&buf, &mut collected);
                break;
            }
            Err(e) if win32_code(&e) == ERROR_MORE_DATA => {
                parse_dir_buffer(&buf, &mut collected);
                if collected.len() > needed {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let start = page.saturating_mul(DIR_PAGE_LIMIT);
    let slice: Vec<DirEntryInfo> = collected
        .iter()
        .skip(start)
        .take(DIR_PAGE_LIMIT)
        .cloned()
        .collect();
    let has_more = collected.len() > start + slice.len();
    Ok((slice, has_more))
}

#[cfg(windows)]
fn parse_dir_buffer(buf: &[u8], out: &mut Vec<DirEntryInfo>) {
    use windows::Win32::Storage::FileSystem::FILE_ID_BOTH_DIR_INFO;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

    let mut offset = 0usize;
    while offset + 8 <= buf.len() {
        let info = unsafe { &*(buf.as_ptr().add(offset) as *const FILE_ID_BOTH_DIR_INFO) };
        let name_len = info.FileNameLength as usize / 2;
        let base = buf.as_ptr() as usize;
        let name_start = info.FileName.as_ptr() as usize - base;
        if name_start + name_len * 2 > buf.len() {
            break;
        }
        let name = String::from_utf16_lossy(unsafe {
            std::slice::from_raw_parts(info.FileName.as_ptr(), name_len)
        });
        if name != "." && name != ".." {
            out.push(DirEntryInfo {
                name,
                is_dir: info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
                size: info.EndOfFile.max(0) as u64,
            });
        }
        if info.NextEntryOffset == 0 {
            break;
        }
        let next = offset + info.NextEntryOffset as usize;
        if next <= offset || next + 8 > buf.len() {
            break;
        }
        offset = next;
    }
}

/// 结果目录安全创建：`result_root`(校验) + `task_id`(ULID) + `{timestamp}_{run_id}`(ULID)。
///
/// 逐级 `create_dir_all` 后 open-first 复核最终路径未被重解析替换；失败 fail closed。
#[allow(dead_code)] // P2 runner：结果落盘。
pub fn create_result_dir(
    app_config_base: &Path,
    result_root: &str,
    task_id: &str,
    run_id: &str,
    timestamp: &str,
) -> Result<PathBuf, String> {
    if ulid::Ulid::from_string(task_id).is_err() {
        return Err("任务缺少合法 ULID，拒绝生成结果路径".to_string());
    }
    if ulid::Ulid::from_string(run_id).is_err() {
        return Err("非法的 run_id".to_string());
    }
    if timestamp.is_empty()
        || timestamp.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|', ' '])
    {
        return Err("非法的时间戳".to_string());
    }
    validate_result_root_in(app_config_base, result_root)?;

    let root = Path::new(result_root);
    let run_dir = root.join(task_id).join(format!("{timestamp}_{run_id}"));
    std::fs::create_dir_all(&run_dir).map_err(|e| format!("创建结果目录失败：{e}"))?;

    // reparse 安全：最终路径必须与预期一致（防 junction 替换）
    verify_result_dir(&run_dir)?;
    Ok(run_dir)
}

/// 目录复核（写入前 / 后）：open-first 取最终路径，必须与预期路径一致，否则 fail closed。
#[allow(dead_code)] // P2 runner：结果写入前后目录复核。
pub fn verify_result_dir(dir: &Path) -> Result<(), String> {
    let resolved = open_readonly_final_path(dir)?;
    if !same_path_ci(&resolved, dir) {
        return Err("结果目录被重解析 / 替换，已拒绝".to_string());
    }
    Ok(())
}


/// 校验 `path` 可安全只读并返回**持句柄**的 [`PathGuard`]：字符串护栏 → open-first →
/// 最终路径仍在授权根之下。调用方只能经守卫从句柄读取，不得按路径重开。
#[cfg(windows)]
#[allow(dead_code)] // P2 runner：读入侧 open-first + 同句柄读取。
pub fn resolve_allowed(path: &Path, roots: &[PathBuf]) -> Result<PathGuard, String> {
    let s = path.to_string_lossy();
    guard_path_string(&s)?;
    let (file, final_path) = open_guard(path)?;
    guard_path_string(&final_path.to_string_lossy())?;
    if !roots.iter().any(|r| is_under_root(&final_path, r)) {
        return Err("路径不在授权根之下".to_string());
    }
    let is_dir = file.metadata().map(|m| m.is_dir()).unwrap_or(false);
    Ok(PathGuard {
        file,
        final_path,
        is_dir,
    })
}

#[cfg(not(windows))]
#[allow(dead_code)] // P2 runner：非 Windows fail closed 占位。
pub fn resolve_allowed(_path: &Path, _roots: &[PathBuf]) -> Result<PathGuard, String> {
    Err("句柄级路径校验仅支持 Windows".to_string())
}

/// 读取预算（单次运行累计 ≤5MiB，不可绕）。
#[allow(dead_code)] // P2 runner：运行级累计读取预算。
pub struct ReadBudget {
    remaining: u64,
}

impl Default for ReadBudget {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)] // 方法由 P2 runner 消费。
impl ReadBudget {
    pub fn new() -> Self {
        Self {
            remaining: READ_BUDGET_LIMIT,
        }
    }

    pub fn remaining(&self) -> u64 {
        self.remaining
    }

    /// 申请 `bytes` 字节额度；超出则失败（不扣减）。
    pub fn reserve(&mut self, bytes: u64) -> Result<(), String> {
        if bytes > self.remaining {
            return Err("超出单次运行读取预算".to_string());
        }
        self.remaining -= bytes;
        Ok(())
    }

    /// 取 `offset` 起、至多 [`FILE_PAGE_LIMIT`] 的一页。
    pub fn page<'a>(&self, data: &'a [u8], offset: usize) -> &'a [u8] {
        let start = offset.min(data.len());
        let end = (start + FILE_PAGE_LIMIT).min(data.len());
        &data[start..end]
    }
}

/// 目录分页：第 `page` 页（从 0 起），每页 [`DIR_PAGE_LIMIT`] 项。
#[allow(dead_code)] // P2 runner：目录分页。
pub fn page_dir(items: &[String], page: usize) -> Vec<String> {
    let start = page.saturating_mul(DIR_PAGE_LIMIT);
    items.iter().skip(start).take(DIR_PAGE_LIMIT).cloned().collect()
}

/// 结果根安全可写探针：护栏 →（非根 / 非配置目录 / 非重解析）→ `create_dir_all` → 写探针。
///
/// `app_config_base` 为应用配置目录（含密钥），其内不允许作为结果目录。
pub fn validate_result_root_in(app_config_base: &Path, path: &str) -> Result<(), String> {
    let p = path.trim();
    if p.is_empty() {
        return Err("结果目录不能为空".to_string());
    }
    guard_path_string(p)?;
    let dir = Path::new(p);
    if dir.parent().is_none() {
        return Err("不能使用盘符根目录作为结果目录".to_string());
    }
    if is_under_root(dir, app_config_base) {
        return Err("结果目录不能位于应用配置目录内".to_string());
    }
    // 目标若已是重解析点 → 拒绝（fail closed）
    if let Ok(md) = std::fs::symlink_metadata(dir) {
        if is_reparse(&md) {
            return Err("结果目录为链接 / 重解析点，已拒绝".to_string());
        }
    }

    std::fs::create_dir_all(dir).map_err(|e| format!("创建结果目录失败：{e}"))?;

    // open-first：解析最终路径，确认未被重解析替换
    let resolved = open_readonly_final_path(dir)?;
    if !same_path_ci(&resolved, dir) {
        return Err("结果目录路径被重解析 / 替换，已拒绝".to_string());
    }

    // 可写探针
    let probe = dir.join(format!(".agent-probe-{}", ulid::Ulid::generate()));
    std::fs::write(&probe, b"probe").map_err(|e| format!("结果目录不可写：{e}"))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

#[cfg(windows)]
fn is_reparse(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse(md: &std::fs::Metadata) -> bool {
    md.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_rejects_dangerous_shapes() {
        let bad = [
            "",
            "relative\\a.txt",
            "\\\\server\\share\\a",
            "\\\\?\\C:\\a",
            "C:\\a\\b:stream",
            "C:\\dir.",
            "C:\\dir ",
            "C:\\a\\..\\b",
            "C:\\CON",
            "C:\\a\\.git\\config",
            "C:\\a\\.ssh\\id_rsa",
            "C:\\proj\\.env.local",
            "C:\\keys\\server.pem",
            "C:\\keys\\my.key",
            "C:\\creds\\credentials.json",
        ];
        for p in bad {
            assert!(guard_path_string(p).is_err(), "应拒绝：{p}");
        }
        let good = ["C:\\docs\\a.txt", "D:\\proj", "C:\\a\\b c.txt"];
        for p in good {
            assert!(guard_path_string(p).is_ok(), "应允许：{p}");
        }
    }

    #[test]
    fn source_root_guard_rejects_root_unc_device_and_dotdot() {
        // P1-4：源授权根与结果根同等强度
        for p in ["C:\\", "D:\\", "\\\\?\\C:\\a", "\\\\server\\share", "C:\\a\\..\\b"] {
            assert!(guard_source_root(p).is_err(), "危险授权根应被拒：{p}");
        }
        assert!(guard_source_root("C:\\docs\\a.txt").is_ok());
        assert!(guard_source_root("D:\\proj").is_ok());
    }

    #[test]
    fn reserved_and_sensitive_helpers() {
        assert!(is_reserved_name("CON"));
        assert!(is_reserved_name("con.txt"));
        assert!(is_reserved_name("COM1"));
        assert!(is_reserved_name("lpt9.doc"));
        assert!(!is_reserved_name("COM0"));
        assert!(!is_reserved_name("console"));
        assert!(is_sensitive_component("id_rsa"));
        assert!(is_sensitive_component(".git"));
        assert!(!is_sensitive_component("readme.md"));
    }

    #[test]
    fn roots_matching_is_case_insensitive() {
        let root = PathBuf::from("C:\\Work\\Proj");
        assert!(is_under_root(Path::new("c:\\work\\proj\\a.txt"), &root));
        assert!(is_under_root(Path::new("C:\\Work\\Proj"), &root));
        assert!(!is_under_root(Path::new("C:\\Work\\Other\\a.txt"), &root));
        assert!(!is_under_root(Path::new("C:\\Work"), &root));
        // 兄弟前缀不得误判：root C:\a 不应覆盖 C:\ab\f
        assert!(!is_under_root(Path::new("C:\\ab\\f"), Path::new("C:\\a")));
        assert!(is_under_root(Path::new("C:\\a\\f"), Path::new("C:\\a")));
    }

    #[test]
    fn budget_and_paging_bounds() {
        let mut b = ReadBudget::new();
        assert_eq!(b.remaining(), READ_BUDGET_LIMIT);
        b.reserve(FILE_PAGE_LIMIT as u64).unwrap();
        assert_eq!(b.remaining(), READ_BUDGET_LIMIT - FILE_PAGE_LIMIT as u64);
        // 超预算失败且不扣减
        let before = b.remaining();
        assert!(b.reserve(before + 1).is_err());
        assert_eq!(b.remaining(), before);

        let data = vec![7u8; FILE_PAGE_LIMIT + 100];
        assert_eq!(b.page(&data, 0).len(), FILE_PAGE_LIMIT);
        assert_eq!(b.page(&data, FILE_PAGE_LIMIT).len(), 100);

        let items: Vec<String> = (0..(DIR_PAGE_LIMIT + 5)).map(|i| i.to_string()).collect();
        assert_eq!(page_dir(&items, 0).len(), DIR_PAGE_LIMIT);
        assert_eq!(page_dir(&items, 1).len(), 5);
    }

    #[test]
    fn open_first_resolves_existing_file_and_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"hi").unwrap();

        // 文件
        let resolved = open_readonly_final_path(&file).unwrap();
        assert!(same_path_ci(&resolved, &file));
        // 目录
        let resolved_dir = open_readonly_final_path(dir.path()).unwrap();
        assert!(same_path_ci(&resolved_dir, dir.path()));

        // 授权根内的文件通过
        let roots = vec![dir.path().to_path_buf()];
        assert!(resolve_allowed(&file, &roots).is_ok());
        // 根外文件失败：授权根为 sub，文件位于其外
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let outside = dir.path().join("outside.txt");
        std::fs::write(&outside, b"x").unwrap();
        assert!(resolve_allowed(&outside, &[sub]).is_err());
    }

    #[test]
    fn guard_reads_from_same_handle() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("data.txt");
        std::fs::write(&file, b"hello-handle").unwrap();

        let mut guard = resolve_allowed(&file, &[dir.path().to_path_buf()]).unwrap();
        assert!(!guard.is_dir());
        let mut budget = ReadBudget::new();
        let bytes = guard.read_page(0, &mut budget).unwrap();
        assert_eq!(bytes, b"hello-handle");
        assert_eq!(budget.remaining(), READ_BUDGET_LIMIT - 12);
        // offset 越过尾部 → 空
        assert!(guard.read_page(1000, &mut budget).unwrap().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn nested_junction_escape_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, b"top").unwrap();

        let link = root.path().join("link");
        let created = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(outside.path())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            return; // 受限环境无法创建 junction → 跳过
        }

        // 候选经根内 junction 指向根外真实文件：最终路径在根外 → 拒绝
        let candidate = link.join("secret.txt");
        assert!(
            resolve_allowed(&candidate, &[root.path().to_path_buf()]).is_err(),
            "根内 junction 逃逸应被拒绝"
        );
    }

    #[test]
    fn result_root_validate_probes_writable() {
        let cfg_base = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let target = work.path().join("results");

        // 不存在的目录会被创建并写探针 → ok
        validate_result_root_in(cfg_base.path(), &target.to_string_lossy()).unwrap();
        assert!(target.is_dir());

        // 盘符根 / UNC / ADS → 报错
        assert!(validate_result_root_in(cfg_base.path(), "C:\\").is_err());
        assert!(validate_result_root_in(cfg_base.path(), "\\\\server\\share").is_err());
        assert!(validate_result_root_in(cfg_base.path(), "C:\\a\\b:stream").is_err());
        assert!(validate_result_root_in(cfg_base.path(), "   ").is_err());

        // 配置目录内 → 报错
        let inside = cfg_base.path().join("sub");
        assert!(validate_result_root_in(cfg_base.path(), &inside.to_string_lossy()).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn result_root_rejects_reparse_junction() {
        let cfg = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let link = work.path().join("link");

        // junction 的创建无需管理员权限；受限环境下创建失败则跳过。
        let created = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(target.path())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            return;
        }

        // 目标为重解析点 → 拒绝
        assert!(
            validate_result_root_in(cfg.path(), &link.to_string_lossy()).is_err(),
            "junction 目标应被拒绝"
        );
        // open-first 会解析到真实目标；授权根为 link 时最终路径不在其下 → 拒绝
        assert!(resolve_allowed(&link, std::slice::from_ref(&link)).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn list_dir_by_handle_pages() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(DIR_PAGE_LIMIT + 5) {
            std::fs::write(dir.path().join(format!("f{i:03}.txt")), b"x").unwrap();
        }
        let mut guard = resolve_allowed(dir.path(), &[dir.path().to_path_buf()]).unwrap();
        assert!(guard.is_dir());
        let (page0, more0) = guard.list_dir_page(0).unwrap();
        assert_eq!(page0.len(), DIR_PAGE_LIMIT);
        assert!(more0, "第一页后仍有剩余");
        let (page1, more1) = guard.list_dir_page(1).unwrap();
        assert_eq!(page1.len(), 5);
        assert!(!more1);
        // 目录句柄不可作为文件读取
        let mut budget = ReadBudget::new();
        assert!(guard.read_page(0, &mut budget).is_err());
    }

    #[test]
    fn result_dir_requires_ulid_segments_and_creates() {
        let cfg = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let root = work.path().join("results");
        let task = ulid::Ulid::generate().to_string();
        let run = ulid::Ulid::generate().to_string();

        let dir = create_result_dir(
            cfg.path(),
            &root.to_string_lossy(),
            &task,
            &run,
            "2026-10-12T01-00-00",
        )
        .unwrap();
        assert!(dir.is_dir());
        assert!(dir.ends_with(format!("2026-10-12T01-00-00_{run}")));

        // 非法 task_id(非 ULID) / run_id / 时间戳 → 拒绝
        assert!(create_result_dir(cfg.path(), &root.to_string_lossy(), "T1", &run, "ts").is_err());
        assert!(create_result_dir(
            cfg.path(),
            &root.to_string_lossy(),
            &task,
            "../evil",
            "ts"
        )
        .is_err());
        assert!(create_result_dir(
            cfg.path(),
            &root.to_string_lossy(),
            &task,
            &run,
            "a/b"
        )
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn verify_result_dir_rejects_junction() {
        let target = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let link = work.path().join("link");
        let created = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(target.path())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            return;
        }
        // link 的最终路径解析到 target，与预期不一致 → 拒绝
        assert!(verify_result_dir(&link).is_err());
        assert!(verify_result_dir(target.path()).is_ok());
    }
}
