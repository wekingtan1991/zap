//! 本地端口占用查询（Windows IP Helper）。
//! 采集侧只读：一次取 IPv4/IPv6 两张 TCP owner 表，补进程名/路径/启动时间，
//! 按 (端口, PID) 合并双栈监听行。结束进程（ports_kill）在命令层，本模块不写系统状态。
//!
//! 取表与进程查询都是阻塞调用，调用方必须走 spawn_blocking（见 commands.rs blocking!）。
//! 日志只写端口/PID/错误码，不落进程路径——路径属本机敏感信息（scripts/check-secrets.mjs）。

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

#[cfg(windows)]
use windows::Win32::{
    Foundation::{CloseHandle, FILETIME},
    NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCP_STATE_LISTEN, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID,
        MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID, TCP_TABLE_CLASS, TCP_TABLE_OWNER_PID_ALL,
        TCP_TABLE_OWNER_PID_LISTENER,
    },
    Networking::WinSock::{AF_INET, AF_INET6},
    System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        TerminateProcess,
    },
};

/// IP Helper 的返回码是裸 u32（windows 0.62 的 GetExtendedTcpTable -> u32）
const NO_ERROR: u32 = 0;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
/// 表缓冲上限：正常机器几 KB，异常值直接拒绝，防畸形 size 撑爆内存
const MAX_TABLE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ROWS: usize = 16_384;
const TABLE_RETRIES: usize = 5;
/// 1601-01-01 → 1970-01-01 的毫秒间隔（FILETIME 转 Unix 用）
const EPOCH_DIFF_MS: i64 = 11_644_473_600_000;

/// 端口占用行（线协议：camelCase 命名字段，不用位置元组）。
/// 前端镜像见 src/plugins/ports/types.ts；process_path/started_at 供确认弹窗
/// 判断「是不是我的 Java 项目」，不参与列表主展示。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortEntry {
    pub port: u16,
    /// "IPv4" / "IPv6" / "IPv4 + IPv6"（同一 PID 双栈监听合并为一行）
    pub family: String,
    pub state: String,
    pub local_addr: String,
    pub pid: u32,
    pub process_name: String,
    pub process_path: Option<String>,
    /// 进程创建时间（Unix 毫秒）；kill 前复核，防 PID 复用
    pub started_at: Option<i64>,
    /// 系统/自身进程：前端禁用「结束」，后端 kill 再挡一次
    pub protected: bool,
}

/// 表内原始行（尚未补进程信息、尚未合并双栈）
#[derive(Debug, Clone, PartialEq)]
struct RawRow {
    port: u16,
    family: &'static str,
    state: u32,
    local_addr: String,
    pid: u32,
}

#[derive(Default)]
struct ProcMeta {
    name: String,
    path: Option<String>,
    started_at: Option<i64>,
}

// ---------- 纯函数（可跨平台单测） ----------

/// dwLocalPort 的低 16 位是网络字节序：8080 = 0x1F90 → 0x901F
pub fn net_port_to_host(raw: u32) -> u16 {
    u16::from_be(raw as u16)
}

pub fn state_name(state: u32) -> &'static str {
    match state {
        1 => "CLOSED",
        2 => "LISTEN",
        3 => "SYN_SENT",
        4 => "SYN_RCVD",
        5 => "ESTAB",
        6 => "FIN_WAIT_1",
        7 => "FIN_WAIT_2",
        8 => "CLOSE_WAIT",
        9 => "CLOSING",
        10 => "LAST_ACK",
        11 => "TIME_WAIT",
        12 => "DELETE_TCB",
        _ => "UNKNOWN",
    }
}

/// 系统关键进程判定：PID + 进程名 + 路径三重，名字可被同名进程冒用，路径才是硬证据。
/// name 需已转小写；路径取 QueryFullProcessImageNameW 的结果。
pub fn is_protected(pid: u32, name: &str, path: Option<&str>) -> bool {
    if pid < 5 || pid == std::process::id() {
        return true;
    }
    const CRITICAL: &[&str] = &[
        "system",
        "smss.exe",
        "csrss.exe",
        "wininit.exe",
        "services.exe",
        "lsass.exe",
        "winlogon.exe",
        "svchost.exe",
        "fontdrvhost.exe",
        "dwm.exe",
        "audiodg.exe",
    ];
    if CRITICAL.contains(&name) {
        return true;
    }
    path.is_some_and(|p| {
        let lower = p.to_ascii_lowercase();
        lower.contains("\\windows\\system32\\") || lower.contains("\\windows\\syswow64\\")
    })
}

/// 万能输入框 / Java 报错文本里的端口提取。**只做字符串判定，绝不枚举系统表**
/// （engine.rs:520-522 的 probe 契约：每次击键都会跑一遍 probe_all）。
/// 覆盖：`8080`、`:8080`、`127.0.0.1:8080`、`[::]:8080`、
/// `Port 8080 was already in use`、`8080 端口被占用`。
pub fn parse_port_query(text: &str) -> Option<u16> {
    let t = text.trim();
    if t.is_empty() || t.len() > 256 {
        return None;
    }
    if t.chars().all(|c| c.is_ascii_digit()) {
        return parse_port_number(t);
    }
    // 首个词是端口：「8080 端口」「8080 already in use」
    if let Some(first) = t.split_whitespace().next()
        && let Some(p) = parse_port_number(first)
    {
        return Some(p);
    }
    let lower = t.to_ascii_lowercase();
    if let Some(pos) = lower.find("port") {
        let rest = &t[pos + 4..];
        if let Some(p) = first_port_number(rest) {
            return Some(p);
        }
    }
    // IPv4/IPv6 地址或 "bind: 8080" 形态：取最后一个冒号后的第一段数字
    if let Some(pos) = t.rfind(':') {
        let rest = &t[pos + 1..];
        if let Some(p) = first_port_number(rest) {
            return Some(p);
        }
    }
    None
}

/// 只认 2–5 位数字：单字符降噪，6 位以上必超 65535
fn parse_port_number(s: &str) -> Option<u16> {
    if !(2..=5).contains(&s.len()) || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u16>().ok().filter(|p| *p > 0)
}

fn first_port_number(s: &str) -> Option<u16> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if let Some(p) = parse_port_number(&s[start..i]) {
                return Some(p);
            }
        } else {
            i += 1;
        }
    }
    None
}

fn read_count(buf: &[u8]) -> Result<usize, String> {
    let head: [u8; 4] = buf
        .get(..4)
        .and_then(|s| s.try_into().ok())
        .ok_or("端口表头不完整")?;
    Ok(u32::from_ne_bytes(head) as usize)
}

/// 行数与缓冲边界校验：表来自内核但 size/row count 都可能在我们读取之间变化，
/// 不校验就按 count 指针步进 = 越界读。
fn check_bounds(count: usize, offset: usize, stride: usize, len: usize) -> Result<(), String> {
    if count > MAX_ROWS {
        return Err(format!("端口表行数异常（{count}）"));
    }
    let need = count
        .checked_mul(stride)
        .and_then(|n| n.checked_add(offset))
        .ok_or("端口表长度溢出")?;
    if need > len {
        return Err(format!(
            "端口表缓冲越界（需要 {need} 字节，实际 {len} 字节）"
        ));
    }
    Ok(())
}

// ---------- Windows 采集 ----------

#[cfg(windows)]
fn read_table(af: u32, class: TCP_TABLE_CLASS) -> Result<Vec<u8>, String> {
    let mut size: u32 = 0;
    unsafe {
        GetExtendedTcpTable(None, &mut size, false, af, class, 0);
    }
    if size == 0 {
        return Ok(Vec::new());
    }
    if size as usize > MAX_TABLE_BYTES {
        return Err(format!("端口表大小异常（{size} 字节）"));
    }
    // Vec<u64> 保证 8 字节对齐；MIB_*TABLE 的行结构要求至少 4 字节对齐
    let mut words = vec![0u64; (size as usize).div_ceil(8).max(1)];
    for _ in 0..TABLE_RETRIES {
        let mut needed = (words.len() * 8) as u32;
        let rc = unsafe {
            GetExtendedTcpTable(
                Some(words.as_mut_ptr().cast()),
                &mut needed,
                false,
                af,
                class,
                0,
            )
        };
        match rc {
            NO_ERROR => {
                let len = (needed as usize).min(words.len() * 8);
                let bytes = unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), len) };
                return Ok(bytes.to_vec());
            }
            ERROR_INSUFFICIENT_BUFFER => {
                if needed as usize > MAX_TABLE_BYTES {
                    return Err(format!("端口表大小异常（{needed} 字节）"));
                }
                words.resize((needed as usize).div_ceil(8).max(1), 0);
            }
            other => return Err(format!("GetExtendedTcpTable 失败（Win32 {other}）")),
        }
    }
    Err("端口表持续变化，请重试".into())
}

#[cfg(windows)]
fn parse_v4(buf: &[u8]) -> Result<Vec<RawRow>, String> {
    if buf.is_empty() {
        return Ok(Vec::new());
    }
    let count = read_count(buf)?;
    let offset = std::mem::offset_of!(MIB_TCPTABLE_OWNER_PID, table);
    let stride = std::mem::size_of::<MIB_TCPROW_OWNER_PID>();
    check_bounds(count, offset, stride, buf.len())?;
    let mut rows = Vec::with_capacity(count);
    for i in 0..count {
        let row: MIB_TCPROW_OWNER_PID = unsafe {
            buf.as_ptr()
                .add(offset + i * stride)
                .cast::<MIB_TCPROW_OWNER_PID>()
                .read_unaligned()
        };
        rows.push(RawRow {
            port: net_port_to_host(row.dwLocalPort),
            family: "IPv4",
            state: row.dwState,
            local_addr: std::net::Ipv4Addr::from(row.dwLocalAddr.to_ne_bytes()).to_string(),
            pid: row.dwOwningPid,
        });
    }
    Ok(rows)
}

#[cfg(windows)]
fn parse_v6(buf: &[u8]) -> Result<Vec<RawRow>, String> {
    if buf.is_empty() {
        return Ok(Vec::new());
    }
    let count = read_count(buf)?;
    let offset = std::mem::offset_of!(MIB_TCP6TABLE_OWNER_PID, table);
    let stride = std::mem::size_of::<MIB_TCP6ROW_OWNER_PID>();
    check_bounds(count, offset, stride, buf.len())?;
    let mut rows = Vec::with_capacity(count);
    for i in 0..count {
        let row: MIB_TCP6ROW_OWNER_PID = unsafe {
            buf.as_ptr()
                .add(offset + i * stride)
                .cast::<MIB_TCP6ROW_OWNER_PID>()
                .read_unaligned()
        };
        rows.push(RawRow {
            port: net_port_to_host(row.dwLocalPort),
            family: "IPv6",
            state: row.dwState,
            local_addr: std::net::Ipv6Addr::from(row.ucLocalAddr).to_string(),
            pid: row.dwOwningPid,
        });
    }
    Ok(rows)
}

#[cfg(windows)]
pub fn filetime_to_unix_ms(ft: FILETIME) -> Option<i64> {
    let raw = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    if raw == 0 {
        return None;
    }
    let ms = (raw / 10_000) as i64;
    let ms = ms.checked_sub(EPOCH_DIFF_MS)?;
    (ms >= 0).then_some(ms)
}

#[cfg(windows)]
fn process_started_at(pid: u32) -> Option<i64> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(handle);
        ok.then(|| filetime_to_unix_ms(creation)).flatten()
    }
}

#[cfg(windows)]
fn process_meta(pid: u32) -> ProcMeta {
    let path = super::sysutil::process_path_of_pid(pid);
    let name = path
        .as_deref()
        .and_then(|p| Path::new(p).file_name())
        .map(|n| n.to_string_lossy().to_lowercase())
        // 取不到路径 = 权限受限或系统进程：给可见兜底，并在 list_ports 里一律保护
        .unwrap_or_else(|| "(unknown)".into());
    ProcMeta {
        name,
        path,
        started_at: process_started_at(pid),
    }
}

/// 列出端口占用。only_listen=true 走 LISTENER 表类（默认，Java 端口冲突就是它），
/// false 走 ALL 取全部连接状态；filter 按端口精确过滤。
/// IPv6 表取不到只降级告警（部分系统禁用 IPv6），IPv4 失败是硬失败。
#[cfg(windows)]
pub fn list_ports(filter: Option<u16>, only_listen: bool) -> Result<Vec<PortEntry>, String> {
    let class = if only_listen {
        TCP_TABLE_OWNER_PID_LISTENER
    } else {
        TCP_TABLE_OWNER_PID_ALL
    };
    let mut rows = Vec::new();
    for (af, label) in [(AF_INET.0 as u32, "IPv4"), (AF_INET6.0 as u32, "IPv6")] {
        let parsed = read_table(af, class).and_then(|buf| {
            if label == "IPv4" {
                parse_v4(&buf)
            } else {
                parse_v6(&buf)
            }
        });
        match parsed {
            Ok(r) => rows.extend(r),
            Err(e) => {
                super::diag::log_warn(&format!("ports\t{label}\t{e}"));
                if label == "IPv4" {
                    return Err(e);
                }
            }
        }
    }

    let mut grouped: HashMap<(u16, u32), PortEntry> = HashMap::new();
    let mut metas: HashMap<u32, ProcMeta> = HashMap::new();
    for r in rows {
        if let Some(want) = filter
            && r.port != want
        {
            continue;
        }
        let meta = metas.entry(r.pid).or_insert_with(|| process_meta(r.pid));
        // 身份不明（拿不到完整路径）一律不可结束：宁可少杀，不可误杀
        let protected =
            meta.path.is_none() || is_protected(r.pid, &meta.name, meta.path.as_deref());
        let key = (r.port, r.pid);
        match grouped.get_mut(&key) {
            Some(entry) => {
                if !entry.family.contains(r.family) {
                    entry.family = format!("{} + {}", entry.family, r.family);
                }
                // 双栈里只要有一侧 LISTEN，整行按 LISTEN 展示
                if r.state == MIB_TCP_STATE_LISTEN.0 as u32 {
                    entry.state = "LISTEN".into();
                }
            }
            None => {
                grouped.insert(
                    key,
                    PortEntry {
                        port: r.port,
                        family: r.family.to_string(),
                        state: state_name(r.state).to_string(),
                        local_addr: r.local_addr,
                        pid: r.pid,
                        process_name: meta.name.clone(),
                        process_path: meta.path.clone(),
                        started_at: meta.started_at,
                        protected,
                    },
                );
            }
        }
    }
    let mut out: Vec<PortEntry> = grouped.into_values().collect();
    out.sort_by(|a, b| a.port.cmp(&b.port).then(a.pid.cmp(&b.pid)));
    Ok(out)
}

#[cfg(not(windows))]
pub fn list_ports(_filter: Option<u16>, _only_listen: bool) -> Result<Vec<PortEntry>, String> {
    Err("端口查询仅支持 Windows".into())
}

/// 结束占用端口的进程。本模块唯一写系统状态的地方；六道校验后强制终止。
/// 只读采集在 list_ports，命令层（ports_kill）只做转发。
#[cfg(windows)]
pub fn kill_port(port: u16, pid: u32, started_at: Option<i64>) -> Result<String, String> {
    // ① 重查端口归属：面板上的行可能已过期，PID 可能已被复用
    let rows = list_ports(Some(port), true)?;
    let row = rows
        .iter()
        .find(|r| r.port == port && r.pid == pid)
        .ok_or_else(|| format!("端口 {port} 已无该进程占用，无需结束"))?;
    // ② 启动时间复核：防 PID 复用杀错进程
    if row.started_at != started_at {
        return Err(format!("端口 {port} 的占用进程已变化，请刷新后再试"));
    }
    // ③ 保护名单后端再挡一次，不信前端
    if row.protected {
        return Err(format!("PID {pid} 属于系统/受保护进程，不能结束"));
    }
    let name = row.process_name.clone();

    // ④⑤ 打开句柄 → 强制终止 → 句柄必须归还（与 sysutil.rs:27-45 同款）
    let result = unsafe {
        let handle = match OpenProcess(PROCESS_TERMINATE, false, pid) {
            Ok(h) => h,
            Err(e) => {
                return Err(format!(
                    "无法打开 PID {pid}：{e}。若为『拒绝访问』，请以管理员身份运行 zap 后重试"
                ));
            }
        };
        let r = TerminateProcess(handle, 1);
        let _ = CloseHandle(handle);
        r
    };
    result.map_err(|e| {
        format!("结束 PID {pid} 失败：{e}。若为『拒绝访问』，请以管理员身份运行 zap 后重试")
    })?;

    // ⑥ 最多 1s 复查端口是否释放；复查失败不推翻已成功的终止
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        match list_ports(Some(port), true) {
            Ok(now) if !now.iter().any(|r| r.port == port && r.pid == pid) => {
                return Ok(format!("已结束 {name}（PID {pid}），端口 {port} 已释放"));
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(format!(
        "已结束 {name}（PID {pid}），但端口 {port} 仍在占用，请点刷新复核"
    ))
}

#[cfg(not(windows))]
pub fn kill_port(_port: u16, _pid: u32, _started_at: Option<i64>) -> Result<String, String> {
    Err("端口查询仅支持 Windows".into())
}

// ---------- 插件协议 ----------

/// 设置页「插件」分区的开关态：启动时由 engine.rs 从 appconfig 加载，
/// 热路径只读原子量，不查库（与 texttools::TEXT_TOOLS_ON 同款）
pub static PORTS_ON: AtomicBool = AtomicBool::new(true);

fn ensure_enabled() -> Result<(), String> {
    if PORTS_ON.load(Ordering::Relaxed) {
        Ok(())
    } else {
        Err("端口查询已停用，可在设置 → 插件中开启".into())
    }
}

pub struct PortsPlugin;

impl PortsPlugin {
    /// 入口条目：search 命中与空面板兜底共用同一构造，文案不漂移
    fn entry_item() -> super::SearchItem {
        super::SearchItem {
            id: "port:entry".into(),
            title: "端口查询".into(),
            subtitle: "查找端口占用并结束进程 · 输入 8080 或 port 直达".into(),
            kind: "port".into(),
            // all = 不预置端口过滤
            target: "all".into(),
            icon: None,
            pinned: false,
            score: 0.0,
        }
    }
}

impl super::plugin::Plugin for PortsPlugin {
    fn search(&self, query: &str, _conn: Option<&rusqlite::Connection>) -> Vec<super::SearchItem> {
        let words = self.meta().map(|m| m.entries).unwrap_or(&[]);
        if ensure_enabled().is_err() || !super::plugin::entry_hit(query, words) {
            return Vec::new();
        }
        vec![Self::entry_item()]
    }

    /// 万能输入框识别：只做字符串判定（parse_port_query），**不枚举系统表**。
    /// engine.rs:520-522 的 probe 契约是「每击键跑一遍廉价判定」。
    /// target = 端口号，前端进工作台时作为初始过滤条件。
    fn probe(&self, text: &str) -> Vec<super::SearchItem> {
        if ensure_enabled().is_err() {
            return Vec::new();
        }
        let Some(port) = parse_port_query(text) else {
            return Vec::new();
        };
        vec![super::SearchItem {
            id: "port:q".into(),
            title: format!("端口 {port} · 查占用并结束进程"),
            subtitle: "回车查看占用进程；确认后可强制结束".into(),
            kind: "port".into(),
            target: port.to_string(),
            icon: None,
            pinned: false,
            score: 0.0,
        }]
    }

    fn fallback_entry(&self) -> Vec<super::SearchItem> {
        if ensure_enabled().is_ok() {
            vec![Self::entry_item()]
        } else {
            Vec::new()
        }
    }

    fn meta(&self) -> Option<super::plugin::PluginMeta> {
        Some(super::plugin::PluginMeta {
            id: "ports",
            title: "端口查询",
            hint: "输入端口号查占用进程并结束；支持粘贴 Java 端口占用报错",
            key: "plugin_ports",
            default_on: true,
            state: &PORTS_ON,
            // 入口词声明化：设置页据此展示；避开 tool/url/json 等既有词
            entries: &["port", "ports", "端口", "端口占用", "查端口", "killport"],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::plugin::Plugin;

    /// 串行化对全局开关的读改窗口（与 texttools 的 TEST_LOCK 同款）
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn 入口词命中出条目() {
        let _g = TEST_LOCK.lock().unwrap();
        for q in ["port", "po", "端口", "dk", "查端口"] {
            let items = PortsPlugin.search(q, None);
            assert_eq!(items.len(), 1, "「{q}」应命中入口");
            assert_eq!(items[0].id, "port:entry");
            assert_eq!(items[0].kind, "port");
            assert_eq!(items[0].target, "all");
        }
    }

    #[test]
    fn 更长的词不命中入口() {
        let _g = TEST_LOCK.lock().unwrap();
        for q in ["portfolio", "transport", "p", ""] {
            assert!(
                PortsPlugin.search(q, None).is_empty(),
                "「{q}」不应命中入口"
            );
        }
    }

    #[test]
    fn 端口识别条目带端口目标() {
        let _g = TEST_LOCK.lock().unwrap();
        for text in [
            "8080",
            "127.0.0.1:8080",
            "Web server failed to start. Port 8080 was already in use.",
        ] {
            let items = PortsPlugin.probe(text);
            assert_eq!(items.len(), 1, "「{text}」应识别");
            assert_eq!(items[0].kind, "port");
            assert_eq!(items[0].target, "8080");
            assert!(items[0].title.contains("8080"));
        }
        assert!(PortsPlugin.probe("hello").is_empty());
        // 10 位时间戳不是端口，别抢文本工具的识别
        assert!(PortsPlugin.probe("1791828000").is_empty());
    }

    #[test]
    fn 停用后入口识别与兜底全拒绝() {
        let _g = TEST_LOCK.lock().unwrap();
        PORTS_ON.store(false, Ordering::Relaxed);
        let a = PortsPlugin.search("port", None);
        let b = PortsPlugin.probe("8080");
        let c = PortsPlugin.fallback_entry();
        PORTS_ON.store(true, Ordering::Relaxed);
        assert!(a.is_empty());
        assert!(b.is_empty());
        assert!(c.is_empty());
    }

    #[test]
    fn 兜底入口与meta一致() {
        let _g = TEST_LOCK.lock().unwrap();
        let items = PortsPlugin.fallback_entry();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "port:entry");
        let meta = PortsPlugin.meta().unwrap();
        assert_eq!(meta.id, "ports");
        assert_eq!(meta.key, "plugin_ports");
        assert!(meta.default_on);
        assert!(meta.entries.contains(&"port"));
    }

    #[test]
    fn 端口文本解析覆盖常见形态() {
        for s in [
            "8080",
            ":8080",
            "localhost:8080",
            "127.0.0.1:8080",
            "0.0.0.0:8080",
            "[::]:8080",
            "8080 端口被占用",
            "Web server failed to start. Port 8080 was already in use.",
            "Port 8080 was already in use",
            "bind: address already in use: 8080",
            "65535",
        ] {
            assert_eq!(
                parse_port_query(s),
                Some(if s.contains("65535") { 65535 } else { 8080 }),
                "应识别「{s}」"
            );
        }
    }

    #[test]
    fn 端口文本解析拒绝无关输入() {
        for s in [
            "",
            "hello",
            "hello world",
            "1",
            "65536",
            "123456",
            "99999",
            "java.net.BindException: Address already in use: bind",
            "port",
            "a".repeat(300).as_str(),
        ] {
            assert!(parse_port_query(s).is_none(), "不应识别「{s}」");
        }
    }

    #[test]
    fn 网络字节序端口换算() {
        assert_eq!(net_port_to_host(0x901F), 8080);
        assert_eq!(net_port_to_host(0xBB01), 443);
        assert_eq!(net_port_to_host(0x0000), 0);
        assert_eq!(state_name(2), "LISTEN");
        assert_eq!(state_name(5), "ESTAB");
        assert_eq!(state_name(11), "TIME_WAIT");
        assert_eq!(state_name(99), "UNKNOWN");
    }

    #[test]
    fn 保护判定按pid名路径三重() {
        assert!(is_protected(0, "", None));
        assert!(is_protected(4, "system", None));
        assert!(!is_protected(999, "", None));
        assert!(is_protected(
            std::process::id(),
            "java.exe",
            Some("D:\\dev\\java.exe")
        ));
        assert!(is_protected(
            1234,
            "svchost.exe",
            Some("D:\\fake\\svchost.exe")
        ));
        assert!(is_protected(
            1234,
            "java.exe",
            Some("C:\\Windows\\System32\\java.exe")
        ));
        assert!(!is_protected(
            1234,
            "java.exe",
            Some("D:\\dev\\jdk\\bin\\java.exe")
        ));
        // 同名冒用：名字不在清单、路径不在系统目录 → 不保护
        assert!(!is_protected(
            1234,
            "myapp.exe",
            Some("D:\\fake\\lsass.exe")
        ));
    }

    #[test]
    fn 缓冲边界校验拦住畸形表() {
        assert_eq!(check_bounds(1, 4, 24, 28), Ok(()));
        assert!(check_bounds(2, 4, 24, 28).is_err(), "行数超出缓冲");
        assert!(check_bounds(MAX_ROWS + 1, 4, 24, usize::MAX).is_err());
        assert!(read_count(&[1, 0]).is_err());
        assert_eq!(read_count(&1u32.to_ne_bytes()), Ok(1));
    }

    #[cfg(windows)]
    #[test]
    fn filetime换算为毫秒() {
        // 1601-01-01 → 1970-01-01 的 100ns 间隔
        let raw = 116_444_736_000_000_000u64;
        let ft = FILETIME {
            dwLowDateTime: raw as u32,
            dwHighDateTime: (raw >> 32) as u32,
        };
        assert_eq!(filetime_to_unix_ms(ft), Some(0));
        assert_eq!(filetime_to_unix_ms(FILETIME::default()), None);
    }

    #[cfg(windows)]
    #[test]
    fn 解析v4表缓冲的偏移与字节序() {
        let row = MIB_TCPROW_OWNER_PID {
            dwState: MIB_TCP_STATE_LISTEN.0 as u32,
            dwLocalAddr: 0x0100_007F, // 127.0.0.1 的主机字节序
            dwLocalPort: 0x901F,      // 8080 的网络字节序
            dwRemoteAddr: 0,
            dwRemotePort: 0,
            dwOwningPid: 4242,
        };
        let offset = std::mem::offset_of!(MIB_TCPTABLE_OWNER_PID, table);
        let stride = std::mem::size_of::<MIB_TCPROW_OWNER_PID>();
        let mut buf = vec![0u8; offset + stride];
        buf[..4].copy_from_slice(&1u32.to_ne_bytes());
        let row_bytes = unsafe {
            std::slice::from_raw_parts(&row as *const MIB_TCPROW_OWNER_PID as *const u8, stride)
        };
        buf[offset..].copy_from_slice(row_bytes);
        let rows = parse_v4(&buf).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].port, 8080);
        assert_eq!(rows[0].local_addr, "127.0.0.1");
        assert_eq!(rows[0].pid, 4242);
        assert_eq!(state_name(rows[0].state), "LISTEN");
        assert_eq!(rows[0].family, "IPv4");
    }

    #[cfg(windows)]
    #[test]
    fn 解析v6表缓冲的偏移() {
        let row = MIB_TCP6ROW_OWNER_PID {
            ucLocalAddr: [0u8; 16], // ::
            dwLocalPort: 0x901F,
            dwState: MIB_TCP_STATE_LISTEN.0 as u32,
            dwOwningPid: 7777,
            ..Default::default()
        };
        let offset = std::mem::offset_of!(MIB_TCP6TABLE_OWNER_PID, table);
        let stride = std::mem::size_of::<MIB_TCP6ROW_OWNER_PID>();
        let mut buf = vec![0u8; offset + stride];
        buf[..4].copy_from_slice(&1u32.to_ne_bytes());
        let row_bytes = unsafe {
            std::slice::from_raw_parts(&row as *const MIB_TCP6ROW_OWNER_PID as *const u8, stride)
        };
        buf[offset..].copy_from_slice(row_bytes);
        let rows = parse_v6(&buf).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].port, 8080);
        assert_eq!(rows[0].local_addr, "::");
        assert_eq!(rows[0].pid, 7777);
    }

    /// 真机对照用：cargo test --lib ports::tests::真机端口表 -- --ignored --nocapture
    /// ① 自绑定端口验证取表/字节序/PID 闭环；② 与 netstat -ano -p tcp 自动对照，
    /// 不写死任何机器相关的端口或 PID。
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn 真机端口表() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定测试端口");
        let port = listener.local_addr().unwrap().port();
        let mine = list_ports(Some(port), true).expect("按端口取表应成功");
        assert!(
            mine.iter()
                .any(|r| { r.port == port && r.pid == std::process::id() && r.state == "LISTEN" }),
            "自绑定端口 {port} 应出现在表中，实际 {mine:?}"
        );
        drop(listener);

        let rows = list_ports(None, true).expect("取表应成功");
        let ours: std::collections::HashSet<(u16, u32)> =
            rows.iter().map(|r| (r.port, r.pid)).collect();
        let out = std::process::Command::new("netstat")
            .args(["-ano", "-p", "tcp"])
            .output()
            .expect("netstat 应可执行");
        let text = String::from_utf8_lossy(&out.stdout);
        let mut checked = 0;
        let mut missing = Vec::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 5 || !f[3].eq_ignore_ascii_case("LISTENING") {
                continue;
            }
            let Some((_, port_s)) = f[1].rsplit_once(':') else {
                continue;
            };
            let Ok(p) = port_s.parse::<u16>() else {
                continue;
            };
            let Ok(pid) = f[4].parse::<u32>() else {
                continue;
            };
            checked += 1;
            if !ours.contains(&(p, pid)) {
                missing.push((p, pid));
            }
        }
        println!(
            "LISTEN 行数：{} / netstat 行数：{checked} / 差异：{missing:?}",
            rows.len()
        );
        for r in rows.iter().take(15) {
            println!(
                "{}\t{}\tpid={}\t{}\tprotected={}",
                r.port, r.family, r.pid, r.process_name, r.protected
            );
        }
        assert!(rows.iter().all(|r| r.state == "LISTEN"));
        assert!(missing.is_empty(), "netstat 有而我们漏掉：{missing:?}");
    }

    /// 真机端到端：真实子进程占用端口 → 查询 → 结束 → 端口释放。
    /// cargo test --lib ports::tests::真机结束占用端口的进程 -- --ignored --nocapture
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn 真机结束占用端口的进程() {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let script = format!(
            "import socket,time; s=socket.socket(); s.bind(('127.0.0.1',{port})); s.listen(5); time.sleep(30)"
        );
        let mut child = std::process::Command::new("python")
            .args(["-c", &script])
            .spawn()
            .expect("python 应可用于真机用例");
        let pid = child.id();
        let mut found = None;
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if let Ok(rows) = list_ports(Some(port), true)
                && let Some(r) = rows.iter().find(|r| r.port == port && r.pid == pid)
            {
                found = Some(r.clone());
                break;
            }
        }
        let row = found.unwrap_or_else(|| {
            let _ = child.kill();
            panic!("子进程 PID {pid} 应出现在端口 {port} 的占用表里")
        });
        assert!(!row.protected, "普通 python 子进程不应被保护");
        let msg = kill_port(port, pid, row.started_at).expect("应能结束子进程");
        println!("{msg}");
        let still = list_ports(Some(port), true)
            .unwrap()
            .iter()
            .any(|r| r.port == port && r.pid == pid);
        assert!(!still, "结束后端口 {port} 应不再被 PID {pid} 占用");
        let _ = child.wait();
    }
}
