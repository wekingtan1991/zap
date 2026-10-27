import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowLeft,
  Check,
  ClipboardPaste,
  Copy,
  Hash,
  Link2,
  RefreshCw,
  Fingerprint,
  Wrench,
  TriangleAlert,
  Clock,
  FileJson2,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { useEscCapture } from "@/hooks/useEscCapture";
import type { PanelProps } from "@/plugins/panel";

/// 后端 texttools::ToolBundle 的线协议镜像
interface ToolBundle {
  textLen: number;
  tsToDate: string | null;
  dateToTs: string | null;
  base64Enc: string;
  base64Dec: string | null;
  urlEnc: string;
  urlDec: string | null;
  md5: string;
  sha1: string;
  sha256: string;
}

interface Props extends PanelProps {
  /// 初始文本（万能输入框识别 = 进入时的搜索词全文 / 搜索入口 = 空串），
  /// 仅挂载时生效；后续换内容走「粘贴替换」
  text?: string;
  /// 剪贴板行号源：进入时回查一次，以识别时刻的内容为准（与 text 互斥）
  clipRow?: number;
}

/// 结果行：标签 + 值（悬停全文）+ 复制；值为 null 显示"不适用"
function ToolRow({
  rowKey,
  label,
  value,
  copied,
  onCopy,
}: {
  rowKey: string;
  label: string;
  value: string | null | undefined;
  copied: string | null;
  onCopy: (key: string, value: string) => void;
}) {
  return (
    <div className="zap-tool-row">
      <span className="zap-tool-label">{label}</span>
      {value ? (
        <>
          <span className="zap-tool-value" title={value}>
            {value}
          </span>
          <Button
            variant="ghost"
            size="icon-sm"
            className="zap-tool-copy"
            aria-label={"复制" + label}
            onClick={() => onCopy(rowKey, value)}
          >
            {copied === rowKey ? (
              <Check className="size-3.5 text-success" />
            ) : (
              <Copy className="size-3.5" />
            )}
          </Button>
        </>
      ) : (
        <span className="zap-tool-empty">不适用</span>
      )}
    </div>
  );
}

/// 文本工具工作台（独立面板）：时间戳/Base64/URL/UUID/哈希。
/// 分析与转换在后端（tool_convert_all 防抖一次返回），前端只做展示与复制
export function ToolWorkbench({ text, clipRow, onBack, focusRef }: Props) {
  const [srcText, setSrcText] = useState<string | null>(
    text !== undefined ? text : null
  );
  const [loadErr, setLoadErr] = useState("");
  const [bundle, setBundle] = useState<ToolBundle | null>(null);
  const [cmdErr, setCmdErr] = useState("");
  const [uuid, setUuid] = useState("");
  const [copied, setCopied] = useState<string | null>(null);
  /// 粘贴替换等待态：点击按钮后下一次 Ctrl+V 整体替换内容，5 秒无粘贴自动取消
  const [awaitPaste, setAwaitPaste] = useState(false);

  // 行号源：进入时回查一次（文本源初值即内容，与 JSON 工作台同构）
  useEffect(() => {
    if (text !== undefined) return;
    let cancelled = false;
    invoke<string>("clip_row", { id: clipRow })
      .then((t) => {
        if (!cancelled) setSrcText(t);
      })
      .catch((e) => {
        if (!cancelled) setLoadErr(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [text, clipRow]);

  const active = srcText ?? "";

  // 独立面板：窗口捕获兜 Esc（焦点掉 body 后面板 onKeyDown 收不到）
  useEscCapture(onBack, true);

  // 粘贴替换：捕获阶段挂 window——焦点在非可编辑元素时 paste 事件仍会到达
  useEffect(() => {
    if (!awaitPaste) return;
    const onPaste = (e: ClipboardEvent) => {
      const t = e.clipboardData?.getData("text");
      if (!t) return;
      e.preventDefault();
      setSrcText(t);
      setAwaitPaste(false);
    };
    window.addEventListener("paste", onPaste, true);
    const timer = setTimeout(() => setAwaitPaste(false), 5000);
    return () => {
      window.removeEventListener("paste", onPaste, true);
      clearTimeout(timer);
    };
  }, [awaitPaste]);

  // 转换防抖：前缀域每击键触发；行号源加载后只触发一次。
  // cancelled 守卫：在途响应无法取消，快速编辑时旧内容的转换不得覆盖新内容
  useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(async () => {
      if (!active.trim()) {
        setBundle(null);
        setCmdErr("");
        return;
      }
      try {
        const b = await invoke<ToolBundle>("tool_convert_all", {
          text: active,
        });
        if (cancelled) return;
        setCmdErr("");
        setBundle(b);
      } catch (e) {
        if (cancelled) return;
        setCmdErr(String(e));
        setBundle(null);
      }
    }, 150);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [active]);

  // UUID 与输入无关：挂载即生成一枚
  const genUuid = useCallback(async () => {
    try {
      setUuid(await invoke<string>("tool_uuid"));
    } catch (e) {
      toast.error(String(e));
    }
  }, []);
  useEffect(() => {
    void genUuid();
  }, [genUuid]);

  const copy = useCallback(async (key: string, value: string) => {
    try {
      const msg = await invoke<string>("copy_text", { text: value });
      setCopied(key);
      setTimeout(() => setCopied((c) => (c === key ? null : c)), 1600);
      toast.success(msg);
    } catch (e) {
      toast.error("复制失败：" + String(e));
    }
  }, []);

  const header = (
    <div className="zap-input-row">
      <span className="zap-logo">
        <Wrench className="size-4 text-primary" />
      </span>
      <span className="zap-vault-tag">文本工具</span>
      <Button
        variant="ghost"
        size="sm"
        className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
        disabled={awaitPaste}
        onClick={() => setAwaitPaste(true)}
      >
        <ClipboardPaste className="size-3" />
        {awaitPaste ? "按 Ctrl+V 粘贴…" : "粘贴替换"}
      </Button>
      <Button
        variant="ghost"
        size="sm"
        className="h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
        onClick={onBack}
      >
        <ArrowLeft className="size-3" />
        返回（Esc）
      </Button>
    </div>
  );

  const body = (
    <div className="zap-tool-body">
      {loadErr ? (
        <div className="zap-json-status zap-json-bad">
          <TriangleAlert className="size-3.5" />
          <span>读取剪贴板原文失败：{loadErr}</span>
        </div>
      ) : cmdErr ? (
        <div className="zap-json-status zap-json-bad">
          <TriangleAlert className="size-3.5" />
          <span>{cmdErr}</span>
        </div>
      ) : bundle ? (
        <div className="zap-json-status">
          <Wrench className="size-3.5" />
          <span>已换算</span>
          <span className="zap-json-meta">{bundle.textLen} B</span>
        </div>
      ) : null}
      <div className="zap-tool-card">
        <div className="zap-tool-card-title">
          <Clock className="size-3" />
          时间戳
        </div>
        <ToolRow
          rowKey="tsToDate"
          label="时间 → 日期"
          value={bundle?.tsToDate}
          copied={copied}
          onCopy={copy}
        />
        <ToolRow
          rowKey="dateToTs"
          label="日期 → 时间戳"
          value={bundle?.dateToTs}
          copied={copied}
          onCopy={copy}
        />
      </div>
      <div className="zap-tool-card">
        <div className="zap-tool-card-title">
          <FileJson2 className="size-3" />
          Base64
        </div>
        <ToolRow
          rowKey="base64Dec"
          label="解码"
          value={bundle?.base64Dec}
          copied={copied}
          onCopy={copy}
        />
        <ToolRow
          rowKey="base64Enc"
          label="编码"
          value={bundle?.base64Enc}
          copied={copied}
          onCopy={copy}
        />
      </div>
      <div className="zap-tool-card">
        <div className="zap-tool-card-title">
          <Link2 className="size-3" />
          URL 编解码
        </div>
        <ToolRow
          rowKey="urlDec"
          label="解码"
          value={bundle?.urlDec}
          copied={copied}
          onCopy={copy}
        />
        <ToolRow
          rowKey="urlEnc"
          label="编码"
          value={bundle?.urlEnc}
          copied={copied}
          onCopy={copy}
        />
      </div>
      <div className="zap-tool-card">
        <div className="zap-tool-card-title">
          <Fingerprint className="size-3" />
          UUID v4
        </div>
        <div className="zap-tool-row">
          <span className="zap-tool-value" title={uuid}>
            {uuid}
          </span>
          <Button
            variant="ghost"
            size="icon-sm"
            className="zap-tool-copy"
            aria-label="重新生成"
            title="重新生成"
            onClick={() => void genUuid()}
          >
            <RefreshCw className="size-3.5" />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            className="zap-tool-copy"
            aria-label="复制 UUID"
            disabled={!uuid}
            onClick={() => void copy("uuid", uuid)}
          >
            {copied === "uuid" ? (
              <Check className="size-3.5 text-success" />
            ) : (
              <Copy className="size-3.5" />
            )}
          </Button>
        </div>
      </div>
      <div className="zap-tool-card">
        <div className="zap-tool-card-title">
          <Hash className="size-3" />
          哈希
        </div>
        <ToolRow
          rowKey="md5"
          label="MD5"
          value={bundle?.md5}
          copied={copied}
          onCopy={copy}
        />
        <ToolRow
          rowKey="sha1"
          label="SHA1"
          value={bundle?.sha1}
          copied={copied}
          onCopy={copy}
        />
        <ToolRow
          rowKey="sha256"
          label="SHA256"
          value={bundle?.sha256}
          copied={copied}
          onCopy={copy}
        />
      </div>
      {!active.trim() && !loadErr && (
        <div className="zap-tool-empty-all">
          {text !== undefined
            ? "按 Ctrl+V 粘贴，或点右上「粘贴替换」"
            : "正在读取剪贴板…"}
        </div>
      )}
    </div>
  );

  return (
    <div className="zap-panel outline-none" tabIndex={-1} ref={focusRef}>
      {header}
      <div className="zap-form">{body}</div>
    </div>
  );
}
