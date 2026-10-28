import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  Check,
  ClipboardPaste,
  Copy,
  Download,
  Image as ImageIcon,
  Pin,
  TriangleAlert,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { useEscCapture } from "@/hooks/useEscCapture";
import type { PanelProps } from "@/plugins/panel";
import type { SearchItem } from "@/plugins/types";

/// 后端 clipimg_preview 的线协议镜像
interface ClipImageMeta {
  width: number;
  height: number;
  size: number;
  createdAt: number;
  pinned: boolean;
  pngBase64: string;
}

/// 与后端 clipimage::human_size 同口径（面板元信息行展示）
function humanSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

interface Props extends PanelProps {
  /// 图片行号（clipImageRowId 提取）：预览/复制/导出/固定同键寻址
  rowid: number;
  /// 面板内固定后通知 App 重拉列表——Esc 退回时 setQuery 同值会被 React
  /// bailout，列表的固定徽标与置顶排序不刷（评审 P2）
  onPinned: () => void;
}

/// 剪贴板图片预览面板（二段式泛化首个消费者，契约见 plugins/panel.ts）：
/// 大图确认 + 元信息，动作全鼠标——粘贴走 open_item 回贴管线（与行内 Enter
/// 同一套），仅复制不粘贴、另存为、固定；Esc 返回列表
export function ClipImagePreview({ rowid, onBack, focusRef, onPinned }: Props) {
  const [meta, setMeta] = useState<ClipImageMeta | null>(null);
  const [loadErr, setLoadErr] = useState("");
  const [pinned, setPinned] = useState(false);
  const [copied, setCopied] = useState(false);

  // 独立面板：窗口捕获兜 Esc（焦点掉 body 后面板 onKeyDown 收不到）
  useEscCapture(onBack, true);

  // 进入时点查一次（行号寻址与回贴同源）；已被清理的错误在面板内呈现
  useEffect(() => {
    let cancelled = false;
    invoke<ClipImageMeta>("clipimg_preview", { id: rowid })
      .then((m) => {
        if (!cancelled) {
          setMeta(m);
          setPinned(m.pinned);
        }
      })
      .catch((e) => {
        if (!cancelled) setLoadErr(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [rowid]);

  // 粘贴：走 open_item 的 clipimg 分支（自动粘贴开启=焦点交还+注入；
  // 关闭=仅回填），与行内 Enter 完全同管线
  const paste = () => {
    const item: SearchItem = {
      id: `clipimg:${rowid}`,
      title: "图片",
      subtitle: "",
      kind: "clipimg",
      target: String(rowid),
      score: 0,
    };
    invoke("open_item", { item })
      .then(() => {
        toast.success("已复制到剪贴板，可粘贴");
        onBack();
      })
      .catch((e) => toast.error("粘贴失败：" + String(e)));
  };

  const copyOnly = () => {
    invoke("clipimg_copy", { id: rowid })
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
        toast.success("已复制，可粘贴");
      })
      .catch((e) => toast.error(String(e)));
  };

  const saveAs = async () => {
    if (!meta) return;
    const path = await save({
      defaultPath: `zap_clip_${meta.width}x${meta.height}.png`,
      filters: [{ name: "PNG 图片", extensions: ["png"] }],
    });
    if (!path) return;
    try {
      await invoke("clipimg_export", { id: rowid, path });
      toast.success("已保存：" + path);
    } catch (e) {
      toast.error(String(e));
    }
  };

  // 直连图片表（clipimg_pin）：clip_pin 文本表先命中，两表自增行号
  // 撞号时会固定到同号的文本行
  const togglePin = () => {
    invoke<boolean>("clipimg_pin", { id: rowid })
      .then((p) => {
        setPinned(p);
        onPinned();
        toast.success(p ? "已固定" : "已取消固定");
      })
      .catch((e) => toast.error("固定失败：" + String(e)));
  };

  const header = (
    <div className="zap-input-row">
      <span className="zap-logo">
        <ImageIcon className="size-4 text-primary" />
      </span>
      <span className="zap-vault-tag">图片预览</span>
      <Button
        variant="ghost"
        size="sm"
        className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
        onClick={onBack}
      >
        <ArrowLeft className="size-3" />
        返回（Esc）
      </Button>
    </div>
  );

  const body = (
    <div className="zap-imgprev-body">
      {loadErr ? (
        <div className="zap-json-status zap-json-bad">
          <TriangleAlert className="size-3.5" />
          <span>读取图片失败：{loadErr}</span>
        </div>
      ) : (
        meta && (
          <>
            <div className="zap-imgprev-stage">
              <img
                src={`data:image/png;base64,${meta.pngBase64}`}
                alt={`剪贴板图片 ${meta.width}×${meta.height}`}
              />
            </div>
            <div className="zap-imgprev-footer">
              <span className="zap-imgprev-meta">
                {meta.width}×{meta.height} · {humanSize(meta.size)} · 复制于{" "}
                {new Date(meta.createdAt).toLocaleString()}
              </span>
              <div className="zap-imgprev-actions">
                <Button variant="default" size="sm" onClick={paste}>
                  <ClipboardPaste className="size-3.5" />
                  粘贴
                </Button>
                <Button variant="outline" size="sm" onClick={copyOnly}>
                  {copied ? (
                    <Check className="size-3.5 text-success" />
                  ) : (
                    <Copy className="size-3.5" />
                  )}
                  仅复制
                </Button>
                <Button variant="outline" size="sm" onClick={() => void saveAs()}>
                  <Download className="size-3.5" />
                  另存为
                </Button>
                <Button variant="outline" size="sm" onClick={togglePin}>
                  <Pin className="size-3.5" />
                  {pinned ? "取消固定" : "固定"}
                </Button>
              </div>
            </div>
          </>
        )
      )}
    </div>
  );

  return (
    <div className="zap-panel outline-none" tabIndex={-1} ref={focusRef}>
      {header}
      {body}
    </div>
  );
}
