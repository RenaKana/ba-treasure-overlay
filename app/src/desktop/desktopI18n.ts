import { useEffect } from 'react';
import { createInstance } from 'i18next';
import { useTranslation } from 'react-i18next';
import { isTauri } from '@tauri-apps/api/core';
import { emit, listen, type UnlistenFn } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  DESKTOP_LOCALES, DESKTOP_LOCALE_EVENT, DESKTOP_LOCALE_STORAGE_KEY,
  isDesktopLocale, resolveDesktopLocale, type DesktopLocale,
} from './desktopLocale.ts';
import { desktopResources } from './desktopMessages.ts';

export { DESKTOP_LOCALES, DESKTOP_LOCALE_STORAGE_KEY, normalizeDesktopLocale, resolveDesktopLocale, type DesktopLocale } from './desktopLocale.ts';
export { translateDesktopMessage } from './desktopNativeMessages.ts';

function initialLocale(): DesktopLocale {
  let stored: string | null = null;
  try { stored = localStorage.getItem(DESKTOP_LOCALE_STORAGE_KEY); } catch { /* Session-only preference. */ }
  const languages = typeof navigator === 'undefined' ? [] : navigator.languages.length > 0 ? navigator.languages : [navigator.language];
  return resolveDesktopLocale(stored, languages);
}

// Explicit instance keeps the upstream web app's translations and detector independent.
export const desktopI18n = createInstance();
void desktopI18n.init({
  lng: initialLocale(),
  fallbackLng: 'zh-CN',
  supportedLngs: DESKTOP_LOCALES.map(({ code }) => code),
  defaultNS: 'desktop',
  ns: ['desktop'],
  resources: desktopResources,
  initImmediate: false,
  keySeparator: false,
  interpolation: { escapeValue: false },
  react: { useSuspense: false },
});

function applyLocale(locale: DesktopLocale, persist: boolean) {
  if (persist) {
    try { localStorage.setItem(DESKTOP_LOCALE_STORAGE_KEY, locale); } catch { /* Works for this session. */ }
  }
  if (desktopI18n.language !== locale) void desktopI18n.changeLanguage(locale);
}

export function setDesktopLocale(locale: DesktopLocale) {
  if (!isDesktopLocale(locale)) return;
  applyLocale(locale, true);
  if (isTauri()) void emit(DESKTOP_LOCALE_EVENT, { locale }).catch((cause) => console.warn('Desktop locale synchronization failed', cause));
}

let subscribers = 0;
let stopSynchronization: (() => void) | undefined;
export function subscribeToDesktopLocale() {
  subscribers += 1;
  if (subscribers === 1) {
    // Reconcile preferences changed between module initialization and view mounting.
    applyLocale(initialLocale(), false);
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    const onStorage = (event: StorageEvent) => {
      if (event.key !== DESKTOP_LOCALE_STORAGE_KEY) return;
      const languages = navigator.languages.length > 0 ? navigator.languages : [navigator.language];
      applyLocale(resolveDesktopLocale(event.newValue, languages), false);
    };
    window.addEventListener('storage', onStorage);
    if (isTauri()) {
      void listen<{ locale: unknown }>(DESKTOP_LOCALE_EVENT, ({ payload }) => {
        if (!disposed && isDesktopLocale(payload?.locale)) applyLocale(payload.locale, true);
      }).then((off) => { if (disposed) off(); else unlisten = off; })
        .catch((cause) => console.warn('Desktop locale listener failed', cause));
    }
    stopSynchronization = () => {
      disposed = true;
      window.removeEventListener('storage', onStorage);
      unlisten?.();
    };
  }
  return () => {
    subscribers -= 1;
    if (subscribers === 0) { stopSynchronization?.(); stopSynchronization = undefined; }
  };
}

let lastNativeTitle = '';
export function useDesktopI18n() {
  const { t, i18n } = useTranslation('desktop', { i18n: desktopI18n });
  const locale = isDesktopLocale(i18n.language) ? i18n.language : 'zh-CN';
  useEffect(subscribeToDesktopLocale, []);
  useEffect(() => {
    document.documentElement.lang = locale;
    const title = `BA ${t('寻宝助手')}`;
    document.title = title;
    if (isTauri() && lastNativeTitle !== title) {
      lastNativeTitle = title;
      void getCurrentWindow().setTitle(title).catch((cause) => console.warn('Desktop title update failed', cause));
    }
  }, [locale, t]);
  return { t, locale, setLocale: setDesktopLocale };
}
