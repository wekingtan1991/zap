//! 拖拽出（Drag Out）：从 zap 直接把条目拖给其他应用（docs/design-drag-out.md v3）
//!
//! PoC + 两轮真机实证的硬约束：
//! 1. `DoDragDrop` 的 `IDropSource` 参数不可为 None（缺它 = CO_E_NOTINITIALIZED）
//! 2. 线程必须 `OleInitialize`（只 CoInitializeEx 的线程 OLE 层未建，同样报未初始化）
//! 3. 文件数据对象 = SHParseDisplayName → SHBindToParent → 父 folder GetUIObjectOf；
//!    禁止 SHCreateDataObject(None, [绝对 pidl])（CF_HDROP 路径无效，目标全拒绝）
//! 4. 调用前必须释放源窗口隐式捕获
//! 5. **拖拽必须在「鼠标按下捕获所在的 GUI 线程」跑**——这是两轮真机用血换的：
//!    专用线程（哪怕建窗 + 加消息泵）`DoDragDrop` 都会进模态循环却感知不到鼠标
//!    事件、阻塞不返回（用户「根本拖不动」）。WebView2 的按下捕获在主线程，
//!    故本模块的拖拽流程由命令层经 `run_on_main_thread` 在主线程执行；
//!    `DoDragDrop` 内部自带泵，主线程 UI 不冻结（Windows 拖拽的标准行为）。
//!
//! 载荷分两路（2026-11-06 用户裁决：文本条目「只给文字，不给 .txt」）：
//! - `clip` 文本：自有 IDataObject 的 **CF_UNICODETEXT**，零临时文件、零 Shell
//!   PIDL 解析，松手即把文字插进目标应用；不再产出 .txt 文件，也不再受
//!   「临时文件 10s 后删除、异步读目标扑空」影响。
//! - `clipimg` 图片 / `stage` 文件：沿用 Shell IDataObject 的 CF_HDROP。
//!
//! 去掉写盘 + Shell COM 冷启动后，文本起拖链路从实测 ~95ms 降到 ~10ms
//! （zap.log 首次 begin→entering 与后续 9–31ms 的差值即冷启动成本）。

use std::path::PathBuf;
#[cfg(windows)]
use std::sync::atomic::AtomicBool;

#[cfg(windows)]
use crate::providers::SearchEngine;

/// 拖出会话互斥 + stagedrop/中转站拖入的全局门（一个标志堵三个口，设计稿 §二）
#[cfg(windows)]
pub static DRAG_OUT_ACTIVE: AtomicBool = AtomicBool::new(false);

/// 拖出结束后面板是否复现：effect=0 = 取消（Esc / 拖回原处 / 禁止目标 /
/// OLE 未收）→ 面板在原位置复现；非 0 = 已落地 → 保持隐藏等下次呼出。
/// 提纯成纯函数便于契约测试（同 panel::should_hide_on_blur 的决策提纯纪律）。
#[cfg(windows)]
pub(crate) fn should_restore_panel(effect: u32) -> bool {
    effect == 0
}

#[cfg(all(test, windows))]
mod tests {
    use super::should_restore_panel;

    #[test]
    fn 取消才恢复面板_落地保持隐藏() {
        assert!(should_restore_panel(0));
        assert!(!should_restore_panel(1)); // DROPEFFECT_COPY
        assert!(!should_restore_panel(2)); // DROPEFFECT_MOVE 等非 COPY 落地
    }
}

/// 一次拖出的载荷。Text 不落盘；File 的 `temporary` 决定结束后是否延迟清理。
#[cfg(windows)]
pub enum DragPayload {
    Text(String),
    File { path: PathBuf, temporary: bool },
}

#[cfg(windows)]
impl DragPayload {
    /// 需要延迟删除的临时文件（Text 与 stage 原件都返回 None）
    pub fn temp_file(&self) -> Option<PathBuf> {
        match self {
            Self::File {
                path,
                temporary: true,
            } => Some(path.clone()),
            _ => None,
        }
    }
}

#[cfg(windows)]
pub(crate) fn left_button_held() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 }
}

#[cfg(windows)]
mod ffi {
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};

    use windows::Win32::Foundation::{
        DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, DV_E_FORMATETC, E_FAIL,
        E_NOTIMPL, GlobalFree, HWND, S_FALSE, S_OK,
    };
    use windows::Win32::System::Com::{
        DATADIR_GET, DVASPECT_CONTENT, FORMATETC, IAdviseSink, IDataObject, IDataObject_Impl,
        IEnumFORMATETC, IEnumFORMATETC_Impl, IEnumSTATDATA, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
    };
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows::Win32::System::Ole::{
        CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY, DoDragDrop, IDropSource, IDropSource_Impl,
        OleInitialize,
    };
    use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
    use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    use windows::Win32::UI::Shell::Common::ITEMIDLIST;
    use windows::Win32::UI::Shell::{ILFree, IShellFolder, SHBindToParent, SHParseDisplayName};
    use windows::core::{BOOL, HRESULT, PCWSTR, Ref, Result as WinResult};

    use super::DragPayload;

    /// 最小拖拽源：Esc → 取消；左键释放 → 落下；GiveFeedback 交系统默认光标
    ///（默认模式不产生 SysDragImage——PoC 2 实证，stagedrop 无需感知）
    #[windows::core::implement(IDropSource)]
    pub(super) struct DragSource;

    impl IDropSource_Impl for DragSource_Impl {
        fn QueryContinueDrag(
            &self,
            fescapepressed: windows::core::BOOL,
            grfkeystate: MODIFIERKEYS_FLAGS,
        ) -> windows::core::HRESULT {
            if fescapepressed.as_bool() {
                return DRAGDROP_S_CANCEL;
            }
            if grfkeystate & MK_LBUTTON == MODIFIERKEYS_FLAGS(0) {
                return DRAGDROP_S_DROP;
            }
            windows::core::HRESULT(0)
        }

        fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> windows::core::HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }

    /// 文本载荷唯一对外的 FORMATETC
    fn text_format() -> FORMATETC {
        FORMATETC {
            cfFormat: CF_UNICODETEXT.0,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        }
    }

    /// 只校验格式与介质：`dwAspect` / `lindex` 各接收方填充不一（Outlook、微信
    /// 等会传 0 或 -1 以外的 lindex），宽容匹配才不会在 DragEnter 被判「无文本」。
    fn is_text_request(p: *const FORMATETC) -> bool {
        if p.is_null() {
            return false;
        }
        let f = unsafe { &*p };
        f.cfFormat == CF_UNICODETEXT.0 && (f.tymed & (TYMED_HGLOBAL.0 as u32)) != 0
    }

    /// 纯文本 IDataObject：只报 CF_UNICODETEXT，GetData 时把 UTF-16（含结尾 NUL）
    /// 拷进 HGLOBAL 交给 OLE；内存所有权随 STGMEDIUM 转移给接收方。
    #[windows::core::implement(IDataObject)]
    pub(super) struct TextDataObject {
        wide: Vec<u16>,
    }

    #[allow(non_snake_case)]
    impl IDataObject_Impl for TextDataObject_Impl {
        fn GetData(&self, pformatetcin: *const FORMATETC) -> WinResult<STGMEDIUM> {
            if !is_text_request(pformatetcin) {
                return Err(DV_E_FORMATETC.into());
            }
            unsafe {
                let bytes = self.wide.len() * std::mem::size_of::<u16>();
                let hglobal = GlobalAlloc(GMEM_MOVEABLE, bytes)?;
                let ptr = GlobalLock(hglobal);
                if ptr.is_null() {
                    let _ = GlobalFree(Some(hglobal));
                    return Err(E_FAIL.into());
                }
                std::ptr::copy_nonoverlapping(
                    self.wide.as_ptr(),
                    ptr.cast::<u16>(),
                    self.wide.len(),
                );
                let _ = GlobalUnlock(hglobal);
                Ok(STGMEDIUM {
                    tymed: TYMED_HGLOBAL.0 as u32,
                    u: STGMEDIUM_0 { hGlobal: hglobal },
                    ..Default::default()
                })
            }
        }

        fn GetDataHere(
            &self,
            _pformatetc: *const FORMATETC,
            _pmedium: *mut STGMEDIUM,
        ) -> WinResult<()> {
            Err(E_NOTIMPL.into())
        }

        fn QueryGetData(&self, pformatetc: *const FORMATETC) -> HRESULT {
            if is_text_request(pformatetc) {
                S_OK
            } else {
                DV_E_FORMATETC
            }
        }

        fn GetCanonicalFormatEtc(
            &self,
            _pformatectin: *const FORMATETC,
            _pformatetcout: *mut FORMATETC,
        ) -> HRESULT {
            DV_E_FORMATETC
        }

        fn SetData(
            &self,
            _pformatetc: *const FORMATETC,
            _pmedium: *const STGMEDIUM,
            _frelease: BOOL,
        ) -> WinResult<()> {
            Err(E_NOTIMPL.into())
        }

        fn EnumFormatEtc(&self, dwdirection: u32) -> WinResult<IEnumFORMATETC> {
            if dwdirection != DATADIR_GET.0 as u32 {
                return Err(E_NOTIMPL.into());
            }
            Ok(OneFormatEnum {
                done: AtomicBool::new(false),
            }
            .into())
        }

        fn DAdvise(
            &self,
            _pformatetc: *const FORMATETC,
            _advf: u32,
            _padvsink: Ref<'_, IAdviseSink>,
        ) -> WinResult<u32> {
            Err(E_NOTIMPL.into())
        }

        fn DUnadvise(&self, _dwconnection: u32) -> WinResult<()> {
            Err(E_NOTIMPL.into())
        }

        fn EnumDAdvise(&self) -> WinResult<IEnumSTATDATA> {
            Err(E_NOTIMPL.into())
        }
    }

    /// 单格式枚举器：让会调 EnumFormatEtc 的目标也能发现 CF_UNICODETEXT
    #[windows::core::implement(IEnumFORMATETC)]
    struct OneFormatEnum {
        done: AtomicBool,
    }

    #[allow(non_snake_case)]
    impl IEnumFORMATETC_Impl for OneFormatEnum_Impl {
        fn Next(&self, celt: u32, rgelt: *mut FORMATETC, pceltfetched: *mut u32) -> HRESULT {
            unsafe {
                if !pceltfetched.is_null() {
                    *pceltfetched = 0;
                }
            }
            if celt == 0 || rgelt.is_null() || self.done.load(Ordering::Relaxed) {
                return S_FALSE;
            }
            unsafe { *rgelt = text_format() };
            self.done.store(true, Ordering::Relaxed);
            unsafe {
                if !pceltfetched.is_null() {
                    *pceltfetched = 1;
                }
            }
            S_OK
        }

        fn Skip(&self, celt: u32) -> WinResult<()> {
            if celt > 0 {
                self.done.store(true, Ordering::Relaxed);
            }
            Ok(())
        }

        fn Reset(&self) -> WinResult<()> {
            self.done.store(false, Ordering::Relaxed);
            Ok(())
        }

        fn Clone(&self) -> WinResult<IEnumFORMATETC> {
            Ok(OneFormatEnum {
                done: AtomicBool::new(self.done.load(Ordering::Relaxed)),
            }
            .into())
        }
    }

    /// 文件载荷：父 folder 的 GetUIObjectOf（Explorer 同源，PoC 第 5 层实证）。
    /// SHCreateDataObject(None, [绝对 pidl]) 会把绝对 pidl 当相对子项用，
    /// CF_HDROP 路径无效 → 目标全拒绝（禁止光标）
    fn shell_data_object(file: &Path) -> Result<IDataObject, String> {
        unsafe {
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            let wide: Vec<u16> = file
                .as_os_str()
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            if SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None).is_err()
                || pidl.is_null()
            {
                return Err("SHParseDisplayName 失败".into());
            }
            let mut child_rel: *mut ITEMIDLIST = std::ptr::null_mut();
            let psf: IShellFolder = match SHBindToParent(pidl, Some(&mut child_rel)) {
                Ok(f) => f,
                Err(e) => {
                    ILFree(Some(pidl as _));
                    return Err(format!("SHBindToParent {e}"));
                }
            };
            let obj =
                match psf.GetUIObjectOf::<IDataObject>(HWND::default(), &[child_rel as _], None) {
                    Ok(o) => o,
                    Err(e) => {
                        ILFree(Some(pidl as _));
                        return Err(format!("GetUIObjectOf {e}"));
                    }
                };
            // 数据对象已持有 pidl 拷贝，原件此刻可释放
            ILFree(Some(pidl as _));
            Ok(obj)
        }
    }

    /// 拖拽主流程（**主线程**调用）：释放 WebView2 隐式捕获 → OLE 层初始化 →
    /// 按载荷构造数据对象 → DoDragDrop。返回 DROPEFFECT（1=COPY，0=取消/失败）。
    pub(super) fn do_drag_out(payload: &DragPayload) -> u32 {
        unsafe {
            // 按下时 WebView2 窗口持有隐式捕获，ole32 接管前必须释放
            //（PoC 第 4 层实证：不释放则 DoDragDrop 瞬时 DRAGDROP_S_DROP）。
            // 本函数就在主线程跑，ReleaseCapture 直接释放本线程队列的捕获。
            let _ = ReleaseCapture();

            // OLE 层初始化（CoInitializeEx 不够——PoC 第 2 轮实证）；
            // 主线程可能已被 tauri/wry 初始化过，S_FALSE 也算过
            if let Err(e) = OleInitialize(None) {
                crate::providers::diag::log(&format!("dragout: OleInitialize {e}"));
                return 0;
            }

            let (data_obj, tag): (IDataObject, &str) = match payload {
                DragPayload::Text(text) => {
                    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                    (TextDataObject { wide }.into(), "text")
                }
                DragPayload::File { path, .. } => match shell_data_object(path) {
                    Ok(o) => (o, "file"),
                    Err(e) => {
                        crate::providers::diag::log(&format!("dragout: 文件载荷失败 {e}"));
                        return 0;
                    }
                },
            };

            let source: IDropSource = DragSource.into();
            crate::providers::diag::log(&format!("dragout: entering DoDragDrop kind={tag}"));
            let mut effect = DROPEFFECT(0);
            let hr = DoDragDrop(&data_obj, &source, DROPEFFECT_COPY, &mut effect);
            // 无条件留痕：成功（含 S_DROP/S_CANCEL）与真错误都要能从 zap.log 还原链路
            crate::providers::diag::log(&format!(
                "dragout: DoDragDrop kind={tag} hr=0x{:08X} effect={}",
                hr.0, effect.0
            ));
            effect.0
        }
    }
}

/// 启动清理：上次进程崩溃残留的拖出临时文件（延迟删除窗口没跑完就退出时产生）。
/// 目录为 zap 专属（%TEMP%\zap-drag），整目录清空安全
#[cfg(windows)]
pub fn cleanup_stale() {
    let dir = temp_dir();
    if dir.is_dir() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(windows)]
fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("zap-drag")
}

/// 备料：文本直接交 owned 全文（零写盘），图片写临时 PNG，中转站取原路径。
/// 两路都遵循「锁内只点查 owned 数据、锁外做慢活」的两段式（评审 R01-P1）。
#[cfg(windows)]
pub fn prepare(engine: &SearchEngine, kind: &str, rowid: i64) -> Result<DragPayload, String> {
    match kind {
        "clip" => engine
            .clip_row(rowid)
            .map(DragPayload::Text)
            .ok_or_else(|| "该条剪贴板已被清理".into()),
        "clipimg" => {
            let full = engine.clip_image_meta(rowid).ok_or("该图片已被清理")?;
            let path = write_temp(&temp_name("image", "png"), &full.png)?;
            Ok(DragPayload::File {
                path,
                temporary: true,
            })
        }
        "stage" => {
            // 只在锁内取 owned path，出作用域（conn guard drop）再 stat：
            // 失效 UNC 路径 stat 可达秒级，持全局 DB 锁会阻塞剪贴板监听与搜索
            //（互验 R01-F2）
            let raw = {
                let conn = engine.conn();
                crate::providers::stage::rows(&conn)
                    .into_iter()
                    .find(|r| r.id == rowid)
                    .ok_or("条目已不在中转站")?
                    .path
            };
            let path = PathBuf::from(raw);
            if !path.exists() {
                return Err("文件已失效，无法拖出".into());
            }
            Ok(DragPayload::File {
                path,
                temporary: false,
            })
        }
        _ => Err("不支持拖出的条目类型".into()),
    }
}

#[cfg(windows)]
fn write_temp(name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let dir = temp_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("临时目录创建失败：{e}"))?;
    let file = dir.join(name);
    std::fs::write(&file, bytes).map_err(|e| format!("临时文件写入失败：{e}"))?;
    Ok(file)
}

/// 接收方展示名 = 文件名（微信会原样发出），带纳秒防同秒覆盖
#[cfg(windows)]
fn temp_name(tag: &str, ext: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{tag}-{nanos:x}.{ext}")
}

/// 拖拽主流程入口。**必须在主线程调用**（WebView2 按下捕获所在线程，
/// 见模块头约束 5）。返回 DROPEFFECT（1=COPY，0=取消/失败）
#[cfg(windows)]
pub fn run_drag_out(payload: &DragPayload) -> u32 {
    ffi::do_drag_out(payload)
}

/// 拖出结束后延迟删临时文件（目标应用 drop 时同步读，个别异步读留窗口）。
/// 删除失败静默——启动时 cleanup_stale 整目录兜底。文本载荷与 stage 原件不调用
#[cfg(windows)]
pub fn schedule_temp_cleanup(file: PathBuf) {
    if file.starts_with(temp_dir()) {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(10));
            let _ = std::fs::remove_file(file);
        });
    }
}
