import {
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  Check,
  ClipboardPaste,
  Copy,
  Download,
  Image as ImageIcon,
  LoaderCircle,
  Minus,
  Pin,
  Plus,
  TriangleAlert,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { useEscCapture } from "@/hooks/useEscCapture";
import { PanelHeader, useCopyFlash, type PanelProps } from "@/plugins/panel";
import type { SearchItem } from "@/plugins/types";
import {
  clampOffset,
  fitZoomState,
  zoomAtPoint,
  zoomCentered,
  ZOOM_MAX,
  ZOOM_MIN,
  ZOOM_STEP,
  ZOOM_WHEEL,
  type Size,
  type ZoomState,
} from "./zoom";

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
  /// 图片行号（clipRowId 提取，与文本 clip 同构）：预览/复制/导出/固定同键寻址
  rowid: number;
  /// 面板内固定后通知 App 重拉列表——Esc 退回时 setQuery 同值会被 React
  /// bailout，列表的固定徽标与置顶排序不刷（评审 P2）
  onPinned: () => void;
}

/// 剪贴板图片预览面板（二段式泛化首个消费者，契约见 plugins/panel.tsx）：
/// 大图确认 + 元信息，动作全鼠标——粘贴走 open_item 回贴管线（与行内 Enter
/// 同一套），仅复制不粘贴、另存为、固定；Esc 返回列表。
/// 缩放：滚轮（光标锚点）与工具条 ±/百分比，下限=完全展示（fit 为 1 倍）；
/// 放大后左键拖拽平移，双击回完全展示——不加任何需记忆的组合键
export function ClipImagePreview({ rowid, onBack, focusRef, onPinned }: Props) {
  const [meta, setMeta] = useState<ClipImageMeta | null>(null);
  const [loadErr, setLoadErr] = useState("");
  const [pinned, setPinned] = useState(false);
  const { copied, flash } = useCopyFlash(1500);
  // 仅复制对大图也可能非瞬时：即时态避免「点了没反应」的体感
  const [copying, setCopying] = useState(false);

  // 缩放与平移状态（数学见 ./zoom）：fit = img 未变换布局尺寸（max 约束
  // 贴合结果）；fit 未测得（图未加载）前滚轮/工具条不动。视口用回调 ref——
  // div 要等 meta 到达才渲染，普通 ref 在挂载期 effect 里恒 null
  const [zoom, setZoom] = useState<ZoomState>(fitZoomState);
  const [fit, setFit] = useState<Size | null>(null);
  const [viewEl, setViewEl] = useState<HTMLDivElement | null>(null);
  const [dragging, setDragging] = useState(false);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const dragRef = useRef<{
    px: number;
    py: number;
    ox: number;
    oy: number;
  } | null>(null);

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

  // 换图重置缩放与 fit 基准：新图从完全展示开始，量测完成前滚轮/工具条
  // 不动（旧 fit 是别的图的尺寸，不能当新图基准）
  useEffect(() => {
    setZoom(fitZoomState());
    setFit(null);
    dragRef.current = null;
    setDragging(false);
  }, [rowid]);

  // fit 尺寸：img 未变换的布局尺寸，加载完成与视口变化时重测
  const measureFit = () => {
    const img = imgRef.current;
    if (!img || !img.naturalWidth) return;
    const next = { w: img.offsetWidth, h: img.offsetHeight };
    setFit((prev) =>
      prev && prev.w === next.w && prev.h === next.h ? prev : next,
    );
  };

  // 滚轮缩放（光标为锚点）：原生非 passive 监听——React 合成 onWheel 被动化，
  // preventDefault 失效会连带动被面板盖住的列表滚动
  useEffect(() => {
    const el = viewEl;
    if (!el || !fit) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = el.getBoundingClientRect();
      const viewport = { w: rect.width, h: rect.height };
      setZoom((z) =>
        zoomAtPoint(
          z,
          {
            x: e.clientX - rect.left - viewport.w / 2,
            y: e.clientY - rect.top - viewport.h / 2,
          },
          e.deltaY < 0 ? ZOOM_WHEEL : 1 / ZOOM_WHEEL,
          viewport,
          fit,
        ),
      );
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [viewEl, fit]);

  // 视口尺寸变化：重测 fit 并按新视口夹紧平移（倍率保持）
  useEffect(() => {
    if (!viewEl) return;
    const ro = new ResizeObserver(() => {
      measureFit();
      const img = imgRef.current;
      if (!img || !img.naturalWidth) return;
      const viewport = {
        w: viewEl.getBoundingClientRect().width,
        h: viewEl.getBoundingClientRect().height,
      };
      setZoom((z) => {
        const o = clampOffset(z.ox, z.oy, z.s, viewport, {
          w: img.offsetWidth,
          h: img.offsetHeight,
        });
        return o.x === z.ox && o.y === z.oy ? z : { ...z, ox: o.x, oy: o.y };
      });
    });
    ro.observe(viewEl);
    return () => ro.disconnect();
  }, [viewEl]);

  // 工具条 ±：视口中心为锚点缩放；百分比标签点击 = 回完全展示
  const zoomStep = (factor: number) => {
    if (!viewEl || !fit) return;
    const rect = viewEl.getBoundingClientRect();
    setZoom((z) =>
      zoomCentered(z, factor, { w: rect.width, h: rect.height }, fit),
    );
  };

  // 放大后左键拖拽平移：基点式（按下手位 + 绝对位移），move 全程夹紧；
  // fit 态（s = MIN）图像已完整可见，不平移
  const onPanDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || zoom.s <= ZOOM_MIN) return;
    dragRef.current = {
      px: e.clientX,
      py: e.clientY,
      ox: zoom.ox,
      oy: zoom.oy,
    };
    e.currentTarget.setPointerCapture(e.pointerId);
    setDragging(true);
  };

  const onPanMove = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = dragRef.current;
    if (!d || !viewEl || !fit) return;
    const rect = viewEl.getBoundingClientRect();
    setZoom((z) => {
      const o = clampOffset(
        d.ox + (e.clientX - d.px),
        d.oy + (e.clientY - d.py),
        z.s,
        { w: rect.width, h: rect.height },
        fit,
      );
      return { s: z.s, ox: o.x, oy: o.y };
    });
  };

  const onPanEnd = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (!dragRef.current) return;
    dragRef.current = null;
    setDragging(false);
    if (e.currentTarget.hasPointerCapture(e.pointerId))
      e.currentTarget.releasePointerCapture(e.pointerId);
  };

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
    if (copying) return;
    setCopying(true);
    invoke("clipimg_copy", { id: rowid })
      .then(() => {
        flash("copy");
        toast.success("已复制，可粘贴");
      })
      .catch((e) => toast.error(String(e)))
      .finally(() => setCopying(false));
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
    <PanelHeader
      icon={<ImageIcon className="size-4 text-primary" />}
      tag="图片预览"
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
              <div
                className={`zap-imgprev-view${zoom.s > ZOOM_MIN ? " zoomed" : ""}${
                  dragging ? " dragging" : ""
                }`}
                ref={setViewEl}
                onPointerDown={onPanDown}
                onPointerMove={onPanMove}
                onPointerUp={onPanEnd}
                onPointerCancel={onPanEnd}
                onDoubleClick={() => setZoom(fitZoomState())}
              >
                <img
                  ref={imgRef}
                  src={`data:image/png;base64,${meta.pngBase64}`}
                  alt={`剪贴板图片 ${meta.width}×${meta.height}`}
                  draggable={false}
                  onLoad={measureFit}
                  // 缩放渲染：中心缩放 + 中心平移（flex 已把 img 居中在视口
                  // 中心，transform-origin 默认即元素中心，与 zoom.ts 的
                  // 「中心 + s·局部偏移 + o」屏幕坐标模型严格一致）
                  style={{
                    transform: `translate(${zoom.ox}px, ${zoom.oy}px) scale(${zoom.s})`,
                  }}
                />
              </div>
              <div className="zap-imgprev-bar">
                <Button variant="ghost" size="sm" onClick={paste}>
                  <ClipboardPaste className="size-3.5" />
                  粘贴
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={copying}
                  onClick={copyOnly}
                >
                  {copying ? (
                    <LoaderCircle className="size-3.5 animate-spin" />
                  ) : copied === "copy" ? (
                    <Check className="size-3.5 text-success" />
                  ) : (
                    <Copy className="size-3.5" />
                  )}
                  {copying ? "复制中…" : "仅复制"}
                </Button>
                <Button variant="ghost" size="sm" onClick={() => void saveAs()}>
                  <Download className="size-3.5" />
                  另存为
                </Button>
                <Button variant="ghost" size="sm" onClick={togglePin}>
                  <Pin className="size-3.5" />
                  {pinned ? "取消固定" : "固定"}
                </Button>
                <div
                  className="zap-imgprev-zoom"
                  role="group"
                  aria-label="缩放"
                >
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={zoom.s <= ZOOM_MIN}
                    aria-label="缩小"
                    onClick={() => zoomStep(1 / ZOOM_STEP)}
                  >
                    <Minus className="size-3.5" />
                  </Button>
                  <button
                    type="button"
                    className="zap-imgprev-zoom-label"
                    title="点击回到完全展示"
                    onClick={() => setZoom(fitZoomState())}
                  >
                    {zoom.s > ZOOM_MIN + 1e-3
                      ? `${Math.round(zoom.s * 100)}%`
                      : "适应"}
                  </button>
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={zoom.s >= ZOOM_MAX - 1e-3}
                    aria-label="放大"
                    onClick={() => zoomStep(ZOOM_STEP)}
                  >
                    <Plus className="size-3.5" />
                  </Button>
                </div>
              </div>
            </div>
            <div className="zap-imgprev-footer">
              <span className="zap-imgprev-meta">
                {meta.width}×{meta.height} · {humanSize(meta.size)} · 复制于{" "}
                {new Date(meta.createdAt).toLocaleString()}
              </span>
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
