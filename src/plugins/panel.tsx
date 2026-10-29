/// 插件子面板契约与公共实现（二段式交互泛化）：插件专属面板与 App.tsx 之间
/// 只通过这几样耦合——
/// 1) Props 继承 PanelProps：onBack 由 App 装配（回主面板 + 恢复进入前搜索词 +
///    焦点回输入框），与 closeWorkbench 同一套退出语义
/// 2) 根节点 <div className="zap-panel outline-none" tabIndex={-1} ref={focusRef}>
/// 3) Esc 一律 useEscCapture(onBack, true) 窗口捕获——焦点掉 body 后面板 onKeyDown
///    收不到，这是栽过两次的坑（2026-10-22 表单 Esc / 2026-10-28 合并模式键盘面）
/// 4) 行号源 / 粘贴常驻监听 / 复制闪烁 / 头部条用下方 hooks 与 PanelHeader，
///    工作台面板不再各自复制这份生命周期
/// 新增面板 = App 的 View 加一个 kind + 渲染分支 + ItemActions 注册动作，
/// 不新增模式特判；消费者：图片预览（clipimg）、JSON 工作台、文本工具工作台
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface PanelProps {
  onBack: () => void;
  /// 子面板焦点接管（防 Esc 失效）
  focusRef?: (el: HTMLDivElement | null) => void;
}

/// 行号源：搜索入口 / 万能输入框进入时 text 为初始全文（仅挂载生效，后续换
/// 内容走「粘贴替换」）；剪贴板识别进入时按 clipRow 回查一次（以识别时刻的
/// 内容为准）。两者互斥，text 优先。返回 active = 当前内容（null 兜底空串）
export function usePanelSource(props: { text?: string; clipRow?: number }) {
  const [srcText, setSrcText] = useState<string | null>(
    props.text !== undefined ? props.text : null
  );
  const [loadErr, setLoadErr] = useState("");
  useEffect(() => {
    if (props.text !== undefined) return;
    let cancelled = false;
    invoke<string>("clip_row", { id: props.clipRow })
      .then((t) => {
        if (!cancelled) setSrcText(t);
      })
      .catch((e) => {
        if (!cancelled) setLoadErr(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [props.text, props.clipRow]);
  return { srcText, setSrcText, loadErr, active: srcText ?? "" };
}

/// 粘贴常驻监听（评审 P1-02）：面板挂载期 window 捕获 paste，任何时刻
/// Ctrl+V（焦点在非可编辑元素时事件仍到达，本 hook 存在的前提）即整体替换
/// 内容——空态文案「按 Ctrl+V 粘贴」从此恒真。旧实现监听只在点过「粘贴替换」
/// 按钮后挂载，新用户按空态提示操作第一次必然哑火，且 5 秒无粘贴还得重按
/// 按钮。消费者均为只读展示面板（无 textarea/input），capture 拦截不会吃掉
/// 编辑区粘贴；新面板引入可编辑元素时必须重新审视这条假设。onPasted 传
/// setState（稳定引用）：监听只挂载一次
export function usePanelPaste(onPasted: (text: string) => void) {
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      const t = e.clipboardData?.getData("text");
      if (!t) return;
      e.preventDefault();
      onPasted(t);
    };
    window.addEventListener("paste", onPaste, true);
    return () => window.removeEventListener("paste", onPaste, true);
  }, [onPasted]);
}

/// 复制成功闪烁：key 标记哪一行在闪，ms 后自动熄灭；同 key 重复复制由
/// setCopied 覆盖续期
export function useCopyFlash(ms = 1600) {
  const [copied, setCopied] = useState<string | null>(null);
  const flash = useCallback(
    (key: string) => {
      setCopied(key);
      setTimeout(() => setCopied((c) => (c === key ? null : c)), ms);
    },
    [ms]
  );
  return { copied, flash };
}

/// 面板头部条：logo 图标 + 标签 + 右侧动作区（动作按钮自带 ml-auto 起到右侧）
export function PanelHeader({
  icon,
  tag,
  children,
}: {
  icon: ReactNode;
  tag: string;
  children?: ReactNode;
}) {
  return (
    <div className="zap-input-row">
      <span className="zap-logo">{icon}</span>
      <span className="zap-vault-tag">{tag}</span>
      {children}
    </div>
  );
}
