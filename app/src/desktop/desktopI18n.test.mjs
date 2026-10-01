import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import upstreamI18n, { createInstance } from 'i18next';
import ts from 'typescript';
import { DESKTOP_LOCALES, DESKTOP_LOCALE_EVENT, DESKTOP_LOCALE_STORAGE_KEY, normalizeDesktopLocale, resolveDesktopLocale } from './desktopLocale.ts';
import { desktopMessageCatalog, desktopResources } from './desktopMessages.ts';
import { translateDesktopMessage } from './desktopNativeMessages.ts';

function translator(locale) {
  const i18n = createInstance();
  void i18n.init({
    lng: locale, fallbackLng: 'zh-CN', resources: desktopResources,
    defaultNS: 'desktop', initImmediate: false, keySeparator: false,
    interpolation: { escapeValue: false },
  });
  return i18n.getFixedT(locale, 'desktop');
}

test('browser language tags map Chinese scripts and regions without changing OCR settings', () => {
  for (const tag of ['zh-Hant', 'zh-Hant-TW', 'zh-Hant-CN', 'zh-TW', 'zh-HK', 'zh-MO', 'ZH_hant_HK']) assert.equal(normalizeDesktopLocale(tag), 'zh-TW', tag);
  for (const tag of ['zh', 'zh-CN', 'zh-Hans', 'zh-SG', 'zh-Hans-CN', 'zh-Hans-HK']) assert.equal(normalizeDesktopLocale(tag), 'zh-CN', tag);
  for (const tag of ['ja', 'ja-JP']) assert.equal(normalizeDesktopLocale(tag), 'ja', tag);
  for (const tag of ['en', 'en-US', 'en-GB']) assert.equal(normalizeDesktopLocale(tag), 'en', tag);
  for (const tag of [null, undefined, '', 'fr', 'ko-KR', 'english', 'zhfoo']) assert.equal(normalizeDesktopLocale(tag), null, String(tag));
});

test('manual preference wins; unsupported system language defaults to Simplified Chinese', () => {
  assert.equal(resolveDesktopLocale('ja', ['zh-TW']), 'ja');
  assert.equal(resolveDesktopLocale('zh-TW', ['en-US']), 'zh-TW');
  assert.equal(resolveDesktopLocale(null, ['zh-HK']), 'zh-TW');
  assert.equal(resolveDesktopLocale('invalid', ['ja-JP']), 'ja');
  assert.equal(resolveDesktopLocale(null, ['fr-FR', 'en-US']), 'zh-CN');
  assert.equal(resolveDesktopLocale(null, []), 'zh-CN');
});

test('all four catalogs contain every message and preserve interpolation parameters', () => {
  const placeholders = (value) => [...value.matchAll(/\{\{(\w+)\}\}/g)].map((match) => match[1]).sort();
  const keys = Object.keys(desktopMessageCatalog).sort();
  assert.deepEqual(Object.keys(desktopResources).sort(), DESKTOP_LOCALES.map(({ code }) => code).sort());
  for (const { code } of DESKTOP_LOCALES) {
    assert.deepEqual(Object.keys(desktopResources[code].desktop).sort(), keys, code);
    for (const [key, value] of Object.entries(desktopResources[code].desktop)) {
      assert.ok(value.trim().length > 0, `${code}: ${key}`);
      assert.deepEqual(placeholders(value), placeholders(key), `${code}: ${key}`);
    }
  }
  assert.equal(translator('zh-CN')('寻宝助手'), '寻宝助手');
  assert.equal(translator('zh-TW')('寻宝助手'), '尋寶助手');
  assert.equal(translator('ja')('寻宝助手'), '宝探しアシスタント');
  assert.equal(translator('en')('寻宝助手'), 'Treasure Helper');
});

test('native statuses and nested validation errors translate at the display boundary', () => {
  const t = translator('en');
  const message = '手动快照 · 已观察 39 格未翻开、2 格为空';
  assert.equal(translateDesktopMessage(message, t), 'Manual snapshot · Observed 39 unopened cells and 2 empty cells');
  assert.equal(message, '手动快照 · 已观察 39 格未翻开、2 格为空');
  assert.equal(translateDesktopMessage('有 3 格无法从真实像素确认类型，请手动校正', t), 'Could not identify 3 cells from pixels; correct them manually');
  assert.equal(translateDesktopMessage('未读到物品 2、3 的剩余件数，请刷新或校正', t), 'Could not read remaining counts for items 2, 3; refresh or correct settings');
  assert.equal(translateDesktopMessage('未读到物品 2 的尺寸；物品总占格数超过 45 格，请刷新或校正', t), 'Could not read sizes for items 2; Total item area exceeds 45 cells; refresh or correct settings');
  assert.equal(translateDesktopMessage('操作未保存，请重试：棋盘已变化，请核对当前画面后重试', t), 'Changes were not saved; retry: Board changed; check the current image and retry');
  assert.equal(translateDesktopMessage('计算失败：计算超时，请点击刷新重试；可校正或重试', t), 'Calculation failed: Calculation timed out; refresh to retry; correct settings or retry');
  assert.equal(translateDesktopMessage('棋盘已同步；点击重新识别', t), 'Board synchronized; click to recognize again');
});

test('native and unknown OS diagnostics retain their original details', () => {
  const detail = 'Access is denied. (0x80070005) / 系统原始诊断；未知路径\nRaw diagnostic line 2';
  for (const { code } of DESKTOP_LOCALES) {
    const t = translator(code);
    assert.ok(translateDesktopMessage(`WGC 捕获失败：${detail}`, t).includes(detail), code);
    assert.ok(translateDesktopMessage(`重新建立捕获失败，请重新连接：${detail}`, t).includes(detail), code);
    assert.ok(translateDesktopMessage(detail, t).includes(detail), code);
    assert.equal(translateDesktopMessage('', t), '');
  }
  assert.equal(translateDesktopMessage(detail, translator('en')), `Diagnostic details: ${detail}`);
});

test('control, overlay and refresh labels all use the desktop catalog', () => {
  for (const filename of ['DesktopApp.tsx', 'ProbabilityView.tsx', 'DesktopZoom.tsx', 'CoverReferencePicker.tsx']) {
    const source = readFileSync(new URL(filename, import.meta.url), 'utf8');
    const tree = ts.createSourceFile(filename, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    function visit(node) {
      if (ts.isJsxText(node)) assert.ok(!/\p{Script=Han}/u.test(node.text), `${filename}: untranslated JSX text ${node.text.trim()}`);
      if (ts.isJsxAttribute(node) && ['title', 'aria-label', 'alt', 'placeholder'].includes(node.name.getText(tree)) && node.initializer && ts.isStringLiteral(node.initializer)) {
        assert.ok(!/\p{Script=Han}/u.test(node.initializer.text), `${filename}: untranslated ${node.name.getText(tree)}`);
      }
      if (ts.isStringLiteral(node) && /\p{Script=Han}/u.test(node.text)) {
        const parent = node.parent;
        const diagnostic = ts.isCallExpression(parent) && parent.expression.getText(tree).startsWith('console.');
        if (!diagnostic) assert.ok(Object.hasOwn(desktopMessageCatalog, node.text), `${filename}: missing catalog message ${node.text}`);
      }
      ts.forEachChild(node, visit);
    }
    visit(tree);
  }
});

test('manual preference persists, storage/native events synchronize views, and upstream i18n stays independent', async () => {
  const descriptors = Object.fromEntries(['window', 'localStorage', 'navigator', 'isTauri'].map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const storage = new Map();
  const fakeWindow = new EventTarget();
  const callbacks = new Map();
  const listeners = new Map();
  const nativeCommands = [];
  let nextId = 1;
  fakeWindow.__TAURI_INTERNALS__ = {
    transformCallback(callback) { const id = nextId++; callbacks.set(id, callback); return id; },
    async invoke(command, args) {
      nativeCommands.push({ command, args });
      if (command === 'plugin:event|listen') { const id = nextId++; listeners.set(id, args); return id; }
      if (command === 'plugin:event|emit') {
        for (const [id, listener] of listeners) if (listener.event === args.event) callbacks.get(listener.handler)({ event: args.event, id, payload: args.payload });
      }
      if (command === 'plugin:event|unlisten') listeners.delete(args.eventId);
    },
  };
  fakeWindow.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  Object.defineProperties(globalThis, {
    window: { configurable: true, value: fakeWindow },
    localStorage: { configurable: true, value: { getItem: (key) => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) } },
    navigator: { configurable: true, value: { languages: ['ja-JP'], language: 'ja-JP' } },
    isTauri: { configurable: true, writable: true, value: false },
  });
  let stop;
  try {
    const { desktopI18n, setDesktopLocale, subscribeToDesktopLocale } = await import('./desktopI18n.ts');
    const upstreamLanguage = upstreamI18n.language;
    assert.notEqual(desktopI18n, upstreamI18n);
    assert.equal(desktopI18n.language, 'ja');
    setDesktopLocale('en');
    assert.equal(desktopI18n.language, 'en');
    assert.equal(storage.get(DESKTOP_LOCALE_STORAGE_KEY), 'en');
    setDesktopLocale('fr');
    assert.equal(desktopI18n.language, 'en');
    globalThis.isTauri = true;
    stop = subscribeToDesktopLocale();
    await new Promise(setImmediate);
    const storageEvent = Object.assign(new Event('storage'), { key: DESKTOP_LOCALE_STORAGE_KEY, newValue: 'zh-TW' });
    fakeWindow.dispatchEvent(storageEvent);
    assert.equal(desktopI18n.language, 'zh-TW');
    const [eventId, listener] = [...listeners][0];
    assert.equal(listener.event, DESKTOP_LOCALE_EVENT);
    callbacks.get(listener.handler)({ event: DESKTOP_LOCALE_EVENT, id: eventId, payload: { locale: 'ja' } });
    assert.equal(desktopI18n.language, 'ja');
    assert.equal(storage.get(DESKTOP_LOCALE_STORAGE_KEY), 'ja');
    setDesktopLocale('zh-CN');
    await new Promise(setImmediate);
    assert.equal(desktopI18n.language, 'zh-CN');
    assert.equal(storage.get(DESKTOP_LOCALE_STORAGE_KEY), 'zh-CN');
    assert.equal(nativeCommands.filter(({ command }) => command === 'plugin:event|emit').length, 1, 'received events must not rebroadcast');
    assert.ok(nativeCommands.every(({ command }) => command.startsWith('plugin:event|')), 'language changes must not invoke capture or solver commands');
    assert.equal(upstreamI18n.language, upstreamLanguage);
    stop();
    stop = undefined;
    await new Promise(setImmediate);
    assert.equal(listeners.size, 0);
    fakeWindow.dispatchEvent(Object.assign(new Event('storage'), { key: DESKTOP_LOCALE_STORAGE_KEY, newValue: 'en' }));
    assert.equal(desktopI18n.language, 'zh-CN', 'unsubscribed views do not receive storage changes');
  } finally {
    stop?.();
    await new Promise(setImmediate);
    for (const [key, descriptor] of Object.entries(descriptors)) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete globalThis[key];
    }
  }
});
