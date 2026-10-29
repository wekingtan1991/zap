//! 运行诊断：GUI 应用的 stderr 不可见，关键路径（密码焚除/剪贴板防护）落盘到
//! `<app_data_dir>/zap.log`，便于线上问题定位。日志失败必须静默——诊断不能影响主流程。

use std::io::Write;
use std::sync::OnceLock;

static LOG_PATH: OnceLock<std::path::PathBuf> = OnceLock::new();

/// setup 中调用一次，指向 app_data_dir/zap.log
pub fn init(path: std::path::PathBuf) {
    let _ = LOG_PATH.set(path);
}

/// 追加一行日志（时间戳为 Unix 毫秒）。超过 1MB 时轮转为 .old（只保留一代），
/// 轮转与其他写入方的竞争一律忽略失败——诊断不能影响主流程。
pub fn log(msg: &str) {
    let Some(path) = LOG_PATH.get() else {
        return;
    };
    if let Ok(meta) = std::fs::metadata(path)
        && meta.len() > 1_000_000
    {
        let old = path.with_extension("log.old");
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(path, &old);
    }
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = writeln!(f, "{ms}\t{msg}");
}

/// panic hook 专用：init 后落 zap.log；init 前的极早期崩溃落临时目录
/// zap-panic.log 兜底（release 的 windows_subsystem 连 stderr 都不可见，
/// 早期死因也要有痕）。全程不持锁、失败静默——hook 里再 panic 在
/// panic="abort" 下直接终止，连这一行都不会有。
pub fn log_panic(msg: &str) {
    if LOG_PATH.get().is_some() {
        log(msg);
        return;
    }
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = std::env::temp_dir().join("zap-panic.log");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| writeln!(f, "{ms}\t{msg}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 日志落盘可读回() {
        let dir = std::env::temp_dir().join(format!("zap-diag-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("zap.log");
        let _ = std::fs::remove_file(&path);
        init(path.clone());
        log("第一行");
        log("第二行");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("第一行") && content.contains("第二行"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
