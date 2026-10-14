//! Everything 托管：一键下载官方便携版 → 预置配置 → 提权启动。
//! 许可说明：运行时从 voidtools 官网下载（非再分发），Everything 本体免费。
//!
//! 关键细节：
//! - `-local`：配置存 exe 目录而非 %APPDATA%，托管路径自洽
//! - 启动前预置最小 ini（Everything 允许部分配置，缺省键走默认值），
//!   避开"配置在退出时才落盘、强杀导致永远生不出来"的坑
//! - 提权批处理一次完成"杀旧 + 启新"，单次 UAC

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::files;

/// 依次尝试的官方版本地址（新版本发布后旧地址会 404，按新到旧探测）
const VERSIONS: [&str; 5] = [
    "1.4.1.1028", "1.4.1.1027", "1.4.1.1026", "1.4.1.1025", "1.4.1.1024",
];

fn install_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join(r"zap\everything")
}

fn exe_path() -> PathBuf {
    install_dir().join("Everything.exe")
}

fn ini_path() -> PathBuf {
    install_dir().join("Everything.ini")
}

pub fn status() -> String {
    if files::probe() {
        return "running".into();
    }
    if exe_path().exists() {
        return "installed_not_running".into();
    }
    "not_installed".into()
}

fn download(dest_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let zip_path = dest_dir.join("everything.zip");
    let mut last_err = String::from("未知错误");
    for v in VERSIONS {
        let url = format!("https://www.voidtools.com/Everything-{v}.x64.zip");
        match client.get(&url).send() {
            Ok(resp) if resp.status().is_success() => {
                let bytes = resp.bytes().map_err(|e| e.to_string())?;
                std::fs::write(&zip_path, &bytes).map_err(|e| e.to_string())?;
                return Ok(());
            }
            Ok(resp) => {
                last_err = format!("{url} -> HTTP {}", resp.status());
            }
            Err(e) => {
                last_err = format!("{url} -> {e}");
            }
        }
    }
    Err(format!("全部版本下载失败（最后错误: {last_err}）"))
}

fn extract(dest_dir: &Path) -> Result<(), String> {
    let zip = dest_dir.join("everything.zip");
    let script = format!(
        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        zip.display(),
        dest_dir.display()
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!("解压失败: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let _ = std::fs::remove_file(&zip);
    Ok(())
}

/// 预置/修补 ini：确保 -local 本地配置里 HTTP 服务器是开着的。
/// 行尾必须用 CRLF——Everything 的解析器按 Windows 惯例处理换行。
/// 注意 allow_http_server 是 1.4.1.1027+ 的安全策略键：
/// 不声明它时，http_server_enabled=1 会在加载时被强制归零。
/// 端口策略：已有配置沿用其端口；全新安装取 40000-59999 随机端口，避开常用端口。
fn seed_ini() {
    const EOL: &str = "\r\n";
    const FIXED: [&str; 3] = [
        "allow_http_server=1",
        "http_server_enabled=1",
        "http_server_bindings=127.0.0.1",
    ];
    let ini = ini_path();
    let existing = std::fs::read_to_string(&ini).ok();
    let port = existing
        .as_deref()
        .and_then(|text| {
            text.lines().find_map(|l| {
                l.trim_start()
                    .strip_prefix("http_server_port=")
                    .and_then(|s| s.trim().parse::<u16>().ok())
            })
        })
        .unwrap_or_else(random_port);
    let port_line = format!("http_server_port={port}");

    let content = match existing {
        Some(text) => {
            let mut out = String::with_capacity(text.len() + 96);
            let mut seen = [false; 4];
            for line in text.lines() {
                let t = line.trim_start();
                let mut replaced = false;
                for (k, req) in FIXED.iter().enumerate() {
                    if t.starts_with(req.split('=').next().unwrap_or(req)) {
                        out.push_str(req);
                        seen[k] = true;
                        replaced = true;
                        break;
                    }
                }
                if !replaced && t.starts_with("http_server_port") {
                    out.push_str(&port_line);
                    seen[3] = true;
                    replaced = true;
                }
                if !replaced {
                    out.push_str(line);
                }
                out.push_str(EOL);
            }
            for (k, req) in FIXED.iter().enumerate() {
                if !seen[k] {
                    out.push_str(req);
                    out.push_str(EOL);
                }
            }
            if !seen[3] {
                out.push_str(&port_line);
                out.push_str(EOL);
            }
            out
        }
        None => {
            let mut out = String::new();
            for req in FIXED {
                out.push_str(req);
                out.push_str(EOL);
            }
            out.push_str(&port_line);
            out.push_str(EOL);
            out
        }
    };
    let _ = std::fs::write(&ini, content);
}

fn random_port() -> u16 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    40000 + (nanos % 20000) as u16
}

/// 写提权重启批处理：杀旧实例 + 启动新实例（-local 托管配置 + -startup 入托盘）
fn write_restart_bat() -> PathBuf {
    let bat = install_dir().join("restart-everything.cmd");
    let content = format!(
        "@echo off\r\ntaskkill /IM Everything.exe /F >nul 2>&1\r\ntimeout /t 1 /nobreak >nul\r\nstart \"\" \"{}\" -local -startup\r\n",
        exe_path().display()
    );
    let _ = std::fs::write(&bat, content);
    bat
}

fn launch_elevated() -> Result<(), String> {
    let bat = write_restart_bat();
    let script = format!(
        "Start-Process -FilePath '{}' -Verb RunAs -Wait",
        bat.display()
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("启动被取消（UAC 未授权）或失败".into());
    }
    Ok(())
}

fn wait_running(timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if files::probe() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

/// 完整安装/启动流程。幂等：已运行直接返回；已下载则跳过下载。
pub fn setup() -> Result<String, String> {
    if files::probe() {
        return Ok("running".into());
    }
    let dir = install_dir();
    if !exe_path().exists() {
        download(&dir)?;
        extract(&dir)?;
    }
    if !exe_path().exists() {
        return Err("解压后未找到 Everything.exe".into());
    }

    seed_ini();
    launch_elevated()?;

    if wait_running(Duration::from_secs(20)) {
        Ok("running".into())
    } else {
        Ok("running_no_http".into())
    }
}
