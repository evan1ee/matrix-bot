import { useState, useEffect } from 'react';

export interface Toast {
  id: string;
  title: string;
  description?: string;
  type?: 'info' | 'warning' | 'success' | 'error';
  action?: {
    label: string;
    onClick: () => void;
  };
  duration?: number;
}

type ToastListener = (toasts: Toast[]) => void;

let toastsState: Toast[] = [];
const listeners = new Set<ToastListener>();

function notify() {
  listeners.forEach((listener) => listener([...toastsState]));
}

export function showToast(toast: Omit<Toast, 'id'>): string {
  const id = Math.random().toString(36).substring(2, 9);
  const newToast: Toast = { ...toast, id };
  toastsState = [newToast, ...toastsState].slice(0, 5); // at most 5 toasts
  notify();

  const duration = toast.duration ?? (toast.type === 'warning' || toast.type === 'error' ? 8000 : 5000);
  if (duration > 0) {
    setTimeout(() => {
      dismissToast(id);
    }, duration);
  }

  return id;
}

export function dismissToast(id: string) {
  toastsState = toastsState.filter((t) => t.id !== id);
  notify();
}

export function useToasts(): Toast[] {
  const [toasts, setToasts] = useState<Toast[]>(toastsState);

  useEffect(() => {
    listeners.add(setToasts);
    return () => {
      listeners.delete(setToasts);
    };
  }, []);

  return toasts;
}
