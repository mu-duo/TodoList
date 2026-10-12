//! 本机密钥存储：Windows DPAPI（当前用户）加密，密文 base64 落独立本机文件。
//!
//! 密钥明文**绝不**回传 JS、写日志或进错误串；仅在内存中解密供 Rust 侧 HTTP 使用，
//! 解密缓冲用 [`zeroize`] 清零。presence 仅暴露布尔。

use std::fs;
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use zeroize::{Zeroize, Zeroizing};

/// 密钥密文文件名（位于应用配置目录，与 `settings.yaml` 同级）。
pub const CREDENTIALS_FILE: &str = "agent.credentials";

/// 密钥存储抽象；便于测试注入假实现，绝不把明文暴露到接口之外。
pub trait SecretStore: Send + Sync {
    /// 保存密钥；空 / 纯空白视为错误。
    fn set(&self, key: &str) -> Result<(), String>;
    /// 删除密钥（幂等）。
    fn clear(&self) -> Result<(), String>;
    /// 是否存在可解密且非空的密钥。
    fn present(&self) -> bool;
    /// 读取密钥供 Rust 侧使用；调用方不得回传 / 记录明文。
    fn get(&self) -> Result<Option<Zeroizing<String>>, String>;
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
}

impl SecretStore for DpapiSecretStore {
    fn set(&self, key: &str) -> Result<(), String> {
        if key.trim().is_empty() {
            return Err("密钥不能为空".to_string());
        }
        let mut plain = key.as_bytes().to_vec();
        let encrypted = protect(&plain);
        plain.zeroize();
        let encrypted = encrypted?;
        let encoded = BASE64.encode(encrypted);
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("创建配置目录失败：{e}"))?;
        }
        crate::storage::atomic_write(&self.path, encoded.as_bytes())
    }

    fn clear(&self) -> Result<(), String> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("删除密钥文件失败：{e}")),
        }
    }

    fn present(&self) -> bool {
        matches!(self.get(), Ok(Some(key)) if !key.is_empty())
    }

    fn get(&self) -> Result<Option<Zeroizing<String>>, String> {
        if !self.path.is_file() {
            return Ok(None);
        }
        let encoded = fs::read_to_string(&self.path).map_err(|e| format!("读取密钥文件失败：{e}"))?;
        let encrypted = BASE64
            .decode(encoded.trim())
            .map_err(|_| "密钥文件格式无效".to_string())?;
        let mut plain = unprotect(&encrypted)?;
        let text = match std::str::from_utf8(&plain) {
            Ok(s) => s.to_string(),
            Err(_) => {
                plain.zeroize();
                return Err("密钥内容无效".to_string());
            }
        };
        plain.zeroize();
        if text.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Zeroizing::new(text)))
        }
    }
}

/// 测试用假密钥存储（仅内存）；由 `#[cfg]` 门禁，release 不启用。
#[cfg(any(test, feature = "agent-test-provider"))]
#[allow(dead_code)]
pub struct FakeSecretStore {
    key: std::sync::Mutex<Option<String>>,
}

#[cfg(any(test, feature = "agent-test-provider"))]
impl FakeSecretStore {
    #[allow(dead_code)]
    pub fn new(key: Option<&str>) -> Self {
        Self {
            key: std::sync::Mutex::new(key.map(|k| k.to_string())),
        }
    }
}

#[cfg(any(test, feature = "agent-test-provider"))]
impl SecretStore for FakeSecretStore {
    fn set(&self, key: &str) -> Result<(), String> {
        if key.trim().is_empty() {
            return Err("密钥不能为空".to_string());
        }
        *self.key.lock().unwrap() = Some(key.to_string());
        Ok(())
    }
    fn clear(&self) -> Result<(), String> {
        *self.key.lock().unwrap() = None;
        Ok(())
    }
    fn present(&self) -> bool {
        self.key
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|k| !k.is_empty())
    }
    fn get(&self) -> Result<Option<Zeroizing<String>>, String> {
        Ok(self.key.lock().unwrap().clone().map(Zeroizing::new))
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

    #[test]
    fn dpapi_roundtrip_present_clear_and_no_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiSecretStore::new(dir.path());
        assert!(!store.present(), "初始无密钥");
        assert!(store.get().unwrap().is_none());

        // 空 / 纯空白拒绝
        assert!(store.set("").is_err());
        assert!(store.set("   ").is_err());

        let marker = "sk-test-9f3c-secret-marker";
        store.set(marker).unwrap();
        assert!(store.present());

        // 落盘密文不含明文；可以 base64 解码
        let raw = fs::read_to_string(dir.path().join(CREDENTIALS_FILE)).unwrap();
        assert!(!raw.contains(marker), "密文文件中不应出现明文");
        assert!(BASE64.decode(raw.trim()).is_ok());

        // 可解密且非空（不打印明文）
        let got = store.get().unwrap().unwrap();
        assert_eq!(got.as_str(), marker);

        // 清除幂等
        store.clear().unwrap();
        store.clear().unwrap();
        assert!(!store.present());
    }

    #[test]
    fn corrupt_base64_reports_error_and_not_present() {
        let dir = tempfile::tempdir().unwrap();
        let store = DpapiSecretStore::new(dir.path());
        // 非法 base64
        fs::write(dir.path().join(CREDENTIALS_FILE), "!!!not-base64!!!").unwrap();
        assert!(store.get().is_err(), "损坏 base64 应报错");
        assert!(!store.present(), "损坏密文不视为存在");

        // 合法 base64 但非 DPAPI 密文 → 解密失败
        fs::write(dir.path().join(CREDENTIALS_FILE), BASE64.encode(b"not-dpapi")).unwrap();
        assert!(store.get().is_err());
        assert!(!store.present());
    }
}
