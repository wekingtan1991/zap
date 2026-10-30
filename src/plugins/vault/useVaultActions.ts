/// 账号库域动作（评审 P1-19，自 App.tsx 抽离）：vault 的数据操作类动作
/// 此前约 300 行栖身 App 根——装配进 actions 对象时依赖数组靠手工对账，
/// 是 stale-closure 温床，且 App 根承载了与启动器核心无关的域逻辑。
/// 本 hook 收拢 refreshVault 与全部读写动作，App 根降为装配层（actions
/// 对象、Alt 键盘面、右键分发、子页面 props 全部引用返回值）。
/// 键盘栈不动（等第二个域出现再抽象，见可行性报告 P1-19）。
import { useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import type { View } from "@/App";
import type { SearchItem } from "@/plugins/types";

/// vault 列表空词时追加的两个伪条目（新增/智能导入），与 App 保持同源
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

interface VaultActionsDeps {
  view: View;
  setView: React.Dispatch<React.SetStateAction<View>>;
  setItems: (items: SearchItem[]) => void;
  setActive: (i: number) => void;
  setQuery: (q: string) => void;
  /// vault 列表的关键词基准（重拉竞态守卫：词已变则丢弃）
  vaultTermRef: { current: string };
  inputRef: { current: HTMLInputElement | null };
}

export function useVaultActions(deps: VaultActionsDeps) {
  const { view, setView, setItems, setActive, setQuery, vaultTermRef, inputRef } = deps;

  /// 按 vault 搜索词重拉列表（vaultAction/pin/delete/form/import 的共用收尾）：
  /// 重拉期间词已变则丢弃，交还给账号库搜索 effect 的正常刷新链路
  const refreshVault = useCallback(
    async (term: string) => {
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
    },
    [vaultTermRef, setItems, setActive]
  );

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
            }).catch((e) => toast.error("打开网址失败：" + String(e)));
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
        // 复制类：短暂显示成功反馈（密码/用户名均不入剪贴板历史）。
        // 收面板与否随「选中即粘贴」设置分道（评审 P2-17 折中）：开启=复制即走
        // 收面板；关闭=用户已选择多步流（先复制后自行粘贴），收面板会打断
        // 连续抄取多个字段——✓ 反馈保留、面板留驻
        const autopaste = await invoke<boolean>("clip_autopaste_enabled").catch(() => true);
        setItems([
          {
            id: "vault-done",
            title: "✓ " + msg,
            subtitle: autopaste ? "稍后自动关闭" : "已复制，可继续抄取",
            kind: "info",
            target: "",
            score: 0,
          },
        ]);
        if (autopaste) {
          setTimeout(() => invoke("hide_panel"), 900);
        }
      } catch (e) {
        // 失败统一 toast（面板保持可见）；复制类成功反馈保留行内 ✓（随后面板收起，toast 会随之消失）
        toast.error("操作失败：" + String(e));
      }
    },
    [refreshVault, vaultTermRef, setItems, setView]
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
    [setView]
  );

  const closeDetail = useCallback(() => {
    setView({ kind: "main" });
    setTimeout(() => inputRef.current?.focus(), 30);
  }, [inputRef, setView]);

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
        toast.error("打开失败：" + String(e));
      }
    },
    [setView]
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
        toast.error("打开失败：" + String(e));
      }
    },
    [setView]
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
        toast.success(msg);
        return true;
      } catch (e) {
        toast.error("复制失败：" + String(e));
        return false;
      }
    },
    []
  );

  const pinCurrent = useCallback(async () => {
    setView((v) => {
      if (v.kind !== "detail") return v;
      void invoke("vault_action", { id: v.data.id, action: "pin" }).catch((e) =>
        toast.error("操作失败：" + String(e))
      );
      return {
        ...v,
        data: { ...v.data, pinned: v.data.pinned === 1 ? 0 : 1 },
      };
    });
    await refreshVault(vaultTermRef.current);
  }, [setView, refreshVault, vaultTermRef]);

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
      toast.error("删除失败：" + String(e));
      return false;
    }
  }, [view, setView, refreshVault, vaultTermRef]);

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
  }, [setView]);

  // 表单保存成功：关表单 + 回账号库列表（空关键词列出全部）
  const formSaved = useCallback(() => {
    setView({ kind: "main" });
    setQuery("pw");
    void refreshVault("");
  }, [setView, setQuery, refreshVault]);

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
  }, [view, setView, inputRef]);

  // 导入收尾：关视图 + 刷新列表（成功反馈由导入视图先展示 700ms）
  const importDone = useCallback(() => {
    setView({ kind: "main" });
    void refreshVault(vaultTermRef.current);
  }, [setView, refreshVault, vaultTermRef]);

  return {
    refreshVault,
    vaultAction,
    startEdit,
    closeDetail,
    openDetail,
    openDeleteConfirm,
    copyField,
    pinCurrent,
    deleteCurrent,
    editCurrent,
    formSaved,
    cancelForm,
    importDone,
  };
}
