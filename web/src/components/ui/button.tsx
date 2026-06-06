import { Slot } from '@radix-ui/react-slot';
import { type VariantProps, cva } from 'class-variance-authority';
import * as React from 'react';

import { cn } from '@/lib/utils';

const buttonVariants = cva(
  'inline-flex items-center justify-center gap-2 whitespace-nowrap text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 disabled:pointer-events-none disabled:opacity-50',
  {
    variants: {
      variant: {
        default:
          'bg-[var(--surface)] border border-[var(--border-2)] text-[var(--muted)] hover:text-[var(--text)] hover:border-[var(--muted)]',
        primary:
          'border border-[var(--ok-border)] text-[var(--ok)] bg-[var(--ok-bg)] hover:border-[var(--ok)]',
        accent:
          'border border-[var(--accent-border)] text-[var(--accent)] bg-[var(--accent-bg)] hover:border-[var(--accent)]',
        danger: 'border border-[var(--err-border)] text-[var(--err)] bg-[var(--err-bg)]',
        ghost:
          'border border-[var(--border-2)] text-[var(--muted)] bg-transparent hover:border-[var(--muted)] hover:text-[var(--text)]',
        link: 'text-[var(--accent)] underline-offset-4 hover:underline',
      },
      size: {
        default: 'h-7 px-3 py-1 text-xs',
        sm: 'h-6 px-2 text-xs',
        lg: 'h-8 px-4',
        icon: 'h-7 w-7 p-0',
      },
    },
    defaultVariants: {
      variant: 'default',
      size: 'default',
    },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : 'button';
    return (
      <Comp className={cn(buttonVariants({ variant, size, className }))} ref={ref} {...props} />
    );
  },
);
Button.displayName = 'Button';

export { Button, buttonVariants };
