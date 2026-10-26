import { useEffect, useState } from "react";import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Check, Copy, Sparkles } from "lucide-react";

/// 后端 superpanel::SuperAction 的线协议镜像
interface SuperAction {
  id: string;
  label: string;
}

interface Props {
  text: string;
  actions: SuperAction[];
}

/// 超级面板动作菜单（独立小窗口）：选中内容长按右键呼出。
/// 点击动作 → 后端执行（复制/换算/搜索/进主面板）→ 窗口由失焦兜底收起；
/// Esc 主动收起。无内容时窗口不展示（后端静默不触发）
export function SuperPanel({ text, actions }: Props) {
  const [done, setDone] = useState<string | null>(null);

  // 新菜单内容到达即清掉上一次的 ✓：窗口常驻（只隐藏不销毁），
  // done 会跨调用存活；actions 每次推送/拉取都是新数组，引用变化即触发
  useEffect(() => {
    setDone(null);
  }, [text, actions]);

  const hide = () => {
    invoke("superpanel_hide").catch(() => {});
  };

  const run = (id: string) => {
    setDone(id);
    setTimeout(() => invoke("superpanel_act", { action: id, text }), 120);
  };

  // Esc 收起；动作执行后窗口失焦收起（on_window_event 的 Focused(false) 兜底）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") hide();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="zap-panel zap-super outline-none" tabIndex={-1}>
      <div className="zap-super-head">
        <Sparkles className="size-3 text-primary" />
        <span className="truncate" title={text}>
          {text || "已选内容"}
        </span>
      </div>
      {actions.map((a) => (
        <button
          key={a.id}
          type="button"
          className="zap-super-item"
          onClick={() => run(a.id)}
        >
          <span className="zap-super-label">{a.label}</span>
          {done === a.id ? (
            <Check className="size-3.5 shrink-0 text-success" />
          ) : (
            <Copy className="size-3.5 shrink-0 text-muted-foreground/60" />
          )}
        </button>
      ))}
    </div>
  );
}

/// 窗口入口：事件推送 + 聚焦拉取双通道。
/// 拉取是兜底主路径——show 后 set_focus 必触发 webview focus 事件，
/// 此时 invoke 拉取必然拿到刚写入的菜单内容，不依赖事件投递时序
export function SuperPanelRoot() {
  const [payload, setPayload] = useState<Props>({
    text: "",
    actions: [],
  });
  useEffect(() => {
    const un = listen<Props>("zap://super-actions", (e) => {
      setPayload(e.payload);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);
  useEffect(() => {
    const pull = () => {
      invoke<Props | null>("superpanel_payload")
        .then((p) => {
          if (p && p.actions.length > 0) setPayload(p);
        })
        .catch(() => {});
    };
    pull();
    window.addEventListener("focus", pull);
    return () => window.removeEventListener("focus", pull);
  }, []);
  return <SuperPanel text={payload.text} actions={payload.actions} />;
}
