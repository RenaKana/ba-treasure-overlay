#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import ts from 'typescript';

const repoRoot = path.resolve(process.argv[2] ?? process.cwd());
const errors = [];

const read = (relativePath) =>
  fs.readFileSync(path.join(repoRoot, relativePath), 'utf8');

const unwrap = (expression) => {
  let current = expression;
  while (
    ts.isAsExpression(current) ||
    ts.isSatisfiesExpression(current) ||
    ts.isParenthesizedExpression(current)
  ) {
    current = current.expression;
  }
  return current;
};

const numericJsxValue = (attribute) => {
  const initializer = attribute.initializer;
  if (initializer == null) return null;
  if (ts.isStringLiteral(initializer)) return Number(initializer.text);
  if (
    ts.isJsxExpression(initializer) &&
    initializer.expression != null &&
    ts.isNumericLiteral(initializer.expression)
  ) {
    return Number(initializer.expression.text);
  }
  return null;
};

const findAttribute = (attributes, name) =>
  attributes.properties.find(
    (attribute) => ts.isJsxAttribute(attribute) && attribute.name.text === name,
  );

const mainAreaText = read('src/components/MainArea.tsx');
const mainArea = ts.createSourceFile(
  'MainArea.tsx',
  mainAreaText,
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);

let predefinedItems = null;
const visitMainArea = (node) => {
  if (
    ts.isVariableDeclaration(node) &&
    node.name.getText() === 'predefinedItems'
  ) {
    predefinedItems = unwrap(node.initializer);
  }
  ts.forEachChild(node, visitMainArea);
};
visitMainArea(mainArea);

if (predefinedItems == null || !ts.isArrayLiteralExpression(predefinedItems)) {
  errors.push('MainArea.tsx: predefinedItems の配列を取得できません');
}

const presets =
  predefinedItems != null && ts.isArrayLiteralExpression(predefinedItems)
    ? predefinedItems.elements.map(unwrap)
    : [];
let maxPresetCount = 0;

if (
  predefinedItems != null &&
  ts.isArrayLiteralExpression(predefinedItems) &&
  presets.length === 0
) {
  errors.push('MainArea.tsx: predefinedItems は1件以上必要です');
}

presets.forEach((preset, presetIndex) => {
  if (!ts.isArrayLiteralExpression(preset)) {
    errors.push(`predefinedItems[${presetIndex}] が配列ではありません`);
    return;
  }
  if (preset.elements.length !== 3) {
    errors.push(
      `predefinedItems[${presetIndex}] は3品ではありません (${preset.elements.length}品)`,
    );
  }

  preset.elements.forEach((element, itemIndex) => {
    const itemSet = unwrap(element);
    if (!ts.isObjectLiteralExpression(itemSet)) {
      errors.push(
        `predefinedItems[${presetIndex}][${itemIndex}] がobjectではありません`,
      );
      return;
    }
    const itemProperty = itemSet.properties.find(
      (property) => property.name?.getText() === 'item',
    );
    const countProperty = itemSet.properties.find(
      (property) => property.name?.getText() === 'count',
    );
    if (
      itemProperty == null ||
      !ts.isPropertyAssignment(itemProperty) ||
      !ts.isObjectLiteralExpression(unwrap(itemProperty.initializer))
    ) {
      errors.push(
        `predefinedItems[${presetIndex}][${itemIndex}].item を取得できません`,
      );
      return;
    }

    const item = unwrap(itemProperty.initializer);
    const indexProperty = item.properties.find(
      (property) => property.name?.getText() === 'index',
    );
    const expectedIndex = itemIndex + 1;
    if (
      indexProperty == null ||
      !ts.isPropertyAssignment(indexProperty) ||
      !ts.isNumericLiteral(indexProperty.initializer) ||
      Number(indexProperty.initializer.text) !== expectedIndex
    ) {
      errors.push(
        `predefinedItems[${presetIndex}][${itemIndex}].item.index は ${expectedIndex} である必要があります`,
      );
    }

    if (countProperty == null || !ts.isPropertyAssignment(countProperty)) {
      errors.push(
        `predefinedItems[${presetIndex}][${itemIndex}].count を取得できません`,
      );
      return;
    }

    const countInitializer = unwrap(countProperty.initializer);
    if (countInitializer.kind === ts.SyntaxKind.NullKeyword) {
      return;
    }
    if (!ts.isNumericLiteral(countInitializer)) {
      errors.push(
        `predefinedItems[${presetIndex}][${itemIndex}].count は整数リテラルまたはnullである必要があります`,
      );
      return;
    }

    const count = Number(countInitializer.text);
    if (!Number.isInteger(count) || count < 0) {
      errors.push(
        `predefinedItems[${presetIndex}][${itemIndex}].count が不正です`,
      );
    }
    maxPresetCount = Math.max(maxPresetCount, count);
  });
});

const controlPane = ts.createSourceFile(
  'ControlPane.tsx',
  read('src/components/ControlPane.tsx'),
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);
const presetMenuValues = [];
const collectPresetMenuValues = (node) => {
  let attributes = null;
  if (
    ts.isJsxSelfClosingElement(node) &&
    node.tagName.getText() === 'MenuItem'
  ) {
    attributes = node.attributes;
  }
  if (
    ts.isJsxElement(node) &&
    node.openingElement.tagName.getText() === 'MenuItem'
  ) {
    attributes = node.openingElement.attributes;
  }

  if (attributes != null) {
    const valueAttribute = findAttribute(attributes, 'value');
    const value =
      valueAttribute == null ? null : numericJsxValue(valueAttribute);
    if (value == null || !Number.isFinite(value)) {
      errors.push(
        'ControlPane.tsx: プリセットのMenuItem valueは数値リテラルである必要があります',
      );
    } else {
      presetMenuValues.push(value);
    }
  }
  ts.forEachChild(node, collectPresetMenuValues);
};
const visitControlPane = (node) => {
  if (
    ts.isJsxElement(node) &&
    node.openingElement.tagName.getText() === 'Select'
  ) {
    const idAttribute = findAttribute(node.openingElement.attributes, 'id');
    if (
      idAttribute?.initializer != null &&
      ts.isStringLiteral(idAttribute.initializer) &&
      idAttribute.initializer.text === 'predefined-choice-select'
    ) {
      ts.forEachChild(node, collectPresetMenuValues);
      return;
    }
  }
  ts.forEachChild(node, visitControlPane);
};
visitControlPane(controlPane);

const expectedMenuValues = Array.from({ length: presets.length }, (_, i) => i);
if (JSON.stringify(presetMenuValues) !== JSON.stringify(expectedMenuValues)) {
  errors.push(
    `ControlPane.tsx: MenuItem value は ${JSON.stringify(expectedMenuValues)} である必要があります (actual: ${JSON.stringify(presetMenuValues)})`,
  );
}

const itemPane = ts.createSourceFile(
  'ItemPane.tsx',
  read('src/components/ItemPane.tsx'),
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);
const countMenuValues = [];
const collectCountMenuValues = (node) => {
  if (
    ts.isJsxElement(node) &&
    node.openingElement.tagName.getText() === 'MenuItem'
  ) {
    const valueAttribute = findAttribute(
      node.openingElement.attributes,
      'value',
    );
    if (valueAttribute != null) {
      const value = numericJsxValue(valueAttribute);
      if (value != null) countMenuValues.push(value);
    }
  }
  ts.forEachChild(node, collectCountMenuValues);
};
const visitItemPane = (node) => {
  if (
    ts.isJsxElement(node) &&
    node.openingElement.tagName.getText() === 'Select'
  ) {
    const onChange = findAttribute(node.openingElement.attributes, 'onChange');
    if (
      onChange?.initializer != null &&
      ts.isJsxExpression(onChange.initializer) &&
      onChange.initializer.expression?.getText() === 'onCountChange'
    ) {
      ts.forEachChild(node, collectCountMenuValues);
    }
  }
  ts.forEachChild(node, visitItemPane);
};
visitItemPane(itemPane);

if (countMenuValues.length === 0) {
  errors.push('ItemPane.tsx: 数量選択肢を取得できません');
} else {
  const expectedCountValues = Array.from(
    { length: Math.max(...countMenuValues) + 1 },
    (_, i) => i,
  );
  if (JSON.stringify(countMenuValues) !== JSON.stringify(expectedCountValues)) {
    errors.push(
      `ItemPane.tsx: 数量選択肢は0からの連番である必要があります (actual: ${JSON.stringify(countMenuValues)})`,
    );
  }
  if (maxPresetCount > Math.max(...countMenuValues)) {
    errors.push(
      `ItemPane.tsx: 最大プリセット数量 ${maxPresetCount} が数量選択肢の上限 ${Math.max(...countMenuValues)} を超えています`,
    );
  }
}

for (const locale of ['ja', 'en', 'ko', 'zh-CN']) {
  const controlPanePath = `public/locales/${locale}/ControlPane.json`;
  const controlPaneJson = JSON.parse(read(controlPanePath));
  const labels = controlPaneJson.predefined_choice_select;
  if (!Array.isArray(labels) || labels.length !== presets.length) {
    errors.push(
      `${controlPanePath}: predefined_choice_select は ${presets.length}件である必要があります`,
    );
  }

  const notificationPath = `public/locales/${locale}/NotificationPanel.json`;
  const notificationJson = JSON.parse(read(notificationPath));
  if (
    !Array.isArray(notificationJson.alert) ||
    notificationJson.alert.length !== 3
  ) {
    errors.push(`${notificationPath}: alert は3要素である必要があります`);
  }

  const mainAreaPath = `public/locales/${locale}/MainArea.json`;
  const mainAreaJson = JSON.parse(read(mainAreaPath));
  for (const key of [
    'item_preset_applied_toast',
    'item_preset_applied_with_unknown_quantity_toast',
  ]) {
    const message = mainAreaJson[key];
    if (typeof message !== 'string' || !message.includes('{{presetLabel}}')) {
      errors.push(
        `${mainAreaPath}: ${key} に{{presetLabel}}を含む文言が必要です`,
      );
    }
  }
}

const validatorMatch = mainAreaText.match(
  /isIntegerInRange\(entry\.item\.count,\s*0,\s*(\d+)\)/,
);
if (validatorMatch == null) {
  errors.push('MainArea.tsx: 保存データの数量上限を取得できません');
} else if (maxPresetCount > Number(validatorMatch[1])) {
  errors.push(
    `MainArea.tsx: 最大プリセット数量 ${maxPresetCount} が保存データ上限 ${validatorMatch[1]} を超えています`,
  );
}

if (errors.length > 0) {
  for (const error of errors) console.error(`ERROR: ${error}`);
  process.exit(1);
}

console.log(
  `OK: ${presets.length} presets, ${presetMenuValues.length} menu items, 4 locales, max preset count ${maxPresetCount}`,
);
