import React from "react";
import { cn } from "../../lib/utils";

interface CardProps extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode;
}

export const Card = React.memo(
  React.forwardRef<HTMLDivElement, CardProps>(
    ({ className, children, ...props }, ref) => {
      return (
        <div
          ref={ref}
          className={cn(
            "bg-zinc-900/90 border border-zinc-800/80 rounded-2xl p-5 shadow-lg transition-colors duration-150 hover:border-zinc-700/80",
            className
          )}
          {...props}
        >
          {children}
        </div>
      );
    }
  )
);
Card.displayName = "Card";
