// Contextual guided tour for SME invoice submission + investor funding (Closes #790).
// Anchor-based steps skip/reposition gracefully; dismiss persisted per user;
// never intercepts underlying wallet-signature prompts (pointer-events passthrough).
import React, { createContext, useContext, useCallback, useEffect, useMemo, useState } from 'react';

export interface TourStep { id: string; target: string; title: string; body: string; }
export type TourId = 'sme-submit' | 'investor-fund';

export const TOURS: Record<TourId, TourStep[]> = {
  'sme-submit': [
    { id: 'upload', target: '[data-tour="invoice-upload"]', title: 'Submit an invoice', body: 'Upload your invoice document here.' },
    { id: 'terms', target: '[data-tour="invoice-terms"]', title: 'Set terms', body: 'Review amount, due date and disclosures.' },
    { id: 'sign', target: '[data-tour="invoice-sign"]', title: 'Sign & submit', body: 'Sign with your wallet to list the invoice.' },
  ],
  'investor-fund': [
    { id: 'browse', target: '[data-tour="listing-browse"]', title: 'Browse listings', body: 'Pick a listed invoice to fund.' },
    { id: 'amount', target: '[data-tour="fund-amount"]', title: 'Enter amount', body: 'Choose how much to contribute.' },
    { id: 'confirm', target: '[data-tour="fund-confirm"]', title: 'Confirm funding', body: 'Sign to fund; tour never blocks the wallet prompt.' },
  ],
};

const key = (tour: TourId) => `kora-tour-${tour}`;
export const isTourDone = (tour: TourId): boolean => { try { return localStorage.getItem(key(tour)) === 'done'; } catch { return true; } };

const TourCtx = createContext<{ start: (t: TourId) => void; dismiss: () => void; step: TourStep | null; index: number; next: () => void; active: TourId | null }>({ start: () => {}, dismiss: () => {}, step: null, index: 0, next: () => {}, active: null });

export function TourProvider({ children }: { children: React.ReactNode }) {
  const [active, setActive] = useState<TourId | null>(null);
  const [index, setIndex] = useState(0);
  const steps = useMemo(() => (active ? TOURS[active] : []), [active]);
  // Skip steps whose anchor is missing (responsive layouts).
  const step = useMemo(() => {
    if (!active) return null;
    for (let i = index; i < steps.length; i++) {
      if (typeof document === 'undefined' || document.querySelector(steps[i].target)) return steps[i];
    }
    return null;
  }, [active, steps, index]);
  const dismiss = useCallback(() => {
    if (active) { try { localStorage.setItem(key(active), 'done'); } catch {} }
    setActive(null); setIndex(0);
  }, [active]);
  const next = useCallback(() => {
    if (index + 1 >= steps.length) dismiss();
    else setIndex(index + 1);
  }, [index, steps.length, dismiss]);
  const start = useCallback((t: TourId) => { if (!isTourDone(t)) { setActive(t); setIndex(0); } }, []);
  useEffect(() => {
    if (active && step === null) dismiss(); // all anchors missing -> auto-dismiss
  }, [active, step, dismiss]);
  return <TourCtx.Provider value={{ start, dismiss, step, index, next, active }}>{children}</TourCtx.Provider>;
}

export const useTour = () => useContext(TourCtx);
