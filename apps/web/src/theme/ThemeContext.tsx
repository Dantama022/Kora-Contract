// Theme provider: system preference detection + persisted manual override (Closes #787).
// No flash of wrong theme: inline script in index.html should set data-theme pre-hydration.
import React, { createContext, useContext, useEffect, useMemo, useState } from 'react';
import { ThemeMode, themes, ThemeTokens } from './tokens';

const STORAGE_KEY = 'kora-theme';
const ThemeCtx = createContext<{ mode: ThemeMode; tokens: ThemeTokens; setMode: (m: ThemeMode | 'system') => void }>({ mode: 'light', tokens: themes.light, setMode: () => {} });

export function resolveSystemMode(): ThemeMode {
  if (typeof window !== 'undefined' && window.matchMedia?.('(prefers-color-scheme: dark)').matches) return 'dark';
  return 'light';
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [override, setOverride] = useState<ThemeMode | 'system'>(() =>
    typeof localStorage !== 'undefined' ? (localStorage.getItem(STORAGE_KEY) as ThemeMode | 'system') || 'system' : 'system');
  const [system, setSystem] = useState<ThemeMode>(() => resolveSystemMode());
  useEffect(() => {
    const mq = window.matchMedia?.('(prefers-color-scheme: dark)');
    const fn = (e: MediaQueryListEvent) => setSystem(e.matches ? 'dark' : 'light');
    mq?.addEventListener?.('change', fn);
    return () => mq?.removeEventListener?.('change', fn);
  }, []);
  const mode: ThemeMode = override === 'system' ? system : override;
  useEffect(() => {
    document.documentElement.dataset.theme = mode;
    document.documentElement.style.colorScheme = mode;
  }, [mode]);
  const value = useMemo(() => ({
    mode, tokens: themes[mode],
    setMode: (m: ThemeMode | 'system') => { localStorage.setItem(STORAGE_KEY, m); setOverride(m); },
  }), [mode, override]);
  return <ThemeCtx.Provider value={value}>{children}</ThemeCtx.Provider>;
}

export const useTheme = () => useContext(ThemeCtx);
