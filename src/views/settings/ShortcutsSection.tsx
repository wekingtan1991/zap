/// 设置页「搜索捷径」分区（自 SettingsView 拆出，审计 E3）：清单的增删改
/// 排序操作自持（本地编辑、随「保存」由父级 shortcuts_save 整单提交——
/// scList 状态仍在父级，dirty 基线与保存都依赖它）
import type { Dispatch, SetStateAction } from "react";
import { ChevronDown, ChevronUp, Globe, Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Section } from "./Section";
import type { Shortcut } from "./types";

interface Props {
  scList: Shortcut[];
  scErr: string;
  setScList: Dispatch<SetStateAction<Shortcut[]>>;
}

export function ShortcutsSection({ scList, scErr, setScList }: Props) {
  const updateSc = (i: number, patch: Partial<Shortcut>) =>
    setScList((list) => list.map((s, j) => (j === i ? { ...s, ...patch } : s)));
  const moveSc = (i: number, dir: -1 | 1) =>
    setScList((list) => {
      const j = i + dir;
      if (j < 0 || j >= list.length) return list;
      const next = [...list];
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });
  const delSc = (i: number) =>
    setScList((list) => list.filter((_, j) => j !== i));
  const addSc = () =>
    setScList((list) => [...list, { keyword: "", name: "", url: "" }]);

  return (
    <Section icon={Globe} title="搜索捷径">
      <div className="pb-1.5 text-[11px] leading-snug text-muted-foreground">
        主面板输入「关键词 + 空格 + 内容」直达搜索，URL 里用
        <code className="mx-1 rounded bg-muted px-1 font-mono">{"{query}"}</code>
        代表内容，如 gh rust clap
      </div>
      {scList.map((s, i) => (
        <div key={i} className="flex min-h-[42px] items-center gap-1.5">
          <Input
            className="h-[30px] w-[64px] shrink-0 px-2 font-mono text-xs"
            placeholder="关键词"
            spellCheck={false}
            value={s.keyword}
            onChange={(e) => updateSc(i, { keyword: e.target.value })}
          />
          <Input
            className="h-[30px] w-[88px] shrink-0 px-2"
            placeholder="名称"
            value={s.name}
            onChange={(e) => updateSc(i, { name: e.target.value })}
          />
          <Input
            className="h-[30px] min-w-0 flex-1 px-2 font-mono text-xs"
            placeholder="https://…?q={query}"
            spellCheck={false}
            value={s.url}
            onChange={(e) => updateSc(i, { url: e.target.value })}
          />
          <span className="flex shrink-0 items-center">
            <Button
              variant="ghost"
              size="icon"
              className="size-7 text-muted-foreground"
              aria-label="上移"
              disabled={i === 0}
              onClick={() => moveSc(i, -1)}
            >
              <ChevronUp className="size-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="size-7 text-muted-foreground"
              aria-label="下移"
              disabled={i === scList.length - 1}
              onClick={() => moveSc(i, 1)}
            >
              <ChevronDown className="size-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="size-7 text-muted-foreground hover:text-destructive"
              aria-label="删除"
              onClick={() => delSc(i)}
            >
              <X className="size-3.5" />
            </Button>
          </span>
        </div>
      ))}
      <div className="flex min-h-[34px] items-center justify-between gap-3">
        <Button variant="outline" size="sm" onClick={addSc}>
          <Plus className="size-3.5" />
          添加捷径
        </Button>
        {scErr && (
          <span className="animate-shake text-[11px] text-destructive">
            {scErr}
          </span>
        )}
      </div>
    </Section>
  );
}
