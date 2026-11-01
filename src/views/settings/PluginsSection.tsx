/// 设置页「插件」分区（自 SettingsView 拆出，审计 E3）：可开关插件清单 +
/// 入口词/冲突展示
import type { Dispatch, SetStateAction } from "react";
import { Puzzle } from "lucide-react";
import { Kbd } from "@/components/ui/kbd";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Section } from "./Section";
import type { AppSettings } from "./types";

interface Props {
  st: AppSettings;
  setSt: Dispatch<SetStateAction<AppSettings | null>>;
}

export function PluginsSection({ st, setSt }: Props) {
  return (
    <Section icon={Puzzle} title="插件">
      {st.plugins.length === 0 ? (
        <div className="py-2 text-[11px] text-muted-foreground">
          暂无可开关插件
        </div>
      ) : (
        st.plugins.map((p) => (
          <div
            key={p.id}
            className="flex min-h-[42px] items-center justify-between gap-3"
          >
            <span className="flex min-w-0 flex-col gap-px">
              <Label
                htmlFor={"set-plugin-" + p.id}
                className="text-[13px] font-normal text-foreground"
              >
                {p.title}
              </Label>
              <span className="text-[11px] leading-snug text-muted-foreground">
                {p.hint}
              </span>
              {(p.entries?.length ?? 0) > 0 && (
                <span className="flex flex-wrap items-center gap-1 pt-0.5">
                  <span className="text-[11px] text-muted-foreground">
                    入口
                  </span>
                  {(p.entries ?? []).map((w) => (
                    <Kbd key={w}>{w}</Kbd>
                  ))}
                </span>
              )}
              {(p.conflicts?.length ?? 0) > 0 && (
                <span className="pt-0.5 text-[11px] leading-snug text-destructive">
                  入口词冲突：{(p.conflicts ?? []).join("、")}
                  {" · 与其他插件或搜索捷径重合，搜索时会同时出多条目"}
                </span>
              )}
            </span>
            <Checkbox
              id={"set-plugin-" + p.id}
              checked={p.enabled}
              onCheckedChange={(v) =>
                setSt((s) =>
                  s
                    ? {
                        ...s,
                        plugins: s.plugins.map((x) =>
                          x.id === p.id ? { ...x, enabled: v === true } : x
                        ),
                      }
                    : s
                )
              }
            />
          </div>
        ))
      )}
    </Section>
  );
}
