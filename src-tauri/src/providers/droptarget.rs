//! 自有 IDropTarget：drop 接收主路径（自 stagedrop.rs 拆出，审计 D4）。
//! 背景：wry 给 WebView2 设的 AllowExternalDrop=false 拒绝外部拖放（真机
//! 实测光标恒禁止、Drop 永不达），故在 WebView2 子窗口上撤销 wry 注册、
//! 挂自有目标——效果恒 COPY，悬停/离开发 zap://stagedrop-hot 驱动前端高亮，
//! Drop 直取 CF_HDROP 经 ON_DROP 回调上抛（lib.rs handle_stage_drop）。
//! 悬停/落点反馈与布局仍归 stagedrop.rs 状态机

use tauri::AppHandle;

#[cfg(windows)]
use super::stagedrop::{HOVERING, ON_DROP, layout_bar};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use tauri::{Emitter, Manager};
#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM};
#[cfg(windows)]
use windows::Win32::System::Ole::IDropTarget;
// ---------- 自有 IDropTarget（drop 接收主路径，见模块头） ----------

/// 在 stagedrop 的 WebView2 子窗口上注册自有 IDropTarget（替换 wry 的注册）。
/// WebView2 初始化异步、子窗口可能晚于 setup 就绪：后台线程经 run_on_main_thread
/// 尝试，先 300ms×10 快速阶段（就绪等待），耗尽后转 60s 低频常驻重试——wry
/// 兜底路径实测收不到 drop，慢机器上一次性耗尽即永久静默失效不可接受。
/// 注册线程 = 窗口所属主线程（OLE STA 要求），故必须经 run_on_main_thread。
#[cfg(windows)]
pub fn register_drop_target(app: &AppHandle) {
    const PENDING: usize = usize::MAX; // 哨兵：闭包未完成（n=0 是合法完成值）
    const FAST_ATTEMPTS: u32 = 10;
    let app = app.clone();
    std::thread::spawn(move || {
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            // run_on_main_thread 的闭包要求 'static，经 Arc 回传结果
            let done = std::sync::Arc::new(AtomicUsize::new(PENDING));
            {
                let done_cb = std::sync::Arc::clone(&done);
                let a = app.clone();
                if app
                    .run_on_main_thread(move || {
                        let n = unsafe { try_register_children(&a) };
                        done_cb.store(n, Ordering::Relaxed);
                    })
                    .is_err()
                {
                    return; // 应用退出中
                }
                // 等待闭包完成（哨兵翻掉即完成，n=0 也算——交由下方重试逻辑处理）
                while done.load(Ordering::Relaxed) == PENDING {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            let n = done.load(Ordering::Relaxed);
            if n > 0 {
                super::diag::log(&format!(
                    "stagedrop: own drop target registered on {n} child hwnd(s) (attempt {attempt})"
                ));
                return;
            }
            if attempt == FAST_ATTEMPTS {
                super::diag::log(
                    "stagedrop: drop target registration failed 10 fast attempts; retrying every 60s",
                );
            }
            // 重试上限（评审 P2-15）：快速 10 次 + 60s 间隔 10 次 ≈ 10 分钟后
            // 放弃并落盘——无限常驻重试是无界生命周期线程（R4 P2），而 10 分钟
            // 仍失败说明窗口结构异常，用户重启 zap 才是正解
            if attempt >= FAST_ATTEMPTS + 10 {
                super::diag::log_err(
                    "stagedrop: drop target registration gave up after ~10min; restart zap to retry",
                );
                return;
            }
            // 子窗口尚未就绪（WebView2 初始化异步）：快速阶段短间隔，之后低频常驻
            let wait = if attempt < FAST_ATTEMPTS { 300 } else { 60_000 };
            std::thread::sleep(Duration::from_millis(wait));
        }
    });
}

/// 非 Windows 无 WebView2/OLE 拖放路径
#[cfg(not(windows))]
pub fn register_drop_target(_app: &AppHandle) {}

/// 枚举子窗口逐个撤销旧注册并挂自有目标，返回成功个数
#[cfg(windows)]
unsafe fn try_register_children(app: &AppHandle) -> usize {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::EnumChildWindows;

    let Some(win) = app.get_webview_window("stagedrop") else {
        return 0;
    };
    let Ok(container) = win.hwnd() else {
        return 0;
    };
    let mut registered = 0usize;
    let mut callback = |child: HWND| -> bool {
        // 闭包体自身是安全上下文，unsafe fn 调用须显式块
        if unsafe { register_on_hwnd(app, child) } {
            registered += 1;
        }
        true
    };
    let mut trait_obj: &mut dyn FnMut(HWND) -> bool = &mut callback;
    let closure_pointer_pointer: *mut std::ffi::c_void =
        unsafe { std::mem::transmute(&mut trait_obj) };
    let lparam = LPARAM(closure_pointer_pointer as _);
    unsafe {
        let _ = EnumChildWindows(Some(container), Some(enumerate_cb), lparam);
    }
    registered
}

#[cfg(windows)]
unsafe extern "system" fn enumerate_cb(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    let closure =
        unsafe { &mut *(lparam.0 as *mut std::ffi::c_void as *mut &mut dyn FnMut(HWND) -> bool) };
    closure(hwnd).into()
}

/// 单个子窗口：撤销 wry/WebView2 的注册，挂自有目标（后者赢得 drop 命中）
#[cfg(windows)]
unsafe fn register_on_hwnd(app: &AppHandle, hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::System::Ole::{RegisterDragDrop, RevokeDragDrop};

    let target: IDropTarget = StageDropTarget {
        app: app.clone(),
        has_files: AtomicBool::new(false),
    }
    .into();
    // edition 2024 unsafe_op_in_unsafe_fn：unsafe fn 体内的不安全操作仍须显式块
    unsafe {
        // 无注册时撤销返回错误属正常，忽略；注册失败（极少）交由上层计数
        let _ = RevokeDragDrop(hwnd);
        // RegisterDragDrop 自持 AddRef，本地引用释放不影响目标存活
        RegisterDragDrop(hwnd, &target).is_ok()
    }
}

/// 从 COM 目标提取 CF_HDROP 路径（返回路径与 HDROP；调用方负责 DragFinish）
#[cfg(windows)]
fn extract_drop_paths(
    data_obj: windows::core::Ref<'_, windows::Win32::System::Com::IDataObject>,
) -> (Vec<String>, Option<windows::Win32::UI::Shell::HDROP>) {
    use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
    use windows::Win32::System::Ole::CF_HDROP;
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Some(data) = data_obj.as_ref() else {
        return (Vec::new(), None);
    };
    let Ok(medium) = (unsafe { data.GetData(&format) }) else {
        return (Vec::new(), None);
    };
    let hdrop = HDROP(unsafe { medium.u.hGlobal }.0 as _);
    let count = unsafe { DragQueryFileW(hdrop, 0xFFFF_FFFF, None) };
    let mut paths = Vec::with_capacity(count as usize);
    for i in 0..count {
        let len = unsafe { DragQueryFileW(hdrop, i, None) } as usize;
        let mut buf = vec![0u16; len + 1];
        unsafe { DragQueryFileW(hdrop, i, Some(&mut buf)) };
        paths.push(String::from_utf16_lossy(&buf[..len]));
    }
    (paths, Some(hdrop))
}

/// 自有拖放目标：CF_HDROP 全收，效果恒 COPY（根治禁止图标）
#[cfg(windows)]
#[windows::core::implement(IDropTarget)]
struct StageDropTarget {
    app: AppHandle,
    /// DragEnter 是否带文件（DragOver/Leave/Drop 的分支依据；OLE 回调均在
    /// 注册线程=主线程，无跨线程访问）
    has_files: AtomicBool,
}

#[cfg(windows)]
#[allow(non_snake_case)]
impl windows::Win32::System::Ole::IDropTarget_Impl for StageDropTarget_Impl {
    fn DragEnter(
        &self,
        pDataObj: windows::core::Ref<'_, windows::Win32::System::Com::IDataObject>,
        _grfKeyState: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
        _pt: &windows::Win32::Foundation::POINTL,
        pdwEffect: *mut windows::Win32::System::Ole::DROPEFFECT,
    ) -> windows::core::Result<()> {
        // DragEnter 只取「是否带文件」判定，HDROP 用完即释放——GetData 返回的
        // STGMEDIUM 归调用方所有，不 DragFinish 即泄漏全局内存块（会话内反复
        // 进出窗口会累积；Drop 路径同款处理见下）
        let (paths, hdrop) = extract_drop_paths(pDataObj);
        if let Some(hdrop) = hdrop {
            unsafe { windows::Win32::UI::Shell::DragFinish(hdrop) };
        }
        let has = !paths.is_empty();
        self.has_files.store(has, Ordering::Relaxed);
        HOVERING.store(has, Ordering::Relaxed);
        // 诊断：展开失效排查——hot 内容出现即证明本回调在跑，这里钉住 has
        super::diag::log(&format!("stagedrop: drag enter has={has}"));
        unsafe {
            *pdwEffect = if has {
                windows::Win32::System::Ole::DROPEFFECT_COPY
            } else {
                windows::Win32::System::Ole::DROPEFFECT_NONE
            };
        }
        if has {
            // QQ 式悬停展开：窗口右下角锚定放大，光标仍在卡片区内
            layout_bar(&self.app, true);
            let _ = self.app.emit_to("stagedrop", "zap://stagedrop-hot", true);
        }
        Ok(())
    }

    fn DragOver(
        &self,
        _grfKeyState: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
        _pt: &windows::Win32::Foundation::POINTL,
        pdwEffect: *mut windows::Win32::System::Ole::DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            *pdwEffect = if self.has_files.load(Ordering::Relaxed) {
                windows::Win32::System::Ole::DROPEFFECT_COPY
            } else {
                windows::Win32::System::Ole::DROPEFFECT_NONE
            };
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        self.has_files.store(false, Ordering::Relaxed);
        HOVERING.store(false, Ordering::Relaxed);
        super::diag::log("stagedrop: drag leave");
        layout_bar(&self.app, false);
        let _ = self.app.emit_to("stagedrop", "zap://stagedrop-hot", false);
        Ok(())
    }

    fn Drop(
        &self,
        pDataObj: windows::core::Ref<'_, windows::Win32::System::Com::IDataObject>,
        _grfKeyState: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS,
        _pt: &windows::Win32::Foundation::POINTL,
        pdwEffect: *mut windows::Win32::System::Ole::DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.has_files.store(false, Ordering::Relaxed);
        HOVERING.store(false, Ordering::Relaxed);
        // 收回细条再入库：反馈文案显示在细条里
        layout_bar(&self.app, false);
        let _ = self.app.emit_to("stagedrop", "zap://stagedrop-hot", false);
        let (paths, hdrop) = extract_drop_paths(pDataObj);
        if let Some(hdrop) = hdrop {
            unsafe { windows::Win32::UI::Shell::DragFinish(hdrop) };
        }
        super::diag::log(&format!("stagedrop: drop paths={}", paths.len()));
        unsafe {
            *pdwEffect = windows::Win32::System::Ole::DROPEFFECT_COPY;
        }
        if !paths.is_empty() {
            // OLE 回调在主线程：入库压后台线程，不阻塞拖拽循环。
            // drop 编排经注册回调上抛（评审 P1-17 步 0）：providers 不再反向
            // 依赖 crate 根的 handle_stage_drop——边反转为 lib.rs setup 时注册
            if let Some(h) = ON_DROP.get() {
                let h = std::sync::Arc::clone(h);
                let app = self.app.clone();
                std::thread::spawn(move || h(&app, paths));
            }
        }
        Ok(())
    }
}
