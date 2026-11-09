//! 搜索引擎的域 facade：剪贴板 / 图标 / 应用索引 / 中转站 / 账号库 / 备份。
//! 从 engine.rs 原样搬出（P3.1，零逻辑变化）——Rust 允许 `impl` 分文件，
//! 且子模块可访问父模块的私有字段与 conn() 方法。
//! 引擎装配、搜索编排与测试仍留在 engine.rs。

use super::super::*;

impl SearchEngine {
    /// 搜索捷径清单（设置页「搜索捷径」分区）
    pub fn shortcuts_list(&self) -> Vec<shortcuts::Shortcut> {
        self.shortcuts.list()
    }

    /// 整单保存搜索捷径：校验 + 写库 + 热替换搜索缓存
    pub fn shortcuts_save(&self, list: Vec<shortcuts::Shortcut>) -> Result<(), String> {
        let conn = self.conn();
        self.shortcuts.save(&conn, list)
    }

    pub fn vault_search(&self, query: &str) -> Vec<SearchItem> {
        let conn = self.conn();
        vault::search(&conn, query)
    }

    /// 按行号取剪贴板原文（JSON 识别条目进工作台时回查当时的内容）
    pub fn clip_row(&self, id: i64) -> Option<String> {
        let conn = self.conn();
        clipboard::get_by_id(&conn, id)
    }

    /// 行号存在性探查（open_item 两段式第一段，评审 P1-07）：毫秒级 PK 点查
    /// 保「已被清理」即时报错语义；全文回查留给粘贴的后台线程
    pub fn clip_row_exists(&self, id: i64) -> bool {
        let conn = self.conn();
        conn.query_row("SELECT 1 FROM clipboard_history WHERE id=?1", [id], |_| {
            Ok(())
        })
        .is_ok()
    }

    /// 按行号删剪贴板条目：kind 分流两表——两表自增行号独立，clipimg 必须打
    /// 图片表（与 clipimg_pin 同款撞号陷阱）。固定条目允许显式删除（用户
    /// 意图优先于固定豁免），返回是否确有删除
    pub fn clip_delete_row(&self, kind: &str, id: i64) -> Result<bool, String> {
        let conn = self.conn();
        let table = if kind == "clipimg" {
            "clipboard_images"
        } else {
            "clipboard_history"
        };
        let n = conn
            .execute(&format!("DELETE FROM {table} WHERE id=?1"), [id])
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    }

    /// 清空未固定剪贴板历史（文本+图片两表）：手动复制的敏感内容的用户补救
    /// 通道（评审 P1-09），返回删除总条数
    pub fn clip_clear_unpinned(&self) -> Result<usize, String> {
        let conn = self.conn();
        let n1 = conn
            .execute("DELETE FROM clipboard_history WHERE pinned=0", [])
            .map_err(|e| e.to_string())?;
        let n2 = conn
            .execute("DELETE FROM clipboard_images WHERE pinned=0", [])
            .map_err(|e| e.to_string())?;
        Ok(n1 + n2)
    }

    /// 退出前 WAL checkpoint（TRUNCATE）：把 -wal 文件收缩回主库（评审 P1-08）。
    /// busy 连接（另一连接还在写）时静默放弃——下次退出再收
    pub fn wal_checkpoint(&self) {
        let conn = self.conn();
        let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
    }

    /// 按给定顺序取多条剪贴板全文（合并粘贴用）：个别行号已被清理则跳过，
    /// 返回按入参顺序的存留文本。锁内纯库操作
    pub fn clip_texts(&self, ids: &[i64]) -> Vec<String> {
        let conn = self.conn();
        ids.iter()
            .filter_map(|id| clipboard::get_by_id(&conn, *id))
            .collect()
    }

    /// 最近一条剪贴板内容（快速粘贴直达热键用）
    pub fn latest_clip(&self) -> Option<String> {
        let conn = self.conn();
        clipboard::latest(&conn).map(|(_, text)| text)
    }

    /// 按行号取剪贴板图片全图（clipimg 条目回贴用）
    pub fn clip_image(&self, id: i64) -> Option<(u32, u32, Vec<u8>)> {
        let conn = self.conn();
        clipimage::get(&conn, id)
    }

    /// 按行号取剪贴板图片全图与元信息（预览面板用）：pinned 是固定按钮初始态
    pub fn clip_image_meta(&self, id: i64) -> Option<clipimage::ImageFull> {
        let conn = self.conn();
        clipimage::get_full(&conn, id)
    }

    /// 图片表固定/取消固定：clip_pin 走「文本表先命中」的兜底链路，两表
    /// 自增行号会撞号（文本 id=5 存在时图片行号 5 永远轮不到）——图片条目
    /// 必须经此直连图片表，前端按 kind 分流
    pub fn clip_image_pin(&self, id: i64) -> Result<bool, String> {
        let conn = self.conn();
        clipimage::pin_toggle(&conn, id)
    }

    /// 剪贴板图片行号是否仍存在：open_item 的同步错误路径用，不拖 PNG 大字段
    pub fn clip_image_exists(&self, id: i64) -> bool {
        let conn = self.conn();
        clipimage::exists(&conn, id)
    }

    /// 后台重建应用索引（面板呼出时调用）。10 秒内已扫过则跳过，
    /// 避免连续呼出反复走目录树 + UWP 枚举；false = 本次跳过或上次未完成
    pub fn refresh_apps(&self) -> bool {
        self.apps.index.refresh_if_stale(apps::REFRESH_MIN_INTERVAL)
    }

    /// 后台重建「最近文件」索引（面板呼出时调用，10 秒节流与 apps 同窗）：
    /// 启动器常驻数日，只靠启动快照会越来越陈旧
    pub fn refresh_recents(&self) -> bool {
        self.recents.refresh_if_stale(apps::REFRESH_MIN_INTERVAL)
    }

    /// admin 提权的来源校验：只放行当前应用索引里的路径。
    /// 前端 admin 本来就只对 app kind 显示（App.tsx:498），后端补上这道复核，
    /// 避免被篡改的 IPC 参数把任意可执行文件以 runas 拉起。
    pub fn is_known_app(&self, target: &str) -> bool {
        self.apps.index.contains_path(target)
    }

    /// open_item 记一次应用启动（frecency 排序依据，按 target 路径计数）
    pub fn record_app_launch(&self, target: &str) {
        let conn = self.conn();
        apps::record_launch(&conn, target);
    }

    /// 剪贴板条目固定/取消固定（返回操作后的状态）
    pub fn clip_pin(&self, id: i64) -> Result<bool, String> {
        let conn = self.conn();
        clipboard::pin_toggle(&conn, id)
    }

    // ---------- 中转站 ----------
    // 锁纪律：磁盘 IO（resolve 的 metadata/lnk 解析、finish 的存在性+图标）
    // 一律在锁外，锁内只做纯库操作——网络路径的 stat 可达秒级

    pub fn stage_list(&self) -> Vec<stage::StageRecord> {
        let rows = {
            let conn = self.conn();
            stage::rows(&conn)
        };
        stage::finish(rows)
    }

    /// 批量放入：逐条尝试，个别失败不影响其余（失败明细回传前端提示）
    pub fn stage_add(&self, paths: &[String]) -> stage::StageAddOutcome {
        let resolved = stage::resolve_paths(paths);
        let conn = self.conn();
        stage::commit_add(&conn, resolved)
    }

    pub fn stage_remove(&self, id: i64) -> Result<(), String> {
        let conn = self.conn();
        stage::remove(&conn, id)
    }

    /// 批量移出（清理失效条目用）：返回实际移出条数，不存在的 id 忽略
    pub fn stage_remove_batch(&self, ids: &[i64]) -> usize {
        let conn = self.conn();
        stage::remove_batch(&conn, ids)
    }

    /// 设置调小容量后立即收敛已有超容条目（平时由放入路径顺带淘汰）
    pub fn stage_enforce_cap(&self) {
        let conn = self.conn();
        stage::enforce_cap(&conn);
    }

    pub fn stage_clear(&self) -> Result<usize, String> {
        let conn = self.conn();
        stage::clear(&conn)
    }

    pub fn stage_pin(&self, id: i64) -> Result<bool, String> {
        let conn = self.conn();
        stage::pin_toggle(&conn, id)
    }

    pub fn stage_relocate(&self, id: i64, path: &str) -> Result<(), String> {
        let (p, is_dir) = stage::resolve_relocate(path)?;
        let conn = self.conn();
        stage::commit_relocate(&conn, id, &p, is_dir)
    }

    /// 批量撤销恢复（清空/移出撤销共用）：逐条幂等插回（已重新放入的路径
    /// 跳过），返回恢复条数
    pub fn stage_restore_batch(&self, items: Vec<stage::RestoreItem>) -> Result<usize, String> {
        let conn = self.conn();
        Ok(stage::restore_batch(&conn, &items))
    }

    pub fn vault_add(
        &self,
        title: &str,
        username: &str,
        password: &str,
        url: &str,
        notes: &str,
        totp: &str,
    ) -> Result<(), String> {
        let conn = self.conn();
        let r = vault::add(&conn, title, username, password, url, notes, totp);
        if r.is_ok() {
            // 新密码若与历史同值（用户复制过后才想起入库），顺手清掉
            let pwd = password.trim().to_string();
            if !pwd.is_empty() {
                vault::purge_clip_history(&conn, &[pwd]);
            }
            vault::invalidate_password_cache();
        }
        r
    }

    pub fn vault_action(
        &self,
        id: i64,
        action: &str,
        pwd_clear_secs: i64,
    ) -> Result<String, String> {
        let conn = self.conn();
        vault::action(&conn, id, action, pwd_clear_secs)
    }

    pub fn vault_get(&self, id: i64) -> Result<vault::VaultRecord, String> {
        let conn = self.conn();
        vault::get(&conn, id)
    }

    // 账号字段一字排开（同 vault::update 的理由），显式放行参数个数提示
    #[allow(clippy::too_many_arguments)]
    pub fn vault_update(
        &self,
        id: i64,
        title: &str,
        username: &str,
        password: &str,
        url: &str,
        notes: &str,
        pinned: i64,
        totp: &str,
    ) -> Result<(), String> {
        let conn = self.conn();
        let r = vault::update(
            &conn, id, title, username, password, url, notes, pinned, totp,
        );
        if r.is_ok() {
            // 改密后旧值不再可从历史翻出；密码集缓存同步失效
            let pwd = password.trim().to_string();
            if !pwd.is_empty() {
                vault::purge_clip_history(&conn, &[pwd]);
            }
            vault::invalidate_password_cache();
        }
        r
    }

    pub fn vault_import(&self, entries: Vec<vault::ParsedAccount>) -> Result<usize, String> {
        // 导入的密码若与历史同值（用户此前手动复制过），按原文等值清掉（评审 P1-09）
        let pwds: Vec<String> = entries
            .iter()
            .map(|e| e.password.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        let conn = self.conn();
        let n = vault::import(&conn, entries)?;
        vault::purge_clip_history(&conn, &pwds);
        vault::invalidate_password_cache();
        Ok(n)
    }

    /// 加密导出全部账号到备份文件，返回条目数。
    /// 两段式（2026-11-04 P1）：锁内只捞密文行；DPAPI 逐条解密、60 万轮
    /// PBKDF2 与写盘全部在 conn 锁外——原先全程闭锁 ~400ms，期间 watcher
    /// 写库被挡会掉条、击键搜索全部排队。时间戳记录单独取一次连接（短持锁）
    pub fn vault_backup_export(&self, path: &str, passphrase: &str) -> Result<usize, String> {
        // 先校边界再跑 60 万轮 PBKDF2：路径非法不该白等半秒
        let dest = crate::providers::pathguard::export_path(
            path,
            crate::providers::pathguard::EXT_VAULT_BACKUP,
        )?;
        let encrypted = {
            let conn = self.conn();
            vault::backup_fetch_encrypted(&conn)?
        };
        let entries = vault::backup_decrypt_rows(encrypted)?;
        let content = backup::encrypt_backup(&entries, passphrase)?;
        std::fs::write(&dest, content).map_err(|e| e.to_string())?;
        // 落盘成功才记时间：设置页「上次备份 N 天前 / 30 天提醒」的事实源
        // （备份文件走用户自选路径，zap 无注册表，库里这个时间戳就是唯一凭证）
        let conn = self.conn();
        let now = util::now_ms();
        let _ = appconfig::set(&conn, appconfig::KEY_VAULT_LAST_EXPORT, &now.to_string());
        Ok(entries.len())
    }

    /// 解密备份文件为预览：条目 + 每条是否与现有账号疑似重复（恢复前交前端确认）
    pub fn vault_backup_decrypt(
        &self,
        path: &str,
        passphrase: &str,
    ) -> Result<backup::BackupPreview, String> {
        let src = crate::providers::pathguard::import_path(
            path,
            crate::providers::pathguard::EXT_VAULT_BACKUP,
        )?;
        let content = std::fs::read_to_string(&src).map_err(|e| e.to_string())?;
        let entries = backup::decrypt_backup(&content, passphrase)?;
        let duplicates = {
            let conn = self.conn();
            vault::backup_duplicates(&conn, &entries)
        };
        Ok(backup::BackupPreview {
            entries,
            duplicates,
        })
    }

    /// 备份恢复：skip_duplicates 时跳过疑似重复，返回 (写入数, 跳过数)
    pub fn vault_backup_restore(
        &self,
        entries: Vec<backup::BackupEntry>,
        skip_duplicates: bool,
    ) -> Result<(usize, usize), String> {
        // 恢复进来的密码同样按原文清历史同值行 + 失效密码缓存（评审 P1-09）
        let pwds: Vec<String> = entries
            .iter()
            .map(|e| e.password.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        let conn = self.conn();
        let r = vault::backup_restore(&conn, entries, skip_duplicates);
        if r.is_ok() {
            vault::purge_clip_history(&conn, &pwds);
            vault::invalidate_password_cache();
        }
        r
    }
}
