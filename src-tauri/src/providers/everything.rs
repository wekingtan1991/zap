//! Everything 托管：一键下载官方便携版 → 预置配置 → 提权启动。
//! 许可说明：运行时从 voidtools 官网下载（非再分发），Everything 本体免费。
//!
//! 关键细节：
//! - `-local`：配置存 exe 目录而非 %APPDATA%，托管路径自洽
//! - 启动前预置最小 ini（Everything 允许部分配置，缺省键走默认值），
//!   避开"配置在退出时才落盘、强杀导致永远生不出来"的坑
//! - 提权批处理一次完成"杀旧 + 启新"，单次 UAC

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::files;

/// 内置兜底版本表：官网解析失败（改版/网络抖动）时仍可尝试已知地址。
/// 主路径是下载页解析——官方发新版后旧地址会 404，纯硬编码会整体失效
const VERSIONS: [&str; 5] = [
    "1.4.1.1028",
    "1.4.1.1027",
    "1.4.1.1026",
    "1.4.1.1025",
    "1.4.1.1024",
];

/// 从下载页 HTML 提取稳定版版本号（数字段降序）。
/// 只认纯数字加点（1.4.1.1028），天然排除 1.5 alpha 的 b 后缀等非稳定命名
fn extract_versions(html: &str) -> Vec<String> {
    const MARKER: &str = "Everything-";
    const SUFFIX: &str = ".x64.zip";
    let mut out: Vec<String> = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find(MARKER) {
        let tail = &rest[i + MARKER.len()..];
        rest = tail;
        let Some(end) = tail.find(SUFFIX) else {
            continue;
        };
        let ver = &tail[..end];
        if ver.is_empty() || !ver.chars().all(|c| c.is_ascii_digit() || c == '.') {
            continue;
        }
        if !out.iter().any(|v| v == ver) {
            out.push(ver.to_string());
        }
    }
    fn key(v: &str) -> Vec<u32> {
        v.split('.').filter_map(|s| s.parse().ok()).collect()
    }
    // 页面顺序不保证新在前
    out.sort_by_key(|v| std::cmp::Reverse(key(v)));
    out
}

/// 主路径官网解析 + 内置表兜底（去重合并）
fn resolve_versions(client: &reqwest::blocking::Client) -> Vec<String> {
    let mut versions = client
        .get("https://www.voidtools.com/downloads/")
        .send()
        .ok()
        .and_then(|resp| resp.text().ok())
        .map(|html| extract_versions(&html))
        .unwrap_or_default();
    for v in VERSIONS {
        if !versions.iter().any(|x| x == v) {
            versions.push((*v).to_string());
        }
    }
    versions
}

fn download(dest_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let zip_path = dest_dir.join("everything.zip");
    let versions = resolve_versions(&client);
    let mut last_err = String::from("未知错误");
    for v in &versions {
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

/// GUI 进程下调用 powershell 不闪控制台黑框
fn quiet(mut command: std::process::Command) -> std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn extract(dest_dir: &Path) -> Result<(), String> {
    let zip = dest_dir.join("everything.zip");
    let script = format!(
        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        zip.display(),
        dest_dir.display()
    );
    let out = quiet(std::process::Command::new("powershell"))
        .args(["-NoProfile", "-Command", &script])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "解压失败: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let _ = std::fs::remove_file(&zip);
    Ok(())
}

/// 预置/修补 ini：确保 -local 本地配置里 HTTP 服务器可用且带认证。
/// 行尾必须用 CRLF——Everything 的解析器按 Windows 惯例处理换行。
/// 注意 allow_http_server 是 1.4.1.1027+ 的安全策略键：
/// 不声明它时，http_server_enabled=1 会在加载时被强制归零。
/// 端口策略：已有配置沿用其端口；全新安装取 40000-59999 随机端口，避开常用端口。
///
/// 「只补缺、不覆盖」（评审 P1-12）：用户显式写过的键一律保留原值——旧实现
/// 把 FIXED 三键每次 setup 强制改回 1，用户手动关闭的 HTTP 服务被静默复活
/// （红队实证）。认证（评审 P1-12/R3 P1-1）：生成随机用户名/口令写
/// http_server_username/password——绑定 127.0.0.1 挡不住浏览器沙箱/网页类
/// 攻击面（能连回环端口但读不了文件系统，DNS rebinding 可绕 CORS 读响应），
/// 匿名查询全盘文件名的口子必须关；同用户进程本就能读 ini 拿到凭证，
/// 认证挡的从来不是它们。凭证与 files.rs 的 Basic Auth 同源（ini 是唯一载体）
fn seed_ini() {
    let existing = std::fs::read_to_string(ini_path()).ok();
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
    let content = merge_ini(existing.as_deref(), port, &random_token(), &random_token());
    let _ = std::fs::write(ini_path(), content);
}

/// ini 合并（纯函数）：existing 已有的键原值保留，缺失键按 wanted 追加到尾部。
/// 全部 wanted 键：allow_http_server（1.4.1.1027+ 安全策略）、HTTP 开关/绑定/
/// 端口、Basic Auth 凭证
fn merge_ini(existing: Option<&str>, port: u16, username: &str, password: &str) -> String {
    const EOL: &str = "\r\n";
    let mut wanted: Vec<(&str, String)> = vec![
        ("allow_http_server", "1".into()),
        ("http_server_enabled", "1".into()),
        ("http_server_bindings", "127.0.0.1".into()),
        ("http_server_port", port.to_string()),
        ("http_server_username", username.into()),
        ("http_server_password", password.into()),
    ];
    let mut out = String::new();
    if let Some(text) = existing {
        for line in text.lines() {
            let key = line
                .trim_start()
                .split('=')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if let Some(pos) = wanted.iter().position(|(k, _)| *k == key) {
                wanted.remove(pos); // 用户已有该键：原值保留，从补缺清单移除
            }
            out.push_str(line);
            out.push_str(EOL);
        }
    }
    for (k, v) in &wanted {
        out.push_str(k);
        out.push('=');
        out.push_str(v);
        out.push_str(EOL);
    }
    out
}

/// 随机凭证 token：系统 CSPRNG 16 字节 hex（getrandom 与 uuid_v4 同源）。
/// 熵源异常的时间+pid 兜底——本机环回口令非密钥材料，够抗网页端枚举
fn random_token() -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        return format!("{nanos:x}{:x}", std::process::id());
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn random_port() -> u16 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    40000 + (nanos % 20000) as u16
}

/// 写提权重启批处理：杀旧实例 + 启动新实例（-local 托管配置 + -startup 入托盘）
/// 安全说明：脚本内容由 zap 固定生成（只有 taskkill 与 start 两条），且紧写紧用；
/// 它位于用户可写目录，理论上存在本地竞态替换可能——但能达到"本地任意代码执行"
/// 前提的攻击者本就无需借此提权，故按可接受风险处理，不做额外 ACL 加固。
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
    let out = quiet(std::process::Command::new("powershell"))
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

// ---------- 自动检测 / 自动启动 ----------

/// 自动启动尝试的节流：失败（含用户取消 UAC）后 5 分钟内不再弹窗骚扰
static LAST_ATTEMPT: Mutex<Option<Instant>> = Mutex::new(None);
const ENSURE_BACKOFF: Duration = Duration::from_secs(300);

/// 自动检测并启动 Everything。返回状态串：
/// - `running`：已在运行或本次启动成功
/// - `not_installed`：未安装（不自动下载，保留面板里的手动入口让用户知情）
/// - `backoff`：距上次失败不足 5 分钟，本轮跳过
/// - `error`：本次启动失败
pub fn ensure() -> String {
    if files::probe() {
        return "running".into();
    }
    if !exe_path().exists() {
        return "not_installed".into();
    }
    let mut last = LAST_ATTEMPT.lock().expect("last_attempt poisoned");
    if last.is_some_and(|t| t.elapsed() < ENSURE_BACKOFF) {
        return "backoff".into();
    }
    *last = Some(Instant::now());
    drop(last);

    eprintln!("zap: everything 未运行，自动启动中…（UAC 请点“是”）");
    match setup() {
        Ok(s) if s == "running" => {
            eprintln!("zap: everything 自动启动成功 ✓ 全盘文件搜索已就绪");
            s
        }
        Ok(s) => {
            eprintln!("zap: everything 已启动但 HTTP 服务未就绪（{s}）");
            s
        }
        Err(e) => {
            eprintln!("zap: everything 自动启动失败: {e}");
            "error".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_versions, merge_ini};

    #[test]
    fn 官网页面解析版本号() {
        let html = r#"<a href="https://www.voidtools.com/Everything-1.4.1.1024.x64.zip">old</a>
            <a href="https://www.voidtools.com/Everything-1.4.1.1028.x64.zip">latest</a>
            <a href="https://www.voidtools.com/Everything-1.5.0.1392b.x64.zip">alpha</a>"#;
        let v = extract_versions(html);
        assert_eq!(
            v,
            vec!["1.4.1.1028".to_string(), "1.4.1.1024".to_string()],
            "b 后缀的 alpha 应被排除，且按版本号新在前"
        );
    }

    #[test]
    fn 无匹配页面返回空表() {
        assert!(extract_versions("<html>no downloads here</html>").is_empty());
    }

    #[test]
    fn ini合并只补缺不覆盖() {
        // 全新安装：六键齐备含凭证
        let fresh = merge_ini(None, 54321, "u1", "p1");
        for key in [
            "allow_http_server=1",
            "http_server_enabled=1",
            "http_server_bindings=127.0.0.1",
            "http_server_port=54321",
            "http_server_username=u1",
            "http_server_password=p1",
        ] {
            assert!(fresh.contains(key), "缺 {key}");
        }
        // 用户已有关闭态：原值保留（旧实现会强制改回 1——红队实证的复活 bug）
        let existing = "http_server_enabled=0\r\nhttp_server_port=12345\r\nhome=xxx\r\n";
        let out = merge_ini(Some(existing), 54321, "u2", "p2");
        assert!(out.contains("http_server_enabled=0"), "用户关闭不被复活");
        assert!(out.contains("http_server_port=12345"), "用户端口不被覆盖");
        assert!(out.contains("home=xxx"), "无关行原样保留");
        assert!(out.contains("http_server_username=u2"), "凭证只补缺");
        assert!(out.contains("allow_http_server=1"), "缺的安全策略键补上");
        assert!(!out.contains("http_server_port=54321"), "已有端口不重复写");
        // 用户已配凭证：不覆盖
        let with_creds = "http_server_username=myu\r\nhttp_server_password=myp\r\n";
        let out = merge_ini(Some(with_creds), 54321, "u3", "p3");
        assert!(out.contains("http_server_username=myu"));
        assert!(out.contains("http_server_password=myp"));
        assert!(!out.contains("u3"), "用户凭证不被覆盖");
    }
}
