import React from "react";
import { VaultDetailView } from "@/plugins/vault/VaultDetailView";
import { StageView } from "@/plugins/stage/StageView";
import { ClipImagePreview } from "@/plugins/clipimg/ClipImagePreview";
import { VaultFormView } from "@/plugins/vault/VaultFormView";
import {
  VaultRevealOverlay,
} from "@/plugins/vault/VaultRevealOverlay";
import { VaultImportView } from "@/plugins/vault/VaultImportView";
import { JsonWorkbench } from "@/plugins/json/JsonWorkbench";
import { ToolWorkbench } from "@/plugins/text/ToolWorkbench";
import { PortsWorkbench } from "@/plugins/ports/PortsWorkbench";
import { SettingsView } from "@/views/SettingsView";
import type { View } from "@/views/appTypes";

type VaultApi = ReturnType<
  typeof import("@/plugins/vault/useVaultActions").useVaultActions
>;
type UpdaterApi = ReturnType<typeof import("@/views/useUpdater").useUpdater>;

export interface SubViewProps {
  view: View;
  vault: VaultApi;
  updater: UpdaterApi;
  focusRef: (el: HTMLDivElement | null) => void;
  setForm: React.ComponentProps<typeof VaultFormView>["setForm"];
  closeImport: () => void;
  closeSettings: () => void;
  closeStage: () => void;
  closeWorkbench: (backQuery: string) => void;
  repullClips: () => void;
}

/// 子视图状态机分发（P3.5 第一步，纯渲染搬迁）。
/// 这是普通函数而非组件：返回的 element 与原来 AppView 的提前 return 同构，
/// 不新增 fiber 层级，focusPanelRef / Esc 栈 / 键盘面全部留在 AppView 内。
/// 解构别名让分支体与搬迁前逐字一致，便于等价核对。
export function renderSubViews(p: SubViewProps): React.ReactElement | null {
  const {
    view,
    vault,
    updater,
    focusRef: focusPanelRef,
    setForm: updateFormData,
    closeImport,
    closeSettings,
    closeStage,
    closeWorkbench,
    repullClips,
  } = p;

  if (view.kind === "form") {
    return (
      <VaultFormView
        form={view.data}
        setForm={updateFormData}
        onSaved={vault.formSaved}
        onCancel={vault.cancelForm}
        focusRef={focusPanelRef}
      />
    );
  }

  if (view.kind === "reveal") {
    return (
      <VaultRevealOverlay
        reveal={view.data}
        // 复用 closeDetail（稳定引用，2026-11-04 评审 R9）：内联箭头每次渲染
        // 新建会让浮层 30s 自动关闭计时器随父级渲染无限重置——安全语义靠
        // 巧合成立；closeDetail 同样回主列表并归还输入焦点
        onClose={vault.closeDetail}
        focusRef={focusPanelRef}
      />
    );
  }

  if (view.kind === "import") {
    return (
      <VaultImportView
        onDone={vault.importDone}
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
        onClose={vault.closeDetail}
        onDelete={vault.deleteCurrent}
        onCopyField={vault.copyField}
        onEdit={vault.editCurrent}
        onPin={() => void vault.pinCurrent()}
        onOpenUrl={() =>
          void vault.vaultAction(
            {
              id: `vault:${view.data.id}`,
              title: view.data.title,
              subtitle: "",
              kind: "vault",
              target: String(view.data.id),
              score: 0,
            },
            "url"
          )
        }
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

  if (view.kind === "port") {
    return (
      <PortsWorkbench
        initialPort={view.port}
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

  return null;
}
