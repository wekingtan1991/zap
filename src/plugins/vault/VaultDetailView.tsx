import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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
  Timer,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useEscCapture } from "@/hooks/useEscCapture";

/// 账号详情页数据（vault_get 的完整记录，密码为解密后明文，默认掩码展示）
export interface VaultDetail {
  id: number;
  title: string;
  username: string;
  password: string;
  url: string;
  notes: string;
  pinned: number;
  totp: string;
}

export type CopyableField =
  | "username"
  | "password"
  | "url"
  | "notes"
  | "totp";

interface Props {
  detail: VaultDetail;
  /// Alt+3 删除入口直接亮起确认弹窗（视图状态机传入，此后弹窗态本地自持）
  initialConfirmDelete?: boolean;
  /// 子页面焦点接管（防 React 根外失焦导致 Esc 失效，见 AppView 同名约定）
  focusRef: (el: HTMLDivElement | null) => void;
  onClose: () => void;
  /// 删除当前账号：成功 true（父级关详情 + 刷新列表）；失败 false（收回弹窗重试）
  onDelete: () => Promise<boolean>;
  /// 行尾复制（父级走受保护通道），返回成功与否供 ✓ 点亮
  onCopyField: (id: number, field: CopyableField) => Promise<boolean>;
  onEdit: () => void;
  onPin: () => void;
}

/// 账号详情：逐行展示，行尾一键复制（密码带掩码切换与阅后即焚，均不入剪贴板历史）。
/// showPw/确认删除弹窗/复制 ✓ 全部本地自持——它们只在本页有意义，随挂载复位
export function VaultDetailView({
  detail,
  initialConfirmDelete = false,
  focusRef,
  onClose,
  onDelete,
  onCopyField,
  onEdit,
  onPin,
}: Props) {
  const [showPw, setShowPw] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(initialConfirmDelete);
  const [copiedField, setCopiedField] = useState<CopyableField | null>(null);

  const rows: { label: string; value: string; field: CopyableField }[] = [
    { label: "用户名", value: detail.username, field: "username" },
    { label: "密码", value: detail.password, field: "password" },
    { label: "网址", value: detail.url, field: "url" },
  ];
  if (detail.notes) {
    rows.push({ label: "备注", value: detail.notes, field: "notes" });
  }

  // TOTP 实时验证码：250ms 采样本地倒计时（与服务端同为 T0=0 的 30s 全局对齐窗口），
  // 窗口翻新时重新取码。仅展示不落任何历史，复制走受保护通道（totp_copy）
  const [totpCode, setTotpCode] = useState("");
  const [totpLeft, setTotpLeft] = useState(0);
  useEffect(() => {
    if (!detail.totp) return;
    let alive = true;
    const fetchCode = () =>
      invoke<string>("vault_action", { id: detail.id, action: "totp" })
        .then((c) => {
          if (alive) setTotpCode(c);
        })
        .catch(() => {});
    fetchCode();
    setTotpLeft(30 - (Math.floor(Date.now() / 1000) % 30));
    const timer = setInterval(() => {
      const left = 30 - (Math.floor(Date.now() / 1000) % 30);
      setTotpLeft(left);
      if (left === 30) fetchCode(); // 窗口翻新
    }, 250);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [detail.id, detail.totp]);

  // 删除确认弹窗存活时，Esc 无条件取消弹窗（Enter 不绑定——删除只能显式点击）
  useEscCapture(() => setConfirmDelete(false), confirmDelete);
  // 弹窗存活时本页让位（enabled 互斥），否则 Esc 关详情页
  useEscCapture(onClose, !confirmDelete);

  const doCopy = (field: CopyableField) => {
    void (async () => {
      const ok = await onCopyField(detail.id, field);
      if (ok) {
        setCopiedField(field);
        setTimeout(() => setCopiedField((c) => (c === field ? null : c)), 1600);
      }
    })();
  };

  const doDelete = () => {
    void (async () => {
      const ok = await onDelete();
      if (!ok) setConfirmDelete(false);
    })();
  };

  return (
    <div
      className="zap-panel vault outline-none"
      tabIndex={-1}
      ref={focusRef}
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
                    onClick={() => setShowPw((v) => !v)}
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
                      onClick={() => doCopy(r.field)}
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
          {detail.totp && (
            <div className="zap-detail-row animate-fade-in">
              <span className="zap-detail-label">验证码</span>
              <span className="zap-detail-value zap-totp-code">
                {totpCode
                  ? `${totpCode.slice(0, 3)} ${totpCode.slice(3)}`
                  : "··· ···"}
                <span className="zap-totp-left">
                  <Timer className="size-3" />
                  {totpLeft}s
                </span>
              </span>
              <span className="zap-detail-ops">
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="secondary"
                      size="icon-sm"
                      className="text-muted-foreground hover:text-foreground"
                      disabled={!totpCode}
                      onClick={() => doCopy("totp")}
                      aria-label="复制验证码"
                    >
                      {copiedField === "totp" ? (
                        <Check className="size-3.5 text-success animate-pop" />
                      ) : (
                        <Copy className="size-3.5" />
                      )}
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>
                    {copiedField === "totp" ? "已复制 ✓" : "复制验证码"}
                  </TooltipContent>
                </Tooltip>
              </span>
            </div>
          )}
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
          <Button variant="destructive" onClick={() => setConfirmDelete(true)}>
            <Trash2 className="size-3.5" />
            删除
          </Button>
        </div>
        {confirmDelete && (
          <div
            className="zap-dialog-backdrop"
            onClick={() => setConfirmDelete(false)}
          >
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
                <Button autoFocus onClick={() => setConfirmDelete(false)}>
                  取消
                </Button>
                <Button variant="destructive" onClick={doDelete}>
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
