import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { Slot } from "@radix-ui/react-slot";

import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center justify-center gap-1 rounded-md border px-1.5 py-0.5 text-[11px] font-medium leading-none w-fit whitespace-nowrap shrink-0 transition-all duration-150 [&>svg]:size-3 [&>svg]:pointer-events-none",
  {
    variants: {
      variant: {
        default: "border-transparent bg-primary/20 text-primary",
        secondary: "border-border bg-secondary text-secondary-foreground",
        destructive:
          "border-destructive/30 bg-destructive/15 text-destructive",
        outline: "border-border text-foreground",
        success:
          "border-transparent bg-success/15 text-success",
        app: "border-transparent bg-[rgba(82,196,26,0.16)] text-[#7ed957] light:text-[#3f8f1f]",
        clip: "border-transparent bg-[rgba(168,85,247,0.18)] text-[#c084fc] light:text-[#7e22ce]",
        win: "border-transparent bg-[rgba(34,211,238,0.16)] text-[#67e8f9] light:text-[#0e7490]",
        recent: "border-transparent bg-[rgba(251,146,60,0.16)] text-[#fdba74] light:text-[#c2410c]",
        file: "border-transparent bg-[rgba(20,184,166,0.16)] text-[#2dd4bf] light:text-[#0f766e]",
        cmd: "border-transparent bg-[rgba(99,102,241,0.18)] text-[#a5b4fc] light:text-[#4338ca]",
        settings: "border-transparent bg-[rgba(244,114,182,0.16)] text-[#f9a8d4] light:text-[#be185d]",
        vault: "border-transparent bg-[rgba(245,158,11,0.16)] text-[#fbbf24] light:text-[#b45309]",
        info: "border-transparent bg-[rgba(148,163,184,0.16)] text-[#cbd5e1] light:text-[#475569]",
        web: "border-transparent bg-[rgba(59,130,246,0.16)] text-[#7ab3ff] light:text-[#1d4ed8]",
        url: "border-transparent bg-[rgba(250,204,21,0.16)] text-[#ffd84d] light:text-[#a16207]",
        json: "border-transparent bg-[rgba(56,189,248,0.16)] text-[#7dd3fc] light:text-[#0369a1]",
        tool: "border-transparent bg-[rgba(163,230,53,0.16)] text-[#bef264] light:text-[#4d7c0f]",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  }
);

function Badge({
  className,
  variant,
  asChild = false,
  ...props
}: React.ComponentProps<"span"> &
  VariantProps<typeof badgeVariants> & { asChild?: boolean }) {
  const Comp = asChild ? Slot : "span";

  return (
    <Comp
      data-slot="badge"
      className={cn(badgeVariants({ variant, className }))}
      {...props}
    />
  );
}

export { Badge, badgeVariants };

/// 徽标语义色全集：插件注册表（src/plugins/registry.tsx）以该类型声明条目配色
export type BadgeVariant = NonNullable<
  VariantProps<typeof badgeVariants>["variant"]
>;
