/**
 * Minimal toast store — allows any screen to trigger the app-shell toast
 * without prop-drilling through the root layout.
 *
 * The root layout reads `message` and clears it after display.
 */

import { create } from 'zustand';

interface ToastState {
  message: string | null;
  show: (msg: string, durationMs?: number) => void;
  clear: () => void;
}

export const useToastStore = create<ToastState>((set) => ({
  message: null,
  show: (msg, durationMs = 4000) => {
    set({ message: msg });
    setTimeout(() => set({ message: null }), durationMs);
  },
  clear: () => set({ message: null }),
}));
