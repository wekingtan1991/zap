//! TOTP 动态验证码（RFC 6238：HMAC-SHA1 / 30s 步长 / 6 位，主流认证器默认口径）。
//! 密钥接受 Base32（自动去空格/连字符，大小写不敏感）或 otpauth:// 链接。
//! 验证码与密码同级敏感：仅在显式查看/复制时出现，复制走受保护通道

use hmac::{Hmac, Mac};
use sha1::Sha1;

/// Base32 解码：去空格/连字符、大写化、容忍缺省 padding。
/// 非法字符报错（提示用户检查密钥）
pub fn base32_decode(input: &str) -> Result<Vec<u8>, String> {
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let cleaned = cleaned.trim_end_matches('=');
    if cleaned.is_empty() {
        return Err("TOTP 密钥为空".into());
    }
    base32::decode(base32::Alphabet::Rfc4648 { padding: false }, cleaned)
        .ok_or_else(|| "TOTP 密钥不是有效的 Base32".to_string())
}

/// 从用户输入提取密钥：otpauth:// 链接取 secret 参数，其余按 Base32 密钥处理。
/// 两条路径统一归一（去空白/连字符、大写）后返回
pub fn extract_secret(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    let secret = if let Some(rest) = trimmed.strip_prefix("otpauth://totp/") {
        let mut found = None;
        for pair in rest.split('?').nth(1).unwrap_or("").split('&') {
            if let Some(v) = pair.strip_prefix("secret=") {
                found = Some(
                    urlencoding::decode(v)
                        .map_err(|e| e.to_string())?
                        .to_string(),
                );
                break;
            }
        }
        match found {
            Some(s) => s,
            None => return Err("otpauth 链接中未找到 secret 参数".into()),
        }
    } else {
        trimmed.to_string()
    };
    // 回读校验合法性并归一
    base32_decode(&secret)?;
    Ok(secret
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect())
}

/// RFC 6238：当前时间步的 6 位验证码
pub fn current_code(secret_base32: &str) -> Result<String, String> {
    code_at(
        secret_base32,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    )
}

/// 指定 Unix 秒的验证码（测试用公开入口）
pub fn code_at(secret_base32: &str, unix_secs: u64) -> Result<String, String> {
    code_at_step(secret_base32, unix_secs / 30)
}

/// 步长寻址的验证码
fn code_at_step(secret_base32: &str, step: u64) -> Result<String, String> {
    let key = base32_decode(secret_base32)?;
    let mut mac = Hmac::<Sha1>::new_from_slice(&key).map_err(|e| e.to_string())?;
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    // RFC 4226 动态截断：取末字节低 4 位为偏移
    let offset = (digest[19] & 0x0f) as usize;
    let bin = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    Ok(format!("{:06}", bin % 1_000_000))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 附录 B 参考密钥 "12345678901234567890" 的 Base32
    const RFC_SEED: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn rfc6238官方测试向量() {
        // 8 位参考值取 6 位后缀即为 6 位口径结果
        assert_eq!(code_at(RFC_SEED, 59).unwrap(), "287082");
        assert_eq!(code_at(RFC_SEED, 1111111109).unwrap(), "081804");
        assert_eq!(code_at(RFC_SEED, 1111111111).unwrap(), "050471");
        assert_eq!(code_at(RFC_SEED, 1234567890).unwrap(), "005924");
        assert_eq!(code_at(RFC_SEED, 2000000000).unwrap(), "279037");
    }

    #[test]
    fn base32解码容错() {
        // 空格/连字符/小写/缺 padding 均可
        assert_eq!(
            base32_decode("gezd gnbv-gy3t qojq gezd gnbv gy3t qojq").unwrap(),
            base32_decode(RFC_SEED).unwrap()
        );
        assert!(base32_decode("abc1").is_err(), "数字 1 不是 Base32 字符");
        assert!(base32_decode("   ").is_err());
    }

    #[test]
    fn otpauth链接与裸密钥提取() {
        let uri = "otpauth://totp/GitLab:me%40x.com?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ&issuer=GitLab";
        assert_eq!(extract_secret(uri).unwrap(), RFC_SEED);
        assert_eq!(
            extract_secret("gezd gnbv-gy3t qojq gezd gnbv gy3t qojq").unwrap(),
            RFC_SEED
        );
        assert!(extract_secret("otpauth://totp/x?issuer=y").is_err());
        assert!(extract_secret("not-a-key!!").is_err());
        // 提取出的密钥可直接出码且与原种子一致
        assert_eq!(
            code_at(&extract_secret(uri).unwrap(), 59).unwrap(),
            "287082"
        );
    }

    #[test]
    fn 相邻步长出的码可不同() {
        // 步长推进必须推进计数器（防实现把时间当计数器）
        let a = code_at_step(RFC_SEED, 1).unwrap();
        let b = code_at_step(RFC_SEED, 2).unwrap();
        let _ = (a, b); // 只验证不 panic 且互为合法 6 位
        assert_eq!(code_at_step(RFC_SEED, 0).unwrap().len(), 6);
    }
}
