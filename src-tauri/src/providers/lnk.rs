//! .lnk 解析与环境变量展开（自 apps.rs 拆出，审计 D5）：
//! resolve_lnk_target 取快捷方式真实目标（IShellLinkW + IPersistFile），
//! 次级操作（管理员运行/所在文件夹/复制路径）要的是真实 exe 而非 lnk 本体。
//! 消费方：apps 索引扫描、stage 条目解析、open_item 分发

use super::util::wide;

/// 展开路径中的环境变量（%ProgramFiles% 等）。展开失败原样返回
fn expand_env_vars(path: &str) -> String {
    use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows::core::PCWSTR;

    let path_w = wide(path);
    unsafe {
        // 先探所需长度（含结尾 NUL），再取展开结果
        let need = ExpandEnvironmentStringsW(PCWSTR(path_w.as_ptr()), None);
        if need == 0 {
            return path.to_string();
        }
        let mut buf = vec![0u16; need as usize];
        let written = ExpandEnvironmentStringsW(PCWSTR(path_w.as_ptr()), Some(&mut buf));
        if written == 0 || written as usize > buf.len() {
            return path.to_string();
        }
        String::from_utf16_lossy(&buf[..(written as usize - 1).min(buf.len() - 1)])
    }
}

/// 解析 .lnk 指向的真实目标路径（IShellLinkW + IPersistFile）。
/// 非 lnk 或解析失败返回 None，调用方按原路径处理。
/// 次级操作（管理员运行/所在文件夹/复制路径）要的是真实 exe，不是开始菜单里的快捷方式。
/// 注意：SLGP_RAWPATH 按设计不展开环境变量，且可能是相对路径——两者都必须处理，
/// 否则管理员运行/定位拿到坏路径必失败（复制路径不校验存在性，坏路径会静默扩散）
pub fn resolve_lnk_target(path: &str) -> Option<String> {
    // 非 .lnk 直接退出：COM 初始化 + IPersistFile::Load 对普通文件/URL 必然失败，
    // 白走一趟（调用方对 None 回退原始路径，语义不变）
    if !path.to_lowercase().ends_with(".lnk") {
        return None;
    }
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        IPersistFile, STGM_READ,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, SLGP_RAWPATH, ShellLink};
    use windows::core::{Interface, PCWSTR};

    let path_w = wide(path);
    unsafe {
        // COM 按线程初始化；已是其他模式时忽略错误继续用现有初始化
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let pf: IPersistFile = link.cast().ok()?;
        pf.Load(PCWSTR(path_w.as_ptr()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        // SLGP_RAWPATH = 4（windows 0.62 的 GetPath 收裸指针 + u32）
        link.GetPath(&mut buf, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
            .ok()?;
        let end = buf.iter().position(|&c| c == 0)?;
        let raw = String::from_utf16_lossy(&buf[..end]);
        if raw.is_empty() {
            return None;
        }
        // 环境变量展开；相对路径拼 lnk 的工作目录（lnk 自身目录兜底）
        let mut target = expand_env_vars(&raw);
        if !std::path::Path::new(&target).is_absolute() {
            let mut wd_buf = [0u16; 1024];
            let work_dir = link
                .GetWorkingDirectory(&mut wd_buf)
                .ok()
                .and_then(|_| {
                    let e = wd_buf.iter().position(|&c| c == 0)?;
                    let wd = String::from_utf16_lossy(&wd_buf[..e]);
                    (!wd.is_empty()).then_some(wd)
                })
                .unwrap_or_else(|| {
                    std::path::Path::new(path)
                        .parent()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default()
                });
            let base = expand_env_vars(&work_dir);
            target = std::path::Path::new(&base)
                .join(&target)
                .to_string_lossy()
                .to_string();
        }
        (!target.is_empty()).then_some(target)
    }
}
