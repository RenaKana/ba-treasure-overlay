# Fullscreen OCR regression provenance

`vision-ocr-fullscreen-3840.png` is a byte-for-byte copy of the local,
read-only Windows.Graphics.Capture frame:

`compatibility_probe/artifacts/20260930T132943Z_3f7d23b8/latest.png`

The source capture is 3840 by 2160, sample 14 of 14; its capture record reports
no generated input and no capture errors. PNG SHA-256 for both source and
fixture:

`56bea3ef89a817c41e22456e23f6d0f99858fcf26aced06fe6f526d9e2abfd93`

The visible HUD ground truth is remaining cells `13 / 45`, round `1`, and
remaining card counts `[0, 1, 1]`. The fixture is used only by the native OCR
regression test, not embedded as a production template or used to infer item
counts from item artwork.

## Measured failure and repair

The local Windows `zh-Hans` OCR probe in `evidence/fullscreen-ocr/probe` uses
the production crop coordinates, interpolation, OCR input format, parsing,
and conflict rule. It varies only the crop scale lower bound. Full results,
source crop rectangles, OCR text and saved OCR-input crops are in
`evidence/fullscreen-ocr/scale-probe.json` and its `crops` directory.

For card slot 1, the original tight/roomy input crops are respectively
134 by 97 and 154 by 86. `clamp(1.0, 4.0)` leaves both at their native large
scale; Windows OCR returns an empty string in both slot 1 and slot 2 views.
Removing the scale lower bound produces 110 by 80 and 143 by 80 inputs.
The roomy views then read `× 1`; the tight views stay empty. The existing
two-view merge rule accepts the one unambiguous numeric reading. No digit
substitution or fallback count is introduced.

| Input | Original counts | Normalized counts | Remaining / round after normalization |
| --- | --- | --- | --- |
| Real fullscreen 3840 by 2160 | `[0, None, None]` | `[0, 1, 1]` | `13 / 1` |
| Original window 1924 by 1142 | `[0, 1, 1]` | `[0, 1, 1]` | `13 / 1` |
| Independent window capture `20260930T125726Z_7be4bf69/latest.png` | `[0, 1, 1]` | `[0, 1, 1]` | `13 / 1` |
| Window fixture resized to width 2560 | `[0, 1, 1]` | `[0, 1, 1]` | `13 / 1` |
| Window fixture resized to width 3840 | `[0, None, 1]` | `[0, 1, 1]` | `13 / 1` |

The last two rows are derived CatmullRom resizes of the complete window
fixture, including its toolbar. Their full OCR output is in
`evidence/fullscreen-ocr/scale-probe-large-windows.json`. The original
window, real fullscreen, and both larger derived window sizes have native
regression coverage for all three HUD fields.

Verification from `app` after the product patch:

`cargo test --release --manifest-path src-tauri/Cargo.toml --bin ba-treasure-overlay`

Result: **22 passed, 0 failed**, including all five OCR tests and
`session_tests::unread_counts_do_not_report_a_size_error`. The log is
`evidence/fullscreen-ocr/native-unit-tests.log`.

## Limits

The original algorithm also misses fields when the window fixture is
downsampled to widths 1440 and 960; the scale change preserves those same
results. At width 1440, remaining is `13`, round is `None`, and counts are
`[0, None, 1]`. At width 960, remaining is `13`, round is `1`, and counts are
`[0, None, None]`. These observed limits are recorded by the probe, not
treated as successful HUD acceptance; unread fields still need correction.

This is local Windows OCR evidence for one actual activity/round and its
derived raster sizes. The WGC fullscreen capture does not establish true
exclusive-fullscreen operation or exhaustive recognition of other artwork,
rounds, resolutions, or installed OCR language configurations.
