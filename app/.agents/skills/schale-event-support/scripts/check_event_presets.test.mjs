import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { afterEach, test } from 'node:test';
import { fileURLToPath } from 'node:url';

const checkerPath = fileURLToPath(
  new URL('./check_event_presets.mjs', import.meta.url),
);
const fixtureDirectories = new Set();

afterEach(() => {
  for (const directory of fixtureDirectories) {
    fs.rmSync(directory, { recursive: true, force: true });
  }
  fixtureDirectories.clear();
});

const range = (max) => Array.from({ length: max + 1 }, (_, index) => index);

const writeFixture = (options = {}) => {
  const directory = fs.mkdtempSync(
    path.join(os.tmpdir(), 'schale-event-presets-'),
  );
  fixtureDirectories.add(directory);

  const presetItemCounts = options.presetItemCounts ?? [3, 3];
  const indices = options.indices ?? [
    [1, 2, 3],
    [1, 2, 3],
  ];
  const counts = options.counts ?? [
    [1, 2, 3],
    [3, 2, 1],
  ];
  const menuValues = options.menuValues ?? [0, 1];
  const extraPresetMenuItems = options.extraPresetMenuItems ?? [];
  const unrelatedMenuValues = options.unrelatedMenuValues ?? [];
  const countMenuValues = range(options.countMenuMax ?? 7);
  const validatorMax = options.validatorMax ?? 7;
  const localeLabelCounts = options.localeLabelCounts ?? {};
  const notificationAlertLengths = options.notificationAlertLengths ?? {};
  const missingUnknownQuantityToastLocales =
    options.missingUnknownQuantityToastLocales ?? [];

  const presets = presetItemCounts
    .map((itemCount, presetIndex) => {
      const items = Array.from({ length: itemCount }, (_, itemIndex) => {
        const index = indices[presetIndex]?.[itemIndex] ?? itemIndex + 1;
        const count = counts[presetIndex]?.[itemIndex] ?? 1;
        return `    { item: { ...supply, index: ${index} }, count: ${count} },`;
      }).join('\n');
      return `  [\n${items}\n  ],`;
    })
    .join('\n');

  const mainArea = `
const supply = { width: 1, height: 1 } as const;
const predefinedItems = [
${presets}
] as const;
const validCount = isIntegerInRange(entry.item.count, 0, ${validatorMax});
`;
  const controlPane = `
const control = (
  <Select id="predefined-choice-select">
${menuValues.map((value) => `    <MenuItem value={${value}}>Round</MenuItem>`).join('\n')}
${extraPresetMenuItems.join('\n')}
  </Select>
);
const unrelatedControl = (
  <Select id="unrelated-select">
${unrelatedMenuValues.map((value) => `    <MenuItem value={${value}}>Other</MenuItem>`).join('\n')}
  </Select>
);
`;
  const itemPane = `
const item = (
  <Select onChange={onCountChange}>
${countMenuValues.map((value) => `    <MenuItem value={${value}}>${value}</MenuItem>`).join('\n')}
  </Select>
);
`;

  const files = {
    'src/components/MainArea.tsx': mainArea,
    'src/components/ControlPane.tsx': controlPane,
    'src/components/ItemPane.tsx': itemPane,
  };

  for (const locale of ['ja', 'en', 'ko', 'zh-CN']) {
    const labelCount = localeLabelCounts[locale] ?? presetItemCounts.length;
    const alertLength = notificationAlertLengths[locale] ?? 3;
    files[`public/locales/${locale}/ControlPane.json`] = JSON.stringify({
      predefined_choice_select: Array.from(
        { length: labelCount },
        (_, index) => `Round ${index + 1}`,
      ),
    });
    files[`public/locales/${locale}/NotificationPanel.json`] = JSON.stringify({
      alert: Array.from({ length: alertLength }, () => 'message'),
    });
    const mainArea = {
      item_preset_applied_toast: 'Applied {{presetLabel}}',
      item_preset_applied_with_unknown_quantity_toast:
        'Applied {{presetLabel}} with unknown quantities',
    };
    if (missingUnknownQuantityToastLocales.includes(locale)) {
      delete mainArea.item_preset_applied_with_unknown_quantity_toast;
    }
    files[`public/locales/${locale}/MainArea.json`] = JSON.stringify(mainArea);
  }

  for (const [relativePath, contents] of Object.entries(files)) {
    const filePath = path.join(directory, relativePath);
    fs.mkdirSync(path.dirname(filePath), { recursive: true });
    fs.writeFileSync(filePath, contents);
  }

  return directory;
};

const runChecker = (options) => {
  const fixture = writeFixture(options);
  return spawnSync(process.execPath, [checkerPath, fixture], {
    encoding: 'utf8',
  });
};

const assertFailure = (options, expectedMessage) => {
  const result = runChecker(options);
  assert.equal(result.status, 1, result.stdout + result.stderr);
  assert.match(result.stderr, expectedMessage);
};

test('整合したイベントプリセットを受理する', () => {
  const result = runChecker();
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.match(result.stdout, /OK: 2 presets, 2 menu items, 4 locales/);
});

test('数量不明のnullを受理する', () => {
  const result = runChecker({
    counts: [
      [1, 2, 3],
      [null, null, null],
    ],
  });
  assert.equal(result.status, 0, result.stdout + result.stderr);
});

test('プリセット以外のMenuItemを検査対象に含めない', () => {
  const result = runChecker({ unrelatedMenuValues: [99] });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.match(result.stdout, /OK: 2 presets, 2 menu items, 4 locales/);
});

test('空のイベントプリセットを検出する', () => {
  assertFailure(
    { presetItemCounts: [], menuValues: [] },
    /predefinedItems は1件以上/,
  );
});

test('3品でないプリセットを検出する', () => {
  assertFailure({ presetItemCounts: [2, 3] }, /2品/);
});

test('表示順と異なるitem.indexを検出する', () => {
  assertFailure(
    {
      indices: [
        [1, 3, 3],
        [1, 2, 3],
      ],
    },
    /item\.index は 2/,
  );
});

test('欠落したプリセットMenuItemを検出する', () => {
  assertFailure({ menuValues: [0] }, /MenuItem value/);
});

test('重複したプリセットMenuItemを検出する', () => {
  assertFailure({ menuValues: [0, 1, 1] }, /MenuItem value/);
});

test('数値リテラルでないプリセットMenuItemを検出する', () => {
  assertFailure(
    {
      extraPresetMenuItems: [
        '    <MenuItem value={extraPreset}>Extra</MenuItem>',
      ],
    },
    /MenuItem valueは数値リテラル/,
  );
});

test('1言語だけ異なるプリセットラベル数を検出する', () => {
  assertFailure({ localeLabelCounts: { en: 1 } }, /en\/ControlPane\.json.*2件/);
});

test('プリセット数量がUI選択上限を超えたことを検出する', () => {
  assertFailure(
    {
      counts: [
        [8, 2, 3],
        [3, 2, 1],
      ],
      validatorMax: 8,
    },
    /数量選択肢の上限 7/,
  );
});

test('プリセット数量が保存データ検証上限を超えたことを検出する', () => {
  assertFailure(
    {
      counts: [
        [8, 2, 3],
        [3, 2, 1],
      ],
      countMenuMax: 8,
    },
    /保存データ上限 7/,
  );
});

test('3要素でない通知文を検出する', () => {
  assertFailure(
    { notificationAlertLengths: { 'zh-CN': 2 } },
    /zh-CN\/NotificationPanel\.json.*3要素/,
  );
});

test('数量不明時のトースト翻訳欠落を検出する', () => {
  assertFailure(
    { missingUnknownQuantityToastLocales: ['ko'] },
    /ko\/MainArea\.json.*unknown_quantity/,
  );
});
