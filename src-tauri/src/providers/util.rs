//! 跨 provider 微工具：时间戳 / 可读字节数 / Win32 宽字符串。
//! 此前各 provider 各持一份逐字复制（apps/clipboard/clipimage/stage/vault 五份
//! now_ms、clipimage/jsontools 两份 human_size、apps 四处内联 wide），审计 B1 收编

/// Unix 毫秒时间戳（时钟异常落 0——仅用于排序/展示，不承担安全语义）
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 字节数 → 人类可读（B/KB/MB，一位小数）：剪贴图片附件与 JSON 报告共用
pub(crate) fn human_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// UTF-16 以 NUL 结尾（Win32 API 的字符串口径）
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
