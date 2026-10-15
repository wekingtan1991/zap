import * as React from "react";

import { cn } from "@/lib/utils";

function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        "flex min-h-16 w-full rounded-md border border-input bg-card px-2.5 py-2 text-[13px] leading-relaxed text-foreground shadow-none transition-all duration-150 outline-none resize-none",
        "placeholder:text-muted-foreground/70 selection:bg-primary/30 selection:text-foreground",
        "focus-visible:border-primary/60 focus-visible:ring-2 focus-visible:ring-ring/35 focus-visible:bg-secondary/60",
        "disabled:cursor-not-allowed disabled:opacity-50",
        className
      )}
      {...props}
    />
  );
}

export { Textarea };
