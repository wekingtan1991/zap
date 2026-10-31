//! 设置链路 owner（自 engine.rs 拆出，审计 C3；域内聚本体是评审 P1-18）：
//! 每组设置的「存档→原子量」装载与「参数→原子量+存档」应用收敛为单一来源，
//! lib.rs 的 setup / save_settings / 托盘三写者不再各自镜像「读键→解析→store」
//! 三步。键值读写基元（get/set/set_many）与三组 owner 同域同文件。
//! 兄弟 provider 经 use super::* 引入；方法仍挂 SearchEngine——调用方零感知

use std::sync::atomic::Ordering;

use super::*;

impl SearchEngine {
    pub fn get_setting(&self, key: &str) -> Option<String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        appconfig::get(&conn, key)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        appconfig::set(&conn, key, value)
    }

    /// 批量写设置：单锁单事务（save_settings 等多处连写场景）。逐条 autocommit
    /// 的 WAL fsync 累积会拉长同步命令的主线程窗口（评审 P1-07）；失败整批
    /// 回滚，原子量动作由调用方在写库前原位完成（失败时内存态与存档短暂
    /// 分叉，重启即归一——与原实现的中途部分写同等级）
    pub fn set_many(&self, kvs: &[(&str, String)]) -> Result<(), String> {
        let conn = self.clip.lock().expect("clip db poisoned");
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| e.to_string())?;
        for (k, v) in kvs {
            if let Err(e) = appconfig::set(&conn, k, v) {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
        conn.execute_batch("COMMIT").map_err(|e| e.to_string())
    }

    // ---------- 设置链路 owner（评审 P1-18）----------
    // 每组设置的「存档→原子量」装载与「参数→原子量+存档」应用收敛为单一
    // 来源：lib.rs 的 setup / save_settings / 托盘三写者不再各自镜像
    // 「读键→解析→store」三步（漏一处即静默失效的镜像面由此收敛）。
    // DTO 下发（get_app_settings 读原子量）与 config 导出不在镜像面上，不动

    /// 剪贴板三项（记录开关/保留条数/选中即粘贴）
    pub fn load_clip_settings(&self) -> clipboard::ClipSettings {
        let conn = self.clip.lock().expect("clip db poisoned");
        let s = clipboard::ClipSettings {
            enabled: appconfig::get(&conn, appconfig::KEY_CLIP_ENABLED)
                .map(|v| v != "0")
                .unwrap_or(true),
            max: appconfig::get(&conn, appconfig::KEY_CLIP_MAX)
                .and_then(|v| v.parse().ok())
                .unwrap_or(appconfig::DEFAULT_CLIP_MAX),
            autopaste: appconfig::get(&conn, appconfig::KEY_CLIP_AUTOPASTE)
                .map(|v| v != "0")
                .unwrap_or(appconfig::DEFAULT_CLIP_AUTOPASTE),
        };
        drop(conn);
        clipboard::CLIP_ENABLED.store(s.enabled, Ordering::Relaxed);
        clipboard::RETENTION_LIMIT.store(s.max, Ordering::Relaxed);
        clipboard::PASTE_TO_PREV.store(s.autopaste, Ordering::Relaxed);
        s
    }

    /// 应用剪贴板三项：原子量 + 单事务落库（参数即最终值，clamp 由调用方先行）
    pub fn apply_clip_settings(&self, s: &clipboard::ClipSettings) -> Result<(), String> {
        clipboard::CLIP_ENABLED.store(s.enabled, Ordering::Relaxed);
        clipboard::RETENTION_LIMIT.store(s.max, Ordering::Relaxed);
        clipboard::PASTE_TO_PREV.store(s.autopaste, Ordering::Relaxed);
        self.set_many(&[
            (
                appconfig::KEY_CLIP_ENABLED,
                if s.enabled { "1" } else { "0" }.into(),
            ),
            (appconfig::KEY_CLIP_MAX, s.max.to_string()),
            (
                appconfig::KEY_CLIP_AUTOPASTE,
                if s.autopaste { "1" } else { "0" }.into(),
            ),
        ])
    }

    /// 托盘「记录剪贴板历史」单键翻转（其余两项不动，load 后改单字段再
    /// apply 亦可，单键高频路径给直用形态）
    pub fn set_clip_enabled(&self, on: bool) -> Result<(), String> {
        clipboard::CLIP_ENABLED.store(on, Ordering::Relaxed);
        self.set_setting(appconfig::KEY_CLIP_ENABLED, if on { "1" } else { "0" })
    }

    /// 中转站容量装载（10–500 之外视为坏值回退默认）；容量收敛
    /// （stage_enforce_cap）的时机仍由调用方决定
    pub fn load_stage_cap(&self) -> i64 {
        let conn = self.clip.lock().expect("clip db poisoned");
        // 10–500 之外视为坏值回退默认（与 save 侧 clamp 同一口径）
        let cap = appconfig::get(&conn, appconfig::KEY_STAGE_CAP)
            .and_then(|v| v.parse().ok())
            .filter(|v| (10..=500).contains(v))
            .unwrap_or(appconfig::DEFAULT_STAGE_CAP);
        stage::CAP.store(cap, Ordering::Relaxed);
        cap
    }

    /// 应用中转站容量（clamp 后原子量 + 落库）
    pub fn apply_stage_cap(&self, cap: i64) -> Result<(), String> {
        let cap = cap.clamp(10, 500);
        stage::CAP.store(cap, Ordering::Relaxed);
        self.set_setting(appconfig::KEY_STAGE_CAP, &cap.to_string())
    }

    /// 拖拽落区条三项装载（开关/放后开面板/停靠角；position 非法存档值在
    /// set_position_setting 内部静默保持默认角）
    pub fn load_stagedrop_settings(&self) -> stagedrop::StagedropSettings {
        let conn = self.clip.lock().expect("clip db poisoned");
        let s = stagedrop::StagedropSettings {
            enabled: appconfig::get(&conn, appconfig::KEY_STAGEDROP_ENABLED)
                .map(|v| v != "0")
                .unwrap_or(appconfig::DEFAULT_STAGEDROP_ENABLED),
            open_panel: appconfig::get(&conn, appconfig::KEY_STAGEDROP_OPEN_PANEL)
                .map(|v| v != "0")
                .unwrap_or(appconfig::DEFAULT_STAGEDROP_OPEN_PANEL),
            position: appconfig::get(&conn, appconfig::KEY_STAGEDROP_POSITION)
                .unwrap_or_else(|| appconfig::DEFAULT_STAGEDROP_POSITION.into()),
        };
        drop(conn);
        stagedrop::STAGEDROP_ENABLED.store(s.enabled, Ordering::Relaxed);
        stagedrop::STAGEDROP_OPEN_PANEL.store(s.open_panel, Ordering::Relaxed);
        stagedrop::set_position_setting(&s.position);
        s
    }

    /// 应用落区条三项。position 与 save 原口径一致：非法值静默跳过该项
    /// （不应用不落库），enabled/open_panel 照常生效
    pub fn apply_stagedrop_settings(&self, s: &stagedrop::StagedropSettings) -> Result<(), String> {
        stagedrop::STAGEDROP_ENABLED.store(s.enabled, Ordering::Relaxed);
        stagedrop::STAGEDROP_OPEN_PANEL.store(s.open_panel, Ordering::Relaxed);
        let position_valid = stagedrop::Corner::parse(&s.position).is_some();
        if position_valid {
            stagedrop::set_position_setting(&s.position);
        }
        let mut kvs = vec![
            (
                appconfig::KEY_STAGEDROP_ENABLED,
                if s.enabled { "1" } else { "0" }.into(),
            ),
            (
                appconfig::KEY_STAGEDROP_OPEN_PANEL,
                if s.open_panel { "1" } else { "0" }.into(),
            ),
        ];
        if position_valid {
            kvs.push((appconfig::KEY_STAGEDROP_POSITION, s.position.clone()));
        }
        self.set_many(&kvs)
    }
}
