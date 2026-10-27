import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as pickPath } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import {
  ArrowLeft,
  FilePlus2,
  FolderPlus,
  Inbox,
  Search,
  TriangleAlert,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { useEscCapture } from "@/hooks/useEscCapture";
import { clampMenuPos } from "@/lib/ctxMenu";
import { VaultRowPin } from "@/plugins/vault/VaultRowPin";

/// stage_list 行协议（providers/stage.rs StageRecord 的 camelCase 镜像）
interface StageRow {
  id: number;
  path: string;
  name: string;
  isDir: boolean;
  pinned: boolean;
  missing: boolean;
  icon?: string | null;
  addedAt: number;
}

/// 右键菜单项：run 缺省 = 纯占位（配合 disabled）
interface MenuItem {
  label: string;
  run?: () => void;
  disabled?: boolean;
}

/// 右键菜单：row 非空 = 条目菜单；null = 空区菜单（清空 / 手动添加）
interface CtxMenu {
  x: number;
  y: number;
  row: StageRow | null;
}

interface Props {
  /// 子页面焦点接管（防 React 根外失焦导致 Esc 失效，见 AppView 同名约定）
  focusRef: (el: HTMLDivElement | null) => void;
  onClose: () => void;
}

/// 中转站视图：拖入即存（存路径引用不存副本）、点击/Enter 打开（与主列表
/// 单击即开心智一致，误开无破坏性）、右键操作、固定条目置顶且不被容量淘汰，
/// 失效条目灰显可重新定位，移出/清空支持 toast 撤销。
/// 视图打开期间面板被 AppView 置为 hold（拖拽自资源管理器时焦点在 Explorer，
/// 面板不能失焦即隐）；Esc / 返回由 AppView 释放 hold。
export function StageView({ focusRef, onClose }: Props) {
  const [rows, setRows] = useState<StageRow[]>([]);
  const [active, setActive] = useState(0);
  const [dragging, setDragging] = useState(false);
  const [menu, setMenu] = useState<CtxMenu | null>(null);
  /// 本地过滤词（容量上限 200 条，滚动翻找不可用）：匹配名称或路径，不走后端
  const [filter, setFilter] = useState("");
  const activeItemRef = useRef<HTMLDivElement | null>(null);

  /// 过滤后的可见行：键盘导航、列表渲染、右键菜单都以它为准
  const shown = useMemo(() => {
    const f = filter.trim().toLowerCase();
    if (!f) return rows;
    return rows.filter((r) => (r.name + " " + r.path).toLowerCase().includes(f));
  }, [rows, filter]);

  // 过滤词变化即回选中首行：旧 active 在过滤后的列表里可能越界
  useEffect(() => {
    setActive(0);
  }, [filter]);

  const load = useCallback(async () => {
    try {
      setRows(await invoke<StageRow[]>("stage_list"));
    } catch {
      // 列表拉取失败保持现状
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  /// 批量放入（拖拽 / 对话框添加共用）：结果分桶提示——新增、重复置顶、失败
  const addPaths = useCallback(
    async (paths: string[]) => {
      if (paths.length === 0) return;
      try {
        const r = await invoke<{
          added: number;
          total: number;
          failed: string[];
        }>("stage_add", { paths });
        await load();
        if (r.added === 0 && r.failed.length > 0) {
          toast.error(r.failed[0]);
        } else if (r.failed.length > 0) {
          toast.warning(`已放入 ${r.added} 条，${r.failed.length} 条失败`);
        } else if (r.added === 0) {
          toast.success(`已在中转站（已置顶）· 共 ${r.total} 条`);
        } else {
          toast.success(`已放入中转站 · 共 ${r.total} 条`);
        }
      } catch (e) {
        toast.error(String(e));
      }
    },
    [load]
  );

  // 拖拽落区：监听只在本视图挂载期间存活，普通搜索态不受影响（误投防护）。
  // cancelled-flag：cleanup 先于 then 执行时立即注销，防 StrictMode 双挂载泄漏
  useEffect(() => {
    let cancelled = false;
    let un: (() => void) | null = null;
    getCurrentWebview()
      .onDragDropEvent((e) => {
        const t = e.payload.type;
        if (t === "enter" || t === "over") {
          setDragging(true);
        } else if (t === "leave") {
          setDragging(false);
        } else if (t === "drop") {
          setDragging(false);
          void addPaths([...e.payload.paths]);
        }
      })
      .then((f) => {
        if (cancelled) f();
        else un = f;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      un?.();
    };
  }, [addPaths]);

  // 方向键把选中行移出可视区时滚动补位（同主列表 nearest 口径）
  useEffect(() => {
    activeItemRef.current?.scrollIntoView({ block: "nearest" });
  }, [active]);

  /// 打开（点击 / Enter 共用）：交给后端 open_item 的 stage 分支，
  /// 成功即收面板（与普通条目启动同口径）
  const openRow = useCallback((row: StageRow) => {
    if (row.missing) {
      toast.error("原文件已不存在，可右键重新定位");
      return;
    }
    invoke("open_item", {
      item: {
        id: `stage:${row.id}`,
        title: row.name,
        subtitle: row.path,
        kind: "stage",
        target: row.path,
        score: 0,
      },
    }).catch((e) => toast.error("打开失败：" + String(e)));
  }, []);

  /// 复制文件到剪贴板（CF_HDROP）：可粘贴到任意位置，中转站「携带」语义
  const copyFile = useCallback(async (row: StageRow) => {
    try {
      await invoke("stage_copy_file", { paths: [row.path] });
      toast.success("已复制文件，可去任意位置粘贴");
    } catch (e) {
      toast.error(String(e));
    }
  }, []);

  const revealRow = useCallback((row: StageRow) => {
    invoke("secondary_action", { action: "reveal", target: row.path }).catch(
      (e) => toast.error(String(e))
    );
  }, []);

  const copyPath = useCallback((row: StageRow) => {
    invoke("secondary_action", { action: "copy_path", target: row.path })
      .then(() => toast.success("已复制"))
      .catch((e) => toast.error(String(e)));
  }, []);

  const togglePin = useCallback(
    async (row: StageRow) => {
      try {
        await invoke("stage_pin", { id: row.id });
        await load();
      } catch (e) {
        toast.error(String(e));
      }
    },
    [load]
  );

  /// 失效条目重绑路径（保留固定态）
  const relocate = useCallback(
    async (row: StageRow) => {
      const picked = await pickPath({ multiple: false });
      if (!picked) return;
      try {
        await invoke("stage_relocate", { id: row.id, path: picked });
        await load();
        toast.success("已重新定位");
      } catch (e) {
        toast.error(String(e));
      }
    },
    [load]
  );

  /// 撤销恢复：按快照原样插回（含固定态与时间戳，排序位不变；
  /// 路径已被重新放入时后端幂等忽略）
  const restoreRows = useCallback(
    async (snapshot: StageRow[]) => {
      try {
        for (const r of snapshot) {
          await invoke("stage_restore", {
            path: r.path,
            isDir: r.isDir,
            pinned: r.pinned,
            addedAt: r.addedAt,
          });
        }
        await load();
      } catch (e) {
        toast.error(String(e));
      }
    },
    [load]
  );

  const removeRow = useCallback(
    async (row: StageRow) => {
      try {
        await invoke("stage_remove", { id: row.id });
        await load();
        toast.success("已移出中转站（原文件不受影响）", {
          action: { label: "撤销", onClick: () => void restoreRows([row]) },
          duration: 6000,
        });
      } catch (e) {
        toast.error(String(e));
      }
    },
    [load, restoreRows]
  );

  const clearUnpinned = useCallback(async () => {
    // 清空前持快照供撤销（撤销按原样插回，顺序与固定态不丢）
    const snapshot = rows.filter((r) => !r.pinned);
    if (snapshot.length === 0) return;
    try {
      const n = await invoke<number>("stage_clear");
      await load();
      toast.success(`已清空 ${n} 条（固定条目保留）`, {
        action: { label: "撤销", onClick: () => void restoreRows(snapshot) },
        duration: 6000,
      });
    } catch (e) {
      toast.error(String(e));
    }
  }, [rows, load, restoreRows]);

  const addFilesDialog = useCallback(async () => {
    const picked = await pickPath({ multiple: true });
    if (picked) void addPaths(picked);
  }, [addPaths]);

  const addDirsDialog = useCallback(async () => {
    const picked = await pickPath({ directory: true, multiple: true });
    if (picked) void addPaths(picked);
  }, [addPaths]);

  const pinnedCount = rows.filter((r) => r.pinned).length;
  const unpinnedCount = rows.length - pinnedCount;

  /// 条目右键菜单：失效条目没有「打开 / 复制文件」（原文件不在了），
  /// 换「重新定位」；reveal / 复制路径对死链仍有意义（帮用户排查）
  const rowMenuItems = (row: StageRow): MenuItem[] => {
    if (row.missing) {
      return [
        { label: "重新定位…", run: () => void relocate(row) },
        { label: "打开所在文件夹", run: () => revealRow(row) },
        { label: "复制路径", run: () => copyPath(row) },
        { label: "固定 / 取消固定", run: () => void togglePin(row) },
        { label: "移出中转站", run: () => void removeRow(row) },
      ];
    }
    return [
      { label: "打开", run: () => openRow(row) },
      { label: "复制文件（可粘贴）", run: () => void copyFile(row) },
      { label: "打开所在文件夹", run: () => revealRow(row) },
      { label: "复制路径", run: () => copyPath(row) },
      { label: "固定 / 取消固定", run: () => void togglePin(row) },
      { label: "移出中转站", run: () => void removeRow(row) },
    ];
  };

  /// 空区右键菜单：清空仅在未固定条目存在时可用
  const emptyMenuItems = (): MenuItem[] => [
    {
      label: `清空临时区（${unpinnedCount} 条）`,
      run: () => void clearUnpinned(),
      disabled: unpinnedCount === 0,
    },
    { label: "添加文件…", run: () => void addFilesDialog() },
    { label: "添加文件夹…", run: () => void addDirsDialog() },
  ];

  const menuItems = menu ? (menu.row ? rowMenuItems(menu.row) : emptyMenuItems()) : [];
  // 菜单定位随条数夹紧（与主面板右键菜单共用同一帮手）
  const menuPos = menu ? clampMenuPos(menu.x, menu.y, menuItems.length) : null;

  // 右键菜单存活时 Esc 优先收菜单（enabled 互斥，同 VaultDetailView 约定）；
  // 过滤词非空时 Esc 先清过滤再谈退出——渐进退出与主面板「有词清词」同口径
  useEscCapture(() => setMenu(null), menu !== null);
  useEscCapture(() => {
    if (filter) {
      setFilter("");
      return;
    }
    onClose();
  }, menu === null);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, Math.max(shown.length - 1, 0)));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const row = shown[active];
      if (row) openRow(row);
    }
  };

  return (
    <div
      className="zap-panel zap-view-in outline-none"
      tabIndex={-1}
      ref={focusRef}
      onKeyDown={onKeyDown}
    >
      <div className="zap-input-row">
        <span className="zap-logo">
          <Inbox className="size-4 text-primary-foreground" />
        </span>
        <span className="zap-vault-tag zap-vault-tag-pill">中转站</span>
        <span className="zap-stage-count">
          共 {rows.length} 条
          {pinnedCount > 0 && ` · 固定 ${pinnedCount} 条`}
          {filter.trim() && ` · 匹配 ${shown.length} 条`}
        </span>
        <Button
          variant="ghost"
          size="sm"
          className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
          onClick={() => void addFilesDialog()}
        >
          <FilePlus2 className="size-3" />
          文件
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
          onClick={() => void addDirsDialog()}
        >
          <FolderPlus className="size-3" />
          文件夹
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
          onClick={onClose}
        >
          <ArrowLeft className="size-3" />
          返回（Esc）
        </Button>
      </div>
      {/* 过滤行：列表非空时出现（空列表无处可滤），本地过滤不走后端 */}
      {rows.length > 0 && (
        <div className="zap-stage-filter">
          <Search className="size-3.5 shrink-0" />
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="过滤名称或路径…"
            spellCheck={false}
          />
        </div>
      )}
      <div
        className={
          "zap-results zap-stage" + (dragging ? " zap-stage-drop" : "")
        }
        onContextMenu={(e) => {
          e.preventDefault();
          setMenu({ x: e.clientX, y: e.clientY, row: null });
        }}
      >
        {rows.length === 0 ? (
          <div className="zap-empty">
            <span className="zap-empty-ico">
              <Inbox className="size-5" />
            </span>
            <span className="zap-empty-title">中转站是空的</span>
            <span className="zap-empty-sub">
              把文件或文件夹拖进面板，松手即存
            </span>
            <span className="zap-empty-sub">
              搜索结果右键「放入中转站」 · 右上角按钮手动添加
            </span>
            <span className="zap-empty-sub">常用文件夹可固定，永不清出</span>
          </div>
        ) : shown.length === 0 ? (
          <div className="zap-empty">
            <span className="zap-empty-ico">
              <Search className="size-5" />
            </span>
            <span className="zap-empty-title">没有匹配的条目</span>
            <span className="zap-empty-sub">换个关键词，或清空过滤框</span>
          </div>
        ) : (
          shown.map((row, i) => (
            <div
              key={row.id}
              ref={i === active ? activeItemRef : undefined}
              className={
                "zap-item" +
                (i === active ? " active" : "") +
                (row.missing ? " zap-stage-dead" : "")
              }
              onMouseEnter={() => setActive(i)}
              onClick={() => openRow(row)}
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setActive(i);
                setMenu({ x: e.clientX, y: e.clientY, row });
              }}
            >
              {row.icon ? (
                <img className="zap-ico" src={row.icon} alt="" draggable={false} />
              ) : (
                <Badge variant="file">{row.isDir ? "文件夹" : "文件"}</Badge>
              )}
              <div className="zap-texts">
                <div className="zap-title">
                  {row.missing && (
                    <TriangleAlert className="zap-title-pin size-3" />
                  )}
                  {row.name}
                </div>
                <div className="zap-sub">{row.path}</div>
              </div>
              {row.missing && (
                <span className="zap-stage-dead-tag">已失效</span>
              )}
              {/* 行尾固定按钮：挡住 dblclick 冒泡，双击按钮不能顺带打开文件 */}
              <span onDoubleClick={(e) => e.stopPropagation()}>
                <VaultRowPin
                  pinned={row.pinned}
                  onPin={() => void togglePin(row)}
                />
              </span>
            </div>
          ))
        )}
        {dragging && <div className="zap-stage-drop-hint">松手放入中转站</div>}
      </div>
      {menu && (
        <>
          <div
            className="zap-ctx-backdrop"
            onClick={() => setMenu(null)}
            onContextMenu={(e) => {
              e.preventDefault();
              setMenu(null);
            }}
          />
          <div className="zap-ctx-menu" style={menuPos ?? undefined}>
            {menuItems.map((mi) => (
              <button
                key={mi.label}
                type="button"
                className="zap-ctx-item"
                disabled={mi.disabled}
                onClick={() => {
                  if (mi.disabled) return;
                  setMenu(null);
                  mi.run?.();
                }}
              >
                {mi.label}
              </button>
            ))}
          </div>
        </>
      )}
      <div className="zap-footer">
        <span className="zap-footer-hints">
          <span>拖入即存 · 点击 / Enter 打开 · 右键更多操作</span>
        </span>
        <span className="zap-footer-hints">
          <span>固定条目永不清出</span>
          <span className="zap-footer-sep" />
          <span>Esc 返回</span>
        </span>
      </div>
    </div>
  );
}
