import { type VariantProps, cva } from 'class-variance-authority';
import type * as React from 'react';

import { cn } from '@/lib/utils';

const badgeVariants = cva(
  'inline-flex items-center border px-1.5 py-0.5 text-[11px] font-medium transition-colors',
  {
    variants: {
      variant: {
        default: 'border-[var(--border-2)] bg-[var(--surface)] text-[var(--muted)]',
        ok: 'border-[var(--ok-border)] bg-[var(--ok-bg)] text-[var(--ok)]',
        warn: 'border-[var(--warn-border)] bg-[var(--warn-bg)] text-[var(--warn)]',
        err: 'border-[var(--err-border)] bg-[var(--err-bg)] text-[var(--err)]',
        accent: 'border-[var(--accent-border)] bg-[var(--accent-bg)] text-[var(--accent)]',
        purple: 'border-[var(--purple-border)] bg-[var(--purple-bg)] text-[var(--purple)]',
      },
    },
    defaultVariants: {
      variant: 'default',
    },
  },
);

export interface BadgeProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, ...props }: BadgeProps) {
  return <div className={cn(badgeVariants({ variant }), className)} {...props} />;
}

export { Badge, badgeVariants };
