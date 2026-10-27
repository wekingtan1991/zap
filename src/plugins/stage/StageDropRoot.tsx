import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Inbox } from "lucide-react";

const DEFAULT_TEXT = "拖到这，放入中转站";

/// 拖拽落区条窗口（label=stagedrop）的全部内容：纯展示组件——drop 的接收、
/// stage_add 与反馈状态机全在 Rust 侧（on_window_event，早于任何 JS 回程，
/// 见 providers/stagedrop.rs），这里只做悬停高亮与反馈文案渲染。
/// 条子窗口 focusable:false，永不持有焦点，无键盘交互。
/// 监听注册防 StrictMode 双挂载泄漏：onDragDropEffect 用 cancelled-flag，
/// listen effect 用 then(f=>f()) 模式（见各自注释）。
export function StageDropRoot() {
  const [hot, setHot] = useState(false);
  const [text, setText] = useState(DEFAULT_TEXT);
  /// 会话计数：条子窗口只 hide/show 不销毁，CSS 入场动画只在 DOM 挂载时播
  /// 一次——key 递增重挂子树，每轮拖拽会话重播入场（同 App.tsx 面板
  /// zap://shown 的 remove/reflow/add 手法，这里用 key 更直白）
  const [session, setSession] = useState(0);

  // 高亮双通道：主路径 = Rust 自有 IDropTarget 的 zap://stagedrop-hot（wry 在
  // 该窗口的拖放被 WebView2 拒绝，见 providers/stagedrop.rs 模块头）；
  // onDragDropEvent 为注册失败时的兜底通道。两者同写一个 state，幂等。
  // leave/drop 都复位（OLE 协议 Drop 成功即终止会话，wry 不发 Leave）
  useEffect(() => {
    let cancelled = false;
    let un: (() => void) | null = null;
    getCurrentWebview()
      .onDragDropEvent((e) => {
        const t = e.payload.type;
        if (t === "enter" || t === "over") {
          setHot(true);
        } else if (t === "leave" || t === "drop") {
          setHot(false);
        }
      })
      .then((f) => {
        if (cancelled) f();
        else un = f;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      un?.();
    };
  }, []);

  useEffect(() => {
    const unHot = listen<boolean>("zap://stagedrop-hot", (e) =>
      setHot(e.payload === true)
    ).catch(() => () => {});
    return () => {
      unHot.then((f) => f()).catch(() => {});
    };
  }, []);

  // 反馈文案：Rust 侧事件推送（drop 后处理中/结果）+ 挂载时拉取兜底 +
  // 新拖拽会话复位。reset 带 payload 分流：start=会话起点（重挂重播入场），
  // stale=拖拽中途旧反馈到期（只复位文案——重挂会把滑入/大卡片动画
  // 重播在拖拽中途，评审 P2）。then(f=>f()) 模式在 cleanup 先于 resolve
  // 时同样注销（f 挂在 resolved promise 链上必然被调），无需 cancelled-flag
  useEffect(() => {
    const unFeedback = listen<string>("zap://stagedrop-feedback", (e) =>
      setText(e.payload)
    ).catch(() => () => {});
    const unReset = listen<string>("zap://stagedrop-reset", (e) => {
      setText(DEFAULT_TEXT);
      if (e.payload === "start") setSession((s) => s + 1);
    }).catch(() => () => {});
    invoke<string | null>("stagedrop_feedback_state")
      .then((t) => {
        if (t) setText(t);
      })
      .catch(() => {});
    return () => {
      unFeedback.then((f) => f()).catch(() => {});
      unReset.then((f) => f()).catch(() => {});
    };
  }, []);

  return (
    <div className={"zap-stagedrop" + (hot ? " hot" : "")} aria-live="polite" key={session}>
      <div className="zap-stagedrop-card">
        {hot ? (
          <div className="zap-stagedrop-hot">
            {/* QQ 式悬停大卡片（窗口由 Rust 侧同步放大到 300x264）；
                内容容器带入场动效：窗口 set_size 无法 CSS 过渡，但内容
                换装可以，遮住尺寸瞬切的生硬感 */}
            <Inbox className="zap-stagedrop-hero" />
            <div className="zap-stagedrop-title">文件拖到这</div>
            <div className="zap-stagedrop-sub">放入中转站 · 松手即存</div>
          </div>
        ) : (
          <>
            <Inbox className="size-4 shrink-0" />
            <span className="zap-stagedrop-text">{text}</span>
          </>
        )}
      </div>
    </div>
  );
}
