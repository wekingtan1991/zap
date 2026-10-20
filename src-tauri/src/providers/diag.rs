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
