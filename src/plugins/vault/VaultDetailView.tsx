import { useEffect } from "react";
import {
  ArrowLeft,
  Check,
  Copy,
  Eye,
  EyeOff,
  KeyRound,
  Pencil,
  Pin,
  PinOff,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";

/// 账号详情页数据（vault_get 的完整记录，密码为解密后明文，默认掩码展示）
export interface VaultDetail {
  id: number;
  title: string;
  username: string;
  password: string;
  url: string;
  notes: string;
  pinned: number;
}

export type CopyableField = "username" | "password" | "url" | "notes";

interface Props {
  detail: VaultDetail;
  showPw: boolean;
  confirmDelete: boolean;
  /// 最近一次复制成功的字段（行尾图标点亮 ✓）
  copiedField: string | null;
  /// 子页面焦点接管（防 React 根外失焦导致 Esc 失效，见 AppView 同名约定）
  focusRef: (el: HTMLDivElement | null) => void;
  onKeyDown: (e: React.KeyboardEvent) => void;
  onClose: () => void;
  onTogglePw: () => void;
  onAskDelete: () => void;
  onCancelDelete: () => void;
  onDelete: () => void;
  onCopyField: (field: CopyableField) => void;
  onEdit: () => void;
  onPin: () => void;
}

/// 账号详情：逐行展示，行尾一键复制（密码带掩码切换与阅后即焚，均不入剪贴板历史）
export function VaultDetailView({
  detail,
  showPw,
  confirmDelete,
  copiedField,
  focusRef,
  onKeyDown,
  onClose,
  onTogglePw,
  onAskDelete,
  onCancelDelete,
  onDelete,
  onCopyField,
  onEdit,
  onPin,
}: Props) {
  const rows: { label: string; value: string; field: CopyableField }[] = [
    { label: "用户名", value: detail.username, field: "username" },
    { label: "密码", value: detail.password, field: "password" },
    { label: "网址", value: detail.url, field: "url" },
  ];
  if (detail.notes) {
    rows.push({ label: "备注", value: detail.notes, field: "notes" });
  }

  // 删除确认弹窗存活期间，Esc 无条件取消弹窗。挂窗口捕获而非面板 onKeyDown：
  // 点击非焦点区后焦点掉到 body，面板收不到 keydown（与 VaultFormView 同因）。
  // Enter 不绑定——删除只能显式点击，或焦点落在按钮上时按 Enter（autoFocus 在「取消」）。
  useEffect(() => {
    if (!confirmDelete) return;
    const onWinKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onCancelDelete();
      }
    };
    window.addEventListener("keydown", onWinKey, true);
    return () => window.removeEventListener("keydown", onWinKey, true);
  }, [confirmDelete, onCancelDelete]);

  // 详情页本体同理：窗口捕获兜 Esc（焦点掉 body 时面板 onKeyDown 收不到）。
  // 确认弹窗存活时让位给上面的弹窗监听（同 target 注册更晚者后执行，这里直接不动）。
  useEffect(() => {
    const onWinKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || confirmDelete) return;
      e.preventDefault();
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", onWinKey, true);
    return () => window.removeEventListener("keydown", onWinKey, true);
  }, [confirmDelete, onClose]);
  return (
    <div
      className="zap-panel vault outline-none"
      tabIndex={-1}
      ref={focusRef}
      onKeyDown={onKeyDown}
    >
      <div className="zap-input-row">
        <span className="zap-logo">
          <KeyRound className="size-4 text-primary" />
        </span>
        <span className="zap-vault-tag">账号详情</span>
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
      <div className="zap-form">
        <div className="zap-detail-hero animate-fade-in">
          <div className="zap-detail-hero-row">
            {detail.pinned === 1 && (
              <Pin className="zap-detail-hero-pin size-4 fill-current text-primary" />
            )}
            <span className="zap-detail-title">{detail.title}</span>
          </div>
          <div className="zap-detail-hero-sub">
            本地账号库 · 密码不进入任何剪贴板历史
          </div>
        </div>
        <div className="zap-detail-rows">
          {rows.map((r, i) => (
            <div
              key={r.label}
              className="zap-detail-row animate-fade-in"
              style={{ animationDelay: `${40 + i * 35}ms` }}
            >
              <span className="zap-detail-label">{r.label}</span>
              {r.field === "password" ? (
                <span
                  className={
                    "zap-detail-value transition-all duration-200" +
                    (showPw ? "" : " zap-detail-mask")
                  }
                >
                  {detail.password
                    ? showPw
                      ? detail.password
                      : "••••••••"
                    : "（空）"}
                </span>
              ) : (
                <span className="zap-detail-value">{r.value || "（空）"}</span>
              )}
              <span className="zap-detail-ops">
                {r.field === "password" && detail.password && (
                  <Button
                    variant="secondary"
                    size="sm"
                    className="h-7 px-2 text-xs text-muted-foreground hover:text-foreground"
                    onClick={onTogglePw}
                  >
                    {showPw ? (
                      <EyeOff className="size-3" />
                    ) : (
                      <Eye className="size-3" />
                    )}
                    {showPw ? "隐藏" : "显示"}
                  </Button>
                )}
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="secondary"
                      size="icon-sm"
                      className="text-muted-foreground hover:text-foreground"
                      disabled={!r.value}
                      onClick={() => onCopyField(r.field)}
                      aria-label={"复制" + r.label}
                    >
                      {copiedField === r.field ? (
                        <Check className="size-3.5 text-success animate-pop" />
                      ) : (
                        <Copy className="size-3.5" />
                      )}
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>
                    {copiedField === r.field ? "已复制 ✓" : "复制" + r.label}
                  </TooltipContent>
                </Tooltip>
              </span>
            </div>
          ))}
        </div>
        <div className="zap-form-actions">
          <Button onClick={onEdit}>
            <Pencil className="size-3.5" />
            编辑
          </Button>
          <Button variant="outline" onClick={onPin}>
            {detail.pinned === 1 ? (
              <PinOff className="size-3.5" />
            ) : (
              <Pin className="size-3.5" />
            )}
            {detail.pinned === 1 ? "取消置顶" : "置顶"}
          </Button>
          <Button variant="destructive" onClick={onAskDelete}>
            <Trash2 className="size-3.5" />
            删除
          </Button>
        </div>
        {confirmDelete && (
          <div className="zap-dialog-backdrop" onClick={onCancelDelete}>
            <div
              className="zap-dialog"
              role="alertdialog"
              aria-modal="true"
              onClick={(e) => e.stopPropagation()}
            >
              <div className="zap-confirm-text">
                <TriangleAlert className="size-3.5" />
                删除后无法恢复，确认删除「{detail.title}」？
              </div>
              <div className="zap-confirm-actions">
                <Button autoFocus onClick={onCancelDelete}>
                  取消
                </Button>
                <Button variant="destructive" onClick={onDelete}>
                  <Trash2 className="size-3.5" />
                  确认删除
                </Button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
