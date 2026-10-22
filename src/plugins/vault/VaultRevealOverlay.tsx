/// 查看密码浮层（Alt+4 快瞄，从 App.tsx 迁出）：
/// 原生复制入口全部拦截，强制走 Rust 受保护通道（排除剪贴板历史 + 焚除）
import { useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeft, Eye, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Kbd } from "@/components/ui/kbd";

export interface RevealInfo {
  id: number;
  title: string;
  password: string;
}

interface Props {
  reveal: RevealInfo;
  onClose: () => void;
  onKeyDown: (e: React.KeyboardEvent) => void;
  focusRef: (el: HTMLDivElement | null) => void;
}

export function VaultRevealOverlay({
  reveal,
  onClose,
  onKeyDown,
  focusRef,
}: Props) {
  // 窗口捕获兜 Esc：焦点掉 body（点击非焦点区）后面板 onKeyDown 收不到（与 VaultFormView 同因）
  useEffect(() => {
    const onWinKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", onWinKey, true);
    return () => window.removeEventListener("keydown", onWinKey, true);
  }, [onClose]);

  return (
    <div
      className="zap-panel vault outline-none"
      tabIndex={-1}
      ref={focusRef}
      onKeyDown={onKeyDown}
    >
      <div className="zap-input-row">
        <span className="zap-logo">
          <Eye className="size-4 text-primary" />
        </span>
        <span className="zap-vault-tag">查看密码</span>
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
          onClick={onClose}
        >
          <ArrowLeft className="size-3" />
          返回（Esc）
        </Button>
      </div>
      <div className="zap-form animate-fade-in">
        <div className="zap-detail-hero">
          <div className="zap-detail-hero-row">
            <span className="zap-detail-title">{reveal.title}</span>
          </div>
          <div className="zap-detail-hero-sub">密码仅本次显示</div>
        </div>
        <div className="zap-pw-card">
          <Input
            autoFocus
            readOnly
            className="zap-pw-reveal h-12 border-0 bg-transparent px-0 font-mono text-[17px] tracking-[0.2em] text-primary shadow-none"
            value={reveal.password}
            onFocus={(e) => e.currentTarget.select()}
            onContextMenu={(e) => e.preventDefault()}
            onKeyDown={(e) => {
              // 拦截一切本机复制入口（Ctrl+C/X/Insert、Shift+F10 与右键菜单）：
              // 强制走 Rust 受保护通道（排除剪贴板历史 + 焚除），
              // 浏览器原生复制会绕过全部防护写入 Win+V 历史
              if (e.shiftKey && e.key === "F10") {
                e.preventDefault();
                return;
              }
              if (e.ctrlKey && ["c", "C", "x", "X", "Insert"].includes(e.key)) {
                e.preventDefault();
                invoke<string>("vault_action", {
                  id: reveal.id,
                  action: "password",
                })
                  .then((msg) => toast.success(msg))
                  .catch((err) => toast.error("复制失败：" + String(err)));
              }
            }}
          />
        </div>
        <div className="zap-pw-hint">
          <ShieldCheck className="size-3.5" />
          <Kbd>Ctrl</Kbd>+<Kbd>C</Kbd> 安全复制（不入剪贴板历史）· 关闭请用{" "}
          <Kbd>Esc</Kbd>
        </div>
      </div>
    </div>
  );
}
