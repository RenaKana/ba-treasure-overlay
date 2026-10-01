# Agent Browserスモークテスト

静的検査では見つからない寸法の転記、プリセットと画面の結線、localStorageによる旧値表示、翻訳後のUIを実ブラウザで確認する。

## 事前準備

1. issueやユーザー指定から作った根拠表を用意する。画面や実装から期待値を作り直さない。
2. `pnpm test:event-presets`、`pnpm lint`、`pnpm build` を先に通す。
3. `pnpm dev --host 127.0.0.1` を起動し、表示されたURLを使う。
4. 他のブラウザ操作と状態を共有しないよう、専用session名 `schale-event-smoke` を使う。

## 初期状態

```bash
agent-browser --session schale-event-smoke open http://127.0.0.1:5173
agent-browser --session schale-event-smoke wait --load networkidle
agent-browser --session schale-event-smoke storage local clear
agent-browser --session schale-event-smoke reload
agent-browser --session schale-event-smoke wait --load networkidle
agent-browser --session schale-event-smoke snapshot -i
```

- タイトルと主要操作が表示されていることを確認する。
- ブラウザの既定言語で表示されるため、日本語以外なら言語メニューから日本語へ切り替える。
- `[role="alert"]` の本文が日本語の確定文と一致することを読む。
- 仮値が残る場合はwarning、確定済みなら意図したseverityであることをMUI Alertのclassまたはアイコンから確認する。
- issueリンクの`href`がこのリポジトリのissuesを指すことを確認する。

localStorageを消した後は必ずreloadしてsnapshotを取り直す。古いrefを再利用しない。

## プリセット値

全プリセットを確認する。各周回について次を繰り返す。

1. snapshotから「備品プリセット」のcomboboxをクリックする。
2. dropdown表示後にsnapshotを取り直し、対象周回のoptionをクリックする。
3. snapshotを取り直し、「適用」をクリックする。
4. 3枚の備品cardについて、comboboxを表示順に読み取る。`ItemPane.tsx` の表示順は高さ、幅、数量である。
5. 読み取った `[height, width, count]` を、根拠表の `[width x height, count]` と独立に比較する。

MUIのSelectがclickで開かない場合は、comboboxへfocusして`ArrowDown`を押す。Tooltipのラッパーにより「適用」のrole locatorが実ボタンを選ばない場合があるため、直前のsnapshotで取得したbutton refを使う。それでも選べない場合だけ、表示文字列が完全一致するbuttonを`eval`でクリックする。

```bash
agent-browser --session schale-event-smoke focus '#predefined-choice-select'
agent-browser --session schale-event-smoke press ArrowDown
agent-browser --session schale-event-smoke snapshot -i
```

DOMからまとめて読み取る場合の例:

```bash
agent-browser --session schale-event-smoke eval --stdin <<'EVALEOF'
Array.from(document.querySelectorAll('.MuiCard-root'))
  .slice(0, 3)
  .map((card) =>
    Array.from(card.querySelectorAll('[role="combobox"]')).map((element) =>
      element.textContent.trim(),
    ),
  );
EVALEOF
```

- 配列indexだけでなく、選択したラベル名も結果表へ記録する。
- 同一パターンの繰り返しも省略しない。推定区間の展開ミスを検出するため、表示される全周回を確認する。
- 「N周目以降」は通常周回と別に確認する。
- プリセット適用後、盤面の開放状態と配置がリセットされることを少なくとも1回確認する。

## 言語と計算

ja、en、ko、zh-CNへ順に切り替え、各言語で次を確認する。

- プリセットoption数が `predefinedItems` と同じ。
- 先頭、最終通常周回、「N周目以降」のラベルが正しい。
- 通知が日本語と同じ進捗状態、推定対象、未知対象を表している。

代表プリセットを1つ適用して確率計算を実行し、実行中表示が解除されて結果が表示されることを確認する。

最後に次を実行する。

```bash
agent-browser --session schale-event-smoke errors
agent-browser --session schale-event-smoke console
agent-browser --session schale-event-smoke close
```

page errorまたは予期しないconsole errorがあれば失敗とする。失敗時だけannotated screenshotを保存し、選択した周回、期待値、実測値、エラーを報告する。dev serverも終了する。

## 完了条件

- 全周回についてラベル、3品の高さ・幅・数量が根拠表と一致する。
- 4言語の選択肢と通知状態が一致する。
- プリセット適用と確率計算が完了する。
- page errorと予期しないconsole errorがない。

ブラウザを起動できなかった場合や一部周回を省略した場合は、成功扱いにせず未確認範囲を明記する。
