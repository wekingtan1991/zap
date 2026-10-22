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
import { SettingsView } from "@/views/SettingsView";
import { useUpdater } from "@/views/useUpdater";

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
  const [form, setForm] = useState<AccountForm | null>(null);
  const [importing, setImporting] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [formError, setFormError] = useState("");
  const [detail, setDetail] = useState<VaultDetail | null>(null);
  const [showPw, setShowPw] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [revealPw, setRevealPw] = useState<RevealInfo | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const queryRef = useRef("");
  const busyRef = useRef(false);
  const vaultTermRef = useRef("");
  const activeItemRef = useRef<HTMLDivElement | null>(null);
  const kbNavRef = useRef(false);
  const [copiedField, setCopiedField] = useState<string | null>(null);
  /// JSON 识别条目进入的工作台（剪贴板行号寻址）
  const [jsonItem, setJsonItem] = useState<SearchItem | null>(null);

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
  // 工具域：json 前缀进入 JSON 工作台，其后文本即待分析内容
  const isJson = !isVault && query.trim().toLowerCase().startsWith("json");
  const jsonText = isJson ? query.trim().slice(4).trim() : "";
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
      setForm(null);
      setDetail(null);
      setRevealPw(null);
      setSettingsOpen(false);
      setJsonItem(null);
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
    if (isVault || isJson) return;
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
    invoke("open_item", { item });
  }, []);

  // ---------- 应用设置 / 自动更新 ----------
  // 设置页视图与更新状态机各自成模块（views/SettingsView、views/useUpdater）；
  // 这里只持视图开关与动作入口。更新 hook 挂在 AppView：启动静默检查不依赖设置页
  const updater = useUpdater();

  const openSettings = useCallback(() => {
    setForm(null);
    setDetail(null);
    setImporting(false);
    setJsonItem(null);
    setSettingsOpen(true);
  }, []);

  const closeSettings = useCallback(() => {
    setSettingsOpen(false);
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
            });
          }
          return;
        }
        if (action === "reveal") {
          const pw = await invoke<string>("vault_action", { id, action });
          setRevealPw({
            id,
            title: item.title.replace(/^📌 /, ""),
            password: pw,
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
        setItems([
          {
            id: "vault-error",
            title: "操作失败",
            subtitle: String(e),
            kind: "info",
            target: "",
            score: 0,
          },
        ]);
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
        const [title, username, password, url, notes, pinned] = await invoke<
          [string, string, string, string, string, number]
        >("vault_get", { id });
        setFormError("");
        setForm({
          id,
          title,
          username,
          password,
          url,
          notes,
          pinned: pinned === 1,
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
    setDetail(null);
    setShowPw(false);
    setConfirmDelete(false);
    setCopiedField(null);
    setTimeout(() => inputRef.current?.focus(), 30);
  }, []);

  // JSON 工作台（识别条目进入）：Esc 返回列表
  const closeJson = useCallback(() => {
    setJsonItem(null);
    setTimeout(() => inputRef.current?.focus(), 30);
  }, []);

  const openDetail = useCallback(
    async (item: SearchItem) => {
      const id = Number(item.target);
      if (Number.isNaN(id)) return;
      try {
        const [title, username, password, url, notes, pinned] = await invoke<
          [string, string, string, string, string, number]
        >("vault_get", { id });
        setShowPw(false);
        setConfirmDelete(false);
        setCopiedField(null);
        setDetail({ id, title, username, password, url, notes, pinned });
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
        const [title, username, password, url, notes, pinned] = await invoke<
          [string, string, string, string, string, number]
        >("vault_get", { id });
        setShowPw(false);
        setCopiedField(null);
        setConfirmDelete(true);
        setDetail({ id, title, username, password, url, notes, pinned });
      } catch (e) {
        feedback(false, "打开失败：" + String(e));
      }
    },
    [feedback]
  );

  /// 详情页行尾复制：username / password / url / notes（密码带阅后即焚，均不入剪贴板历史）
  const copyField = useCallback(
    async (field: "username" | "password" | "url" | "notes") => {
      if (!detail) return;
      try {
        const msg = await invoke<string>("vault_action", {
          id: detail.id,
          action: field,
        });
        // 行尾图标点亮为 ✓（1.6s 后还原），同时弹 toast 说明
        setCopiedField(field);
        setTimeout(() => setCopiedField((c) => (c === field ? null : c)), 1600);
        feedback(true, msg);
      } catch (e) {
        feedback(false, "复制失败：" + String(e));
      }
    },
    [detail, feedback]
  );

  const pinCurrent = useCallback(async () => {
    if (!detail) return;
    try {
      await invoke<string>("vault_action", { id: detail.id, action: "pin" });
      setDetail((d) => (d ? { ...d, pinned: d.pinned === 1 ? 0 : 1 } : d));
      await refreshVault(vaultTermRef.current);
    } catch (e) {
      feedback(false, "操作失败：" + String(e));
    }
  }, [detail, feedback, refreshVault]);

  const deleteCurrent = useCallback(async () => {
    if (!detail) return;
    try {
      await invoke<string>("vault_action", { id: detail.id, action: "delete" });
      closeDetail();
      await refreshVault(vaultTermRef.current);
    } catch (e) {
      feedback(false, "删除失败：" + String(e));
      setConfirmDelete(false);
    }
  }, [detail, feedback, refreshVault, closeDetail]);

  /// 详情页编辑：记录已在手上，直接预填表单（无需再取一次）
  const editCurrent = useCallback(() => {
    if (!detail) return;
    setFormError("");
    setForm({
      id: detail.id,
      title: detail.title,
      username: detail.username,
      password: detail.password,
      url: detail.url,
      notes: detail.notes,
      pinned: detail.pinned === 1,
    });
    setDetail(null);
  }, [detail]);

  // ---------- 统一条目分发 ----------
  // Enter 与点击共用：伪条目（新增/导入）→ 注册表 activate → 默认 openExternal。
  // kind 行为收敛在 plugins/registry.tsx，这里只接装配，不写 kind 特判
  const actions: ItemActions = {
    openVaultDetail: (it) => void openDetail(it),
    openJson: (it) => setJsonItem(it),
    runCommand: (target) =>
      void runCommand(target, {
        busyRef,
        setBusy,
        setItems,
      } satisfies CommandCtx),
    openExternal: activate,
    vaultPin: (it) => void vaultAction(it, "pin"),
  };

  const runItem = useCallback(
    (it: SearchItem | undefined) => {
      if (!it || busyRef.current) return;
      // 保护域伪条目：id 寻址（列表展示与普通条目无异，动作单独接）
      if (it.id === "vault:add") {
        setFormError("");
        setForm({ ...EMPTY_FORM });
        return;
      }
      if (it.id === "vault:import") {
        setImporting(true);
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

  // 表单保存成功：关表单 + 回账号库列表（空关键词列出全部）
  const formSaved = useCallback(() => {
    setForm(null);
    setQuery("pw");
    void refreshVault("");
  }, [refreshVault]);

  // 表单取消：编辑态（有 id）返回该账号详情页（编辑入口在详情页，取消不应退到列表），
  // 新增态回账号库列表；Esc 与「取消」按钮共用
  const cancelForm = useCallback(() => {
    const id = form?.id;
    setForm(null);
    setFormError("");
    if (id == null) {
      setTimeout(() => inputRef.current?.focus(), 30);
      return;
    }
    void (async () => {
      try {
        const [title, username, password, url, notes, pinned] = await invoke<
          [string, string, string, string, string, number]
        >("vault_get", { id });
        setConfirmDelete(false);
        setCopiedField(null);
        setDetail({ id, title, username, password, url, notes, pinned });
      } catch {
        // 详情打不开（账号已被删等）退回旧行为：列表 + 焦点回输入框
        setTimeout(() => inputRef.current?.focus(), 30);
      }
    })();
  }, [form]);

  // 导入收尾：关视图 + 刷新列表（成功反馈由导入视图先展示 700ms）
  const importDone = useCallback(() => {
    setImporting(false);
    void refreshVault(vaultTermRef.current);
  }, [refreshVault]);

  const closeImport = useCallback(() => {
    setImporting(false);
    setTimeout(() => inputRef.current?.focus(), 30);
  }, []);

  const onKeyDown = (e: React.KeyboardEvent) => {
    // 表单模式的 Esc/Ctrl+Enter 由 VaultFormView 自持，不再经过这里

    // 查看密码浮层：Esc 返回
    if (revealPw) {
      if (e.key === "Escape") {
        e.preventDefault();
        setRevealPw(null);
      }
      return;
    }

    // 账号详情：Esc 返回列表（行内按钮负责复制/编辑/删除/置顶）；
    // 删除确认弹窗存活时 Esc 只取消弹窗，不连详情页一起退
    if (detail) {
      if (e.key === "Escape") {
        e.preventDefault();
        if (confirmDelete) {
          setConfirmDelete(false);
        } else {
          closeDetail();
        }
      }
      return;
    }

    // 导入模式：Esc 返回
    if (importing) {
      if (e.key === "Escape") {
        e.preventDefault();
        closeImport();
      }
      return;
    }

    // 设置页：Esc 返回
    if (settingsOpen) {
      if (e.key === "Escape") {
        e.preventDefault();
        closeSettings();
      }
      return;
    }

    // JSON 工作台（识别条目进入）：Esc 返回列表
    if (jsonItem) {
      if (e.key === "Escape") {
        e.preventDefault();
        closeJson();
      }
      return;
    }

    // Ctrl+, 打开设置
    if (e.ctrlKey && e.key === ",") {
      e.preventDefault();
      openSettings();
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
      if (!isJson) runItem(items[active]);
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

  // ---------- 表单模式（渲染收敛进插件视图） ----------
  if (form) {
    return (
      <VaultFormView
        form={form}
        formError={formError}
        setForm={setForm}
        setFormError={setFormError}
        onSaved={formSaved}
        onCancel={cancelForm}
        focusRef={focusPanelRef}
      />
    );
  }

  // ---------- 查看密码（渲染收敛进插件视图） ----------
  if (revealPw) {
    return (
      <VaultRevealOverlay
        reveal={revealPw}
        onClose={() => setRevealPw(null)}
        onKeyDown={onKeyDown}
        focusRef={focusPanelRef}
      />
    );
  }

  // ---------- 智能导入（渲染收敛进插件视图） ----------
  if (importing) {
    return (
      <VaultImportView
        onDone={importDone}
        onClose={closeImport}
        onKeyDown={onKeyDown}
        focusRef={focusPanelRef}
      />
    );
  }

  // ---------- 账号详情：点击标题打开，逐行展示，行尾一键复制（渲染收敛进插件视图） ----------
  if (detail) {
    return (
      <VaultDetailView
        detail={detail}
        showPw={showPw}
        confirmDelete={confirmDelete}
        copiedField={copiedField}
        focusRef={focusPanelRef}
        onKeyDown={onKeyDown}
        onClose={closeDetail}
        onTogglePw={() => setShowPw((v) => !v)}
        onAskDelete={() => setConfirmDelete(true)}
        onCancelDelete={() => setConfirmDelete(false)}
        onDelete={() => void deleteCurrent()}
        onCopyField={(f) => void copyField(f)}
        onEdit={editCurrent}
        onPin={() => void pinCurrent()}
      />
    );
  }

  // ---------- JSON 工作台：识别条目进入（完整面板） ----------
  if (jsonItem) {
    const rowid = Number(jsonItem.target);
    return (
      <JsonWorkbench
        clipRow={Number.isNaN(rowid) ? undefined : rowid}
        full
        onBack={closeJson}
        focusRef={focusPanelRef}
        onKeyDown={onKeyDown}
      />
    );
  }

  // ---------- 设置页（渲染收敛进 views/SettingsView） ----------
  if (settingsOpen) {
    return (
      <SettingsView
        updater={updater}
        onKeyDown={onKeyDown}
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
