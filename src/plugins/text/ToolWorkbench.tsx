import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowLeft,
  Check,
  ClipboardPaste,
  Copy,
  Hash,
  Link2,
  Link2Off,
  Eraser,
  RefreshCw,
  Fingerprint,
  Maximize2,
  Wrench,
  TriangleAlert,
  Clock,
  FileJson2,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { useDialogA11y } from "@/hooks/useDialogA11y";
import { useEscCapture } from "@/hooks/useEscCapture";
import {
  PanelHeader,
  usePanelPaste,
  useCopyFlash,
  usePanelSource,
  type PanelProps,
} from "@/plugins/panel";

/// 后端 texttools::ToolBundle 的线协议镜像
interface ToolBundle {
  textLen: number;
  tsToDate: string | null;
  dateToTs: string | null;
  base64Enc: string;
  base64Dec: string | null;
  urlEnc: string;
  urlDec: string | null;
  urlClean: string | null;
  zeroWidthStripped: string | null;
  quotesNormalized: string | null;
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

/// 折叠行溢出检测：值单行省略号折叠，scrollWidth 超出可视宽即判溢出；
/// 编辑区（field-sizing 到顶后）走纵向 scrollHeight。ResizeObserver 跟随
/// 面板宽度实时重判（窄窗口下 SHA256 这类定长值也会出按钮）
function useTextOverflow<T extends HTMLElement>(value: string) {
  const ref = useRef<T>(null);
  const [over, setOver] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const check = () =>
      setOver(
        el.scrollWidth > el.clientWidth + 1 ||
          el.scrollHeight > el.clientHeight + 1
      );
    check();
    const ro = new ResizeObserver(check);
    ro.observe(el);
    return () => ro.disconnect();
  }, [value]);
  return { ref, over };
}

function utf8Bytes(text: string): number {
  return new TextEncoder().encode(text).length;
}

/// 全文浮窗的计数口径：字符数按码点（emoji/生僻字算 1），字节数按 UTF-8
function textCounts(text: string): { chars: number; bytes: number } {
  return { chars: [...text].length, bytes: utf8Bytes(text) };
}

/// 结果行：标签 + 值（单行折叠，溢出才出现展开钮）+ 复制；值为 null 显示"不适用"
function ToolRow({
  rowKey,
  label,
  value,
  copied,
  onCopy,
  onExpand,
}: {
  rowKey: string;
  label: string;
  value: string | null | undefined;
  copied: string | null;
  onCopy: (key: string, value: string) => void;
  onExpand: (label: string, value: string) => void;
}) {
  const { ref, over } = useTextOverflow<HTMLSpanElement>(value ?? "");
  return (
    <div className="zap-tool-row">
      <span className="zap-tool-label">{label}</span>
      {value ? (
        <>
          <span ref={ref} className="zap-tool-value">
            {value}
          </span>
          {over && (
            <Button
              variant="ghost"
              size="icon-sm"
              className="zap-tool-expand"
              aria-label={"展开" + label}
              title="展开查看全文"
              onClick={() => onExpand(label, value)}
            >
              <Maximize2 className="size-3.5" />
            </Button>
          )}
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

/// 粘贴内容行：可输入编辑区（先开面板再键入的路径；框内粘贴走原生光标位，
/// 点面板空白处 Ctrl+V 仍是整体替换），field-sizing 自增到顶后内滚并出展开钮；
/// 无复制钮——全文浮窗里已有「复制全文」
function SourceRow({
  value,
  onExpand,
  onChange,
}: {
  value: string;
  onExpand: (label: string, value: string) => void;
  onChange: (text: string) => void;
}) {
  const { ref, over } = useTextOverflow<HTMLTextAreaElement>(value);
  return (
    <div className="zap-tool-row zap-tool-src-row">
      <textarea
        ref={ref}
        className="zap-tool-src-input"
        value={value}
        placeholder="输入或 Ctrl+V 粘贴"
        aria-label="粘贴内容"
        spellCheck={false}
        autoFocus
        onChange={(e) => onChange(e.target.value)}
      />
      {over && (
        <Button
          variant="ghost"
          size="icon-sm"
          className="zap-tool-expand"
          aria-label="展开粘贴内容"
          title="展开查看全文"
          onClick={() => onExpand("粘贴内容", value)}
        >
          <Maximize2 className="size-3.5" />
        </Button>
      )}
    </div>
  );
}

/// 文本工具工作台（独立面板）：时间戳/Base64/URL/UUID/哈希。
/// 分析与转换在后端（tool_convert_all 防抖一次返回），前端只做展示与复制。
/// 行号源/粘贴替换/复制闪烁/头部条为 panel 契约公共实现（与 JSON 工作台同构）
export function ToolWorkbench({ text, clipRow, onBack, focusRef }: Props) {
  const { srcText, setSrcText, loadErr, active } = usePanelSource({
    text,
    clipRow,
  });
  const [bundle, setBundle] = useState<ToolBundle | null>(null);
  const [cmdErr, setCmdErr] = useState("");
  const [uuid, setUuid] = useState("");
  const [expanded, setExpanded] = useState<{ label: string; text: string } | null>(
    null,
  );
  const dlgRef = useRef<HTMLDivElement | null>(null);
  const { copied, flash } = useCopyFlash();
  const uuidOverflow = useTextOverflow<HTMLSpanElement>(uuid);
  usePanelPaste(setSrcText);

  useDialogA11y(dlgRef, expanded !== null);
  // Esc 互斥：全文浮窗存活时只关浮窗，否则退回主面板（PortsWorkbench.tsx:34-36 同款）
  useEscCapture(() => setExpanded(null), expanded !== null);
  useEscCapture(onBack, expanded === null);

  // 粘贴内容的 UTF-8 字节数（粘贴内容卡标题右侧展示）
  const srcBytes = useMemo(() => utf8Bytes(active), [active]);

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

  const copy = useCallback(
    async (key: string, value: string) => {
      try {
        const msg = await invoke<string>("copy_text", { text: value });
        flash(key);
        toast.success(msg);
      } catch (e) {
        toast.error("复制失败：" + String(e));
      }
    },
    [flash]
  );

  const openExpand = useCallback((label: string, text: string) => {
    setExpanded({ label, text });
  }, []);

  const header = (
    <PanelHeader
      icon={<Wrench className="size-4 text-primary" />}
      tag="文本工具"
    >
      <Button
        variant="ghost"
        size="sm"
        className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
        onClick={onBack}
      >
        <ArrowLeft className="size-3" />
        返回（Esc）
      </Button>
    </PanelHeader>
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
      ) : null}
      <div className="zap-tool-card zap-tool-card-editable">
        <div className="zap-tool-card-title">
          <ClipboardPaste className="size-3" />
          粘贴内容
          <span className="zap-tool-src-meta">{srcBytes} B</span>
        </div>
        <SourceRow value={active} onExpand={openExpand} onChange={setSrcText} />
      </div>
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
          onExpand={openExpand}
        />
        <ToolRow
          rowKey="dateToTs"
          label="日期 → 时间戳"
          value={bundle?.dateToTs}
          copied={copied}
          onCopy={copy}
          onExpand={openExpand}
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
          onExpand={openExpand}
        />
        <ToolRow
          rowKey="base64Enc"
          label="编码"
          value={bundle?.base64Enc}
          copied={copied}
          onCopy={copy}
          onExpand={openExpand}
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
          onExpand={openExpand}
        />
        <ToolRow
          rowKey="urlEnc"
          label="编码"
          value={bundle?.urlEnc}
          copied={copied}
          onCopy={copy}
          onExpand={openExpand}
        />
      </div>
      <div className="zap-tool-card">
        <div className="zap-tool-card-title">
          <Link2Off className="size-3" />
          链接清洗
        </div>
        <ToolRow
          rowKey="urlClean"
          label="去追踪参数"
          value={bundle?.urlClean}
          copied={copied}
          onCopy={copy}
          onExpand={openExpand}
        />
      </div>
      <div className="zap-tool-card">
        <div className="zap-tool-card-title">
          <Fingerprint className="size-3" />
          UUID v4
        </div>
        <div className="zap-tool-row">
          <span className="zap-tool-label">当前值</span>
          <span ref={uuidOverflow.ref} className="zap-tool-value" title={uuid}>
            {uuid}
          </span>
          {uuidOverflow.over && (
            <Button
              variant="ghost"
              size="icon-sm"
              className="zap-tool-expand"
              aria-label="展开 UUID"
              title="展开查看全文"
              onClick={() => openExpand("UUID", uuid)}
            >
              <Maximize2 className="size-3.5" />
            </Button>
          )}
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
          onExpand={openExpand}
        />
        <ToolRow
          rowKey="sha1"
          label="SHA1"
          value={bundle?.sha1}
          copied={copied}
          onCopy={copy}
          onExpand={openExpand}
        />
        <ToolRow
          rowKey="sha256"
          label="SHA256"
          value={bundle?.sha256}
          copied={copied}
          onCopy={copy}
          onExpand={openExpand}
        />
      </div>
      <div className="zap-tool-card">
        <div className="zap-tool-card-title">
          <Eraser className="size-3" />
          文本清洗
        </div>
        <ToolRow
          rowKey="zeroWidthStripped"
          label="去零宽字符"
          value={bundle?.zeroWidthStripped}
          copied={copied}
          onCopy={copy}
          onExpand={openExpand}
        />
        <ToolRow
          rowKey="quotesNormalized"
          label="弯引号 → 半角"
          value={bundle?.quotesNormalized}
          copied={copied}
          onCopy={copy}
          onExpand={openExpand}
        />
      </div>
      {srcText === null && !loadErr && text === undefined && (
        <div className="zap-tool-empty-all">正在读取剪贴板…</div>
      )}
    </div>
  );

  const counts = expanded ? textCounts(expanded.text) : null;

  return (
    <div className="zap-panel outline-none" tabIndex={-1} ref={focusRef}>
      {header}
      <div className="zap-form">{body}</div>
      {expanded && counts && (
        <div className="zap-dialog-backdrop" onClick={() => setExpanded(null)}>
          <div
            ref={dlgRef}
            className="zap-dialog zap-tool-dialog"
            role="dialog"
            aria-modal="true"
            aria-label={"查看全文：" + expanded.label}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="zap-tool-dlg-head">
              <span className="zap-tool-dlg-title">{expanded.label}</span>
              <span className="zap-tool-dlg-meta">
                {counts.chars} 字符 · {counts.bytes} B
              </span>
            </div>
            <div className="zap-tool-dlg-body">
              <pre className="zap-tool-dlg-text">{expanded.text}</pre>
            </div>
            <div className="zap-tool-dlg-actions">
              {/* 焦点默认停在「关闭」：误触 Enter/Esc 的落点都是无害动作 */}
              <Button
                variant="outline"
                autoFocus
                onClick={() => setExpanded(null)}
              >
                关闭
              </Button>
              <Button
                onClick={() => void copy("dlg:" + expanded.label, expanded.text)}
              >
                {copied === "dlg:" + expanded.label ? (
                  <Check className="size-3.5 text-success" />
                ) : (
                  <Copy className="size-3.5" />
                )}
                {copied === "dlg:" + expanded.label ? "已复制" : "复制全文"}
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
