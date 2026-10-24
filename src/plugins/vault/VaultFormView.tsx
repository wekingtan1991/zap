/// 新增/编辑账号表单（从 App.tsx 迁出）：字段渲染 + 密码生成 + 保存。
/// 表单数据由 AppView 的视图状态机持有（多处入口写入），此处受控渲染；
/// 表单校验错误本地自持（挂载即清空）
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeft, KeyRound } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { useEscCapture } from "@/hooks/useEscCapture";

export interface AccountForm {
  id?: number;
  title: string;
  username: string;
  password: string;
  url: string;
  notes: string;
  pinned: boolean;
  totp: string;
}

export const EMPTY_FORM: AccountForm = {
  title: "",
  username: "",
  password: "",
  url: "",
  notes: "",
  pinned: false,
  totp: "",
};

interface Props {
  form: AccountForm;
  setForm: React.Dispatch<React.SetStateAction<AccountForm | null>>;
  /// 保存成功后回调（父级：关表单 + 回账号库 + 刷新列表）
  onSaved: () => void;
  /// 取消（Esc 与「取消」按钮共用；父级负责焦点回主输入框）
  onCancel: () => void;
  focusRef: (el: HTMLDivElement | null) => void;
}

export function VaultFormView({
  form,
  setForm,
  onSaved,
  onCancel,
  focusRef,
}: Props) {
  const [formError, setFormError] = useState("");

  // Esc 取消：窗口捕获（点击表单空白处/Label 后焦点掉到 body，面板收不到 keydown）。
  // Ctrl+Enter 保存仍是独立监听（useEscCapture 只管 Esc）
  useEscCapture(onCancel);

  // 表单域键盘自持：Ctrl+Enter 保存。不传依赖数组：每次渲染重挂，闭包永远拿到最新表单。
  useEffect(() => {
    const onWinKey = (e: KeyboardEvent) => {
      if (e.key === "Enter" && e.ctrlKey) {
        e.preventDefault();
        e.stopPropagation();
        void saveForm();
      }
    };
    window.addEventListener("keydown", onWinKey, true);
    return () => window.removeEventListener("keydown", onWinKey, true);
  });
  const setFormField = (key: keyof AccountForm, value: string | boolean) => {
    setForm((f) => (f ? { ...f, [key]: value } : f));
  };

  // 表单密码生成：crypto 随机 16 位（去掉易混淆字符）
  const genPassword = () => {
    const chars =
      "abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#$%^&*";
    const arr = new Uint32Array(16);
    crypto.getRandomValues(arr);
    const pw = Array.from(arr, (n) => chars[n % chars.length]).join("");
    setFormField("password", pw);
  };

  const saveForm = async () => {
    if (!form.title.trim()) {
      setFormError("请填写标题——用于记录这是什么账号，如“行云账号”");
      return;
    }
    setFormError("");
    try {
      if (form.id != null) {
        await invoke("vault_update", {
          id: form.id,
          title: form.title,
          username: form.username,
          password: form.password, // 留空 = 保持原密码
          url: form.url,
          notes: form.notes,
          pinned: form.pinned ? 1 : 0,
          totp: form.totp, // 空串 = 移除 TOTP
        });
      } else {
        await invoke("vault_add", {
          title: form.title,
          username: form.username,
          password: form.password,
          url: form.url,
          notes: form.notes,
          totp: form.totp,
        });
      }
      onSaved();
    } catch (e) {
      // 表单视图不渲染结果列表，错误必须显示在表单内
      setFormError("保存失败：" + String(e));
    }
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
        <span className="zap-vault-tag">
          {form.id != null ? "编辑账号" : "新增账号"}
        </span>
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
          onClick={onCancel}
        >
          <ArrowLeft className="size-3" />
          取消（Esc）
        </Button>
      </div>
      <div className="zap-form">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="f-title">标题 *</Label>
          <Input
            id="f-title"
            autoFocus
            value={form.title}
            onChange={(e) => setFormField("title", e.target.value)}
            placeholder="例如：行云账号"
          />
          <span className="zap-form-hint">
            标题用于记录这是什么账号（如“行云账号”），保存后显示在列表里
          </span>
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="f-user">用户名</Label>
          <Input
            id="f-user"
            value={form.username}
            onChange={(e) => setFormField("username", e.target.value)}
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="f-pass">
            密码（DPAPI 加密存储，不入剪贴板历史
            {form.id != null ? "；留空 = 保持原密码" : ""}）
          </Label>
          <div className="zap-pass-row">
            <Input
              id="f-pass"
              type="password"
              value={form.password}
              onChange={(e) => setFormField("password", e.target.value)}
            />
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={genPassword}
              title="生成 16 位强密码"
            >
              生成
            </Button>
          </div>
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="f-url">网址</Label>
          <Input
            id="f-url"
            value={form.url}
            onChange={(e) => setFormField("url", e.target.value)}
            placeholder="https://…"
          />
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="f-totp">TOTP 密钥（可选）</Label>
          <Input
            id="f-totp"
            value={form.totp}
            onChange={(e) => setFormField("totp", e.target.value)}
            placeholder="Base32 密钥或 otpauth:// 链接"
            spellCheck={false}
          />
          <span className="zap-form-hint">
            动态验证码（2FA）显示在详情页；编辑时清空 = 移除
          </span>
        </div>
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="f-notes">备注</Label>
          <Textarea
            id="f-notes"
            rows={2}
            value={form.notes}
            onChange={(e) => setFormField("notes", e.target.value)}
          />
        </div>
        <div className="flex items-center gap-2.5">
          <Checkbox
            id="f-pin"
            checked={form.pinned}
            onCheckedChange={(v) => setFormField("pinned", v === true)}
          />
          <Label htmlFor="f-pin" className="text-[13px] text-foreground">
            固定置顶（常用账号排在最前）
          </Label>
        </div>
        <div className="zap-form-actions">
          <Button variant="outline" onClick={onCancel}>
            取消（Esc）
          </Button>
          <Button onClick={() => void saveForm()}>保存（Ctrl+Enter）</Button>
        </div>
        {formError && (
          <div className="zap-hk-err animate-shake">{formError}</div>
        )}
      </div>
    </div>
  );
}
