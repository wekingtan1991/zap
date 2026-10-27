import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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
  const [active, setActive] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);

  // 新菜单内容到达即清掉上一次的 ✓ 并把键盘选中复位到首项：窗口常驻
  // （只隐藏不销毁），done/active 会跨调用存活；actions 每次推送/拉取
  // 都是新数组，引用变化即触发。同时重播呼出动画——窗口不重挂，
  // 动画不会随 show 自动重播
  useEffect(() => {
    setDone(null);
    setActive(0);
    const panel = rootRef.current;
    if (panel) {
      panel.classList.remove("zap-panel-in");
      void panel.offsetWidth; // 强制 reflow：同一帧内重置动画
      panel.classList.add("zap-panel-in");
    }
  }, [text, actions]);

  const hide = () => {
    invoke("superpanel_hide").catch(() => {});
  };

  const run = (id: string) => {
    setDone(id);
    setTimeout(() => invoke("superpanel_act", { action: id, text }), 120);
  };

  // Esc 收起 / ↑↓ 移动选中 / Enter 执行——键盘语义与主面板一致
  // （调起/Esc/方向键/Enter）。不传依赖数组：每次渲染重挂监听保闭包
  // 新鲜（useEscCapture 同款约定）。Enter 的 preventDefault 会一并压掉
  // 聚焦按钮的原生 click，不会双触发
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") hide();
      else if (e.key === "ArrowDown") {
        e.preventDefault();
        setActive((a) => Math.max(0, Math.min(a + 1, actions.length - 1)));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setActive((a) => Math.max(a - 1, 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const act = actions[active];
        if (act) run(act.id);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  return (
    <div
      ref={rootRef}
      className="zap-panel zap-super outline-none"
      tabIndex={-1}
    >
      <div className="zap-super-head">
        <Sparkles className="size-3 text-primary" />
        <span className="truncate" title={text}>
          {text || "已选内容"}
        </span>
      </div>
      {actions.map((a, i) => (
        <button
          key={a.id}
          type="button"
          className={"zap-super-item" + (i === active ? " zap-super-active" : "")}
          onMouseEnter={() => setActive(i)}
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
