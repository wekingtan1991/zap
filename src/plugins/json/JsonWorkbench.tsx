import React, { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowLeft,
  Braces,
  Check,
  ChevronRight,
  Copy,
  TriangleAlert,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";

/// 后端 jsontools::JsonReport 的线协议镜像
interface JsonReport {
  valid: boolean;
  kind: string;
  bytes: number;
  error: string | null;
}

interface Props {
  /// 前缀域文本源（随输入实时变化）；识别条目进入时不传
  text?: string;
  /// 剪贴板行号源：进入时回查一次，以识别时刻的内容为准
  clipRow?: number;
  /// 完整面板（识别条目进入：自带头部与返回按钮）；缺省嵌在结果区（前缀域）
  full?: boolean;
  onBack: () => void;
  /// 子页面焦点接管（防 Esc 失效，见 AppView 同名约定），仅 full 模式使用
  focusRef?: (el: HTMLDivElement | null) => void;
  onKeyDown?: (e: React.KeyboardEvent) => void;
}

/// JSON 工作台：状态条 + 树/文本双视图 + 复制美化/压缩。
/// 分析与格式化在后端（serde_json），前端只做展示与树形折叠
export function JsonWorkbench({
  text,
  clipRow,
  full,
  onBack,
  focusRef,
  onKeyDown,
}: Props) {
  const [srcText, setSrcText] = useState<string | null>(
    text !== undefined ? text : null
  );
  const [loadErr, setLoadErr] = useState("");
  const [report, setReport] = useState<JsonReport | null>(null);
  const [pretty, setPretty] = useState("");
  const [compact, setCompact] = useState("");
  const [mode, setMode] = useState<"tree" | "text">("tree");
  const [copied, setCopied] = useState<
    "pretty" | "compact" | "yaml" | "ts" | null
  >(null);
  /// 后端命令级错误（插件停用等）：与内容级校验错误（report.error）分开呈现
  const [cmdErr, setCmdErr] = useState("");

  const textMode = text !== undefined;

  // 行号源：进入时回查一次
  useEffect(() => {
    if (textMode) return;
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
  }, [textMode, clipRow]);

  const active = textMode ? (text ?? "") : (srcText ?? "");

  // 分析防抖：前缀域每击键触发；行号源加载后只触发一次
  useEffect(() => {
    const timer = setTimeout(async () => {
      if (!active.trim()) {
        setReport(null);
        setPretty("");
        setCompact("");
        setCmdErr("");
        return;
      }
      try {
        const rep = await invoke<JsonReport>("json_analyze", { text: active });
        setCmdErr("");
        setReport(rep);
        if (rep.valid) {
          const [p, c] = await Promise.all([
            invoke<string>("json_format", { text: active, pretty: true }),
            invoke<string>("json_format", { text: active, pretty: false }),
          ]);
          setPretty(p);
          setCompact(c);
        }
      } catch (e) {
        // 命令级失败（插件停用等）：状态条呈现，不残留旧内容
        setCmdErr(String(e));
        setReport(null);
        setPretty("");
        setCompact("");
      }
    }, 120);
    return () => clearTimeout(timer);
  }, [active]);

  const treeRoot = useMemo(() => {
    if (!report?.valid || !pretty) return undefined;
    try {
      return JSON.parse(pretty) as unknown;
    } catch {
      return undefined;
    }
  }, [report?.valid, pretty]);

  const copy = useCallback(
    async (which: "pretty" | "compact") => {
      const t = which === "pretty" ? pretty : compact;
      if (!t) return;
      try {
        const msg = await invoke<string>("copy_text", { text: t });
        setCopied(which);
        setTimeout(() => setCopied((c) => (c === which ? null : c)), 1600);
        toast.success(msg);
      } catch (e) {
        toast.error("复制失败：" + String(e));
      }
    },
    [pretty, compact]
  );

  /// 转换复制：点击时按需转换（每击键都转 TS/YAML 太浪费），失败 toast 报因
  const convertCopy = useCallback(
    async (kind: "yaml" | "ts") => {
      if (!active.trim()) return;
      try {
        const out = await invoke<string>(
          kind === "yaml" ? "json_to_yaml" : "json_to_ts",
          { text: active }
        );
        const msg = await invoke<string>("copy_text", { text: out });
        setCopied(kind);
        setTimeout(() => setCopied((c) => (c === kind ? null : c)), 1600);
        toast.success(msg);
      } catch (e) {
        toast.error((kind === "yaml" ? "转 YAML 失败：" : "转 TS 失败：") + String(e));
      }
    },
    [active]
  );

  const header = (
    <div className="zap-input-row">
      <span className="zap-logo">
        <Braces className="size-4 text-primary" />
      </span>
      <span className="zap-vault-tag">JSON 工具</span>
      {full && (
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
          onClick={onBack}
        >
          <ArrowLeft className="size-3" />
          返回（Esc）
        </Button>
      )}
    </div>
  );

  const body = (
    <div className="zap-json-body">
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
      ) : (
        report && (
          <div
            className={"zap-json-status" + (report.valid ? "" : " zap-json-bad")}
          >
            {report.valid ? (
              <Check className="size-3.5" />
            ) : (
              <TriangleAlert className="size-3.5" />
            )}
            <span>
              {report.valid ? `合法 JSON · ${report.kind}` : report.error}
            </span>
            <span className="zap-json-meta">{report.bytes} B</span>
          </div>
        )
      )}
      <div className="zap-json-view">
        {!active.trim() ? (
          <div className="zap-json-empty">
            <Braces className="size-5" />
            <span>
              {loadErr
                ? "无法加载内容"
                : textMode
                  ? "在 json 后继续输入或粘贴 JSON 文本"
                  : "正在读取剪贴板…"}
            </span>
          </div>
        ) : mode === "tree" && treeRoot !== undefined ? (
          <JsonNode name="" value={treeRoot} depth={0} />
        ) : !pretty ? (
          <div className="zap-json-empty">
            <span>{cmdErr ? "无法分析" : "分析中…"}</span>
          </div>
        ) : (
          <>
            {pretty
              .split("\n")
              .slice(0, MAX_DISPLAY_LINES)
              .map((ln, i) => (
                <div className="zap-json-line" key={i}>
                  <span className="zap-json-ln">{i + 1}</span>
                  <span>{ln}</span>
                </div>
              ))}
            {pretty.split("\n").length > MAX_DISPLAY_LINES && (
              <div className="zap-json-count">
                …已截断展示前 {MAX_DISPLAY_LINES} 行，完整内容请复制
              </div>
            )}
          </>
        )}
      </div>
      <div className="zap-json-actions">
        <Button
          variant="outline"
          size="sm"
          disabled={!report?.valid || treeRoot === undefined}
          onClick={() => setMode(mode === "tree" ? "text" : "tree")}
        >
          {mode === "tree" ? "文本视图" : "树形视图"}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          disabled={!report?.valid}
          onClick={() => void copy("pretty")}
        >
          {copied === "pretty" ? (
            <Check className="size-3.5 text-success" />
          ) : (
            <Copy className="size-3.5" />
          )}
          复制美化
        </Button>
        <Button
          variant="secondary"
          size="sm"
          disabled={!report?.valid}
          onClick={() => void copy("compact")}
        >
          {copied === "compact" ? (
            <Check className="size-3.5 text-success" />
          ) : (
            <Copy className="size-3.5" />
          )}
          复制压缩
        </Button>
      </div>
      <div className="zap-json-actions">
        <Button
          variant="secondary"
          size="sm"
          disabled={!active.trim()}
          onClick={() => void convertCopy("yaml")}
        >
          {copied === "yaml" ? (
            <Check className="size-3.5 text-success" />
          ) : (
            <Copy className="size-3.5" />
          )}
          复制 YAML
        </Button>
        <Button
          variant="secondary"
          size="sm"
          disabled={!active.trim()}
          onClick={() => void convertCopy("ts")}
        >
          {copied === "ts" ? (
            <Check className="size-3.5 text-success" />
          ) : (
            <Copy className="size-3.5" />
          )}
          复制 TS 类型
        </Button>
      </div>
    </div>
  );

  if (full) {
    return (
      <div
        className="zap-panel outline-none"
        tabIndex={-1}
        ref={focusRef}
        onKeyDown={onKeyDown}
      >
        {header}
        <div className="zap-form">{body}</div>
      </div>
    );
  }
  return body;
}

/// 大文本展示防线：树/文本模式行数截断（渲染帧率优先，完整内容走复制）
const MAX_DISPLAY_LINES = 3000;

/// 标量值渲染（按类型着色）
function JsonScalar({ value }: { value: unknown }) {
  if (value === null) return <span className="zap-json-null">null</span>;
  switch (typeof value) {
    case "string":
      return <span className="zap-json-str">{JSON.stringify(value)}</span>;
    case "number":
      return <span className="zap-json-num">{String(value)}</span>;
    case "boolean":
      return <span className="zap-json-bool">{String(value)}</span>;
    default:
      return <span className="zap-json-null">{String(value)}</span>;
  }
}

/// 树节点：对象/数组可折叠（默认展开两层），标量直接着色。
/// 排版对齐主流树视图（VS Code/DevTools）：箭头占固定槽位，{ 与 } 同列，
/// 键与冒号紧贴为一个词元（"test": 1，冒号前不留空）
function JsonNode({
  name,
  value,
  depth,
}: {
  name: string;
  value: unknown;
  depth: number;
}) {
  const [open, setOpen] = useState(depth < 2);

  const kv =
    name !== "" ? (
      <span className="zap-json-kv">
        <span className="zap-json-key">{JSON.stringify(name)}</span>
        <span className="zap-json-colon">:</span>
      </span>
    ) : null;

  if (value === null || typeof value !== "object") {
    return (
      <div className="zap-json-row">
        <span className="zap-json-gutter" />
        {kv}
        <JsonScalar value={value} />
      </div>
    );
  }

  const isArray = Array.isArray(value);
  const entries: [string, unknown][] = isArray
    ? (value as unknown[]).map((v, i) => [String(i), v])
    : Object.entries(value as Record<string, unknown>);
  const braceOpen = isArray ? "[" : "{";
  const braceClose = isArray ? "]" : "}";

  return (
    <div>
      <button
        className="zap-json-toggle"
        onClick={() => setOpen((o) => !o)}
        type="button"
      >
        <span className="zap-json-gutter">
          <ChevronRight className={"size-3" + (open ? " zap-json-open" : "")} />
        </span>
        {kv}
        <span>{braceOpen}</span>
        {!open && <span className="zap-json-count">{entries.length} 项</span>}
        {!open && <span>{braceClose}</span>}
      </button>
      {open && (
        <div className="zap-json-node">
          {entries.map(([k, v], i) => (
            <JsonNode key={`${depth}:${i}`} name={k} value={v} depth={depth + 1} />
          ))}
        </div>
      )}
      {open && (
        <div className="zap-json-row">
          <span className="zap-json-gutter" />
          <span>{braceClose}</span>
        </div>
      )}
    </div>
  );
}
