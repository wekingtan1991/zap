/// 智能导入视图（从 App.tsx 迁出的自包含视图）：
/// 粘贴文本 → vault_parse 解析 → 逐条校对 → vault_import 入库。
/// 解析/导入状态全部内部持有，导入成功经 onDone 交还父级（关视图 + 刷新列表）
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeft, Inbox, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { useEscCapture } from "@/hooks/useEscCapture";

export interface ParsedAccount {
  title: string;
  username: string;
  password: string;
  url: string;
}

interface Props {
  /// 导入成功后的收尾（父级：关视图 + 按当前账号库关键词刷新列表）
  onDone: () => void;
  onClose: () => void;
  focusRef: (el: HTMLDivElement | null) => void;
}

export function VaultImportView({ onDone, onClose, focusRef }: Props) {
  const [importText, setImportText] = useState("");
  const [parsed, setParsed] = useState<ParsedAccount[] | null>(null);
  const [importBusy, setImportBusy] = useState(false);
  const [importMsg, setImportMsg] = useState<{ ok: boolean; text: string } | null>(
    null
  );

  const updParsed = (i: number, key: keyof ParsedAccount, value: string) => {
    setParsed((arr) =>
      arr ? arr.map((x, ix) => (ix === i ? { ...x, [key]: value } : x)) : arr
    );
  };

  const closeAndReset = () => {
    setImportText("");
    setParsed(null);
    setImportBusy(false);
    setImportMsg(null);
    onClose();
  };

  // Esc 关导入（窗口捕获兜焦点掉 body，顺带清内部态）
  useEscCapture(closeAndReset);

  return (
    <div
      className="zap-panel vault outline-none"
      tabIndex={-1}
      ref={focusRef}
    >
      <div className="zap-input-row">
        <span className="zap-logo">
          <Inbox className="size-4 text-primary-foreground" />
        </span>
        <span className="zap-vault-tag">智能导入</span>
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
          onClick={closeAndReset}
        >
          <ArrowLeft className="size-3" />
          返回（Esc）
        </Button>
      </div>
      <div className="zap-form">
        {parsed === null ? (
          <>
            <Textarea
              autoFocus
              rows={7}
              value={importText}
              onChange={(e) => setImportText(e.target.value)}
              placeholder={"粘贴记事本 / 聊天记录里的账号文本，例如：\n接入地址\nhttps://…\n账号\n用户名\n密码\n密码"}
            />
            <Button
              disabled={importBusy}
              onClick={async () => {
                if (importBusy) return;
                setImportBusy(true);
                setImportMsg(null);
                try {
                  const res = await invoke<ParsedAccount[]>("vault_parse", {
                    text: importText,
                  });
                  setParsed(res);
                } catch (e) {
                  setImportMsg({ ok: false, text: "解析失败：" + String(e) });
                } finally {
                  setImportBusy(false);
                }
              }}
            >
              {importBusy && <Loader2 className="size-3.5 animate-spin" />}
              解析
            </Button>
          </>
        ) : (
          <>
            {parsed.length === 0 && (
              <div className="zap-vault-tag animate-fade-in">
                未识别到账号，请检查文本后返回重试
              </div>
            )}
            {parsed.map((p, i) => (
              <div key={i} className="zap-import-row animate-fade-in">
                <Input
                  value={p.title}
                  placeholder="标题（留空则按用户名/网址命名）"
                  onChange={(e) => updParsed(i, "title", e.target.value)}
                />
                <div className="zap-import-grid">
                  <Input
                    value={p.username}
                    placeholder="用户名"
                    onChange={(e) => updParsed(i, "username", e.target.value)}
                  />
                  <Input
                    value={p.password}
                    placeholder="密码"
                    onChange={(e) => updParsed(i, "password", e.target.value)}
                  />
                </div>
                <Input
                  value={p.url}
                  placeholder="网址"
                  onChange={(e) => updParsed(i, "url", e.target.value)}
                />
              </div>
            ))}
            {parsed.length > 0 && (
              <Button
                disabled={importBusy}
                onClick={async () => {
                  if (importBusy) return;
                  setImportBusy(true);
                  setImportMsg(null);
                  let n: number;
                  try {
                    n = await invoke<number>("vault_import", {
                      entries: parsed.filter(
                        (p) => p.title.trim() || p.username || p.password
                      ),
                    });
                  } catch (e) {
                    // 失败时保持面板打开并展示原因，便于修正后重试（不再静默）
                    setImportMsg({ ok: false, text: "导入失败：" + String(e) });
                    setImportBusy(false);
                    return;
                  }
                  if (n <= 0) {
                    setImportMsg({
                      ok: false,
                      text: "没有可导入的条目（每条至少需要 标题/用户名/密码 之一）",
                    });
                    setImportBusy(false);
                    return;
                  }
                  setImportMsg({ ok: true, text: `✓ 已导入 ${n} 条` });
                  // 明确反馈后再关闭并刷新列表（空关键词时列出全部账号）
                  setTimeout(onDone, 700);
                }}
              >
                {importBusy && <Loader2 className="size-3.5 animate-spin" />}
                导入全部（{parsed.length} 条）
              </Button>
            )}
          </>
        )}
        {importMsg && (
          <div
            className={
              "animate-fade-in " +
              (importMsg.ok ? "zap-set-saved" : "zap-hk-err")
            }
          >
            {importMsg.text}
          </div>
        )}
      </div>
    </div>
  );
}
