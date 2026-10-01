export const DESKTOP_LOCALES = [
  { code: 'zh-CN', label: '简体中文' },
  { code: 'zh-TW', label: '繁體中文' },
  { code: 'ja', label: '日本語' },
  { code: 'en', label: 'English' },
] as const;

export type DesktopLocale = typeof DESKTOP_LOCALES[number]['code'];
export const DESKTOP_LOCALE_STORAGE_KEY = 'ba-desktop-locale';
export const DESKTOP_LOCALE_EVENT = 'desktop-locale-changed';

export function isDesktopLocale(value: unknown): value is DesktopLocale {
  return DESKTOP_LOCALES.some(({ code }) => code === value);
}

/** UI locale only; this does not change native OCR or the game language. */
export function normalizeDesktopLocale(value: string | null | undefined): DesktopLocale | null {
  const tag = value?.trim().replace(/_/g, '-').toLowerCase();
  if (!tag) return null;
  if (tag === 'zh' || tag.startsWith('zh-')) {
    const parts = tag.split('-');
    if (parts.includes('hant')) return 'zh-TW';
    if (parts.includes('hans')) return 'zh-CN';
    if (parts.includes('tw') || parts.includes('hk') || parts.includes('mo')) return 'zh-TW';
    return 'zh-CN';
  }
  if (tag === 'ja' || tag.startsWith('ja-')) return 'ja';
  if (tag === 'en' || tag.startsWith('en-')) return 'en';
  return null;
}

export function resolveDesktopLocale(stored: string | null | undefined, languages: readonly string[] = []): DesktopLocale {
  const preferred = normalizeDesktopLocale(stored);
  if (preferred !== null) return preferred;
  return normalizeDesktopLocale(languages[0]) ?? 'zh-CN';
}
