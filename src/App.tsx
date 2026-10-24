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
import { CircleHelp, Lock, Pin, Search, TriangleAlert, Zap } from "lucide-react";
import "./App.css";
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
import { isPinned, type SearchItem } from "@/plugins/types";
import {
  kickEverythingEnsure,
  runCommand,
  startupEntries,
  type CommandCtx,
} from "@/plugins/commands";
import { VaultDetailView, type VaultDetail } from "@/plugins/vault/VaultDetailView";
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
import { applyAppearance, type Appearance } from "@/lib/appearance";

/// 面板视图状态机：同一时刻恰有一个子页面，取代原先 8 个并列的布尔/对象状态。
/// 新增子页面 = 加一个 kind + 渲染分支，zap://shown 复位与互斥天然成立
type View =
  | { kind: "main" }
  | { kind: "settings" }
  | { kind: "detail"; data: VaultDetail; askDelete?: boolean }
  | { kind: "form"; data: AccountForm }
  | { kind: "reveal"; data: RevealInfo }
  | { kind: "import" }
  | { kind: "json"; data: SearchItem }
  | { kind: "tool"; data: SearchItem };

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
  const vaultTermRef = useRef("");
  const activeItemRef = useRef<HTMLDivElement | null>(null);
  const kbNavRef = useRef(false);
  /// 可开关插件状态缓存（json/tool 前缀域的门控依据）：
  /// 挂载与设置页关闭后刷新，停用的插件前缀词落回普通搜索
  const [pluginOn, setPluginOn] = useState<Record<string, boolean>>({});
  /// 条目右键菜单（次级操作）：光标位置 + 目标条目
  const [ctxMenu, setCtxMenu] = useState<{
    x: number;
    y: number;
    item: SearchItem;
  } | null>(null);

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
  useEffect(() => {
    busyRef.current = busy;
  }, [busy]);

  // 保护域：pw 前缀进入账号库
  const isVault = query.trim().toLowerCase().startsWith("pw");
  const vaultTerm = isVault ? query.trim().slice(2).trim() : "";
  // 工具域：json 前缀进入 JSON 工作台，其后文本即待分析内容。
  // 前缀域按插件开关门控：停用的插件前缀词落回普通搜索（与命令层守卫双保险）
  const isJson =
    !isVault &&
    (pluginOn["json-tools"] ?? true) &&
    query.trim().toLowerCase().startsWith("json");
  const jsonText = isJson ? query.trim().slice(4).trim() : "";
  // 文本工具域：tool 前缀（后接空格或为空才进域，避免误伤含 tool 的搜索词）
  const isTool =
    !isVault &&
    !isJson &&
    (pluginOn["text-tools"] ?? true) &&
    (query.trim().toLowerCase() === "tool" ||
      query.trim().toLowerCase().startsWith("tool "));
  const toolText = isTool
    ? query.trim().toLowerCase() === "tool"
      ? ""
      : query.trim().slice(5)
    : "";

  // 插件开关缓存刷新：挂载 + 设置页关闭（保存与否都重新拉取，保证门控即时）。
  // 同一趟顺带取外观模式并应用（设置页改主题时它自己也会即时预览）
  const refreshPluginOn = useCallback(() => {
    invoke<{ plugins: { id: string; enabled: boolean }[]; appearance: string }>(
      "get_app_settings"
    )
      .then((s) => {
        setPluginOn(
          Object.fromEntries(s.plugins.map((p) => [p.id, p.enabled]))
        );
        const mode = (["dark", "light", "system"] as const).includes(
          s.appearance as Appearance
        )
          ? (s.appearance as Appearance)
          : "dark";
        applyAppearance(mode);
      })
      .catch(() => {});
  }, []);
  useEffect(() => {
    refreshPluginOn();
  }, [refreshPluginOn]);
  useEffect(() => {
    if (view.kind !== "settings") refreshPluginOn();
  }, [view.kind, refreshPluginOn]);
  useEffect(() => {
    vaultTermRef.current = vaultTerm;
  }, [vaultTerm]);

  // 账号库搜索
  useEffect(() => {
    if (!isVault) return;
    const timer = setTimeout(async () => {
      try {
        const res = await invoke<SearchItem[]>("vault_search", { query: vaultTerm });
        const list = [...res];
        if (!vaultTerm) list.push(VAULT_ADD_ENTRY, VAULT_IMPORT_ENTRY);
        setItems(list);
        setActive(0);
      } catch {
        // 忽略
      }
    }, 60);
    return () => clearTimeout(timer);
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

  // 普通域搜索（保护域/工具域下不生效）
  useEffect(() => {
    if (isVault || isJson || isTool) return;
    const timer = setTimeout(async () => {
      try {
        const res = await invoke<SearchItem[]>("search", { query: query.trim() });
        setItems(res);
        setActive(0);
      } catch {
        // 后端异常时保持当前列表不变
      }
    }, 80);
    return () => clearTimeout(timer);
  }, [query, isVault]);

  const activate = useCallback((item: SearchItem) => {
    // 打开失败后端不收面板：这里 toast 报因，用户可另选条目
    invoke("open_item", { item }).catch((e) => {
      toast.error("打开失败：" + String(e));
    });
  }, []);

  /// 条目次级操作（键盘触发）：admin=管理员运行 / reveal=所在文件夹 / copy_path=复制路径。
  /// 按 kind 限权：窗口/剪贴板/账号库等没有路径语义，按键不响应
  const secondaryAction = useCallback(
    async (action: "admin" | "reveal" | "copy_path", it: SearchItem | undefined) => {
      if (!it) return;
      const allow: Record<typeof action, string[]> = {
        admin: ["app"],
        reveal: ["app", "file"],
        copy_path: ["app", "file"],
      };
      if (!allow[action].includes(it.kind)) return;
      try {
        await invoke("secondary_action", { action, target: it.target });
        if (action === "copy_path") toast.success("路径已复制");
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
    setView({ kind: "settings" });
  }, []);

  const closeSettings = useCallback(() => {
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

  const refreshVault = useCallback(async (term: string) => {    try {
      const res = await invoke<SearchItem[]>("vault_search", { query: term });
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

  /// 剪贴板固定/取消固定：成功后按当前词重拉列表（固定项置顶/取消复位）
  const clipPin = useCallback(async (item: SearchItem) => {
    const id = Number(item.id.replace(/^clip:/, ""));
    if (Number.isNaN(id)) return;
    try {
      await invoke<boolean>("clip_pin", { id });
      const res = await invoke<SearchItem[]>("search", {
        query: queryRef.current.trim(),
      });
      setItems(res);
      setActive(0);
    } catch (e) {
      toast.error("固定失败：" + String(e));
    }
  }, []);

  // ---------- 统一条目分发 ----------
  // Enter 与点击共用：伪条目（新增/导入）→ 注册表 activate → 默认 openExternal。
  // kind 行为收敛在 plugins/registry.tsx，这里只接装配，不写 kind 特判
  const actions: ItemActions = {
    openVaultDetail: (it) => void openDetail(it),
    openJson: (it) => setView({ kind: "json", data: it }),
    openTool: (it) => setView({ kind: "tool", data: it }),
    runCommand: (target) =>
      void runCommand(target, {
        busyRef,
        setBusy,
        setItems,
      } satisfies CommandCtx),
    openExternal: activate,
    vaultPin: (it) => void vaultAction(it, "pin"),
    clipPin: (it) => void clipPin(it),
  };

  const runItem = useCallback(
    (it: SearchItem | undefined) => {
      if (!it || busyRef.current) return;
      // 保护域伪条目：id 寻址（列表展示与普通条目无异，动作单独接）
      if (it.id === "vault:add") {
        setView({ kind: "form", data: { ...EMPTY_FORM } });
        return;
      }
      if (it.id === "vault:import") {
        setView({ kind: "import" });
        return;
      }
      const ui = kindUi(it.kind);
      if (!ui) {
        // 未注册 kind：按旧约定交给后端（条目结构向后兼容）
        activate(it);
        return;
      }
      if (ui.activate) {
        ui.activate(it, actions);
        return;
      }
      if (ui.external !== false) activate(it);
    },
    // actions 引用稳定的 useState setter / useCallback 闭包，逐项列出即可
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [activate, openDetail, vaultAction]
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

    // Ctrl+, 打开设置
    if (e.ctrlKey && e.key === ",") {
      e.preventDefault();
      openSettings();
      return;
    }

    // 条目次级操作（普通域）：Ctrl+Shift+Enter 管理员 / Ctrl+Enter 所在文件夹 /
    // Ctrl+Shift+C 复制路径。域内按键（isVault/isJson）不响应，次级动作按 kind 限权
    if (!isVault && !isJson && e.ctrlKey && e.key === "Enter") {
      e.preventDefault();
      if (e.shiftKey) void secondaryAction("admin", items[active]);
      else void secondaryAction("reveal", items[active]);
      return;
    }
    if (
      !isVault &&
      !isJson &&
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
      // 工具域无列表（items 是进入前的陈旧结果），Enter 不动作
      if (!isJson && !isTool) runItem(items[active]);
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

  if (view.kind === "json") {
    const rowid = Number(view.data.target);
    return (
      <JsonWorkbench
        clipRow={Number.isNaN(rowid) ? undefined : rowid}
        full
        onBack={() => {
          setView({ kind: "main" });
          setTimeout(() => inputRef.current?.focus(), 30);
        }}
        focusRef={focusPanelRef}
      />
    );
  }

  if (view.kind === "tool") {
    const rowid = Number(view.data.target);
    return (
      <ToolWorkbench
        clipRow={Number.isNaN(rowid) ? undefined : rowid}
        full
        onBack={() => {
          setView({ kind: "main" });
          setTimeout(() => inputRef.current?.focus(), 30);
        }}
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
    <div className={"zap-panel" + (isVault ? " vault" : "")}>
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
            isVault
              ? "搜索账号…"
              : isJson
                ? "在 json 后粘贴或输入 JSON，实时校验美化"
                : isTool
                  ? "在 tool 后粘贴文本，换算时间戳/编解码/哈希"
                  : "搜索应用、文件、窗口、剪贴板…"
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
      <div className="zap-results">
        {isJson ? (
          <JsonWorkbench text={jsonText} onBack={() => setQuery("")} />
        ) : isTool ? (
          <ToolWorkbench text={toolText} onBack={() => setQuery("")} />
        ) : items.length === 0 ? (
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
                // 应用/文件条目弹次级操作菜单；其余 kind 只压掉 WebView 默认菜单
                e.preventDefault();
                if (it.kind === "app" || it.kind === "file") {
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
            style={{
              left: Math.min(ctxMenu.x, 720 - 176),
              top: Math.min(ctxMenu.y, 480 - 132),
            }}
          >
            {(ctxMenu.item.kind === "app"
              ? ([
                  ["reveal", "打开所在文件夹"],
                  ["admin", "管理员运行"],
                  ["copy_path", "复制路径"],
                ] as const)
              : ([
                  ["reveal", "打开所在文件夹"],
                  ["copy_path", "复制路径"],
                ] as const)
            ).map(([act, label]) => (
              <button
                key={act}
                type="button"
                className="zap-ctx-item"
                onClick={() => {
                  setCtxMenu(null);
                  void secondaryAction(act, ctxMenu.item);
                }}
              >
                {label}
              </button>
            ))}
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
              <Kbd>pw</Kbd>
              <span>账号库</span>
              <span className="zap-footer-sep" />
              <Kbd>json</Kbd>
              <span>JSON 工具</span>
              <span className="zap-footer-sep" />
              <Kbd>tool</Kbd>
              <span>文本工具</span>
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
