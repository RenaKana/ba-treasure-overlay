import type { TFunction } from 'i18next';
import { desktopMessageCatalog } from './desktopMessages.ts';

const messageTemplates: readonly [RegExp, string, readonly string[]][] = [
  [/^有 (\d+) 格无法从真实像素确认类型，请手动校正$/, '有 {{count}} 格无法从真实像素确认类型，请手动校正', ['count']],
  [/^已观察 (\d+) 格未翻开、(\d+) 格为空$/, '已观察 {{hidden}} 格未翻开、{{empty}} 格为空', ['hidden', 'empty']],
  [/^未读到物品 (.+) 的尺寸$/, '未读到物品 {{items}} 的尺寸', ['items']],
  [/^未读到物品 (.+) 的剩余件数$/, '未读到物品 {{items}} 的剩余件数', ['items']],
  [/^物品 (.+) 的尺寸无法放入 9×5 棋盘（含旋转）$/, '物品 {{items}} 的尺寸无法放入 9×5 棋盘（含旋转）', ['items']],
  [/^物品 (.+) 的剩余件数超出支持范围（0–7 件）$/, '物品 {{items}} 的剩余件数超出支持范围（0–7 件）', ['items']],
];

/** Translate at the view boundary so native state/version and solver decisions stay unchanged. */
export function translateDesktopMessage(message: string, t: TFunction<'desktop'>): string {
  if (message === '') return '';
  if (Object.prototype.hasOwnProperty.call(desktopMessageCatalog, message)) return t(message);

  const nestedWrappers: readonly [RegExp, string][] = [
    [/^手动快照 · (.+)$/, '手动快照 · {{message}}'],
    [/^(.+)；点击重新识别$/, '{{message}}；点击重新识别'],
    [/^操作未保存，请重试：(.+)$/s, '操作未保存，请重试：{{message}}'],
    [/^计算失败：(.+)；可校正或重试$/s, '计算失败：{{message}}；可校正或重试'],
    [/^(.+)，请刷新或校正$/, '{{message}}，请刷新或校正'],
  ];
  for (const [pattern, key] of nestedWrappers) {
    const matched = message.match(pattern);
    if (matched !== null) return t(key, { message: translateDesktopMessage(matched[1], t) });
  }

  const diagnosticWrappers: readonly [RegExp, string][] = [
    [/^WGC 捕获失败：(.+)$/s, 'WGC 捕获失败：{{detail}}'],
    [/^重新建立捕获失败，请重新连接：(.+)$/s, '重新建立捕获失败，请重新连接：{{detail}}'],
  ];
  for (const [pattern, key] of diagnosticWrappers) {
    const matched = message.match(pattern);
    if (matched !== null) return t(key, { detail: matched[1] });
  }

  const parts = message.split('；');
  if (parts.length > 1 && parts.every((part) => Object.prototype.hasOwnProperty.call(desktopMessageCatalog, part) || messageTemplates.some(([pattern]) => pattern.test(part)))) {
    return parts.map((part) => translateDesktopMessage(part, t)).join(t('；'));
  }
  for (const [pattern, key, names] of messageTemplates) {
    const matched = message.match(pattern);
    if (matched === null) continue;
    return t(key, Object.fromEntries(names.map((name, index) => [
      name, name === 'items' ? matched[index + 1].split('、').join(t('、')) : name === 'count' ? Number(matched[index + 1]) : matched[index + 1],
    ])));
  }
  return t('诊断详情：{{detail}}', { detail: message });
}
