/// 设置页分区壳（审计 E3 拆出，后由分隔线改轻卡片）：zap-set-card 底 + 图标
/// 标题头 + 内容——十二个分区的同构外壳（VaultBackupSection 也走这里），
/// 分区间距由 SettingsView 滚动容器的 gap 提供
import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

export function Section({
  icon: Icon,
  title,
  children,
}: {
  icon: LucideIcon;
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="zap-set-card">
      <div className="mb-0.5 flex items-center gap-1.5 text-[11px] font-semibold tracking-[0.15em] text-muted-foreground/80">
        <Icon className="size-3 text-primary/75" />
        {title}
      </div>
      {children}
    </div>
  );
}
