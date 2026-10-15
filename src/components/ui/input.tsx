import * as React from "react";

import { cn } from "@/lib/utils";

function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      type={type}
      data-slot="input"
      className={cn(
        "flex h-8 w-full min-w-0 rounded-md border border-input bg-card px-2.5 py-1 text-[13px] text-foreground shadow-none transition-all duration-150 outline-none",
        "placeholder:text-muted-foreground/70 selection:bg-primary/30 selection:text-foreground",
        "focus-visible:border-primary/60 focus-visible:ring-2 focus-visible:ring-ring/35 focus-visible:bg-secondary/60",
        "disabled:cursor-not-allowed disabled:opacity-50",
        "file:inline-flex file:h-6 file:border-0 file:bg-transparent file:text-xs file:font-medium file:text-foreground",
        "aria-invalid:border-destructive aria-invalid:ring-destructive/30",
        className
      )}
      {...props}
    />
  );
}

export { Input };
