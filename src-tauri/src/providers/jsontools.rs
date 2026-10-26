//! JSON 工具插件：万能输入框识别 + 搜索入口条目 + 剪贴板识别 + 美化/压缩。
//! 识别条目只带类型摘要、不携带内容（列表会明文渲染 title/subtitle）；
//! target 三态：q = 以当前输入框文本为工作台初始内容，entry = 入口进入等粘贴，
//! 数字 = 剪贴板行号（Enter 后 clip_row 回查原文）

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};

use super::SearchItem;

/// 单条处理上限：超出直接拒绝（树形渲染与 IPC 都不为大文本买单）
pub const MAX_ANALYZE_BYTES: usize = 256 * 1024;

/// 运行时开关：启动从 appconfig 加载、设置保存时同步（热路径只读原子量）
pub static JSON_TOOLS_ON: AtomicBool = AtomicBool::new(true);

fn disabled_msg() -> String {
    "JSON 工具已停用，可在设置 → 插件中开启".into()
}

/// 停用守卫：开关关闭时所有入口（识别/分析/格式化/转换）一致拒绝
fn ensure_enabled() -> Result<(), String> {
    if JSON_TOOLS_ON.load(Ordering::Relaxed) {
        Ok(())
    } else {
        Err(disabled_msg())
    }
}

pub struct JsonToolsPlugin;

impl JsonToolsPlugin {
    /// 入口条目：搜索命中与兜底清单共用同一构造，文案不漂移
    fn entry_item() -> SearchItem {
        SearchItem {
            id: "json:entry".into(),
            title: "JSON 工具".into(),
            subtitle: "粘贴 JSON 实时校验 · 美化 / 压缩 / 转 YAML / TS 类型".into(),
            kind: "json".into(),
            // entry = 入口进入：工作台空内容等待粘贴
            target: "entry".into(),
            icon: None,
            score: 0.0,
        }
    }
}

impl super::plugin::Plugin for JsonToolsPlugin {
    fn search(&self, query: &str, _conn: Option<&rusqlite::Connection>) -> Vec<SearchItem> {
        let words = self.meta().map(|m| m.entries).unwrap_or(&[]);
        if ensure_enabled().is_err() || !super::plugin::entry_hit(query, words) {
            return Vec::new();
        }
        vec![Self::entry_item()]
    }

    fn fallback_entry(&self) -> Vec<SearchItem> {
        if ensure_enabled().is_ok() {
            vec![Self::entry_item()]
        } else {
            Vec::new()
        }
    }

    fn meta(&self) -> Option<super::plugin::PluginMeta> {
        Some(super::plugin::PluginMeta {
            id: "json-tools",
            title: "JSON 工具",
            hint: "粘贴/键入 JSON 即时识别（美化/压缩/校验/YAML/TS），搜索入口词进入",
            key: "plugin_json_tools",
            default_on: true,
            state: &JSON_TOOLS_ON,
            // 入口词声明化：搜索能命中的词都在这里，设置页据此展示
            entries: &["json", "json工具", "格式化", "美化", "压缩", "校验", "yaml"],
        })
    }

    fn detect(&self, clip_rowid: i64, clip_text: &str) -> Vec<SearchItem> {
        if ensure_enabled().is_err() {
            return Vec::new();
        }
        let Some(v) = parse_clip_json(clip_text) else {
            return Vec::new();
        };
        vec![SearchItem {
            id: format!("json:clip:{clip_rowid}"),
            title: format!("检测到剪贴板 JSON（{}）", describe(&v)),
            subtitle: format!(
                "JSON 工具 · {} · 美化 / 压缩 / 校验，Enter 打开",
                human_size(clip_text.trim().len())
            ),
            kind: "json".into(),
            // target = 剪贴板行号：前端进视图后经 clip_row 回查当时内容
            target: clip_rowid.to_string(),
            icon: None,
            score: 0.0,
        }]
    }

    /// 万能输入框识别：形似 JSON 才参与——合法出识别条目；形似但非法
    /// （{ 或 [ 开头、≥4 字符）出「疑似」条目，Enter 进工作台看行列错误。
    /// 比 detect（呼出剪贴板仅认合法）宽一档：输入内容是用户当下的明确意图
    fn probe(&self, text: &str) -> Vec<SearchItem> {
        if ensure_enabled().is_err() {
            return Vec::new();
        }
        let t = text.trim();
        if t.len() > MAX_ANALYZE_BYTES || !(t.starts_with('{') || t.starts_with('[')) {
            return Vec::new();
        }
        match serde_json::from_str::<serde_json::Value>(t) {
            Ok(v) => vec![SearchItem {
                id: "json:q".into(),
                title: format!("识别到输入 JSON（{}）", describe(&v)),
                subtitle: format!(
                    "JSON 工具 · {} · 美化 / 压缩 / 校验，Enter 打开",
                    human_size(t.len())
                ),
                kind: "json".into(),
                target: "q".into(),
                icon: None,
                score: 0.0,
            }],
            Err(_) if t.chars().count() >= 4 => vec![SearchItem {
                id: "json:q".into(),
                title: "疑似 JSON · 校验未通过".into(),
                subtitle: "JSON 工具 · Enter 打开查看错误位置".into(),
                kind: "json".into(),
                target: "q".into(),
                icon: None,
                score: 0.0,
            }],
            Err(_) => Vec::new(),
        }
    }
}

/// 剪贴板识别的准入：形似 JSON（{ 或 [ 开头）且合法且不超限才识别。
/// 无效片段保持沉默——呼出面板是「顺手发现」场景，诊断请走 "json" 前缀域
fn parse_clip_json(text: &str) -> Option<serde_json::Value> {
    let t = text.trim();
    if t.len() > MAX_ANALYZE_BYTES {
        return None;
    }
    if !(t.starts_with('{') || t.starts_with('[')) {
        return None;
    }
    serde_json::from_str(t).ok()
}

fn describe(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Object(_) => "对象",
        serde_json::Value::Array(_) => "数组",
        _ => "JSON",
    }
}

fn human_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// 分析报告：仅元信息，不回传内容（原文前端已有，或按行号回查）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonReport {
    pub valid: bool,
    /// 对象 / 数组 / 标量；无效时为空串
    pub kind: String,
    /// 字节数（trim 后）
    pub bytes: usize,
    /// 无效时的可读错误（含行列号）
    pub error: Option<String>,
}

/// 分析文本：校验 + 类型判定 + 错误定位
pub fn analyze(text: &str) -> Result<JsonReport, String> {
    ensure_enabled()?;
    let t = text.trim();
    if t.len() > MAX_ANALYZE_BYTES {
        return Ok(JsonReport {
            valid: false,
            kind: String::new(),
            bytes: t.len(),
            error: Some(oversize_msg(t.len())),
        });
    }
    Ok(match serde_json::from_str::<serde_json::Value>(t) {
        Ok(v) => JsonReport {
            valid: true,
            kind: match v {
                serde_json::Value::Object(_) => "对象".into(),
                serde_json::Value::Array(_) => "数组".into(),
                _ => "标量".into(),
            },
            bytes: t.len(),
            error: None,
        },
        Err(e) => JsonReport {
            valid: false,
            kind: String::new(),
            bytes: t.len(),
            error: Some(format_json_error(&e)),
        },
    })
}

/// 解析前置：停用守卫 + 大小上限 + serde 解析（格式化/转换共用）
fn parse_limited(text: &str) -> Result<serde_json::Value, String> {
    ensure_enabled()?;
    let t = text.trim();
    if t.len() > MAX_ANALYZE_BYTES {
        return Err(oversize_msg(t.len()));
    }
    serde_json::from_str(t).map_err(|e| format_json_error(&e))
}

/// 美化（pretty = true，2 空格缩进）/ 压缩
pub fn format_text(text: &str, pretty: bool) -> Result<String, String> {
    let v = parse_limited(text)?;
    if pretty {
        serde_json::to_string_pretty(&v)
    } else {
        serde_json::to_string(&v)
    }
    .map_err(|e| e.to_string())
}

/// JSON → YAML
pub fn to_yaml(text: &str) -> Result<String, String> {
    let v = parse_limited(text)?;
    serde_yaml_ng::to_string(&v).map_err(|e| e.to_string())
}

/// JSON → TypeScript 类型定义。
/// 对象生成 interface（按键路径 PascalCase 命名），数组聚合成员类型；
/// 同形接口复用同一名称，同名异体自动加序号
pub fn to_ts(text: &str) -> Result<String, String> {
    let v = parse_limited(text)?;
    let mut em = TsEmitter {
        emitted: Vec::new(),
    };
    let root_type = em.type_of("Root", &v);
    let mut out = String::new();
    // 根为对象时 interface Root 已在 emitted 里；标量/数组根补 type 别名
    if !matches!(v, serde_json::Value::Object(_)) {
        out.push_str(&format!("export type Root = {root_type};\n\n"));
    }
    // Root 放最前（递归生成时子接口先入列）
    em.emitted.sort_by_key(|(name, _)| name != "Root");
    for (name, body) in &em.emitted {
        out.push_str(&format!("export interface {name} {body}\n\n"));
    }
    Ok(out.trim_end().to_string())
}

/// TS 类型发射器：emitted 累积已生成的 (接口名, 接口体)
struct TsEmitter {
    emitted: Vec<(String, String)>,
}

impl TsEmitter {
    /// 返回该值的类型表达式；对象/数组元素接口在此过程中入列
    fn type_of(&mut self, hint: &str, v: &serde_json::Value) -> String {
        match v {
            serde_json::Value::Object(map) => {
                let mut body = String::from("{\n");
                for (k, val) in map {
                    let t = self.type_of(&format!("{hint} {k}"), val);
                    let field = if is_plain_ident(k) {
                        k.clone()
                    } else {
                        format!("\"{}\"", k.replace('"', "\\\""))
                    };
                    body.push_str(&format!("  {field}: {t};\n"));
                }
                body.push('}');
                let base = pascal(hint);
                // 同体复用（数组元素同形是最常见情形；结构相同的不同路径对象也合并）
                if let Some((n, _)) = self.emitted.iter().find(|(_, b)| b == &body) {
                    return n.clone();
                }
                // 同名异体加序号
                let mut name = base.clone();
                let mut i = 2;
                while self.emitted.iter().any(|(n, _)| n == &name) {
                    name = format!("{base}{i}");
                    i += 1;
                }
                self.emitted.push((name.clone(), body));
                name
            }
            serde_json::Value::Array(arr) => {
                if arr.is_empty() {
                    return "unknown[]".into();
                }
                let mut members: Vec<String> = Vec::new();
                for (i, item) in arr.iter().enumerate() {
                    let hint = if i == 0 {
                        format!("{hint}Item")
                    } else {
                        format!("{hint}Item{i}")
                    };
                    let t = self.type_of(&hint, item);
                    if !members.contains(&t) {
                        members.push(t);
                    }
                }
                if members.len() == 1 {
                    format!("{}[]", members[0])
                } else {
                    format!("({})[]", members.join(" | "))
                }
            }
            serde_json::Value::String(_) => "string".into(),
            serde_json::Value::Number(_) => "number".into(),
            serde_json::Value::Bool(_) => "boolean".into(),
            serde_json::Value::Null => "null".into(),
        }
    }
}

/// 合法 TS 标识符（无需引号的字段名）
fn is_plain_ident(s: &str) -> bool {
    !s.is_empty()
        && !s.chars().next().is_some_and(|c| c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// 空白分词后逐词首字母大写拼接；数字开头补下划线（TS 接口名约束）
fn pascal(s: &str) -> String {
    let mut out = String::new();
    for part in s.split_whitespace() {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if out.is_empty() {
        out.push_str("Field");
    }
    out
}

fn oversize_msg(bytes: usize) -> String {
    format!(
        "内容 {} 超过 {} KB 上限，仅支持校验不参与美化/压缩",
        human_size(bytes),
        MAX_ANALYZE_BYTES / 1024
    )
}

/// serde 错误 → 带行列的可读摘要（Display 自带的 "at line X column Y" 英文尾巴换成中文前置）
fn format_json_error(e: &serde_json::Error) -> String {
    let raw = e.to_string();
    let msg = match raw.find(" at line ") {
        Some(i) => raw[..i].to_string(),
        None => raw,
    };
    format!("第 {} 行第 {} 列：{}", e.line(), e.column(), msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::plugin::Plugin;
    use std::sync::Mutex;

    /// 串行化：所有入口都读全局开关原子量，停用测试的变更窗口
    /// 与其他用例并发时会互踩（同 mod.rs plugin_tests 的教训）
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn 识别合法对象与数组() {
        let _g = TEST_LOCK.lock().unwrap();
        let items = JsonToolsPlugin.detect(7, "  {\"a\": 1}  ");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, "json");
        assert_eq!(items[0].target, "7", "target 应为剪贴板行号");
        assert!(items[0].title.contains("对象"));

        let items = JsonToolsPlugin.detect(8, "[1, 2, 3]");
        assert_eq!(items.len(), 1);
        assert!(items[0].title.contains("数组"));
    }

    #[test]
    fn 忽略非json标量与无效片段() {
        let _g = TEST_LOCK.lock().unwrap();
        for text in ["hello world", "42", "\"quoted\"", "{ broken", "", "   "] {
            assert!(
                JsonToolsPlugin.detect(1, text).is_empty(),
                "「{text}」不应识别"
            );
        }
    }

    #[test]
    fn 入口词命中出条目() {
        let _g = TEST_LOCK.lock().unwrap();
        // 前缀（正在输入）/ 全词 / 中文 / 拼音全拼与首字母，与常规搜索同一打分器
        for q in ["json", "js", "JSON", "格式化", "geshihua", "yaml", "美化"] {
            let items = JsonToolsPlugin.search(q, None);
            assert_eq!(items.len(), 1, "「{q}」应命中入口");
            assert_eq!(items[0].id, "json:entry");
            assert_eq!(items[0].kind, "json");
            assert_eq!(items[0].target, "entry");
        }
    }

    #[test]
    fn 更长的词不再命中入口() {
        let _g = TEST_LOCK.lock().unwrap();
        // 前缀域退役后的关键行为：jsonview 是在搜别的东西，不劫持、不出条目
        for q in ["jsonview", "json格式化工具下载", "yamllint", "j", ""] {
            assert!(
                JsonToolsPlugin.search(q, None).is_empty(),
                "「{q}」不应命中"
            );
        }
    }

    #[test]
    fn 万能输入框识别合法与疑似() {
        let _g = TEST_LOCK.lock().unwrap();
        let items = JsonToolsPlugin.probe(" {\"a\": 1} ");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].target, "q", "识别条目应携带 q 源标记");
        assert!(items[0].title.contains("对象"));
        // 形似但非法（≥4 字符）出疑似档：Enter 进工作台看行列错误
        let items = JsonToolsPlugin.probe("{\"a\": 1");
        assert_eq!(items.len(), 1);
        assert!(items[0].title.contains("疑似"));
        // 普通文本与过短片段不识别
        assert!(JsonToolsPlugin.probe("hello world").is_empty());
        assert!(JsonToolsPlugin.probe("{\"a").is_empty());
    }

    #[test]
    fn 停用后入口与识别拒绝() {
        let _g = TEST_LOCK.lock().unwrap();
        JSON_TOOLS_ON.store(false, Ordering::Relaxed);
        let search_off = JsonToolsPlugin.search("json", None);
        let probe_off = JsonToolsPlugin.probe("{\"a\":1}");
        JSON_TOOLS_ON.store(true, Ordering::Relaxed);
        assert!(search_off.is_empty());
        assert!(probe_off.is_empty());
    }

    #[test]
    fn 超长内容不识别() {
        let _g = TEST_LOCK.lock().unwrap();
        let big = format!("[{}]", "1,".repeat(200 * 1024));
        assert!(big.len() > MAX_ANALYZE_BYTES);
        assert!(JsonToolsPlugin.detect(1, &big).is_empty());
    }

    #[test]
    fn 停用后识别与分析全部拒绝() {
        let _g = TEST_LOCK.lock().unwrap();
        JSON_TOOLS_ON.store(false, Ordering::Relaxed);
        let detect_off = JsonToolsPlugin.detect(1, "{\"a\":1}");
        let analyze_off = analyze("{\"a\":1}");
        let format_off = format_text("{\"a\":1}", true);
        let yaml_off = to_yaml("{\"a\":1}");
        let ts_off = to_ts("{\"a\":1}");
        JSON_TOOLS_ON.store(true, Ordering::Relaxed);
        assert!(detect_off.is_empty());
        assert!(analyze_off.is_err());
        assert!(format_off.is_err());
        assert!(yaml_off.is_err());
        assert!(ts_off.is_err());
        // 恢复后同一路径可用
        assert!(analyze("{\"a\":1}").is_ok());
    }

    #[test]
    fn 分析报告含类型与字节数() {
        let _g = TEST_LOCK.lock().unwrap();
        let rep = analyze("{\"a\": 1}").expect("分析成功");
        assert!(rep.valid);
        assert_eq!(rep.kind, "对象");
        assert_eq!(rep.bytes, 8);
        assert!(rep.error.is_none());
    }

    #[test]
    fn 分析错误带行列() {
        let _g = TEST_LOCK.lock().unwrap();
        let rep = analyze("{\n  \"a\": 1\n  \"b\": 2\n}").expect("返回报告");
        assert!(!rep.valid);
        let err = rep.error.expect("应有错误");
        assert!(err.contains("第 3 行"), "错误应定位到第 3 行：{err}");
    }

    #[test]
    fn 美化与压缩互逆() {
        let _g = TEST_LOCK.lock().unwrap();
        let src = "{\"a\":1,\"b\":[1,2]}";
        let pretty = format_text(src, true).expect("美化成功");
        assert_eq!(pretty, "{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2\n  ]\n}");
        let compact = format_text(&pretty, false).expect("压缩成功");
        assert_eq!(compact, src);
    }

    #[test]
    fn 格式化错误返回可读信息() {
        let _g = TEST_LOCK.lock().unwrap();
        let err = format_text("{bad", true).expect_err("应失败");
        assert!(err.contains("第 1 行"), "错误应含行列：{err}");
    }

    #[test]
    fn yaml转换扁平与嵌套() {
        let _g = TEST_LOCK.lock().unwrap();
        assert_eq!(to_yaml("{\"a\": 1}").expect("yaml"), "a: 1\n");
        assert_eq!(to_yaml("[1, 2]").expect("yaml"), "- 1\n- 2\n");
        assert_eq!(
            to_yaml("{\"a\": {\"b\": \"x\"}}").expect("yaml"),
            "a:\n  b: x\n"
        );
    }

    #[test]
    fn ts类型生成嵌套对象() {
        let _g = TEST_LOCK.lock().unwrap();
        let ts = to_ts("{\"id\":1,\"user\":{\"name\":\"a\"}}").expect("ts");
        // Root 接口置顶输出
        assert_eq!(
            ts,
            "export interface Root {\n  id: number;\n  user: RootUser;\n}\n\nexport interface RootUser {\n  name: string;\n}"
        );
    }

    #[test]
    fn ts类型生成数组去重同形元素() {
        let _g = TEST_LOCK.lock().unwrap();
        let ts = to_ts("{\"items\":[{\"a\":1},{\"a\":2},{\"b\":true}]}").expect("ts");
        // 前两个元素同形 → 复用 RootItemsItem；第三个异形 → RootItemsItem2，联合数组
        assert!(ts.contains("export interface RootItemsItem {\n  a: number;\n}"));
        assert!(ts.contains("export interface RootItemsItem2 {\n  b: boolean;\n}"));
        assert!(ts.contains("items: (RootItemsItem | RootItemsItem2)[];"));
    }

    #[test]
    fn ts类型生成标量根与特殊键名() {
        let _g = TEST_LOCK.lock().unwrap();
        assert_eq!(to_ts("[1,2]").expect("ts"), "export type Root = number[];");
        assert_eq!(to_ts("\"s\"").expect("ts"), "export type Root = string;");
        let ts = to_ts("{\"weird-key\":1,\"2fa\":true,\"ok\":null}").expect("ts");
        assert!(ts.contains("\"weird-key\": number;"));
        assert!(ts.contains("\"2fa\": boolean;"));
        assert!(ts.contains("ok: null;"));
    }
}
