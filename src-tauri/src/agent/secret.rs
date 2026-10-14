//! 本机密钥存储：Windows DPAPI（当前用户）加密，密文 base64 落独立本机文件。
//!
//! 密钥明文**绝不**回传 JS、写日志或进错误串；仅在内存中解密供 Rust 侧 HTTP 使用，
//! 解密缓冲用 [`zeroize`] 清零。presence 仅暴露布尔。
//!
//! 文件内明文为 `{providerId: secret}` JSON（多 provider 分槽）；DPAPI + base64 的
//! **文件形态不变**。读取到旧单值格式时自动迁移到 [`presets::DEFAULT_PROVIDER_ID`] 槽
//! （幂等、原子写回；写回失败**不丢**旧 key）。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use zeroize::{Zeroize, Zeroizing};

use super::presets;

/// 密钥密文文件名（位于应用配置目录，与 `settings.yaml` 同级）。
pub const CREDENTIALS_FILE: &str = "agent.credentials";

/// 密钥存储抽象（按 provider 分槽）；便于测试注入假实现，绝不把明文暴露到接口之外。
pub trait SecretStore: Send + Sync {
    /// 保存某 provider 的密钥；空 / 纯空白视为错误。
    fn set(&self, provider: &str, key: &str) -> Result<(), String>;
    /// 删除某 provider 的密钥（幂等）。
    fn clear(&self, provider: &str) -> Result<(), String>;
    /// 某 provider 是否存在可解密且非空的密钥。
    fn present(&self, provider: &str) -> bool;
    /// 读取某 provider 的密钥供 Rust 侧使用；调用方不得回传 / 记录明文。
    fn get(&self, provider: &str) -> Result<Option<Zeroizing<String>>, String>;
}

/// DPAPI（当前用户）密钥存储，密文落 `base/agent.credentials`。
pub struct DpapiSecretStore {
    path: PathBuf,
}

impl DpapiSecretStore {
    /// `base` 为应用配置目录（`settings_dir()`）。
    pub fn new(base: &Path) -> Self {
        Self {
            path: base.join(CREDENTIALS_FILE),
        }
    }

    /// 解密并解析全部槽；缺失 → `None`。
    ///
    /// 新格式为 JSON map；解析失败视为**旧单值格式**，迁移到默认 provider 槽并原子写回。
    /// 写回失败时忽略错误、仍返回内存中的迁移结果（**不丢旧 key**）。
    fn read_map(&self) -> Result<Option<BTreeMap<String, String>>, String> {
        if !self.path.is_file() {
            return Ok(None);
        }
        let encoded =
            fs::read_to_string(&self.path).map_err(|e| format!("读取密钥文件失败：{e}"))?;
        let encrypted = BASE64
            .decode(encoded.trim())
            .map_err(|_| "密钥文件格式无效".to_string())?;
        let mut plain = unprotect(&encrypted)?;
        let mut text = match std::str::from_utf8(&plain) {
            Ok(s) => s.to_string(),
            Err(_) => {
                plain.zeroize();
                return Err("密钥内容无效".to_string());
            }
        };
        plain.zeroize();
        if text.trim().is_empty() {
            text.zeroize();
            return Ok(None);
        }
        if let Ok(map) = serde_json::from_str::<BTreeMap<String, String>>(&text) {
            // 成功路径同样清零解密明文缓冲（不残留于内存）。
            text.zeroize();
            return Ok(Some(map));
        }
        // 旧单值格式：整体归入默认 provider 槽。
        let mut map = BTreeMap::new();
        map.insert(
            presets::DEFAULT_PROVIDER_ID.to_string(),
            text.clone(),
        );
        // 幂等：写回成功后文件即为新格式；写回失败忽略（key 仍在内存 map 中返回）。
        let _ = self.write_map(&map);
        text.zeroize();
        Ok(Some(map))
    }

    /// 序列化 + DPAPI 加密 + base64 原子写回。
    fn write_map(&self, map: &BTreeMap<String, String>) -> Result<(), String> {
        let json = serde_json::to_string(map).map_err(|_| "序列化密钥失败".to_string())?;
        let mut plain = json.into_bytes();
        let encrypted = protect(&plain);
        plain.zeroize();
        let encrypted = encrypted?;
        let encoded = BASE64.encode(encrypted);
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败：{e}"))?;
        }
        crate::storage::atomic_write(&self.path, encoded.as_bytes())
    }
}

impl SecretStore for DpapiSecretStore {
    fn set(&self, provider: &str, key: &str) -> Result<(), String> {
        if key.trim().is_empty() {
            return Err("密钥不能为空".to_string());
        }
        // 读取失败（损坏）时**拒绝覆盖**，避免丢失其它槽。
        let mut map = self.read_map()?.unwrap_or_default();
        map.insert(provider.to_string(), key.to_string());
        self.write_map(&map)
    }

    fn clear(&self, provider: &str) -> Result<(), String> {
        let mut map = match self.read_map()? {
            Some(map) => map,
            None => return Ok(()),
        };
        if map.remove(provider).is_none() {
            return Ok(());
        }
        if map.is_empty() {
            match fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("删除密钥文件失败：{e}")),
            }
        } else {
            self.write_map(&map)
        }
    }

    fn present(&self, provider: &str) -> bool {
        matches!(self.get(provider), Ok(Some(key)) if !key.is_empty())
    }

    fn get(&self, provider: &str) -> Result<Option<Zeroizing<String>>, String> {
        match self.read_map()? {
            None => Ok(None),
            Some(map) => match map.get(provider) {
                Some(v) if !v.is_empty() => Ok(Some(Zeroizing::new(v.clone()))),
                _ => Ok(None),
            },
        }
    }
}

/// 测试用假密钥存储（仅内存，按 provider 分槽）；由 `#[cfg]` 门禁，release 不启用。
#[cfg(any(test, feature = "agent-test-provider"))]
#[allow(dead_code)]
pub struct FakeSecretStore {
    slots: std::sync::Mutex<BTreeMap<String, String>>,
}

#[cfg(any(test, feature = "agent-test-provider"))]
impl FakeSecretStore {
    /// 在默认 provider 槽写入 `key`（`None` = 空）。
    #[allow(dead_code)]
    pub fn new(key: Option<&str>) -> Self {
        let mut slots = BTreeMap::new();
        if let Some(k) = key {
            slots.insert(presets::DEFAULT_PROVIDER_ID.to_string(), k.to_string());
        }
        Self {
            slots: std::sync::Mutex::new(slots),
        }
    }

    /// 在指定 provider 槽写入 `key`。
    #[allow(dead_code)]
    pub fn with_slot(provider: &str, key: &str) -> Self {
        let mut slots = BTreeMap::new();
        slots.insert(provider.to_string(), key.to_string());
        Self {
            slots: std::sync::Mutex::new(slots),
        }
    }
}

#[cfg(any(test, feature = "agent-test-provider"))]
impl SecretStore for FakeSecretStore {
    fn set(&self, provider: &str, key: &str) -> Result<(), String> {
        if key.trim().is_empty() {
            return Err("密钥不能为空".to_string());
        }
        self.slots
            .lock()
            .unwrap()
            .insert(provider.to_string(), key.to_string());
        Ok(())
    }
    fn clear(&self, provider: &str) -> Result<(), String> {
        self.slots.lock().unwrap().remove(provider);
        Ok(())
    }
    fn present(&self, provider: &str) -> bool {
        self.slots
            .lock()
            .unwrap()
            .get(provider)
            .is_some_and(|k| !k.is_empty())
    }
    fn get(&self, provider: &str) -> Result<Option<Zeroizing<String>>, String> {
        Ok(self
            .slots
            .lock()
            .unwrap()
            .get(provider)
            .cloned()
            .map(Zeroizing::new))
    }
}

#[cfg(windows)]
fn protect(plaintext: &[u8]) -> Result<Vec<u8>, String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: plaintext.len() as u32,
        pbData: plaintext.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|_| "加密密钥失败".to_string())?;
        let result =
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData as *mut core::ffi::c_void)));
        Ok(result)
    }
}

#[cfg(windows)]
fn unprotect(ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: ciphertext.len() as u32,
        pbData: ciphertext.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|_| "解密密钥失败".to_string())?;
        let result =
            std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData as *mut core::ffi::c_void)));
        Ok(result)
    }
}

#[cfg(not(windows))]
fn protect(_plaintext: &[u8]) -> Result<Vec<u8>, String> {
    Err("密钥保护仅支持 Windows DPAPI".to_string())
}

#[cfg(not(windows))]
fn unprotect(_ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    Err("密钥保护仅支持 Windows DPAPI".to_string())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    const DS: &str = presets::DEFAULT_PROVIDER_ID;

    #[test]
    fn dpapi_roundtrip_present_clear_and_no_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiSecretStore::new(dir.path());
        assert!(!store.present(DS), "初始无密钥");
        assert!(store.get(DS).unwrap().is_none());

        // 空 / 纯空白拒绝
        assert!(store.set(DS, "").is_err());
        assert!(store.set(DS, "   ").is_err());

        let marker = "sk-test-9f3c-secret-marker";
        store.set(DS, marker).unwrap();
        assert!(store.present(DS));

        // 落盘密文不含明文；可以 base64 解码
        let raw = fs::read_to_string(dir.path().join(CREDENTIALS_FILE)).unwrap();
        assert!(!raw.contains(marker), "密文文件中不应出现明文");
        assert!(BASE64.decode(raw.trim()).is_ok());

        // 可解密且非空（不打印明文）
        let got = store.get(DS).unwrap().unwrap();
        assert_eq!(got.as_str(), marker);

        // 清除幂等
        store.clear(DS).unwrap();
        store.clear(DS).unwrap();
        assert!(!store.present(DS));
    }

    #[test]
    fn slots_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiSecretStore::new(dir.path());
        store.set("deepseek", "sk-ds").unwrap();
        store.set("anthropic", "sk-an").unwrap();
        assert_eq!(store.get("deepseek").unwrap().unwrap().as_str(), "sk-ds");
        assert_eq!(store.get("anthropic").unwrap().unwrap().as_str(), "sk-an");
        assert!(store.get("openai").unwrap().is_none());

        // 清除一个槽不影响另一个
        store.clear("deepseek").unwrap();
        assert!(!store.present("deepseek"));
        assert!(store.present("anthropic"));
        assert_eq!(store.get("anthropic").unwrap().unwrap().as_str(), "sk-an");
    }

    #[test]
    fn new_format_read_returns_values_after_zeroize() {
        // 新格式成功路径在返回前清零解密明文缓冲；返回值仍须正确。
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiSecretStore::new(dir.path());
        store.set("deepseek", "sk-ds-zeroize").unwrap();
        store.set("anthropic", "sk-an-zeroize").unwrap();
        assert_eq!(
            store.get("deepseek").unwrap().unwrap().as_str(),
            "sk-ds-zeroize"
        );
        assert_eq!(
            store.get("anthropic").unwrap().unwrap().as_str(),
            "sk-an-zeroize"
        );
        // 再次读取（幂等）仍正确
        assert_eq!(
            store.get("deepseek").unwrap().unwrap().as_str(),
            "sk-ds-zeroize"
        );
    }

    #[test]
    fn corrupt_base64_reports_error_and_not_present() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiSecretStore::new(dir.path());
        // 非法 base64
        fs::write(dir.path().join(CREDENTIALS_FILE), "!!!not-base64!!!").unwrap();
        assert!(store.get(DS).is_err(), "损坏 base64 应报错");
        assert!(!store.present(DS), "损坏密文不视为存在");

        // 合法 base64 但非 DPAPI 密文 → 解密失败
        fs::write(dir.path().join(CREDENTIALS_FILE), BASE64.encode(b"not-dpapi")).unwrap();
        assert!(store.get(DS).is_err());
        assert!(!store.present(DS));
    }

    #[test]
    fn corrupt_file_is_not_clobbered_by_set() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiSecretStore::new(dir.path());
        fs::write(dir.path().join(CREDENTIALS_FILE), "!!!not-base64!!!").unwrap();
        let before = fs::read_to_string(dir.path().join(CREDENTIALS_FILE)).unwrap();
        // 坏文件时写入应报错，且**不**覆盖原文件（不丢数据）
        assert!(store.set(DS, "sk-new").is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join(CREDENTIALS_FILE)).unwrap(),
            before
        );
    }

    /// 运行级迁移证据：构造旧单值格式 → 读取路径 → 断言迁移到 deepseek 槽且幂等。零真实网络。
    #[test]
    fn legacy_single_value_migrates_to_default_slot() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiSecretStore::new(dir.path());
        let marker = "sk-legacy-marker";

        // 旧格式：base64(DPAPI(raw secret))
        let encoded = BASE64.encode(protect(marker.as_bytes()).unwrap());
        fs::write(dir.path().join(CREDENTIALS_FILE), &encoded).unwrap();

        // 读取路径：旧单值 → 默认（deepseek）槽
        let got = store.get(DS).unwrap().unwrap();
        assert_eq!(got.as_str(), marker, "旧 key 自动归入默认槽");

        // 原子写回：文件已是新 JSON map 格式，含 deepseek 槽
        let raw = fs::read_to_string(dir.path().join(CREDENTIALS_FILE)).unwrap();
        let decrypted = unprotect(&BASE64.decode(raw.trim()).unwrap()).unwrap();
        let map: BTreeMap<String, String> = serde_json::from_slice(&decrypted).unwrap();
        assert_eq!(map.get("deepseek").map(String::as_str), Some(marker));
        assert!(!map.contains_key("anthropic"));

        // 幂等：再次读取结果不变
        let again = store.get(DS).unwrap().unwrap();
        assert_eq!(again.as_str(), marker);
        assert!(store.present(DS));
    }

    /// 迁移写回失败时**不丢旧 key**（只读目标使原子 rename 失败）。
    #[test]
    #[allow(clippy::permissions_set_readonly_false)] // 仅测试用：置回可写以便临时目录清理。
    fn migration_write_failure_still_returns_legacy_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiSecretStore::new(dir.path());
        let marker = "sk-legacy-readonly";
        let path = dir.path().join(CREDENTIALS_FILE);
        let encoded = BASE64.encode(protect(marker.as_bytes()).unwrap());
        fs::write(&path, encoded).unwrap();

        // 置为只读 → 原子写回（rename 覆盖）失败
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms).unwrap();

        // 仍能读取旧 key（内存中返回迁移结果）
        let got = store.get(DS).unwrap().unwrap();
        assert_eq!(got.as_str(), marker, "写回失败不得丢旧 key");
        assert!(store.present(DS));

        // 还原可写，便于临时目录清理
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).unwrap();
    }
}
