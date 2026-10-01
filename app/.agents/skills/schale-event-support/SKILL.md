---
name: schale-event-support
description: Schale Inventory Managementでイベント開始に合わせて備品の種類・寸法・周回別数量・プリセット・多言語通知を更新するスキル。数量が未知の新規イベントと、過去データを再利用する復刻イベントの実装・検証・PR準備に使う。
---

# Schale Event Support

イベントデータの根拠と不確実性を保ったまま、`predefinedItems`、UI、翻訳、通知を一貫して更新する。

## 最初に確定すること

1. 作業元が最新の `main` か、worktreeがcleanか確認し、イベント専用ブランチを作る。既存作業を混ぜない。
2. issue、ユーザー指定、参考PR・コミットを読み、次を表にする。
   - 正式イベント名、開始日、新規か復刻か
   - 各周回の3備品について、名前、`width x height`、数量、情報源
   - 通常周回と「N周目以降」の境界
   - 確認済み、推定、未知の区別
3. 不明値を勝手に補わない。実装結果を変える情報が欠けていれば、推定方法または仮値をユーザーに確認する。

新規イベントなら [references/new-event.md](references/new-event.md) を読む。復刻イベントなら [references/rerun-event.md](references/rerun-event.md) を読む。

## 実装上の契約

- `src/components/MainArea.tsx`
  - イベントの正式名称を備品定義のコメントに書く。
  - 備品名に対応する識別可能な英語変数名を使い、`width` と `height` を情報源どおりに設定する。
  - 現行の慣例どおり、使わなくなったイベントの定義はコメントアウトし、対象イベントの定義だけを有効にする。コメント境界を目視し、buildで構文を検証する。
  - `predefinedItems[presetIndex]` は画面の同じindexの周回ラベルに対応させる。
  - 各プリセットは3品、`item.index` は表示順に `1, 2, 3` とする。同じ備品でも周回内の表示位置に応じたindexを付ける。
  - 推定区間や仮値の直前に、対象範囲が誤解されないコメントを書く。
- `src/components/ControlPane.tsx`
  - `MenuItem` の値を `0` から連番にし、`predefinedItems` と同数にする。
- `public/locales/{ja,en,ko,zh-CN}/ControlPane.json`
  - `predefined_choice_select` の長さと順序を全言語で揃える。
  - 最終プリセットが範囲なら、各言語で「N周目以降」の意味を保つ。
- `public/locales/{ja,en,ko,zh-CN}/NotificationPanel.json`
  - `alert` はリンク前・リンク文字列・リンク後の3要素を維持する。
  - 日本語を確定してから他言語へ同じ状態と不確実性を翻訳する。「対応中」と「対応完了」、「数量」と「種類・数量」を混同しない。
- `src/components/NotificationPanel.tsx`
  - 未知の数量や明示的な仮値が残る間は `warning` にする。情報が確定した追補では仮値コメントを除き、通知を更新して `success` に戻す。

備品数量が現在の選択肢または保存データ検証の上限を超える場合、データだけを変更しない。`ItemPane.tsx` の数量選択肢、`MainArea.tsx` の保存データ検証、WASM側の前提を調べ、必要な範囲を揃える。寸法が1〜4を外れる場合も同様にUI・検証・solverを確認する。

## 検証

次を実行する。

```bash
pnpm test:event-presets
pnpm lint
pnpm build
git diff --check
```

イベントデータまたは表示範囲を変えた場合は、静的検査だけで完了とせず、[references/browser-smoke-test.md](references/browser-smoke-test.md) を読んでagent-browserスモークテストを実行する。

## コミットとPR

- コミット、push、PR作成はユーザーの指示がある段階で行う。マージやpushまで自動で広げない。
- 暫定値が残る初回PRは過去例にならいタイトルを `YYYY/M/D開始イベントへの対応 part1` とし、本文に確認済み範囲、推定範囲、仮値を明記する。
- 関連issueは本文で参照する。未知値の追補が必要なら初回PRでcloseせず、確定値を反映する追補PRでcloseする。
- 復刻元を使う場合は、どの開催時データを流用したかと、今回確認した範囲を本文に書く。
- PR本文には変更内容と `pnpm test:event-presets`、`pnpm lint`、`pnpm build`、`git diff --check`、ブラウザ確認結果を記載する。
