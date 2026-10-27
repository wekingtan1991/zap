import React, {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { CircleHelp, Layers, Lock, Pin, Search, TriangleAlert, Zap } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Kbd } from "@/components/ui/kbd";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Toaster } from "@/components/ui/sonner";
import { kindUi, type ItemActions } from "@/plugins/registry";
import { routeItem } from "@/plugins/dispatch";
import { isPinned, clipRowId, clipImageRowId, type SearchItem } from "@/plugins/types";
import {
  kickEverythingEnsure,
  runCommand,
  startupEntries,
  type CommandCtx,
} from "@/plugins/commands";
import { VaultDetailView, type VaultDetail } from "@/plugins/vault/VaultDetailView";
import { StageView } from "@/plugins/stage/StageView";
import { ClipImagePreview } from "@/plugins/clipimg/ClipImagePreview";
import {
  EMPTY_FORM,
  VaultFormView,
  type AccountForm,
} from "@/plugins/vault/VaultFormView";
import {
  VaultRevealOverlay,
  type RevealInfo,
} from "@/plugins/vault/VaultRevealOverlay";
import { VaultImportView } from "@/plugins/vault/VaultImportView";
import { JsonWorkbench } from "@/plugins/json/JsonWorkbench";
import { ToolWorkbench } from "@/plugins/text/ToolWorkbench";
import { SettingsView } from "@/views/SettingsView";
import { useUpdater } from "@/views/useUpdater";
import { applyAppearance, applyPanelTransparency, type Appearance } from "@/lib/appearance";
import { clampMenuPos } from "@/lib/ctxMenu";

/// 工作台内容源：入口进入 = 空文本等待粘贴；输入识别（万能输入框）= 携带
/// 进入时刻的搜索词全文；剪贴板识别 = 按行号回查呼出时刻的内容
type WorkbenchSource =
  | { kind: "text"; text: string }
  | { kind: "clip"; rowid: number }
  | { kind: "empty" };

/// 面板视图状态机：同一时刻恰有一个子页面，取代原先 8 个并列的布尔/对象状态。
/// 新增子页面 = 加一个 kind + 渲染分支，zap://shown 复位与互斥天然成立
type View =
  | { kind: "main" }
  | { kind: "settings" }
  | { kind: "stage" }
  | { kind: "detail"; data: VaultDetail; askDelete?: boolean }
  | { kind: "form"; data: AccountForm }
  | { kind: "reveal"; data: RevealInfo }
  | { kind: "import" }
  | { kind: "json"; source: WorkbenchSource; backQuery: string }
  | { kind: "tool"; source: WorkbenchSource; backQuery: string }
  | { kind: "clipimg"; rowid: number; backQuery: string };

const VAULT_ADD_ENTRY: SearchItem = {
  id: "vault:add",
  title: "＋ 新增账号",
  subtitle: "标题 / 用户名 / 密码 / 网址 / 备注",
  kind: "info",
  target: "",
  score: -1,
};

const VAULT_IMPORT_ENTRY: SearchItem = {
  id: "vault:import",
  title: "智能导入（粘贴文本批量识别）",
  subtitle: "从记事本 / 聊天记录粘贴账号文本，自动解析字段后校对入库",
  kind: "info",
  target: "",
  score: -2,
};

/// 账号库保护域的 Alt 组合键说明（底栏问号图标的悬停提示内容）
const ALT_ACTIONS: [string, string][] = [
  ["1", "打开网址"],
  ["2", "复制密码（阅后即焚）"],
  ["3", "删除（弹确认条，不会直接删）"],
  ["4", "查看密码"],
  ["5", "置顶 / 取消置顶"],
  ["6", "编辑"],
];

/// 普通域条目的次级操作说明（底栏问号图标悬停提示）
const SECONDARY_ACTIONS: [string, string][] = [
  ["Ctrl+Shift+Enter", "管理员运行"],
  ["Ctrl+Enter", "打开所在文件夹"],
  ["Ctrl+Shift+C", "复制路径"],
];

/// 渲染兜底：任何视图抛错都不再整页白屏，而是提示可读信息 + 一键复位
class ErrorBoundary extends React.Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  render() {
    if (this.state.error) {
      return (
        <div className="zap-panel vault">
          <div className="zap-input-row">
            <span className="zap-logo">
              <TriangleAlert className="size-4 text-primary-foreground" />
            </span>
            <span className="zap-vault-tag">界面出错了</span>
            <button
              className="zap-link-btn"
              onClick={() => this.setState({ error: null })}
            >
              重新加载
            </button>
          </div>
          <div className="zap-form">
            <div className="zap-hk-err">
              {String(this.state.error?.message ?? this.state.error)}
            </div>
            <div className="zap-form-hint">
              面板内容渲染失败，按「重新加载」可恢复；完整信息见终端日志。
            </div>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

function AppView() {
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<SearchItem[]>([]);
  const [active, setActive] = useState(0);
  const [busy, setBusy] = useState(false);
  const [view, setView] = useState<View>({ kind: "main" });
  const inputRef = useRef<HTMLInputElement>(null);
  const queryRef = useRef("");
  const busyRef = useRef(false);
  /// runItem 同条目去重锚点（双击行 = 两次 click，见 runItem 注释）
  const lastRunRef = useRef({ t: 0, key: "" });
  const vaultTermRef = useRef("");
  const activeItemRef = useRef<HTMLDivElement | null>(null);
  const kbNavRef = useRef(false);
  /// 主面板根节点：zap://shown 时重播呼出动画（见 shown 监听内注释）
  const panelRef = useRef<HTMLDivElement>(null);
  /// 结果滚动容器：query 变化时回滚顶部（见滚动重置 effect 注释）
  const resultsRef = useRef<HTMLDivElement>(null);
  /// 条目右键菜单（次级操作）：光标位置 + 目标条目
  const [ctxMenu, setCtxMenu] = useState<{
    x: number;
    y: number;
    item: SearchItem;
  } | null>(null);
  /// 剪贴板合并粘贴模式：mergeSel 收勾选的 rowid，数组保序（点选顺序 =
  /// 拼接顺序）。仅普通域剪贴板条目可勾；面板隐藏/呼出/进保护域即清
  const [mergeMode, setMergeMode] = useState(false);
  const [mergeSel, setMergeSel] = useState<number[]>([]);

  // 子页面（设置/详情/表单/导入/查看密码）挂载时接管焦点：主面板卸载后焦点会掉到
  // body（React 根节点之外），面板上的 onKeyDown 收不到任何按键，Esc 将失效。
  // 焦点已在面板内（如浮层输入框的 autoFocus）则不抢。
  const focusPanelRef = useCallback((el: HTMLDivElement | null) => {
    if (el && !el.contains(document.activeElement)) el.focus();
  }, []);

  useEffect(() => {
    queryRef.current = query;
  }, [query]);

  // .zap-results 是滚动容器但不自动跟随：方向键把选中行移出可视区时滚动补位；
  // nearest 保证已在视口内时不抖动。只在键盘导航时补位——悬停也会改 active，
  // 若跟随滚动会让内容在光标下漂移。
  useEffect(() => {
    if (!kbNavRef.current) return;
    kbNavRef.current = false;
    activeItemRef.current?.scrollIntoView({ block: "nearest" });
  }, [active]);

  // 搜索词变化即回滚列表顶部：active 已复位到 0，滚动不跟随会让键盘导航
  // 从视口外起跳。置顶等动作后的列表刷新不走这里（query 未变），视线留在原地
  useEffect(() => {
    resultsRef.current?.scrollTo({ top: 0 });
  }, [query]);
  useEffect(() => {
    busyRef.current = busy;
  }, [busy]);

  // 保护域：「pw」单独输入 = 空词浏览，「pw 」后接搜索词；「pwsh」这类 pw 开头的
  // 常规搜索目标不再被劫持（旧口径 startsWith("pw") 会吃掉整个 pw* 命名空间）
  const trimmedQuery = query.trim();
  const lowered = trimmedQuery.toLowerCase();
  const isVault = lowered === "pw" || lowered.startsWith("pw ");
  const vaultTerm = isVault ? trimmedQuery.slice(2).trim() : "";

  // 外观与透明面板随设置刷新（设置页改主题即时预览）：挂载与设置页关闭后重拉。
  // 插件开关的门控已收敛到后端（probe/入口条目按 plugin_off 跳过），前端不再缓存
  const refreshAppearance = useCallback(() => {
    invoke<{ appearance: string; panelTransparency: boolean }>("get_app_settings")
      .then((s) => {
        const mode = (["dark", "light", "system"] as const).includes(
          s.appearance as Appearance
        )
          ? (s.appearance as Appearance)
          : "dark";
        applyAppearance(mode);
        applyPanelTransparency(s.panelTransparency ?? true);
      })
      .catch(() => {});
  }, []);
  useEffect(() => {
    refreshAppearance();
  }, [refreshAppearance]);
  useEffect(() => {
    if (view.kind !== "settings") refreshAppearance();
  }, [view.kind, refreshAppearance]);
  useEffect(() => {
    vaultTermRef.current = vaultTerm;
  }, [vaultTerm]);

  // 账号库搜索（cancelled 守卫：在途响应无法取消，快速输入时旧词的慢响应
  // 不得覆盖新词结果——后端各请求独立并发执行，完成顺序无保证）
  useEffect(() => {
    if (!isVault) return;
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const res = await invoke<SearchItem[]>("vault_search", { query: vaultTerm });
        if (cancelled) return;
        const list = [...res];
        if (!vaultTerm) list.push(VAULT_ADD_ENTRY, VAULT_IMPORT_ENTRY);
        setItems(list);
        setActive(0);
      } catch {
        // 忽略
      }
    }, 60);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [isVault, vaultTerm]);

  // 热键钩子启停命令归 SettingsView 持有；这里保留兜底停止（呼出重置时调用）
  const stopHkCapture = useCallback(() => {
    invoke("stop_hotkey_capture").catch(() => {});
  }, []);

  // 面板收起时清空搜索词：下次呼出天然是空状态，用户不会看到"旧内容闪一下被擦掉"。
  // 同一会话内（列表 ↔ 详情页之间）不触发此事件，搜索词自然保留。
  useEffect(() => {
    const un = listen("zap://hidden", () => {
      setQuery("");
      setActive(0);
      // 合并勾选是即时意图：跨呼出保留没有意义，隐藏即清
      setMergeMode(false);
      setMergeSel([]);
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  // 面板每次被呼出时重置视图状态并拉取最近剪贴板（zap://shown 由 Rust 侧在 show 后发出）
  useEffect(() => {
    const unlisten = listen("zap://shown", () => {
      setQuery(""); // 兜底：正常已在 zap://hidden 清空，这里不会产生可见变化
      setView({ kind: "main" });
      stopHkCapture();
      setActive(0);
      // 重播呼出动画：面板窗口只隐藏不销毁，跨 shown 组件不重挂——
      // 移除→reflow→重挂让 panel-in 重新起播；从子视图返回走重挂的静态类
      const panel = panelRef.current;
      if (panel) {
        panel.classList.remove("zap-panel-in");
        void panel.offsetWidth; // 强制 reflow：同一帧内重置动画
        panel.classList.add("zap-panel-in");
      }
      // query 多半未变（hidden 已清空），effect 不会触发：呼出即显式回滚
      resultsRef.current?.scrollTo({ top: 0 });
      setTimeout(() => inputRef.current?.focus(), 30);
      if (busyRef.current) return;
      invoke<SearchItem[]>("search", { query: "" })
        .then(async (res) => {
          if (queryRef.current) return;
          // 呼出期状态注入：Everything 未运行追加启用条目（逻辑收敛在 commands 模块）
          const extra = await startupEntries();
          if (!queryRef.current) setItems([...res, ...extra]);
          if (extra.length > 0) {
            kickEverythingEnsure(() => {
              if (!queryRef.current) {
                setItems((cur) => cur.filter((x) => x.id !== "cmd:everything"));
              }
            });
          }
        })
        .catch(() => {});
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // ---------- 中转站 ----------
  // 视图期间 hold 住面板：拖拽自资源管理器的第一步就把焦点交给了 Explorer，
  // 失焦即隐会直接杀死拖放——hold 是拖拽落区的前提。释放只挂在几个出口：
  // 返回/Esc（closeStage）、进设置页（openSettings/closeSettings 统一释放）、
  // 任何隐藏路径（后端 hide_panel_window 统一释放）与重新呼出（后端兜底清零）

  const openStage = useCallback(() => {
    void invoke("set_hold_panel", { hold: true }).catch(() => {});
    setView({ kind: "stage" });
  }, []);

  const closeStage = useCallback(() => {
    void invoke("set_hold_panel", { hold: false }).catch(() => {});
    setView({ kind: "main" });
    setTimeout(() => inputRef.current?.focus(), 30);
  }, []);

  // 直达热键预设词：Rust 在 shown 之后追加发出（顺序即生效顺序），
  // 据此让面板直接落在账号库等保护域；"stage" 是视图名不是搜索词，直接切视图
  useEffect(() => {
    const un = listen<string>("zap://preset", (e) => {
      if (e.payload === "stage") {
        openStage();
        return;
      }
      setQuery(e.payload);
    });
    return () => {
      un.then((f) => f());
    };
  }, [openStage]);

  // 普通域搜索（账号库保护域下不生效；万能输入框识别与工具入口条目
  // 由后端 probe/entry 在 search 结果里置顶返回，前端无感知）。
  // cancelled 守卫同账号库搜索：files 源带 250ms HTTP 超时，竞态窗口真实存在
  useEffect(() => {
    if (isVault) return;
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const res = await invoke<SearchItem[]>("search", { query: query.trim() });
        if (cancelled) return;
        setItems(res);
        setActive(0);
      } catch {
        // 后端异常时保持当前列表不变
      }
    }, 80);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [query, isVault]);

  const activate = useCallback((item: SearchItem) => {
    // 打开失败后端不收面板：这里 toast 报因，用户可另选条目
    invoke("open_item", { item })
      .then(() => {
        // 剪贴板条目的可见反馈：自动粘贴开启时面板已收（toast 随窗口不可见，
        // 无代价）；关闭时回填是静默的，用户感知为「点了没反应」
        if (item.kind === "clip" || item.kind === "clipimg") {
          toast.success("已复制到剪贴板，可粘贴");
        }
      })
      .catch((e) => {
        toast.error("打开失败：" + String(e));
      });
  }, []);

  /// 条目次级操作（键盘触发与右键菜单共用）：
  /// admin=管理员运行 / reveal=所在文件夹 / copy_path=复制路径或链接 /
  /// clip_copy=剪贴板条目仅复制 / close_window=关闭窗口。
  /// 按 kind 限权；后端 .lnk 解析对 recent 同样生效（指针→真实目标）
  const secondaryAction = useCallback(
    async (
      action:
        | "admin"
        | "reveal"
        | "copy_path"
        | "clip_copy"
        | "close_window",
      it: SearchItem | undefined
    ) => {
      if (!it) return;
      const allow: Record<typeof action, string[]> = {
        admin: ["app"],
        reveal: ["app", "file", "recent"],
        copy_path: ["app", "file", "recent", "web", "url"],
        clip_copy: ["clip"],
        close_window: ["win"],
      };
      if (!allow[action].includes(it.kind)) return;
      try {
        const target =
          action === "clip_copy"
            ? it.id.replace(/^clip:/, "")
            : it.target;
        await invoke("secondary_action", { action, target });
        if (action === "copy_path" || action === "clip_copy")
          toast.success("已复制");
      } catch (e) {
        toast.error(String(e));
      }
    },
    []
  );

  // ---------- 应用设置 / 自动更新 ----------
  // 设置页视图与更新状态机各自成模块（views/SettingsView、views/useUpdater）；
  // 这里只持视图开关与动作入口。更新 hook 挂在 AppView：启动静默检查不依赖设置页
  const updater = useUpdater();

  const openSettings = useCallback(() => {
    // 释放视图残留的 hold（中转站视图经托盘/Ctrl+, 进设置时不能带着幽灵 hold；
    // 更新下载等自持 hold 的流程有自己的 finally 兜底，此处释放无冲突）
    void invoke("set_hold_panel", { hold: false }).catch(() => {});
    setView({ kind: "settings" });
  }, []);

  const closeSettings = useCallback(() => {
    void invoke("set_hold_panel", { hold: false }).catch(() => {});
    setView({ kind: "main" });
    setTimeout(() => inputRef.current?.focus(), 30);
  }, []);

  useEffect(() => {
    const un = listen("zap://open-settings", () => {
      openSettings();
    });
    return () => {
      un.then((f) => f());
    };
  }, [openSettings]);

  // Ctrl+, 打开设置（窗口捕获，全视图可用）：子页面原先进不了设置（面板
  // onKeyDown 只在 main 视图生效），只能先 Esc 回主面板。设置页内重复触发
  // 按 no-op 跳过；热键录制态不冲突（, 不是可录键，录制监听只认可录键名）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey || e.key !== ",") return;
      e.preventDefault();
      e.stopPropagation();
      if (view.kind !== "settings") openSettings();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  /// 账号库列表重拉（pin/delete/表单收尾后）：响应回来时关键词已变（用户
  /// 继续输入）则丢弃，交还给账号库搜索 effect 的正常刷新链路
  const refreshVault = useCallback(async (term: string) => {
    try {
      const res = await invoke<SearchItem[]>("vault_search", { query: term });
      if (term !== vaultTermRef.current) return;
      const list = [...res];
      if (!term) list.push(VAULT_ADD_ENTRY, VAULT_IMPORT_ENTRY);
      setItems(list);
      setActive(0);
    } catch {
      // 忽略
    }
  }, []);

  const vaultAction = useCallback(
    async (
      item: SearchItem,
      action: "username" | "password" | "url" | "delete" | "reveal" | "pin"
    ) => {
      const id = Number(item.target);
      if (Number.isNaN(id)) return;
      try {
        if (action === "url") {
          const url = await invoke<string>("vault_action", { id, action });
          if (url) {
            invoke("open_item", {
              item: {
                id: "vault-url",
                title: "",
                subtitle: "",
                kind: "url",
                target: url,
                score: 0,
              },
            }).catch((e) => feedback(false, "打开网址失败：" + String(e)));
          }
          return;
        }
        if (action === "reveal") {
          const pw = await invoke<string>("vault_action", { id, action });
          setView({
            kind: "reveal",
            data: {
              id,
              title: item.title.replace(/^📌 /, ""),
              password: pw,
            },
          });
          return;
        }
        const msg = await invoke<string>("vault_action", { id, action });
        if (action === "pin" || action === "delete") {
          await refreshVault(vaultTermRef.current);
          return;
        }
        // 复制类：短暂显示成功反馈后自动关闭（密码/用户名均不入剪贴板历史）
        setItems([
          {
            id: "vault-done",
            title: "✓ " + msg,
            subtitle: "稍后自动关闭",
            kind: "info",
            target: "",
            score: 0,
          },
        ]);
        setTimeout(() => invoke("hide_panel"), 900);
      } catch (e) {
        // 失败统一 toast（面板保持可见）；复制类成功反馈保留行内 ✓（随后面板收起，toast 会随之消失）
        toast.error("操作失败：" + String(e));
      }
    },
    [refreshVault]
  );

  // Alt+6：取完整记录并打开编辑表单（密码明文预填，留空保存 = 保持不变）
  const startEdit = useCallback(
    async (item: SearchItem) => {
      const id = Number(item.target);
      if (Number.isNaN(id)) return;
      try {
        const [title, username, password, url, notes, pinned, totp] = await invoke<
          [string, string, string, string, string, number, string]
        >("vault_get", { id });
        setView({
          kind: "form",
          data: {
            id,
            title,
            username,
            password,
            url,
            notes,
            pinned: pinned === 1,
            totp,
          },
        });
      } catch {
        // 忽略
      }
    },
    []
  );

  // ---------- 账号详情：点击标题打开，逐行展示，行尾一键复制 ----------
  // 反馈统一走 toast（滑入 + 图标动效）
  const feedback = useCallback((ok: boolean, text: string) => {
    if (ok) toast.success(text);
    else toast.error(text);
  }, []);

  const closeDetail = useCallback(() => {
    setView({ kind: "main" });
    setTimeout(() => inputRef.current?.focus(), 30);
  }, []);

  const openDetail = useCallback(
    async (item: SearchItem) => {
      const id = Number(item.target);
      if (Number.isNaN(id)) return;
      try {
        const [title, username, password, url, notes, pinned, totp] = await invoke<
          [string, string, string, string, string, number, string]
        >("vault_get", { id });
        setView({
          kind: "detail",
          data: { id, title, username, password, url, notes, pinned, totp },
        });
      } catch (e) {
        feedback(false, "打开失败：" + String(e));
      }
    },
    [feedback]
  );

  /// Alt+3 删除入口：打开详情页并直接亮起确认删除条（复用详情页的二次确认，
  /// 取消/Esc 回列表），列表里永不直接删除
  const openDeleteConfirm = useCallback(
    async (item: SearchItem) => {
      const id = Number(item.target);
      if (Number.isNaN(id)) return;
      try {
        const [title, username, password, url, notes, pinned, totp] = await invoke<
          [string, string, string, string, string, number, string]
        >("vault_get", { id });
        setView({
          kind: "detail",
          data: { id, title, username, password, url, notes, pinned, totp },
          askDelete: true,
        });
      } catch (e) {
        feedback(false, "打开失败：" + String(e));
      }
    },
    [feedback]
  );

  /// 详情页行尾复制：username / password / url / notes / totp
  /// （密码带阅后即焚、验证码排除剪贴板历史，均不入历史）。
  /// 返回成功与否，✓ 点亮状态由详情页本地持有
  const copyField = useCallback(
    async (
      id: number,
      field: "username" | "password" | "url" | "notes" | "totp"
    ): Promise<boolean> => {
      try {
        const msg = await invoke<string>("vault_action", {
          id,
          action: field === "totp" ? "totp_copy" : field,
        });
        feedback(true, msg);
        return true;
      } catch (e) {
        feedback(false, "复制失败：" + String(e));
        return false;
      }
    },
    [feedback]
  );

  const pinCurrent = useCallback(async () => {
    setView((v) => {
      if (v.kind !== "detail") return v;
      void invoke("vault_action", { id: v.data.id, action: "pin" }).catch(
        (e) => feedback(false, "操作失败：" + String(e))
      );
      return {
        ...v,
        data: { ...v.data, pinned: v.data.pinned === 1 ? 0 : 1 },
      };
    });
    await refreshVault(vaultTermRef.current);
  }, [feedback, refreshVault]);

  /// 删除当前账号（详情页确认弹窗触发）：成功关详情 + 刷新列表，失败由详情页收回弹窗
  const deleteCurrent = useCallback(async (): Promise<boolean> => {
    if (view.kind !== "detail") return false;
    try {
      await invoke<string>("vault_action", {
        id: view.data.id,
        action: "delete",
      });
      setView({ kind: "main" });
      await refreshVault(vaultTermRef.current);
      return true;
    } catch (e) {
      feedback(false, "删除失败：" + String(e));
      return false;
    }
  }, [view, feedback, refreshVault]);

  /// 详情页编辑：记录已在手上，直接预填表单（无需再取一次）
  const editCurrent = useCallback(() => {
    setView((v) =>
      v.kind === "detail"
        ? {
            kind: "form",
            data: {
              id: v.data.id,
              title: v.data.title,
              username: v.data.username,
              password: v.data.password,
              url: v.data.url,
              notes: v.data.notes,
              pinned: v.data.pinned === 1,
              totp: v.data.totp,
            },
          }
        : v
    );
  }, []);

  /// 固定后的列表重拉（clipPin 与图片预览面板共用）：按当前词重拉，
  /// 重拉期间词已变则丢弃（交还给搜索 effect）
  const repullClips = useCallback(() => {
    const q = queryRef.current.trim();
    invoke<SearchItem[]>("search", { query: q })
      .then((res) => {
        if (q !== queryRef.current.trim()) return;
        setItems(res);
        setActive(0);
      })
      .catch(() => {});
  }, []);

  /// 剪贴板固定/取消固定（文本与图片条目共用）：
  /// 固定项置顶/取消复位靠重拉。两套寻址键各走各的助手，且图片条目必须走
  /// clipimg_pin 直连图片表——clip_pin 文本表先命中，两表自增行号撞号时
  /// 会固定到同号的文本行
  const clipPin = useCallback(
    async (item: SearchItem) => {
      const isImg = item.kind === "clipimg";
      const id = isImg ? clipImageRowId(item) : clipRowId(item);
      if (Number.isNaN(id)) return;
      try {
        await invoke<boolean>(isImg ? "clipimg_pin" : "clip_pin", { id });
        repullClips();
      } catch (e) {
        toast.error("固定失败：" + String(e));
      }
    },
    [repullClips]
  );

  // ---------- 剪贴板合并粘贴 ----------
  /// 模式开关：退出即清勾选（重进从零开始，避免残留旧意图）
  const toggleMerge = useCallback(() => {
    setMergeMode((m) => !m);
    setMergeSel([]);
  }, []);

  /// 勾选/取消一条（保序，点击与 Enter 共用）：仅文本剪贴板条目；
  /// 行号取自 id（clipRowId）——target 是内容本体，取数恒 NaN
  const toggleClipSelection = useCallback((it: SearchItem | undefined) => {
    if (!it || it.kind !== "clip") return;
    const id = clipRowId(it);
    if (Number.isNaN(id)) return;
    setMergeSel((s) =>
      s.includes(id) ? s.filter((x) => x !== id) : [...s, id]
    );
  }, []);

  /// 执行合并：勾选顺序 = 拼接顺序。**只清勾选、不退模式**——自动粘贴关闭时
  /// 面板不收，用户往往要连着合下一批；自动粘贴开启时面板收起，zap://hidden
  /// 兜底清掉整个模式。个别条目已被清理时后端跳过并返回实际条数；全部被
  /// 清理则报错（面板不收，toast 可见）
  const performMergePaste = useCallback(async () => {
    const ids = [...mergeSel];
    setMergeSel([]);
    if (ids.length === 0) return;
    try {
      const r = await invoke<{ count: number; pasted: boolean }>(
        "clip_merge_paste",
        { ids }
      );
      const skipped = ids.length - r.count;
      const head = r.pasted
        ? `已合并粘贴 ${r.count} 条`
        : `已合并 ${r.count} 条到剪贴板`;
      toast.success(skipped > 0 ? `${head}（${skipped} 条已被清理，已跳过）` : head);
    } catch (e) {
      toast.error(String(e));
    }
  }, [mergeSel]);

  // 进入保护域（pw）即退合并模式：勾选只对普通域剪贴板列表有意义，
  // 保护域下 Enter/Ctrl+Enter 另有语义，不能被合并分支截走
  useEffect(() => {
    if (isVault && mergeMode) {
      setMergeMode(false);
      setMergeSel([]);
    }
  }, [isVault, mergeMode]);

  // 合并模式的键盘面挂窗口捕获——**不能挂面板/输入框 onKeyDown**：点击
  // 底栏按钮或条目后焦点掉到按钮/body，输入框的 onKeyDown 收不到任何键
  // （与 2026-10-22 表单 Esc 失效同根因，本次在合并功能上重犯后按同一
  // 修法收口）。捕获阶段拦下合并语义键并 stopPropagation，防输入框的
  // onKeyDown 对同一按键再处理一遍；其余按键放行不干扰打字。
  // 不传依赖数组：每次渲染重挂保闭包新鲜（VaultFormView Esc 同款）。
  // isComposing 卫兵挡 IME 选词键（合成期 keydown 不该触发勾选/合并）
  useEffect(() => {
    if (!mergeMode) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing) return;
      // 右键菜单存活时 Esc 只关菜单，其余键放行给菜单语义（同 onKeyDown 口径）
      if (ctxMenu) {
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          setCtxMenu(null);
        }
        return;
      }
      if (e.key === "Enter" && e.ctrlKey) {
        e.preventDefault();
        e.stopPropagation();
        void performMergePaste();
      } else if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        toggleClipSelection(items[active]);
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        toggleMerge();
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        e.stopPropagation();
        kbNavRef.current = true;
        setActive((a) =>
          e.key === "ArrowDown"
            ? Math.min(a + 1, Math.max(items.length - 1, 0))
            : Math.max(a - 1, 0)
        );
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });

  /// 工作台进入：按条目 target 定内容源——q = 万能输入框识别（携带当前
  /// 搜索词全文）、entry = 搜索入口（空内容等待粘贴）、数字 = 剪贴板行号回查；
  /// backQuery 记下返回词，Esc 退出时恢复主搜索现场
  const openWorkbench = useCallback((kind: "json" | "tool", it: SearchItem) => {
    const backQuery = queryRef.current;
    const source: WorkbenchSource =
      it.target === "q"
        ? { kind: "text", text: queryRef.current }
        : it.target === "entry"
          ? { kind: "empty" }
          : { kind: "clip", rowid: Number(it.target) };
    if (source.kind === "clip" && Number.isNaN(source.rowid)) return;
    setView(
      kind === "json"
        ? { kind: "json", source, backQuery }
        : { kind: "tool", source, backQuery }
    );
  }, []);

  /// 工作台退出（Esc / 返回按钮共用）：回主面板并恢复进入前的搜索词
  const closeWorkbench = useCallback((backQuery: string) => {
    setView({ kind: "main" });
    setQuery(backQuery);
    setTimeout(() => inputRef.current?.focus(), 30);
  }, []);

  // ---------- 图片预览面板 ----------
  // 右键「预览大图」入口（Enter 保留回贴不占——粘贴是图片条目的第一动作）：
  // 行号寻址在 target（clipImageRowId，与文本 clip 的 id 寻址相反）；
  // 面板内粘贴/仅复制/另存为/固定，Esc 经 closeWorkbench 回列表恢复搜索词
  const openClipImagePreview = useCallback((it: SearchItem) => {
    const rowid = clipImageRowId(it);
    if (Number.isNaN(rowid)) return;
    setView({ kind: "clipimg", rowid, backQuery: queryRef.current });
  }, []);

  // ---------- 中转站 ----------
  /// 搜索结果右键「放入中转站」：失败明细由 stage_add 返回，这里只报成败
  const stageAdd = useCallback(async (it: SearchItem) => {
    try {
      const r = await invoke<{
        added: number;
        total: number;
        failed: string[];
      }>("stage_add", { paths: [it.target] });
      if (r.failed.length > 0) {
        toast.error(r.failed[0]);
      } else if (r.added === 0) {
        toast.success(`已在中转站（已置顶）· 共 ${r.total} 条`);
      } else {
        toast.success(`已放入中转站 · 共 ${r.total} 条`);
      }
    } catch (e) {
      toast.error(String(e));
    }
  }, []);

  // ---------- 统一条目分发 ----------
  // Enter 与点击共用：伪条目（新增/导入）→ 注册表 activate → 默认 openExternal。
  // kind 行为收敛在 plugins/registry.tsx，这里只接装配，不写 kind 特判
  const actions: ItemActions = {
    openVaultDetail: (it) => void openDetail(it),
    openJson: (it) => openWorkbench("json", it),
    openTool: (it) => openWorkbench("tool", it),
    runCommand: (target) =>
      void runCommand(target, {
        busyRef,
        setBusy,
        setItems,
      } satisfies CommandCtx),
    openExternal: activate,
    vaultPin: (it) => void vaultAction(it, "pin"),
    clipPin: (it) => void clipPin(it),
    openStage: () => void openStage(),
    openClipImagePreview: openClipImagePreview,
  };

  const runItem = useCallback(
    (it: SearchItem | undefined) => {
      if (!it || busyRef.current) return;
      // 合并粘贴模式：点击文本剪贴板条目 = 勾选/取消（保序），不打开、不触发
      // 分发；放在同条目去重之前——快速二连点取消勾选不能被双击防抖吞掉
      if (mergeMode && it.kind === "clip") {
        toggleClipSelection(it);
        return;
      }
      // 同一条目 300ms 内去重：双击行（Explorer 迁移习惯）= 两次 click，
      // 文件/应用类会 ShellExecute 两次开两份；Enter 连击同理
      const now = performance.now();
      if (
        now - lastRunRef.current.t < 300 &&
        lastRunRef.current.key === it.id
      ) {
        return;
      }
      lastRunRef.current = { t: now, key: it.id };
      // 分发决策收敛在 plugins/dispatch.ts（纯函数，可单测）；这里只执行
      const route = routeItem(it);
      switch (route.type) {
        case "form-add":
          setView({ kind: "form", data: { ...EMPTY_FORM } });
          return;
        case "import":
          setView({ kind: "import" });
          return;
        case "activate":
          // 路由保证 activate 存在（dispatch.ts 契约，有单测锁定）
          route.ui.activate!(it, actions);
          return;
        case "external":
          activate(it);
          return;
        case "none":
          return;
      }
    },
    // actions 引用稳定的 useState setter / useCallback 闭包，逐项列出即可
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [activate, openDetail, vaultAction, mergeMode]
  );

  // 表单数据更新器：与旧 setForm 同签名（Dispatch<SetStateAction<AccountForm|null>>），
  // VaultFormView 的受控接口不变；置 null 视为关表单回主面板
  const updateFormData = useCallback(
    (action: React.SetStateAction<AccountForm | null>) => {
      setView((v) => {
        if (v.kind !== "form") return v;
        const next =
          typeof action === "function"
            ? (action as (prev: AccountForm) => AccountForm | null)(v.data)
            : action;
        return next === null ? { kind: "main" } : { ...v, data: next };
      });
    },
    []
  );

  // 表单保存成功：关表单 + 回账号库列表（空关键词列出全部）
  const formSaved = useCallback(() => {
    setView({ kind: "main" });
    setQuery("pw");
    void refreshVault("");
  }, [refreshVault]);

  // 表单取消：编辑态（有 id）返回该账号详情页（编辑入口在详情页，取消不应退到列表），
  // 新增态回账号库列表；Esc 与「取消」按钮共用
  const cancelForm = useCallback(() => {
    if (view.kind !== "form") return;
    const id = view.data.id;
    setView({ kind: "main" });
    if (id == null) {
      setTimeout(() => inputRef.current?.focus(), 30);
      return;
    }
    void (async () => {
      try {
        const [title, username, password, url, notes, pinned, totp] = await invoke<
          [string, string, string, string, string, number, string]
        >("vault_get", { id });
        setView({
          kind: "detail",
          data: { id, title, username, password, url, notes, pinned, totp },
        });
      } catch {
        // 详情打不开（账号已被删等）退回旧行为：列表 + 焦点回输入框
        setTimeout(() => inputRef.current?.focus(), 30);
      }
    })();
  }, [view]);

  // 导入收尾：关视图 + 刷新列表（成功反馈由导入视图先展示 700ms）
  const importDone = useCallback(() => {
    setView({ kind: "main" });
    void refreshVault(vaultTermRef.current);
  }, [refreshVault]);

  const closeImport = useCallback(() => {
    setView({ kind: "main" });
    setTimeout(() => inputRef.current?.focus(), 30);
  }, []);

  const onKeyDown = (e: React.KeyboardEvent) => {
    // 各子页面的 Esc 已由其自身 useEscCapture（窗口捕获）处理；这里只剩
    // 右键菜单与主面板自身的导航键——view 非 main 时子页面自持，直接放行
    if (ctxMenu) {
      if (e.key === "Escape") {
        e.preventDefault();
        setCtxMenu(null);
      }
      return;
    }
    if (view.kind !== "main") return;

    // 条目次级操作（普通域）：Ctrl+Shift+Enter 管理员 / Ctrl+Enter 所在文件夹 /
    // Ctrl+Shift+C 复制路径。账号库域不响应，次级动作按 kind 限权
    if (!isVault && e.ctrlKey && e.key === "Enter") {
      e.preventDefault();
      if (e.shiftKey) void secondaryAction("admin", items[active]);
      else void secondaryAction("reveal", items[active]);
      return;
    }
    if (
      !isVault &&
      e.ctrlKey &&
      e.shiftKey &&
      (e.key === "c" || e.key === "C")
    ) {
      e.preventDefault();
      void secondaryAction("copy_path", items[active]);
      return;
    }

    if (isVault) {
      // 保护域动作组：Alt+1 网址 / Alt+2 密码 / Alt+3 删除 / Alt+4 查看 / Alt+5 置顶 / Alt+6 编辑
      if (e.altKey && ["1", "2", "3", "4", "5", "6"].includes(e.key)) {
        e.preventDefault();
        const it = items[active];
        if (!it || it.kind !== "vault") return;
        if (e.key === "1") vaultAction(it, "url");
        if (e.key === "2") vaultAction(it, "password");
        // 删除是保护域里唯一的破坏性操作：不直接删，打开详情页的确认删除条
        if (e.key === "3") void openDeleteConfirm(it);
        if (e.key === "4") vaultAction(it, "reveal");
        if (e.key === "5") vaultAction(it, "pin");
        if (e.key === "6") void startEdit(it);
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        runItem(items[active]);
        return;
      }
      // Esc 落到下方通用导航
    }

    if (e.key === "ArrowDown") {
      e.preventDefault();
      kbNavRef.current = true;
      setActive((a) => Math.min(a + 1, Math.max(items.length - 1, 0)));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      kbNavRef.current = true;
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      runItem(items[active]);
    } else if (e.key === "Escape") {
      // 渐进式退出：有词清词（账号库下即退回主面板），空了才收起面板
      e.preventDefault();
      if (query.trim()) {
        setQuery("");
      } else {
        invoke("hide_panel");
      }
    }
  };

  // ---------- 子页面渲染（view 状态机分发） ----------

  if (view.kind === "form") {
    return (
      <VaultFormView
        form={view.data}
        setForm={updateFormData}
        onSaved={formSaved}
        onCancel={cancelForm}
        focusRef={focusPanelRef}
      />
    );
  }

  if (view.kind === "reveal") {
    return (
      <VaultRevealOverlay
        reveal={view.data}
        onClose={() => setView({ kind: "main" })}
        focusRef={focusPanelRef}
      />
    );
  }

  if (view.kind === "import") {
    return (
      <VaultImportView
        onDone={importDone}
        onClose={closeImport}
        focusRef={focusPanelRef}
      />
    );
  }

  if (view.kind === "detail") {
    return (
      <VaultDetailView
        detail={view.data}
        initialConfirmDelete={view.askDelete === true}
        focusRef={focusPanelRef}
        onClose={closeDetail}
        onDelete={deleteCurrent}
        onCopyField={copyField}
        onEdit={editCurrent}
        onPin={() => void pinCurrent()}
      />
    );
  }

  if (view.kind === "stage") {
    return <StageView focusRef={focusPanelRef} onClose={closeStage} />;
  }

  if (view.kind === "json") {
    return (
      <JsonWorkbench
        text={
          view.source.kind === "text"
            ? view.source.text
            : view.source.kind === "empty"
              ? ""
              : undefined
        }
        clipRow={view.source.kind === "clip" ? view.source.rowid : undefined}
        onBack={() => closeWorkbench(view.backQuery)}
        focusRef={focusPanelRef}
      />
    );
  }

  if (view.kind === "tool") {
    return (
      <ToolWorkbench
        text={
          view.source.kind === "text"
            ? view.source.text
            : view.source.kind === "empty"
              ? ""
              : undefined
        }
        clipRow={view.source.kind === "clip" ? view.source.rowid : undefined}
        onBack={() => closeWorkbench(view.backQuery)}
        focusRef={focusPanelRef}
      />
    );
  }

  if (view.kind === "clipimg") {
    return (
      <ClipImagePreview
        rowid={view.rowid}
        onBack={() => closeWorkbench(view.backQuery)}
        onPinned={repullClips}
        focusRef={focusPanelRef}
      />
    );
  }

  if (view.kind === "settings") {
    return (
      <SettingsView
        updater={updater}
        focusRef={focusPanelRef}
        onClose={closeSettings}
      />
    );
  }

  // ---------- 主面板 ----------
  return (
    <div
      ref={panelRef}
      className={"zap-panel zap-panel-in" + (isVault ? " vault" : "")}
    >
      <div className="zap-input-row">
        <span className="zap-logo">
          {isVault ? (
            <Lock className="size-3.5 text-primary-foreground" />
          ) : (
            <Zap className="size-3.5 text-primary-foreground" />
          )}
        </span>
        {isVault && (
          <span className="zap-vault-tag zap-vault-tag-pill">账号库</span>
        )}
        <input
          ref={inputRef}
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={
            isVault ? "搜索账号…" : "搜索应用、文件、窗口、剪贴板…粘贴内容自动识别"
          }
          spellCheck={false}
        />
        <Button
          variant="ghost"
          size="icon"
          className="zap-gear size-7 text-muted-foreground/70 hover:text-foreground"
          title="设置（Ctrl+,）"
          onClick={openSettings}
        >
          <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
            <path d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 0 0 .12-.61l-1.92-3.32a.49.49 0 0 0-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.48.48 0 0 0-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96a.49.49 0 0 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58a.49.49 0 0 0-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.49.49 0 0 0-.12-.61l-2.01-1.58zM12 15.6A3.6 3.6 0 1 1 12 8.4a3.6 3.6 0 0 1 0 7.2z" />
          </svg>
        </Button>
      </div>
      <div className="zap-results" ref={resultsRef}>
        {items.length === 0 ? (
          <div className="zap-empty">
            <span className="zap-empty-ico">
              {isVault ? (
                <Lock className="size-5" />
              ) : (
                <Search className="size-5" />
              )}
            </span>
            <span className="zap-empty-title">
              {isVault ? "没有匹配的账号" : "开始输入，即时搜索"}
            </span>
            <span className="zap-empty-sub">
              {isVault
                ? "换个关键词，或清空后按 Enter 新增账号"
                : "应用 · 文件 · 窗口 · 剪贴板 · 最近使用"}
            </span>
            {!isVault && (
              <span className="zap-empty-sub">
                试试 pw 账号库 · 右键条目有更多操作 · Ctrl+, 设置
              </span>
            )}
          </div>
        ) : (
          items.map((it, i) => {
            const ui = kindUi(it.kind);
            return (
            <div
              key={it.id + String(i)}
              ref={i === active ? activeItemRef : undefined}
              className={"zap-item" + (i === active ? " active" : "")}
              onMouseEnter={() => setActive(i)}
              onClick={() => runItem(it)}
              onContextMenu={(e) => {
                // 有右键动作的 kind 弹次级操作菜单；其余只压掉 WebView 默认菜单
                e.preventDefault();
                if ((kindUi(it.kind)?.ctxActions?.length ?? 0) > 0) {
                  setCtxMenu({ x: e.clientX, y: e.clientY, item: it });
                }
              }}
            >
              {it.icon ? (
                <img className="zap-ico" src={it.icon} alt="" draggable={false} />
              ) : (
                <Badge variant={ui?.badge ?? "secondary"}>
                  {ui?.label ?? it.kind}
                </Badge>
              )}
              <div className="zap-texts">
                <div className="zap-title">
                  {isPinned(it) && <Pin className="zap-title-pin size-3" />}
                  {it.title.replace(/^📌 /, "")}
                </div>
                {it.subtitle && <div className="zap-sub">{it.subtitle}</div>}
              </div>
              {/* 合并模式：文本剪贴板条目行尾勾选框，点选顺序即拼接顺序 */}
              {mergeMode && it.kind === "clip" && (
                <span
                  className={
                    "zap-merge-check" +
                    (mergeSel.includes(clipRowId(it)) ? " on" : "")
                  }
                  aria-hidden="true"
                >
                  ✓
                </span>
              )}
              {ui?.rowAction?.(it, actions)}
            </div>
            );
          })
          )}
      </div>
      {ctxMenu && (
        <>
          <div
            className="zap-ctx-backdrop"
            onClick={() => setCtxMenu(null)}
            onContextMenu={(e) => {
              e.preventDefault();
              setCtxMenu(null);
            }}
          />
          <div
            className="zap-ctx-menu"
            style={clampMenuPos(
              ctxMenu.x,
              ctxMenu.y,
              kindUi(ctxMenu.item.kind)?.ctxActions?.length ?? 0,
            )}
          >
            {(kindUi(ctxMenu.item.kind)?.ctxActions ?? []).map(
              ([act, label]) => (
                <button
                  key={act}
                  type="button"
                  className="zap-ctx-item"
                  onClick={() => {
                    setCtxMenu(null);
                    if (act === "stage_add") void stageAdd(ctxMenu.item);
                    else if (act === "clip_pin") void clipPin(ctxMenu.item);
                    else if (act === "clipimg_preview")
                      openClipImagePreview(ctxMenu.item);
                    else void secondaryAction(
                      act as "admin" | "reveal" | "copy_path" | "clip_copy" | "close_window",
                      ctxMenu.item
                    );
                  }}
                >
                  {label}
                </button>
              )
            )}
          </div>
        </>
      )}
      <div className="zap-footer">
        <span className="zap-footer-hints">
          <Kbd>↑</Kbd>
          <Kbd>↓</Kbd>
          <span>选择</span>
          <span className="zap-footer-sep" />
          <Kbd>Enter</Kbd>
          <span>{isVault ? "详情" : "启动"}</span>
          {isVault ? (
            <>
              <span className="zap-footer-sep" />
              <Kbd>Alt</Kbd>
              <span>+</span>
              <Kbd>1~6</Kbd>
              <span>行内操作</span>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span
                    className="zap-footer-help"
                    aria-label="Alt 组合键说明"
                    tabIndex={-1}
                  >
                    <CircleHelp className="size-3.5" />
                  </span>
                </TooltipTrigger>
                <TooltipContent side="top" align="start">
                  <div className="zap-alt-help">
                    {ALT_ACTIONS.map(([key, desc]) => (
                      <div key={key} className="zap-alt-help-row">
                        <span className="zap-alt-help-keys">
                          <Kbd>Alt</Kbd>
                          <span>+</span>
                          <Kbd>{key}</Kbd>
                        </span>
                        <span>{desc}</span>
                      </div>
                    ))}
                  </div>
                </TooltipContent>
              </Tooltip>
            </>
          ) : (
            <>
              <span className="zap-footer-sep" />
              {mergeMode ? (
                <>
                  <span className="zap-merge-status">
                    合并模式 · 已选 {mergeSel.length} 条
                  </span>
                  <button
                    type="button"
                    className="zap-merge-go"
                    disabled={mergeSel.length === 0}
                    onClick={() => void performMergePaste()}
                  >
                    <Kbd>Ctrl</Kbd>
                    <span>+</span>
                    <Kbd>Enter</Kbd>
                    <span>合并粘贴</span>
                  </button>
                  <button
                    type="button"
                    className="zap-merge-exit"
                    onClick={toggleMerge}
                  >
                    退出
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  className="zap-merge-enter"
                  onClick={toggleMerge}
                  title="连续点选多条剪贴板文本条目，按点选顺序合并成一条粘贴"
                >
                  <Layers className="size-3" />
                  <span>合并粘贴</span>
                </button>
              )}
              <span className="zap-footer-sep" />
              <Kbd>pw</Kbd>
              <span>账号库</span>
              <span className="zap-footer-sep" />
              <Kbd>Ctrl</Kbd>
              <span>+</span>
              <Kbd>V</Kbd>
              <span>粘贴即识别</span>
              <span className="zap-footer-sep" />
              <Kbd>json</Kbd>
              <span>/</span>
              <Kbd>tool</Kbd>
              <span>搜索工具</span>
              <Tooltip>
                <TooltipTrigger asChild>
                  <span
                    className="zap-footer-help"
                    aria-label="次级操作说明"
                    tabIndex={-1}
                  >
                    <CircleHelp className="size-3.5" />
                  </span>
                </TooltipTrigger>
                <TooltipContent side="top" align="start">
                  <div className="zap-alt-help">
                    {SECONDARY_ACTIONS.map(([keys, desc]) => (
                      <div key={keys} className="zap-alt-help-row">
                        <span className="zap-alt-help-keys">
                          {keys.split("+").map((k, i) => (
                            <span
                              key={k}
                              className="flex items-center gap-0.5"
                            >
                              {i > 0 && <span>+</span>}
                              <Kbd>{k}</Kbd>
                            </span>
                          ))}
                        </span>
                        <span>{desc}</span>
                      </div>
                    ))}
                  </div>
                </TooltipContent>
              </Tooltip>
            </>
          )}
        </span>
        <span className="zap-footer-hints">
          <Kbd>Ctrl</Kbd>
          <span>+</span>
          <Kbd>,</Kbd>
          <span>设置</span>
          <span className="zap-footer-sep" />
          <Kbd>Esc</Kbd>
          <span>清空 / 关闭</span>
        </span>
      </div>
    </div>
  );
}

/// 统一外壳：Tooltip 必须在 Provider 内（否则 Radix 抛错导致整页白屏），
/// Toaster 承载全局反馈动效；放在最外层可一次覆盖所有视图。
function App() {
  return (
    <ErrorBoundary>
      <TooltipProvider>
        <AppView />
        <Toaster />
      </TooltipProvider>
    </ErrorBoundary>
  );
}

export default App;
