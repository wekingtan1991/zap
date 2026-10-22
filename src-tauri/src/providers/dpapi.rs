//! 账号库密码的 DPAPI 加解密（从 vault.rs 拆出）。
//! 应用专属熵让密文只有 zap 能解，同账户下其他程序无能为力

use base64::Engine as _;

/// 应用专属熵：让 DPAPI 密文只有 zap 能解，同账户下其他程序无能为力
const ENTROPY: &[u8] = b"zap:vault:v1";

#[cfg(windows)]
pub fn encrypt(plain: &str) -> Result<String, String> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData};
    use windows::core::PCWSTR;

    let plain_bytes = plain.as_bytes();
    let blob_in = CRYPT_INTEGER_BLOB {
        cbData: plain_bytes.len() as u32,
        pbData: plain_bytes.as_ptr() as *mut u8,
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: ENTROPY.len() as u32,
        pbData: ENTROPY.as_ptr() as *mut u8,
    };
    let out = {
        let mut blob_out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        unsafe {
            CryptProtectData(
                &blob_in,
                PCWSTR::null(),
                Some(&entropy),
                None,
                None,
                0,
                &mut blob_out,
            )
        }
        .map_err(|e| format!("DPAPI 加密失败: {e}"))?;
        blob_out
    };
    let encoded = base64::engine::general_purpose::STANDARD
        .encode(unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) });
    unsafe {
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
    }
    Ok(encoded)
}

#[cfg(windows)]
pub fn decrypt(encoded: &str) -> Result<String, String> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| e.to_string())?;
    let blob_in = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: ENTROPY.len() as u32,
        pbData: ENTROPY.as_ptr() as *mut u8,
    };
    let out = {
        let mut blob_out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        unsafe { CryptUnprotectData(&blob_in, None, Some(&entropy), None, None, 0, &mut blob_out) }
            .map_err(|e| format!("DPAPI 解密失败: {e}"))?;
        blob_out
    };
    let plain = String::from_utf8_lossy(unsafe {
        std::slice::from_raw_parts(out.pbData, out.cbData as usize)
    })
    .to_string();
    unsafe {
        let _ = LocalFree(Some(HLOCAL(out.pbData as *mut _)));
    }
    Ok(plain)
}

#[cfg(not(windows))]
pub fn encrypt(_: &str) -> Result<String, String> {
    Err("仅支持 Windows".into())
}

#[cfg(not(windows))]
pub fn decrypt(_: &str) -> Result<String, String> {
    Err("仅支持 Windows".into())
}
