//! 拖拽出（Drag Out）PoC 探针 v2 · docs/design-drag-out.md 第四节
//!
//! 前三轮实证链：
//!   - 裸线程 + DoDragDrop(None source)        → CO_E_NOTINITIALIZED（缺 IDropSource）
//!   - 裸线程 + IDropSource + CoInitializeEx   → CO_E_NOTINITIALIZED（缺 OLE 层）
//!   - 裸线程 + IDropSource + OleInitialize    → 模态循环建立但感知不到鼠标释放
//!     （无窗口纯线程拿不到拖拽输入状态——本轮裁决核心）
//!
//! 本轮：可见窗口 + 标准消息泵，窗口过程 WM_MOUSEMOVE 里起拖
//! （完全复刻真实拖拽源形态：GUI 线程、窗口消息、按下起拖）。
//! 窗口版成功 → 生产实现改为「专用线程 + 隐式窗口 + 泵」；
//! 窗口版也失败 → 整案换骨架。
//!
//! 操作：按住弹出小窗口的内部区域，晃动 6px 即起拖，拖到资源管理器/桌面松手。
//!       %TEMP%\zap-drag-poc\hello.txt 已就绪。

#![cfg(windows)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::thread;
use std::time::Duration;

use windows::Win32::Foundation::{
    DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{ILFree, IShellFolder, SHBindToParent, SHParseDisplayName};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW,
    GetMessageW, IDC_CROSS, LoadCursorW, MSG, PostQuitMessage, RegisterClassW, TranslateMessage,
    WINDOW_EX_STYLE, WM_DESTROY, WM_LBUTTONDOWN, WM_MOUSEMOVE, WNDCLASSW, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsWindowVisible};
use windows::core::{PCWSTR, w};

use std::cell::RefCell;

thread_local! {
    static DATA: RefCell<Option<IDataObject>> = const { RefCell::new(None) };
    static SOURCE: RefCell<Option<IDropSource>> = const { RefCell::new(None) };
}
static DRAGGING: AtomicBool = AtomicBool::new(false);
static DOWN: AtomicBool = AtomicBool::new(false);
static DOWN_X: AtomicI32 = AtomicI32::new(0);
static DOWN_Y: AtomicI32 = AtomicI32::new(0);

fn main() {
    let done = Arc::new(AtomicBool::new(false));
    let observer = {
        let done = Arc::clone(&done);
        thread::spawn(move || observer(done))
    };

    println!("=== zap 拖拽出 PoC 探针 v2（GUI 窗口版）===");
    println!("按住小窗口内区域拖动 → 拖到资源管理器/桌面松手看 hello.txt 是否落地");
    println!("关闭窗口即退出。");

    let handle = thread::spawn(worker);
    let _ = handle.join();
    done.store(true, Ordering::SeqCst);
    let _ = observer.join();
}

fn worker() {
    unsafe {
        let hr = match OleInitialize(None) {
            Ok(()) => "S_OK",
            Err(e) => {
                println!("[ole] OleInitialize FAIL 0x{:08X}", e.code().0);
                return;
            }
        };
        println!("[ole] OleInitialize {hr}");

        let Some(path) = prepare_file() else {
            println!("[fail] 临时文件创建失败");
            return;
        };
        println!("[prep] {}", path);

        // 路径 → 绝对 pidl → 父 folder 的数据对象。
        // 不用 SHCreateDataObject(None, [绝对 pidl])：把绝对 pidl 当相对子项是语义错误，
        // CF_HDROP 路径无效 → 目标全拒绝（禁止光标，上一轮实证）。
        // 正统路径 = SHBindToParent 拆 parent+child → GetUIObjectOf(IDataObject)，
        // 与 Explorer 自身拖拽源的生成方式一致。
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        if SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None).is_err()
            || pidl.is_null()
        {
            println!("[fail] SHParseDisplayName");
            return;
        }
        let mut child_rel: *mut ITEMIDLIST = std::ptr::null_mut();
        let psf: IShellFolder = match SHBindToParent(pidl, Some(&mut child_rel)) {
            Ok(f) => f,
            Err(e) => {
                println!("[fail] SHBindToParent：{e}");
                ILFree(Some(pidl as _));
                return;
            }
        };
        let data_obj: IDataObject = match psf.GetUIObjectOf(
            windows::Win32::Foundation::HWND::default(),
            &[child_rel as _],
            None,
        ) {
            Ok(o) => o,
            Err(e) => {
                println!("[fail] GetUIObjectOf(IDataObject)：{e}");
                ILFree(Some(pidl as _));
                return;
            }
        };
        // 数据对象已持有自己的 pidl 拷贝，原件此刻才可释放
        ILFree(Some(pidl as _));
        DATA.with(|d| *d.borrow_mut() = Some(data_obj));
        SOURCE.with(|s| *s.borrow_mut() = Some(IDropSource::from(ProbeDragSource)));
        println!("[ole] IDataObject 就绪");

        // 注册窗口类 + 建可见窗口 + 标准泵
        let hinstance = GetModuleHandleW(None).unwrap();
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap(),
            hbrBackground: HBRUSH::default(),
            lpszClassName: w!("ZapDragOutProbeWnd"),
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            println!("[fail] RegisterClassW");
            return;
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("ZapDragOutProbeWnd"),
            w!("zap 拖拽出 PoC — 按住窗口内拖动"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            320,
            160,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
        .unwrap();
        let _ = hwnd;
        println!("[wnd] 窗口已建立，开始消息泵");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        println!("[exit] 泵退出");
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            let x = (lp.0 & 0xFFFF) as u16 as i32;
            let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i32;
            DOWN.store(true, Ordering::SeqCst);
            DOWN_X.store(x, Ordering::SeqCst);
            DOWN_Y.store(y, Ordering::SeqCst);
            println!("[down] ({x},{y})——晃动 6px 起拖");
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if !DOWN.load(Ordering::SeqCst) || wp.0 as u32 & MK_LBUTTON.0 == 0 {
                return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
            }
            let x = (lp.0 & 0xFFFF) as u16 as i32;
            let y = ((lp.0 >> 16) & 0xFFFF) as u16 as i32;
            let dx = x - DOWN_X.load(Ordering::SeqCst);
            let dy = y - DOWN_Y.load(Ordering::SeqCst);
            if dx * dx + dy * dy > 36 && !DRAGGING.swap(true, Ordering::SeqCst) {
                println!("[drag] 位移达标（{dx},{dy}），进入 DoDragDrop…");
                let data = DATA.with(|d| d.borrow().as_ref().cloned());
                let source = SOURCE.with(|s| s.borrow().as_ref().cloned());
                if let (Some(data), Some(source)) = (data, source) {
                    // 经典拖拽源模式：按下时本窗口持有隐式捕获，
                    // 必须先 ReleaseCapture 让 ole32 接管，否则它拿不到鼠标事件流
                    let _ =
                        unsafe { windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture() };
                    let t0 = std::time::Instant::now();
                    let mut effect = DROPEFFECT(0);
                    let hr = unsafe { DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect) };
                    println!(
                        "[drag] DoDragDrop 返回（耗时 {:?}）hr=0x{:08X} effect=0x{:X}（1=COPY 0=取消）",
                        t0.elapsed(),
                        hr.0,
                        effect.0
                    );
                }
                DRAGGING.store(false, Ordering::SeqCst);
                DOWN.store(false, Ordering::SeqCst);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

/// 最小拖拽源：Esc → 取消；左键释放 → 落下；光标反馈交系统默认。
#[windows::core::implement(IDropSource)]
struct ProbeDragSource;

impl IDropSource_Impl for ProbeDragSource_Impl {
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

/// 观察线程：50ms 轮询 SysDragImage（PoC 2：裸 IDataObject 是否产生拖影窗口）
fn observer(done: Arc<AtomicBool>) {
    let mut last = false;
    while !done.load(Ordering::SeqCst) {
        let now = unsafe {
            match FindWindowW(w!("SysDragImage"), None) {
                Ok(h) => IsWindowVisible(h).as_bool(),
                Err(_) => false,
            }
        };
        if now != last {
            println!(
                "[obs] SysDragImage {}",
                if now {
                    "出现 ⚠️（stagedrop 会误判拖入）"
                } else {
                    "消失"
                }
            );
            last = now;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn prepare_file() -> Option<String> {
    let base = std::env::var("TEMP").ok()?;
    let dir = std::path::Path::new(&base).join("zap-drag-poc");
    std::fs::create_dir_all(&dir).ok()?;
    let file = dir.join("hello.txt");
    std::fs::write(&file, "zap drag-out probe\n按住我拖出去\n").ok()?;
    Some(file.to_string_lossy().into_owned())
}
