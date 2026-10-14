import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

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

interface AppSettings {
  hotkey: string;
  clipEnabled: boolean;
  clipMax: number;
  pwdClearSecs: number;
  autostart: boolean;
}

function App() {
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<SearchItem[]>([]);
  const [active, setActive] = useState(0);
  const [busy, setBusy] = useState(false);
  const [form, setForm] = useState<AccountForm | null>(null);
  const [importing, setImporting] = useState(false);
  const [importText, setImportText] = useState("");
  const [parsed, setParsed] = useState<ParsedAccount[] | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [st, setSt] = useState<AppSettings | null>(null);
  const [hkRecording, setHkRecording] = useState(false);
  const [hkError, setHkError] = useState("");
  const [saveMsg, setSaveMsg] = useState("");
  const [revealPw, setRevealPw] = useState<{ title: string; password: string } | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const queryRef = useRef("");
  const busyRef = useRef(false);
  const vaultTermRef = useRef("");

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

  // 面板每次被呼出时清空状态并拉取最近剪贴板（zap://shown 由 Rust 侧在 show 后发出）
  useEffect(() => {
    const unlisten = listen("zap://shown", () => {
      setQuery(""); // 顺带退出保护域
      setForm(null);
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
          try {
            const st = await invoke<string>("everything_status");
            if (st !== "running") {
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
      setImporting(false);
      setSt(s);
      setHkRecording(false);
      setHkError("");
      setSaveMsg("");
      setSettingsOpen(true);
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

  const refreshVault = useCallback(async (term: string) => {
    try {
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
    if (!form.title.trim()) return;
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
      setForm(null);
      setQuery("pw");
      await refreshVault("");
    } catch (e) {
      setItems([
        {
          id: "form-error",
          title: "保存失败",
          subtitle: String(e),
          kind: "info",
          target: "",
          score: 0,
        },
      ]);
    }
  }, [form, refreshVault]);

  const closeImport = useCallback(() => {
    setImporting(false);
    setParsed(null);
    setImportText("");
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
          setForm({ ...EMPTY_FORM });
          return;
        }
        if (it.id === "vault:import") {
          setImporting(true);
          return;
        }
        if (it.kind === "vault") vaultAction(it, "username");
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
          <span className="zap-logo">🔒</span>
          <span className="zap-vault-tag">
            {form.id != null ? "编辑账号" : "新增账号"}
          </span>
        </div>
        <div className="zap-form">
          <label>
            标题 *
            <input
              autoFocus
              value={form.title}
              onChange={(e) => setFormField("title", e.target.value)}
              placeholder="例如：公司邮箱"
            />
          </label>
          <label>
            用户名
            <input
              value={form.username}
              onChange={(e) => setFormField("username", e.target.value)}
            />
          </label>
          <label>
            密码（DPAPI 加密存储，不入剪贴板历史
            {form.id != null ? "；留空 = 保持原密码" : ""}）
            <div className="zap-pass-row">
              <input
                type="password"
                value={form.password}
                onChange={(e) => setFormField("password", e.target.value)}
              />
              <button
                type="button"
                className="zap-link-btn"
                onClick={genPassword}
                title="生成 16 位强密码"
              >
                生成
              </button>
            </div>
          </label>
          <label>
            网址
            <input
              value={form.url}
              onChange={(e) => setFormField("url", e.target.value)}
              placeholder="https://…"
            />
          </label>
          <label>
            备注
            <textarea
              rows={2}
              value={form.notes}
              onChange={(e) => setFormField("notes", e.target.value)}
            />
          </label>
          <label className="zap-set-check">
            <input
              type="checkbox"
              checked={form.pinned}
              onChange={(e) =>
                setForm((f) => (f ? { ...f, pinned: e.target.checked } : f))
              }
            />
            固定置顶（常用账号排在最前）
          </label>
          <div className="zap-form-actions">
            <button onClick={() => setForm(null)}>取消（Esc）</button>
            <button className="primary" onClick={() => saveForm()}>
              保存（Ctrl+Enter）
            </button>
          </div>
        </div>
      </div>
    );
  }

  // ---------- 查看密码 ----------
  if (revealPw) {
    return (
      <div className="zap-panel vault" onKeyDown={onKeyDown}>
        <div className="zap-input-row">
          <span className="zap-logo">🔒</span>
          <span className="zap-vault-tag">查看密码</span>
          <button className="zap-link-btn" onClick={() => setRevealPw(null)}>
            返回（Esc）
          </button>
        </div>
        <div className="zap-form">
          <label>
            {revealPw.title}
            <input
              autoFocus
              readOnly
              className="zap-pw-reveal"
              value={revealPw.password}
              onFocus={(e) => e.currentTarget.select()}
            />
          </label>
          <div className="zap-vault-tag">
            可框选复制 · 关闭密码请用 Alt+2（阅后即焚，不入剪贴板历史）
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
          <span className="zap-logo">🔒</span>
          <span className="zap-vault-tag">智能导入</span>
          <button className="zap-link-btn" onClick={closeImport}>
            返回（Esc）
          </button>
        </div>
        <div className="zap-form">
          {parsed === null ? (
            <>
              <textarea
                autoFocus
                rows={7}
                value={importText}
                onChange={(e) => setImportText(e.target.value)}
                placeholder={"粘贴记事本 / 聊天记录里的账号文本，例如：\n接入地址\nhttps://…\n账号\n用户名\n密码\n密码"}
              />
              <button
                className="primary"
                onClick={async () => {
                  const res = await invoke<ParsedAccount[]>("vault_parse", {
                    text: importText,
                  }).catch(() => []);
                  setParsed(res);
                }}
              >
                解析
              </button>
            </>
          ) : (
            <>
              {parsed.length === 0 && (
                <div className="zap-vault-tag">
                  未识别到账号，请检查文本后返回重试
                </div>
              )}
              {parsed.map((p, i) => (
                <div key={i} className="zap-import-row">
                  <input
                    value={p.title}
                    placeholder="标题"
                    onChange={(e) => updParsed(i, "title", e.target.value)}
                  />
                  <div className="zap-import-grid">
                    <input
                      value={p.username}
                      placeholder="用户名"
                      onChange={(e) => updParsed(i, "username", e.target.value)}
                    />
                    <input
                      value={p.password}
                      placeholder="密码"
                      onChange={(e) => updParsed(i, "password", e.target.value)}
                    />
                  </div>
                  <input
                    value={p.url}
                    placeholder="网址"
                    onChange={(e) => updParsed(i, "url", e.target.value)}
                  />
                </div>
              ))}
              {parsed.length > 0 && (
                <button
                  className="primary"
                  onClick={async () => {
                    const n = await invoke<number>("vault_import", {
                      entries: parsed.filter(
                        (p) => p.title.trim() || p.username || p.password
                      ),
                    }).catch(() => -1);
                    if (n >= 0) {
                      closeImport();
                      await refreshVault(vaultTermRef.current);
                    }
                  }}
                >
                  导入全部（{parsed.length} 条）
                </button>
              )}
            </>
          )}
        </div>
      </div>
    );
  }

  // ---------- 设置页 ----------
  if (settingsOpen && st) {
    return (
      <div className="zap-panel vault" onKeyDown={onKeyDown}>
        <div className="zap-input-row">
          <span className="zap-logo">⚙</span>
          <span className="zap-vault-tag">设置</span>
          <button className="zap-link-btn" onClick={closeSettings}>
            返回（Esc）
          </button>
        </div>
        <div className="zap-form">
          <label>
            呼出热键（点击输入框后按下新组合，Esc 取消）
            {hkRecording ? (
              <input readOnly className="zap-hk-rec" value="按下新组合…（Esc 取消）" />
            ) : (
              <input
                readOnly
                className="zap-hk"
                value={st.hotkey}
                onClick={() => {
                  invoke("start_hotkey_capture")
                    .then(() => setHkRecording(true))
                    .catch((e) => setHkError("启动录制失败：" + e));
                }}
              />
            )}
          </label>
          {hkError && <div className="zap-hk-err">{hkError}</div>}
          <label className="zap-set-check">
            <input
              type="checkbox"
              checked={st.clipEnabled}
              onChange={(e) =>
                setSt((s) => (s ? { ...s, clipEnabled: e.target.checked } : s))
              }
            />
            记录剪贴板历史
          </label>
          <label>
            历史保留条数
            <select
              value={st.clipMax}
              disabled={!st.clipEnabled}
              onChange={(e) =>
                setSt((s) => (s ? { ...s, clipMax: Number(e.target.value) } : s))
              }
            >
              <option value={100}>100</option>
              <option value={500}>500</option>
              <option value={1000}>1000</option>
              <option value={5000}>5000</option>
            </select>
          </label>
          <label>
            密码复制后自动清空剪贴板
            <select
              value={st.pwdClearSecs}
              onChange={(e) =>
                setSt((s) => (s ? { ...s, pwdClearSecs: Number(e.target.value) } : s))
              }
            >
              <option value={0}>关闭</option>
              <option value={30}>30 秒</option>
              <option value={45}>45 秒</option>
              <option value={60}>60 秒</option>
            </select>
          </label>
          <label className="zap-set-check">
            <input
              type="checkbox"
              checked={st.autostart}
              onChange={async (e) => {
                const v = e.target.checked;
                setSt((s) => (s ? { ...s, autostart: v } : s));
                try {
                  await invoke("autostart_set", { enable: v });
                } catch {
                  setSt((s) => (s ? { ...s, autostart: !v } : s));
                }
              }}
            />
            开机自启
          </label>
          <div className="zap-form-actions">
            <button onClick={closeSettings}>取消（Esc）</button>
            <button className="primary" onClick={saveSettings}>
              保存
            </button>
          </div>
          {saveMsg && <div className="zap-set-saved">{saveMsg}</div>}
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
              ? "搜索账号…（Enter 用户名 · Alt+1 网址 · Alt+2 密码 · Alt+3 删除 · Alt+4 查看 · Alt+5 置顶 · Alt+6 编辑）"
              : "搜索应用、文件、剪贴板…（pw 进账号库 · Esc 关闭）"
          }
          spellCheck={false}
        />
        <button className="zap-gear" title="设置（Ctrl+,）" onClick={openSettings}>
          <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
            <path d="M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58a.49.49 0 0 0 .12-.61l-1.92-3.32a.49.49 0 0 0-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54a.48.48 0 0 0-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96a.49.49 0 0 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58a.49.49 0 0 0-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32a.49.49 0 0 0-.12-.61l-2.01-1.58zM12 15.6A3.6 3.6 0 1 1 12 8.4a3.6 3.6 0 0 1 0 7.2z" />
          </svg>
        </button>
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
                setForm({ ...EMPTY_FORM });
                return;
              }
              if (it.id === "vault:import") {
                setImporting(true);
                return;
              }
              if (it.kind === "info") return;
              if (isVault) {
                if (it.kind === "vault") vaultAction(it, "username");
                return;
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
              <span className={"zap-badge badge-" + it.kind}>
                {KIND_LABEL[it.kind] ?? it.kind}
              </span>
            )}
            <div className="zap-texts">
              <div className="zap-title">{it.title}</div>
              <div className="zap-sub">{it.subtitle}</div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

export default App;
