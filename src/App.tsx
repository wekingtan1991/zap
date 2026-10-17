import React, {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { toast } from "sonner";
import {
  ArrowLeft,
  Check,
  Copy,
  Eye,
  EyeOff,
  KeyRound,
  Loader2,
  Pencil,
  Pin,
  PinOff,
  Settings as SettingsIcon,
  Trash2,
} from "lucide-react";
import "./App.css";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Badge } from "@/components/ui/badge";
import { Kbd } from "@/components/ui/kbd";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Toaster } from "@/components/ui/sonner";

interface SearchItem {
  id: string;
  title: string;
  subtitle: string;
  kind: string;
  target: string;
  icon?: string | null;
  score: number;
}

const KIND_LABEL: Record<string, string> = {
  app: "应用",
  clip: "剪贴板",
  file: "文件",
  win: "窗口",
  recent: "最近",
  settings: "设置",
  vault: "账号",
  web: "搜索",
  url: "网址",
  cmd: "功能",
  info: "状态",
};

/// 列表徽标配色（shadcn Badge 变体），与各类源的语义色一一对应
const BADGE_VARIANT: Record<
  string,
  | "app"
  | "clip"
  | "win"
  | "recent"
  | "file"
  | "cmd"
  | "settings"
  | "vault"
  | "web"
  | "url"
  | "info"
> = {
  app: "app",
  clip: "clip",
  file: "file",
  win: "win",
  recent: "recent",
  settings: "settings",
  vault: "vault",
  web: "web",
  url: "url",
  cmd: "cmd",
  info: "info",
};

/// 是否已置顶：后端给置顶条目的标题加了 📌 前缀
const pinned = (it: SearchItem) => it.title.startsWith("📌");

interface AccountForm {
  id?: number;
  title: string;
  username: string;
  password: string;
  url: string;
  notes: string;
  pinned: boolean;
}

const EMPTY_FORM: AccountForm = {
  title: "",
  username: "",
  password: "",
  url: "",
  notes: "",
  pinned: false,
};

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
  title: "📥 智能导入（粘贴文本批量识别）",
  subtitle: "从记事本 / 聊天记录粘贴账号文本，自动解析字段后校对入库",
  kind: "info",
  target: "",
  score: -2,
};

interface ParsedAccount {
  title: string;
  username: string;
  password: string;
  url: string;
}

/// 账号详情页数据（vault_get 的完整记录，密码为解密后明文，默认掩码展示）
interface VaultDetail {
  id: number;
  title: string;
  username: string;
  password: string;
  url: string;
  notes: string;
  pinned: number;
}

interface AppSettings {
  hotkey: string;
  clipEnabled: boolean;
  clipMax: number;
  pwdClearSecs: number;
  autostart: boolean;
  everythingAutostart: boolean;
}

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
            <span className="zap-logo">⚠</span>
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
  const [importText, setImportText] = useState("");
  const [parsed, setParsed] = useState<ParsedAccount[] | null>(null);
  const [importBusy, setImportBusy] = useState(false);
  const [importMsg, setImportMsg] = useState<{ ok: boolean; text: string } | null>(
    null
  );
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [st, setSt] = useState<AppSettings | null>(null);
  const [appVersion, setAppVersion] = useState("");
  const [hkRecording, setHkRecording] = useState(false);
  const [hkError, setHkError] = useState("");
  const [saveMsg, setSaveMsg] = useState("");
  const [formError, setFormError] = useState("");
  const [detail, setDetail] = useState<VaultDetail | null>(null);
  const [showPw, setShowPw] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [revealPw, setRevealPw] = useState<{ title: string; password: string } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const queryRef = useRef("");
  const busyRef = useRef(false);
  const vaultTermRef = useRef("");
  const [copiedField, setCopiedField] = useState<string | null>(null);

  useEffect(() => {
    queryRef.current = query;
  }, [query]);
  useEffect(() => {
    busyRef.current = busy;
  }, [busy]);

  // 保护域：pw 前缀进入账号库
  const isVault = query.trim().toLowerCase().startsWith("pw");
  const vaultTerm = isVault ? query.trim().slice(2).trim() : "";
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

  // ---------- 热键录制 ----------
  // 组合由 Rust 低级键盘钩子经 zap://hk-capture 事件送达——连被输入法接管的
  // Ctrl+Space（中英切换）也能捕获；页面只负责 Esc 取消与钩子启停
  const stopHkCapture = useCallback(() => {
    invoke("stop_hotkey_capture").catch(() => {});
  }, []);

  useEffect(() => {
    const un = listen<string>("zap://hk-capture", (e) => {
      setSt((s) => (s ? { ...s, hotkey: e.payload } : s));
      setHkRecording(false);
      stopHkCapture();
    });
    return () => {
      un.then((f) => f());
    };
  }, [stopHkCapture]);

  // 物理按键码 → 热键键名（不受输入法/键盘布局影响；e.key 在中文输入法下常为 "Process"）
  const codeToKeyName = (code: string): string | null => {
    if (code.startsWith("Key")) return code.slice(3).toLowerCase(); // KeyJ → j
    if (code.startsWith("Digit")) return code.slice(5); // Digit5 → 5
    if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code.toLowerCase(); // F2 → f2
    return code === "Space" ? "space" : null;
  };

  useEffect(() => {
    if (!hkRecording) return;
    // 页面级快速路径：捕获普通组合（焦点在面板内即可）；被输入法接管的组合由
    // Rust 侧轮询兜底（zap://hk-capture 事件）
    const onWinKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        setHkRecording(false);
        stopHkCapture();
        return;
      }
      const key = codeToKeyName(e.code);
      if (!key) return;
      const mods: string[] = [];
      if (e.ctrlKey) mods.push("ctrl");
      if (e.altKey) mods.push("alt");
      if (e.shiftKey) mods.push("shift");
      if (e.metaKey) mods.push("meta");
      if (mods.length === 0) return; // 必须带修饰键
      e.preventDefault();
      e.stopPropagation();
      setSt((s) => (s ? { ...s, hotkey: [...mods, key].join("+") } : s));
      setHkRecording(false);
      stopHkCapture();
    };
    window.addEventListener("keydown", onWinKey, true);
    return () => window.removeEventListener("keydown", onWinKey, true);
  }, [hkRecording, stopHkCapture]);

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
      setSt(null);
      setHkRecording(false);
      setHkError("");
      stopHkCapture();
      setActive(0);
      setTimeout(() => inputRef.current?.focus(), 30);
      if (busyRef.current) return;
      invoke<SearchItem[]>("search", { query: "" })
        .then(async (res) => {
          if (queryRef.current) return;
          let list = res;
          let evDown = false;
          try {
            const st = await invoke<string>("everything_status");
            evDown = st !== "running";
            if (evDown) {
              list = [
                ...list,
                {
                  id: "cmd:everything",
                  title: "启用全盘文件搜索（Everything）",
                  subtitle:
                    st === "installed_not_running"
                      ? "启动已就绪的 Everything · 弹出 UAC 请点“是”"
                      : "自动下载并配置，约 2MB · 弹出 UAC 请点“是”",
                  kind: "cmd",
                  target: "everything_setup",
                  score: 0,
                },
              ];
            }
          } catch {
            // 状态查询失败不影响列表
          }
          if (!queryRef.current) setItems(list);
          // Everything 未运行：后台自动启动（后端按设置与节流决定是否真的动作）
          if (evDown) {
            invoke<string>("everything_ensure")
              .then((after) => {
                if (after === "running" && !queryRef.current) {
                  setItems((cur) => cur.filter((x) => x.id !== "cmd:everything"));
                }
              })
              .catch(() => {});
          }
        })
        .catch(() => {});
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // 普通域搜索（保护域下不生效）
  useEffect(() => {
    if (isVault) return;
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

  // ---------- 应用设置 ----------
  const openSettings = useCallback(async () => {
    try {
      const s = await invoke<AppSettings>("get_app_settings");
      setForm(null);
      setDetail(null);
      setImporting(false);
      setSt(s);
      setHkRecording(false);
      setHkError("");
      setSaveMsg("");
      setSettingsOpen(true);
      // 版本号异步读取（tauri.conf.json 的 version），失败时留空
      getVersion()
        .then((v) => setAppVersion(v))
        .catch(() => {});
    } catch {
      // 忽略
    }
  }, []);

  const closeSettings = useCallback(() => {
    setHkRecording(false);
    stopHkCapture();
    setSettingsOpen(false);
    setSt(null);
    setTimeout(() => inputRef.current?.focus(), 30);
  }, [stopHkCapture]);

  useEffect(() => {
    const un = listen("zap://open-settings", () => {
      openSettings();
    });
    return () => {
      un.then((f) => f());
    };
  }, [openSettings]);

  const saveSettings = async () => {
    if (!st) return;
    try {
      const res = await invoke<{ hotkeyError: string | null }>("save_settings", {
        hotkey: st.hotkey,
        clipEnabled: st.clipEnabled,
        clipMax: st.clipMax,
        pwdClearSecs: st.pwdClearSecs,
        everythingAutostart: st.everythingAutostart,
      });
      if (res.hotkeyError) {
        setHkError(res.hotkeyError);
        setSt(await invoke<AppSettings>("get_app_settings")); // 回显实际生效值
      } else {
        setHkError("");
        setSaveMsg("已保存 ✓");
        setTimeout(() => setSaveMsg(""), 2000);
      }
    } catch (e) {
      setHkError(String(e));
    }
  };

  // ---------- 自动更新 ----------
  const updateRef = useRef<Update | null>(null);
  const updateNotifiedRef = useRef(false);
  const [updateStatus, setUpdateStatus] = useState<
    "idle" | "checking" | "available" | "downloading" | "ready"
  >("idle");
  const [updateVersion, setUpdateVersion] = useState("");
  const [updateProgress, setUpdateProgress] = useState(0);
  const [updateMsg, setUpdateMsg] = useState("");

  // 拿到检查结果后的公共收尾：有新版本则记录并置为可下载，否则回到空闲
  const applyUpdate = useCallback((u: Update | null) => {
    updateRef.current = u;
    if (u) {
      setUpdateVersion(u.version);
      setUpdateStatus("available");
    } else {
      setUpdateStatus("idle");
    }
  }, []);

  // 启动静默检查一次：离线/网络失败不打扰；面板隐藏时 toast 不可见，
  // 命中新版本只记标记，等下次面板呼出再提示
  useEffect(() => {
    check()
      .then((u) => {
        if (u) {
          applyUpdate(u);
          updateNotifiedRef.current = true;
        }
      })
      .catch(() => {});
  }, [applyUpdate]);

  useEffect(() => {
    const un = listen("zap://shown", () => {
      if (updateNotifiedRef.current && updateRef.current) {
        updateNotifiedRef.current = false;
        toast.info(`发现新版本 v${updateRef.current.version}，可在设置中更新`);
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, []);

  const checkForUpdate = useCallback(async () => {
    if (updateStatus === "checking" || updateStatus === "downloading") return;
    setUpdateStatus("checking");
    setUpdateMsg("");
    try {
      const u = await check();
      applyUpdate(u);
      if (!u) setUpdateMsg("已是最新版本 ✓");
    } catch (e) {
      setUpdateStatus("idle");
      setUpdateMsg("检查失败：" + String(e));
    }
  }, [updateStatus, applyUpdate]);

  // 下载并静默安装 NSIS 安装包；期间保持面板可见（UAC/安装器抢焦点不触发失焦即隐）
  const downloadUpdate = useCallback(async () => {
    const u = updateRef.current;
    if (!u || updateStatus === "downloading") return;
    setUpdateStatus("downloading");
    setUpdateProgress(0);
    await invoke("set_hold_panel", { hold: true }).catch(() => {});
    let received = 0;
    let total = 0;
    try {
      await u.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? 0;
        } else if (event.event === "Progress") {
          received += event.data.chunkLength;
          if (total > 0)
            setUpdateProgress(
              Math.min(99, Math.round((received / total) * 100))
            );
        }
      });
      setUpdateProgress(100);
      setUpdateStatus("ready");
    } catch (e) {
      setUpdateStatus("available");
      toast.error("下载更新失败：" + String(e));
    } finally {
      await invoke("set_hold_panel", { hold: false }).catch(() => {});
    }
  }, [updateStatus]);

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
          setRevealPw({ title: item.title.replace(/^📌 /, ""), password: pw });
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

  // 表单密码生成：crypto 随机 16 位（去掉易混淆字符）
  const genPassword = () => {
    const chars =
      "abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#$%^&*";
    const arr = new Uint32Array(16);
    crypto.getRandomValues(arr);
    const pw = Array.from(arr, (n) => chars[n % chars.length]).join("");
    if (form) setForm({ ...form, password: pw });
  };

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

  const runEverythingSetup = useCallback(async () => {
    if (busyRef.current) return;
    setBusy(true);
    setItems([
      {
        id: "setup-busy",
        title: "正在准备 Everything…",
        subtitle: "下载约 2MB 并自动配置；若弹出 UAC 请点“是”",
        kind: "info",
        target: "",
        score: 0,
      },
    ]);
    await invoke("set_hold_panel", { hold: true }).catch(() => {});
    const result = await invoke<string>("everything_setup").catch(
      (e) => "error: " + e
    );
    await invoke("set_hold_panel", { hold: false }).catch(() => {});
    if (result === "running") {
      setItems([
        {
          id: "setup-done",
          title: "全盘搜索已启用 ✓",
          subtitle: "输入任意文件名即可搜索全盘，Esc 关闭后立即生效",
          kind: "info",
          target: "",
          score: 0,
        },
      ]);
    } else {
      setItems([
        {
          id: "setup-fail",
          title: "启用未完成",
          subtitle: String(result),
          kind: "info",
          target: "",
          score: 0,
        },
      ]);
    }
    setBusy(false);
  }, []);

  const saveForm = useCallback(async () => {
    if (!form) return;
    if (!form.title.trim()) {
      setFormError("请填写标题——用于记录这是什么账号，如“行云账号”");
      return;
    }
    setFormError("");
    try {
      if (form.id != null) {
        await invoke("vault_update", {
          id: form.id,
          title: form.title,
          username: form.username,
          password: form.password, // 留空 = 保持原密码
          url: form.url,
          notes: form.notes,
          pinned: form.pinned ? 1 : 0,
        });
      } else {
        await invoke("vault_add", {
          title: form.title,
          username: form.username,
          password: form.password,
          url: form.url,
          notes: form.notes,
        });
      }
      setFormError("");
      setForm(null);
      setQuery("pw");
      await refreshVault("");
    } catch (e) {
      // 表单视图不渲染结果列表，错误必须显示在表单内
      setFormError("保存失败：" + String(e));
    }
  }, [form, refreshVault]);

  const closeImport = useCallback(() => {
    setImporting(false);
    setParsed(null);
    setImportText("");
    setImportBusy(false);
    setImportMsg(null);
    setTimeout(() => inputRef.current?.focus(), 30);
  }, []);

  const updParsed = useCallback(
    (i: number, key: keyof ParsedAccount, value: string) => {
      setParsed((arr) =>
        arr ? arr.map((x, ix) => (ix === i ? { ...x, [key]: value } : x)) : arr
      );
    },
    []
  );

  const onKeyDown = (e: React.KeyboardEvent) => {
    // 表单模式：Ctrl+Enter 保存，Esc 取消
    if (form) {
      if (e.key === "Escape") {
        e.preventDefault();
        setForm(null);
        setTimeout(() => inputRef.current?.focus(), 30);
      } else if (e.key === "Enter" && e.ctrlKey) {
        e.preventDefault();
        saveForm();
      }
      return;
    }

    // 查看密码浮层：Esc 返回
    if (revealPw) {
      if (e.key === "Escape") {
        e.preventDefault();
        setRevealPw(null);
      }
      return;
    }

    // 账号详情：Esc 返回列表（行内按钮负责复制/编辑/删除/置顶）
    if (detail) {
      if (e.key === "Escape") {
        e.preventDefault();
        closeDetail();
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
        if (e.key === "3") vaultAction(it, "delete");
        if (e.key === "4") vaultAction(it, "reveal");
        if (e.key === "5") vaultAction(it, "pin");
        if (e.key === "6") void startEdit(it);
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const it = items[active];
        if (!it) return;
        if (it.id === "vault:add") {
          setFormError("");
          setForm({ ...EMPTY_FORM });
          return;
        }
        if (it.id === "vault:import") {
          setImporting(true);
          return;
        }
        if (it.kind === "vault") void openDetail(it);
        return;
      }
      if (e.key === "Escape") {
        invoke("hide_panel");
        return;
      }
      // ↑↓ 落到下方通用导航
    }

    if (e.key === "ArrowDown") {
      e.preventDefault();
      setActive((a) => Math.min(a + 1, Math.max(items.length - 1, 0)));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => Math.max(a - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const it = items[active];
      if (!it || busy) return;
      if (it.kind === "info") return;
      if (it.kind === "cmd") {
        runEverythingSetup();
        return;
      }
      activate(it);
    } else if (e.key === "Escape") {
      invoke("hide_panel");
    }
  };

  const setFormField = (key: keyof AccountForm, value: string) => {
    if (form) setForm({ ...form, [key]: value });
  };

  // ---------- 表单模式 ----------
  if (form) {
    return (
      <div className="zap-panel vault" onKeyDown={onKeyDown}>
        <div className="zap-input-row">
          <span className="zap-logo">
            <KeyRound className="size-4 text-primary" />
          </span>
          <span className="zap-vault-tag">
            {form.id != null ? "编辑账号" : "新增账号"}
          </span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
            onClick={() => setForm(null)}
          >
            <ArrowLeft className="size-3" />
            取消（Esc）
          </Button>
        </div>
        <div className="zap-form">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="f-title">标题 *</Label>
            <Input
              id="f-title"
              autoFocus
              value={form.title}
              onChange={(e) => setFormField("title", e.target.value)}
              placeholder="例如：行云账号"
            />
            <span className="zap-form-hint">
              标题用于记录这是什么账号（如“行云账号”），保存后显示在列表里
            </span>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="f-user">用户名</Label>
            <Input
              id="f-user"
              value={form.username}
              onChange={(e) => setFormField("username", e.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="f-pass">
              密码（DPAPI 加密存储，不入剪贴板历史
              {form.id != null ? "；留空 = 保持原密码" : ""}）
            </Label>
            <div className="zap-pass-row">
              <Input
                id="f-pass"
                type="password"
                value={form.password}
                onChange={(e) => setFormField("password", e.target.value)}
              />
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={genPassword}
                title="生成 16 位强密码"
              >
                生成
              </Button>
            </div>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="f-url">网址</Label>
            <Input
              id="f-url"
              value={form.url}
              onChange={(e) => setFormField("url", e.target.value)}
              placeholder="https://…"
            />
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="f-notes">备注</Label>
            <Textarea
              id="f-notes"
              rows={2}
              value={form.notes}
              onChange={(e) => setFormField("notes", e.target.value)}
            />
          </div>
          <div className="flex items-center gap-2.5">
            <Checkbox
              id="f-pin"
              checked={form.pinned}
              onCheckedChange={(v) =>
                setForm((f) => (f ? { ...f, pinned: v === true } : f))
              }
            />
            <Label htmlFor="f-pin" className="text-[13px] text-foreground">
              固定置顶（常用账号排在最前）
            </Label>
          </div>
          <div className="zap-form-actions">
            <Button variant="outline" onClick={() => setForm(null)}>
              取消（Esc）
            </Button>
            <Button onClick={() => saveForm()}>保存（Ctrl+Enter）</Button>
          </div>
          {formError && (
            <div className="zap-hk-err animate-shake">{formError}</div>
          )}
        </div>
      </div>
    );
  }

  // ---------- 查看密码 ----------
  if (revealPw) {
    return (
      <div className="zap-panel vault" onKeyDown={onKeyDown}>
        <div className="zap-input-row">
          <span className="zap-logo">
            <Eye className="size-4 text-primary" />
          </span>
          <span className="zap-vault-tag">查看密码</span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
            onClick={() => setRevealPw(null)}
          >
            <ArrowLeft className="size-3" />
            返回（Esc）
          </Button>
        </div>
        <div className="zap-form animate-fade-in">
          <div className="flex flex-col gap-1.5">
            <Label>{revealPw.title}</Label>
            <Input
              autoFocus
              readOnly
              className="zap-pw-reveal h-9 font-mono text-[15px] tracking-widest text-primary"
              value={revealPw.password}
              onFocus={(e) => e.currentTarget.select()}
            />
          </div>
          <div className="zap-vault-tag flex items-center gap-1.5">
            可框选复制 · 关闭密码请用 <Kbd>Esc</Kbd>（阅后即焚，不入剪贴板历史）
          </div>
        </div>
      </div>
    );
  }

  // ---------- 智能导入 ----------
  if (importing) {
    return (
      <div className="zap-panel vault" onKeyDown={onKeyDown}>
        <div className="zap-input-row">
          <span className="zap-logo">📥</span>
          <span className="zap-vault-tag">智能导入</span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
            onClick={closeImport}
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
                    setTimeout(() => {
                      closeImport();
                      void refreshVault(vaultTermRef.current);
                    }, 700);
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

  // ---------- 账号详情：点击标题打开，逐行展示，行尾一键复制 ----------
  if (detail) {
    const rows: {
      label: string;
      value: string;
      field: "username" | "password" | "url" | "notes";
    }[] = [
      { label: "用户名", value: detail.username, field: "username" },
      { label: "密码", value: detail.password, field: "password" },
      { label: "网址", value: detail.url, field: "url" },
    ];
    if (detail.notes) {
      rows.push({ label: "备注", value: detail.notes, field: "notes" });
    }
    return (
      <div className="zap-panel vault" onKeyDown={onKeyDown}>
        <div className="zap-input-row">
          <span className="zap-logo">
            <KeyRound className="size-4 text-primary" />
          </span>
          <span className="zap-vault-tag">账号详情</span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
            onClick={closeDetail}
          >
            <ArrowLeft className="size-3" />
            返回（Esc）
          </Button>
        </div>
        <div className="zap-form">
          <div className="zap-detail-title flex items-center gap-1.5 animate-fade-in">
            {detail.pinned === 1 && (
              <Pin className="size-3.5 fill-current text-primary" />
            )}
            {detail.title}
          </div>
          {rows.map((r, i) => (
            <div
              key={r.label}
              className="zap-detail-row animate-fade-in"
              style={{ animationDelay: `${40 + i * 35}ms` }}
            >
              <span className="zap-detail-label">{r.label}</span>
              {r.field === "password" ? (
                <span
                  className={
                    "zap-detail-value transition-all duration-200" +
                    (showPw ? "" : " zap-detail-mask")
                  }
                >
                  {detail.password
                    ? showPw
                      ? detail.password
                      : "••••••••"
                    : "（空）"}
                </span>
              ) : (
                <span className="zap-detail-value">{r.value || "（空）"}</span>
              )}
              <span className="zap-detail-ops">
                {r.field === "password" && (
                  <Button
                    variant="secondary"
                    size="sm"
                    className="h-7 px-2 text-xs text-muted-foreground hover:text-foreground"
                    onClick={() => setShowPw((v) => !v)}
                  >
                    {showPw ? (
                      <EyeOff className="size-3" />
                    ) : (
                      <Eye className="size-3" />
                    )}
                    {showPw ? "隐藏" : "显示"}
                  </Button>
                )}
                <Tooltip>
                  <TooltipTrigger asChild>
                    <Button
                      variant="secondary"
                      size="icon-sm"
                      className="text-muted-foreground hover:text-foreground"
                      disabled={!r.value}
                      onClick={() => void copyField(r.field)}
                      aria-label={"复制" + r.label}
                    >
                      {copiedField === r.field ? (
                        <Check className="size-3.5 text-success animate-pop" />
                      ) : (
                        <Copy className="size-3.5" />
                      )}
                    </Button>
                  </TooltipTrigger>
                  <TooltipContent>
                    {copiedField === r.field ? "已复制 ✓" : "复制" + r.label}
                  </TooltipContent>
                </Tooltip>
              </span>
            </div>
          ))}
          <div className="zap-form-actions">
            <Button variant="outline" onClick={editCurrent}>
              <Pencil className="size-3.5" />
              编辑
            </Button>
            <Button variant="outline" onClick={() => void pinCurrent()}>
              {detail.pinned === 1 ? (
                <PinOff className="size-3.5" />
              ) : (
                <Pin className="size-3.5" />
              )}
              {detail.pinned === 1 ? "取消置顶" : "置顶"}
            </Button>
            {confirmDelete ? (
              <>
                <Button
                  variant="destructive"
                  className="animate-pulse-ring"
                  onClick={() => void deleteCurrent()}
                >
                  <Trash2 className="size-3.5" />
                  确认删除
                </Button>
                <Button variant="ghost" onClick={() => setConfirmDelete(false)}>
                  取消
                </Button>
              </>
            ) : (
              <Button variant="destructive" onClick={() => setConfirmDelete(true)}>
                <Trash2 className="size-3.5" />
                删除
              </Button>
            )}
          </div>
        </div>
      </div>
    );
  }

  // ---------- 设置页 ----------
  if (settingsOpen && st) {
    return (
      <div className="zap-panel vault" onKeyDown={onKeyDown}>
        <div className="zap-input-row">
          <span className="zap-logo">
            <SettingsIcon className="size-4 text-primary" />
          </span>
          <span className="zap-vault-tag">设置</span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
            onClick={closeSettings}
          >
            <ArrowLeft className="size-3" />
            返回（Esc）
          </Button>
        </div>
        <div className="zap-form">
          <div className="flex flex-col gap-1.5">
            <Label>呼出热键（点击输入框后按下新组合，Esc 取消）</Label>
            {hkRecording ? (
              <Input
                readOnly
                className="zap-hk-rec animate-pulse-ring"
                value="按下新组合…（Esc 取消）"
              />
            ) : (
              <Input
                readOnly
                className="zap-hk cursor-pointer"
                value={st.hotkey}
                onClick={() => {
                  invoke("start_hotkey_capture")
                    .then(() => setHkRecording(true))
                    .catch((e) => setHkError("启动录制失败：" + e));
                }}
              />
            )}
          </div>
          {hkError && (
            <div className="zap-hk-err animate-shake">{hkError}</div>
          )}
          <div className="flex items-center gap-2.5">
            <Checkbox
              id="set-clip"
              checked={st.clipEnabled}
              onCheckedChange={(v) =>
                setSt((s) => (s ? { ...s, clipEnabled: v === true } : s))
              }
            />
            <Label htmlFor="set-clip" className="text-[13px] text-foreground">
              记录剪贴板历史
            </Label>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label>历史保留条数</Label>
            <Select
              value={String(st.clipMax)}
              disabled={!st.clipEnabled}
              onValueChange={(v) =>
                setSt((s) => (s ? { ...s, clipMax: Number(v) } : s))
              }
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="100">100</SelectItem>
                <SelectItem value="500">500</SelectItem>
                <SelectItem value="1000">1000</SelectItem>
                <SelectItem value="5000">5000</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="flex flex-col gap-1.5">
            <Label>密码复制后自动清空剪贴板</Label>
            <Select
              value={String(st.pwdClearSecs)}
              onValueChange={(v) =>
                setSt((s) => (s ? { ...s, pwdClearSecs: Number(v) } : s))
              }
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="0">关闭</SelectItem>
                <SelectItem value="30">30 秒</SelectItem>
                <SelectItem value="45">45 秒</SelectItem>
                <SelectItem value="60">60 秒</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="flex items-center gap-2.5">
            <Checkbox
              id="set-autostart"
              checked={st.autostart}
              onCheckedChange={async (v) => {
                const on = v === true;
                setSt((s) => (s ? { ...s, autostart: on } : s));
                try {
                  await invoke("autostart_set", { enable: on });
                } catch {
                  setSt((s) => (s ? { ...s, autostart: !on } : s));
                }
              }}
            />
            <Label htmlFor="set-autostart" className="text-[13px] text-foreground">
              开机自启
            </Label>
          </div>
          <div className="flex items-center gap-2.5">
            <Checkbox
              id="set-ev"
              checked={st.everythingAutostart}
              onCheckedChange={(v) =>
                setSt((s) =>
                  s ? { ...s, everythingAutostart: v === true } : s
                )
              }
            />
            <Label htmlFor="set-ev" className="text-[13px] text-foreground">
              自动启动 Everything（未运行时自动拉起；不会自动下载）
            </Label>
          </div>
          <div className="zap-form-actions">
            <Button variant="outline" onClick={closeSettings}>
              取消（Esc）
            </Button>
            <Button onClick={saveSettings}>保存</Button>
          </div>
          {saveMsg && (
            <div className="zap-set-saved animate-fade-in">{saveMsg}</div>
          )}
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <span className="min-w-0 flex-1 truncate">
              {updateStatus === "checking" && "正在检查更新…"}
              {updateStatus === "downloading" && `正在下载更新 ${updateProgress}%`}
              {updateStatus === "ready" && "更新已就绪，重启 zap 完成安装"}
              {updateStatus === "available" && `发现新版本 v${updateVersion}`}
              {updateStatus === "idle" && updateMsg}
            </span>
            {updateStatus === "idle" && (
              <Button
                variant="outline"
                size="sm"
                className="shrink-0"
                onClick={() => void checkForUpdate()}
              >
                检查更新
              </Button>
            )}
            {updateStatus === "checking" && (
              <Button variant="outline" size="sm" className="shrink-0" disabled>
                <Loader2 className="size-3.5 animate-spin" />
                检查中
              </Button>
            )}
            {updateStatus === "downloading" && (
              <Button size="sm" className="shrink-0" disabled>
                <Loader2 className="size-3.5 animate-spin" />
                {updateProgress}%
              </Button>
            )}
            {updateStatus === "available" && (
              <Button
                size="sm"
                className="shrink-0"
                onClick={() => void downloadUpdate()}
              >
                下载并安装
              </Button>
            )}
            {updateStatus === "ready" && (
              <Button
                size="sm"
                className="shrink-0"
                onClick={() => void relaunch()}
              >
                重启以完成更新
              </Button>
            )}
          </div>
          <div className="zap-version">
            zap <span className="zap-version-num">v{appVersion || "…"}</span>
          </div>
        </div>
      </div>
    );
  }

  // ---------- 主面板 ----------
  return (
    <div className={"zap-panel" + (isVault ? " vault" : "")}>
      <div className="zap-input-row">
        <span className="zap-logo">{isVault ? "🔒" : "⚡"}</span>
        {isVault && <span className="zap-vault-tag">账号库</span>}
        <input
          ref={inputRef}
          autoFocus
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={
            isVault
              ? "搜索账号…（Enter 打开详情 · 行尾图标一键复制 · Esc 关闭）"
              : "搜索应用、文件、剪贴板…（pw 进账号库 · Esc 关闭）"
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
        {items.map((it, i) => (
          <div
            key={it.id + String(i)}
            className={"zap-item" + (i === active ? " active" : "")}
            onMouseEnter={() => setActive(i)}
            onClick={() => {
              if (busy) return;
              if (it.id === "vault:add") {
                setFormError("");
                setForm({ ...EMPTY_FORM });
                return;
              }
              if (it.id === "vault:import") {
                setImporting(true);
                return;
              }
              if (it.kind === "info") return;
              if (isVault) {
                if (it.kind === "vault") {
                  void openDetail(it);
                  return;
                }
              }
              if (it.kind === "cmd") {
                runEverythingSetup();
                return;
              }
              activate(it);
            }}
          >
            {it.icon ? (
              <img className="zap-ico" src={it.icon} alt="" draggable={false} />
            ) : (
              <Badge variant={BADGE_VARIANT[it.kind] ?? "secondary"}>
                {KIND_LABEL[it.kind] ?? it.kind}
              </Badge>
            )}
            <div className="zap-texts">
              <div className="zap-title">{it.title}</div>
              {it.subtitle && <div className="zap-sub">{it.subtitle}</div>}
            </div>
            {it.kind === "vault" && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="zap-row-pin text-muted-foreground hover:text-primary"
                    aria-label={pinned(it) ? "取消置顶" : "置顶"}
                    onClick={(e) => {
                      // 行尾动作：阻止冒泡，避免顺带打开详情页
                      e.stopPropagation();
                      void vaultAction(it, "pin");
                    }}
                  >
                    {pinned(it) ? (
                      <PinOff className="size-3.5" />
                    ) : (
                      <Pin className="size-3.5" />
                    )}
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{pinned(it) ? "取消置顶" : "置顶"}</TooltipContent>
              </Tooltip>
            )}
          </div>
        ))}
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
