//! 导入/导出路径边界校验（信任边界收口）。
//!
//! 前端经 plugin-dialog 让用户自选路径，后端仍必须自己复核：Tauri 自定义命令
//! 不受 capabilities 约束，path 是纯字符串，一旦前端被注入或参数被改写，
//! 无校验的 fs::write / read_to_string 就是任意文件读写口子。
//!
//! 取舍：不把路径限制死在 app_data_dir——备份/配置导出到 U 盘、网盘、桌面
//! 是产品承诺（README「换机重装有出路」）。改为「绝对路径 + 固定扩展名 +
//! 拒绝系统/启动/自身数据目录 + 导入大小上限」的组合边界，正常用法零影响。

use std::path::{Component, Path, PathBuf};

pub const EXT_CLIPIMG: &str = "png";
pub const EXT_VAULT_BACKUP: &str = "zapvault";
pub const EXT_CONFIG: &str = "json";

/// 导入文件上限：配置与账号备份都是文本，64 MiB 远超实际体量。
/// 防止 read_to_string 被指向超大文件打爆内存。
const MAX_IMPORT_BYTES: u64 = 64 * 1024 * 1024;

/// 不允许读写的目录（大小写不敏感、组件级前缀比较）。
/// 系统/程序目录防覆盖系统文件；启动目录防持久化执行；自身数据目录防自毁。
fn forbidden_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for key in [
        "SystemRoot",
        "windir",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
    ] {
        if let Ok(v) = std::env::var(key) {
            out.push(PathBuf::from(v));
        }
    }
    for (key, tail) in [
        ("APPDATA", r"Microsoft\Windows\Start Menu\Programs\Startup"),
        (
            "PROGRAMDATA",
            r"Microsoft\Windows\Start Menu\Programs\Startup",
        ),
    ] {
        if let Ok(v) = std::env::var(key) {
            out.push(PathBuf::from(v).join(tail));
        }
    }
    if let Ok(v) = std::env::var("APPDATA") {
        out.push(PathBuf::from(v).join("com.ais.zap"));
    }
    out
}

/// 组件级前缀比较：先整体小写（Windows 路径大小写不敏感），再交给
/// Path::starts_with，避免 C:\Program Files 与 C:\Program FilesX 的字符串误判。
fn under(path: &Path, root: &Path) -> bool {
    let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
    lower(path).starts_with(lower(root))
}

/// 纯语法校验（不碰磁盘、不读环境变量），便于单测。
fn check_syntax(path: &str, ext: &str) -> Result<PathBuf, String> {
    if path.trim().is_empty() {
        return Err("路径不能为空".into());
    }
    if path.chars().any(char::is_control) {
        return Err("路径含有非法控制字符".into());
    }
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err("请选择绝对路径".into());
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("路径不能包含 ..".into());
    }
    let got = p.extension().and_then(|s| s.to_str()).unwrap_or("");
    if !got.eq_ignore_ascii_case(ext) {
        return Err(format!("只能处理 .{ext} 文件"));
    }
    Ok(p.to_path_buf())
}

fn validate(path: &str, ext: &str) -> Result<PathBuf, String> {
    let p = check_syntax(path, ext)?;
    if forbidden_roots().iter().any(|root| under(&p, root)) {
        return Err("该位置不允许导入/导出，请换一个目录".into());
    }
    Ok(p)
}

/// 导出：绝对路径 + 扩展名白名单 + 目标不是目录 + 父目录存在。
pub fn export_path(path: &str, ext: &str) -> Result<PathBuf, String> {
    let p = validate(path, ext)?;
    if p.is_dir() {
        return Err("目标是目录，请指定文件名".into());
    }
    if p.parent().is_some_and(|parent| !parent.is_dir()) {
        return Err("目标目录不存在".into());
    }
    Ok(p)
}

/// 导入：在导出边界之上，再要求文件存在、是普通文件、体积有上限。
pub fn import_path(path: &str, ext: &str) -> Result<PathBuf, String> {
    let p = validate(path, ext)?;
    let meta = std::fs::metadata(&p).map_err(|_| "文件不存在或无法读取".to_string())?;
    if !meta.is_file() {
        return Err("目标不是文件".into());
    }
    if meta.len() > MAX_IMPORT_BYTES {
        return Err("文件过大，疑似非备份文件".into());
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 语法校验拒绝相对路径与越界写法() {
        assert!(check_syntax("out.png", EXT_CLIPIMG).is_err());
        assert!(check_syntax(r"C:\tmp\..\out.png", EXT_CLIPIMG).is_err());
        assert!(check_syntax("C:\\tmp\\out\u{0}.png", EXT_CLIPIMG).is_err());
        assert!(check_syntax(r"C:\tmp\out.exe", EXT_CLIPIMG).is_err());
        assert!(check_syntax("", EXT_CLIPIMG).is_err());
    }

    #[test]
    fn 扩展名大小写不敏感且路径合法时放行() {
        let p = check_syntax(r"D:\backup\zap.PNG", EXT_CLIPIMG).unwrap();
        assert!(p.is_absolute());
        assert!(check_syntax(r"D:\backup\zap.zapvault", EXT_VAULT_BACKUP).is_ok());
        assert!(check_syntax(r"D:\backup\zap.json", EXT_CONFIG).is_ok());
    }

    #[test]
    fn 前缀比较按组件而非字符串() {
        let root = Path::new(r"c:\program files");
        assert!(under(Path::new(r"C:\PROGRAM FILES\zap\a.json"), root));
        assert!(!under(Path::new(r"C:\PROGRAM FILESX\a.json"), root));
        assert!(!under(Path::new(r"D:\a.json"), root));
    }

    #[test]
    fn 导入拒绝不存在的文件与目录() {
        assert!(import_path(r"D:\__zap_missing__.json", EXT_CONFIG).is_err());
        let as_json = std::env::temp_dir().join("zap-pathguard-dir.json");
        let _ = std::fs::create_dir_all(&as_json);
        let r = import_path(as_json.to_str().unwrap(), EXT_CONFIG);
        let _ = std::fs::remove_dir(&as_json);
        assert!(r.is_err(), "目录不能当文件导入");
    }
}
