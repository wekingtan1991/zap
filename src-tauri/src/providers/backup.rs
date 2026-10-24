//! 账号库备份的加解密：PBKDF2-HMAC-SHA256（600k 轮）派生密钥 → AES-256-GCM。
//! 文件为文本 JSON（盐/nonce/密文均 base64），扩展名 .zapvault；
//! 口令错误 = GCM 校验失败，报「口令错误或文件已损坏」，不泄露任何内容。

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// 备份格式版本（未来换 KDF/加密方案时递增）
const FORMAT_VERSION: u64 = 1;
/// OWASP 2023 建议：PBKDF2-HMAC-SHA256 ≥ 600k 轮
const KDF_ITERATIONS: u32 = 600_000;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
/// 单份备份条目上限（防御性：远超个人账号库规模即拒绝）
const MAX_ENTRIES: usize = 10_000;

/// 备份条目：恢复时全字段还原（含 pinned 与 TOTP 密钥），与智能导入的 ParsedAccount 是两条线
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupEntry {
    pub title: String,
    pub username: String,
    /// 明文——密文由本模块整体加密保护，不落明文盘
    pub password: String,
    pub url: String,
    pub notes: String,
    pub pinned: i64,
    /// TOTP Base32 密钥明文（随信封整体加密），空串 = 未配置
    pub totp: String,
}

/// 解密预览：条目 + 每条是否与现有账号库疑似重复（同有效标题与用户名）。
/// 只做提示不做决策——同名不同站由用户在预览里自行判断
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupPreview {
    pub entries: Vec<BackupEntry>,
    pub duplicates: Vec<bool>,
}

/// 备份文件的 JSON 外壳
#[derive(Serialize, Deserialize)]
struct BackupEnvelope {
    v: u64,
    kdf: String,
    iter: u32,
    salt: String,
    nonce: String,
    ct: String,
}

fn b64_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| format!("备份文件不是有效的 base64：{e}"))
}

fn derive_key(passphrase: &str, salt: &[u8]) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), salt, KDF_ITERATIONS, &mut key);
    key
}

fn random_bytes(len: usize) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; len];
    getrandom::fill(&mut buf).map_err(|e| format!("随机数生成失败：{e}"))?;
    Ok(buf)
}

/// 加密全部条目为备份文件内容（文本 JSON）
pub fn encrypt_backup(entries: &[BackupEntry], passphrase: &str) -> Result<String, String> {
    if entries.len() > MAX_ENTRIES {
        return Err(format!(
            "条目数 {} 超出备份上限 {MAX_ENTRIES}",
            entries.len()
        ));
    }
    if passphrase.is_empty() {
        return Err("备份口令不能为空".into());
    }
    let salt = random_bytes(SALT_LEN)?;
    let nonce = random_bytes(NONCE_LEN)?;
    let key = derive_key(passphrase, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let plain = serde_json::to_vec(entries).map_err(|e| e.to_string())?;
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plain.as_ref())
        .map_err(|e| format!("加密失败：{e}"))?;
    serde_json::to_string(&BackupEnvelope {
        v: FORMAT_VERSION,
        kdf: "PBKDF2-SHA256".into(),
        iter: KDF_ITERATIONS,
        salt: b64_encode(&salt),
        nonce: b64_encode(&nonce),
        ct: b64_encode(&ct),
    })
    .map_err(|e| e.to_string())
}

/// 解密备份文件内容为条目列表。口令错误与密文损坏统一报错，不区分（防枚举试探无意义化）
pub fn decrypt_backup(content: &str, passphrase: &str) -> Result<Vec<BackupEntry>, String> {
    let env: BackupEnvelope =
        serde_json::from_str(content.trim()).map_err(|e| format!("不是 zap 备份文件：{e}"))?;
    if env.v != FORMAT_VERSION {
        return Err(format!("备份格式版本不支持（v{}）", env.v));
    }
    if env.kdf != "PBKDF2-SHA256" {
        return Err(format!("未知的 KDF：{}", env.kdf));
    }
    let salt = b64_decode(&env.salt)?;
    let nonce = b64_decode(&env.nonce)?;
    let ct = b64_decode(&env.ct)?;
    let key = derive_key(passphrase, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let plain = cipher
        .decrypt(Nonce::from_slice(&nonce), ct.as_ref())
        .map_err(|_| "口令错误或文件已损坏".to_string())?;
    let entries: Vec<BackupEntry> =
        serde_json::from_slice(&plain).map_err(|e| format!("备份内容解析失败：{e}"))?;
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<BackupEntry> {
        vec![
            BackupEntry {
                title: "行云账号".into(),
                username: "tan@corp".into(),
                password: "P@ss 中文✓".into(),
                url: "https://a.com".into(),
                notes: "备注".into(),
                pinned: 1,
                totp: "GEZDGNBVGY3TQOJQ".into(),
            },
            BackupEntry {
                title: "gitlab".into(),
                username: "u2".into(),
                password: "pw2".into(),
                url: String::new(),
                notes: String::new(),
                pinned: 0,
                totp: String::new(),
            },
        ]
    }

    #[test]
    fn 加密解密往返全字段还原() {
        let entries = sample();
        let file = encrypt_backup(&entries, "口令123").unwrap();
        assert!(!file.contains("P@ss"), "文件内容不得含明文密码");
        assert!(!file.contains("GEZDGNBV"), "文件内容不得含明文 TOTP 密钥");
        let back = decrypt_backup(&file, "口令123").unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].password, "P@ss 中文✓");
        assert_eq!(back[0].pinned, 1);
        assert_eq!(back[0].totp, "GEZDGNBVGY3TQOJQ");
        assert_eq!(back[1].title, "gitlab");
        assert_eq!(back[1].totp, "");
    }

    #[test]
    fn 口令错误统一报错() {
        let file = encrypt_backup(&sample(), "正确口令").unwrap();
        let err = decrypt_backup(&file, "错误口令").unwrap_err();
        assert!(err.contains("口令错误"), "实际：{err}");

        // 密文被篡改同样报错（GCM 完整性）
        let mut tampered = file.clone();
        let mid = tampered.len() / 2;
        tampered.replace_range(
            mid..mid + 1,
            if &tampered[mid..mid + 1] == "A" {
                "B"
            } else {
                "A"
            },
        );
        assert!(decrypt_backup(&tampered, "正确口令").is_err());
    }

    #[test]
    fn 非备份文件与空口令拒绝() {
        assert!(decrypt_backup("hello world", "x").is_err());
        assert!(decrypt_backup("{\"v\":2}", "x").is_err(), "版本不符应拒绝");
        assert!(encrypt_backup(&sample(), "").is_err(), "空口令应拒绝");
    }
}
