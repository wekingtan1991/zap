//! 全盘文件源：走 Everything 的 HTTP/JSON 服务（本机回环）。
//! 未安装/未开启时静默休眠，每 30 秒重试探测一次，不影响其他源。

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::SearchItem;

const HTTP_TIMEOUT: Duration = Duration::from_millis(250);
const RETRY_BACKOFF: Duration = Duration::from_secs(30);

static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
static PORT: Mutex<Option<u16>> = Mutex::new(None);
static LAST_FAIL: Mutex<Option<Instant>> = Mutex::new(None);

fn client() -> &'static reqwest::blocking::Client {
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("构建 HTTP 客户端失败")
    })
}

fn recently_failed() -> bool {
    LAST_FAIL
        .lock()
        .expect("last_fail poisoned")
        .is_some_and(|t| t.elapsed() < RETRY_BACKOFF)
}

fn mark_failed() {
    *LAST_FAIL.lock().expect("last_fail poisoned") = Some(Instant::now());
}

/// 探测可用端口；全部失败则记录退避时间
fn resolve_port() -> Option<u16> {
    if let Some(&p) = PORT.lock().expect("port poisoned").as_ref() {
        return Some(p);
    }
    for &port in &port_candidates() {
        let url = format!("http://127.0.0.1:{port}/?json=1&n=1&s=zap-probe");
        if client().get(&url).send().is_ok() {
            *PORT.lock().expect("port poisoned") = Some(port);
            return Some(port);
        }
    }
    mark_failed();
    None
}

/// 探测候选：托管实例的 ini 端口优先，其余兼容手动安装的默认端口
fn port_candidates() -> Vec<u16> {
    let mut candidates = Vec::new();
    if let Ok(text) = std::fs::read_to_string(managed_ini_path()) {
        for line in text.lines() {
            if let Some(rest) = line.trim_start().strip_prefix("http_server_port=") {
                if let Ok(port) = rest.trim().parse::<u16>() {
                    candidates.push(port);
                }
                break;
            }
        }
    }
    candidates.extend([8080, 80]);
    candidates
}

fn managed_ini_path() -> std::path::PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(|base| std::path::PathBuf::from(base).join(r"zap\everything\Everything.ini"))
        .unwrap_or_else(|_| std::path::PathBuf::from(r"zap\everything\Everything.ini"))
}

/// 是否有 Everything 在响应（供托管模块与状态查询使用）
pub fn probe() -> bool {
    resolve_port().is_some()
}

pub fn search(query: &str, limit: usize) -> Vec<SearchItem> {
    // 正常路径（退避跳过、未探测到端口、成功解析）一律静默：搜索是每击键
    // 一次的热路径，这里刷日志只有噪音；真正的失败才落 stderr
    if recently_failed() {
        return Vec::new();
    }
    let Some(port) = resolve_port() else {
        return Vec::new();
    };
    let url = format!(
        "http://127.0.0.1:{port}/?json=1&n={limit}&path_column=1&s={}",
        urlencoding::encode(query)
    );
    let Ok(resp) = client().get(&url).send() else {
        eprintln!("zap: files 请求失败: {url}");
        PORT.lock().expect("port poisoned").take();
        mark_failed();
        return Vec::new();
    };
    let status = resp.status();
    let Ok(value) = resp.json::<serde_json::Value>() else {
        eprintln!("zap: files JSON 解析失败（HTTP {status}）");
        return Vec::new();
    };

    let Some(results) = value.get("results").and_then(serde_json::Value::as_array) else {
        eprintln!("zap: files 响应无 results 字段（HTTP {status}）");
        return Vec::new();
    };
    let items: Vec<SearchItem> = results
        .iter()
        .filter_map(|r| {
            let name = r.get("name")?.as_str()?.to_string();
            let dir = r.get("path")?.as_str()?.to_string();
            Some((name, dir))
        })
        .enumerate()
        .map(|(i, (name, dir))| {
            let full = if dir.ends_with('\\') {
                format!("{dir}{name}")
            } else {
                format!("{dir}\\{name}")
            };
            SearchItem {
                id: format!("file:{i}"),
                title: name,
                subtitle: dir,
                kind: "file".into(),
                target: full,
                icon: None,
                score: 45.0 - i as f32,
            }
        })
        .collect();
    items
}

/// 全盘文件源插件适配（Everything HTTP）。搜索热路径上含 250ms 超时的网络请求，
/// 保持 needs_conn = false：不得在 clip 锁内执行
pub struct FilesPlugin;

impl super::plugin::Plugin for FilesPlugin {
    fn search(&self, query: &str, _conn: Option<&rusqlite::Connection>) -> Vec<SearchItem> {
        search(query, 8)
    }
}
