//! 导入/导出路径边界校验（信任边界收口）。
//!
//! 前端经 plugin-dialog 让用户自选路径，后端仍必须自己复核：Tauri 自定义命令
//! 不受 capabilities 约束，path 是纯字符串，一旦前端被注入或参数被改写，
//! 无校验的 fs::write / read_to_string 就是任意文件读写口子。
//!
//! 取舍：不把路径限制死在 app_data_dir——备份/配置导出到 U 盘、网盘、桌面
//! 是产品承诺（README「换机重装有出路」）。改为「绝对路径 + 固定扩展名 +
//! 拒绝系统/启动/自身数据目录 + 导入大小上限」的组合边界，正常用法零影响。
//!
//! 禁区比较一律基于 canonicalize 后的真实路径：Windows 的 `\\?\C:\...`
//! verbatim 前缀、8.3 短名（C:\PROGRA~1）与 junction/符号链接都会让纯词法
//! 前缀比较被绕过。导入规范化整路径，导出规范化父目录后拼回文件名，
//! 禁区根同样规范化，两侧同形态再按组件比较（2026-11-09 勘界 P2）。

use std::path::{Component, Path, PathBuf};

pub const EXT_CLIPIMG: &str = "png";
pub const EXT_VAULT_BACKUP: &str = "zapvault";
pub const EXT_CONFIG: &str = "json";

/// 导入文件上限：配置与账号备份都是文本，64 MiB 远超实际体量。
/// 防止 read_to_string 被指向超大文件打爆内存。
const MAX_IMPORT_BYTES: u64 = 64 * 1024 * 1024;

/// 不允许读写的目录。
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

/// 禁区根规范化：目录不存在（如某些环境变量未配）时退回原值，
/// 但比较的一侧仍需与 canonicalize 后的目标同形态。
fn canonical_root(root: &Path) -> PathBuf {
    root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
}

fn forbidden_hit(canonical: &Path, roots: &[PathBuf]) -> bool {
    roots
        .iter()
        .any(|root| under(canonical, &canonical_root(root)))
}

/// 把待访问路径解析成真实形态，供禁区比较与实际 IO 共用。
/// whole=true（导入）：文件已存在，规范化整条路径；
/// whole=false（导出）：目标文件通常还不存在，规范化父目录后拼回文件名。
/// canonicalize 同时抹平 `\\?\` 前缀、短名、junction 与 `..`。
fn canonical_target(p: &Path, whole: bool) -> Result<PathBuf, String> {
    // 目标已存在时整路径规范化：导出覆盖的旧文件若是 junction/符号链接，
    // 只规范化父目录会留下「指向系统文件」的别名窗口。
    if whole || p.exists() {
        return p
            .canonicalize()
            .map_err(|_| "文件不存在或无法读取".to_string());
    }
    let parent = p.parent().ok_or("目标目录不存在")?;
    let name = p.file_name().ok_or("缺少文件名")?;
    let real_parent = parent
        .canonicalize()
        .map_err(|_| "目标目录不存在".to_string())?;
    Ok(real_parent.join(name))
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

/// 返回规范化后的路径给调用方，避免「校验用 A 形态、IO 用 B 形态」的
/// TOCTOU 别名窗口。
fn validate(path: &str, ext: &str, whole: bool) -> Result<PathBuf, String> {
    let p = check_syntax(path, ext)?;
    let canonical = canonical_target(&p, whole)?;
    if forbidden_hit(&canonical, &forbidden_roots()) {
        return Err("该位置不允许导入/导出，请换一个目录".into());
    }
    Ok(canonical)
}

/// 导出：绝对路径 + 扩展名白名单 + 目标不是目录 + 父目录存在。
pub fn export_path(path: &str, ext: &str) -> Result<PathBuf, String> {
    let p = validate(path, ext, false)?;
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
    let p = validate(path, ext, true)?;
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
    fn 导出规范化父目录后拼回文件名() {
        let dir = std::env::temp_dir().join("zap-pathguard-canon-out");
        let _ = std::fs::create_dir_all(&dir);
        let raw = dir.join("out.json");
        let got = canonical_target(&raw, false).unwrap();
        assert_eq!(got, dir.canonicalize().unwrap().join("out.json"));
        assert!(canonical_target(&dir.join("missing-sub").join("out.json"), false).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 导入规范化整路径且不存在即报错() {
        let dir = std::env::temp_dir().join("zap-pathguard-canon-in");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("in.json");
        let _ = std::fs::write(&file, "{}");
        assert_eq!(
            canonical_target(&file, true).unwrap(),
            file.canonicalize().unwrap()
        );
        assert!(canonical_target(&dir.join("missing.json"), true).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 两侧都规范化后禁区比较命中别名路径() {
        // Windows canonicalize 返回 \\?\ 前缀形态；两侧同形态才能命中，
        // 纯词法比较会让 verbatim / 短名 / junction 别名绕过禁区。
        let root = std::env::temp_dir().join("zap-pathguard-canon-root");
        let child_dir = root.join("sub");
        let _ = std::fs::create_dir_all(&child_dir);
        let file = child_dir.join("a.json");
        let _ = std::fs::write(&file, "{}");
        let canonical_root = root.canonicalize().unwrap();
        let canonical_file = file.canonicalize().unwrap();
        assert!(forbidden_hit(&canonical_file, &[canonical_root]));
        let other = std::env::temp_dir().canonicalize().unwrap();
        assert!(!forbidden_hit(
            &canonical_file,
            &[other.join("__not_there__")]
        ));
        let _ = std::fs::remove_dir_all(&root);
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
