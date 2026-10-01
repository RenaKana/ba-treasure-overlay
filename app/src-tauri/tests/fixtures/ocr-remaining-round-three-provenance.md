# Round-three remaining-cell OCR regression provenance

`vision-ocr-round-three-remaining-15.png` is a byte-for-byte copy of the local,
read-only Windows.Graphics.Capture frame:

`compatibility_probe/artifacts/20260930T151653Z_a5eef696/latest.png`

The source is 1924 by 1142, sample 4 of 4. Its capture record reports no
capture errors. Source and fixture PNG SHA-256:

`966419ced8bb8266515fb78c3fe814db58ec27b2d097075180414677a327d166`

Visible HUD ground truth is remaining cells `15 / 45`, round `3`, and
remaining card counts `[0, 0, 2]`. The fixture is test-only; it is not a
production template, and neither covered-cell observations nor remaining
card counts supply the HUD value.

## Measured cause and repair

With the unchanged production crop `[0.649, 0.222, 0.153, 0.035]` and
80-pixel OCR input height, local Windows `zh-Hans` OCR returns:

`剩 余 格 子 数 量 ： 1 5 / 45`

The original generic `numbers()` parser produces `[1, 5, 45]`. Selecting
the penultimate numeric word incorrectly returns `5`; the tens digit is
present in the OCR result rather than missing from the image.

The dedicated remaining-cell parser requires one explicit `/`, isolates
the fraction after the label's colon, and joins only whitespace within
its two numeric fields. It requires denominator `45` and numerator `0..45`.
Unexpected characters, competing fractions and out-of-range fields remain
unread. Label digits cannot be joined into the numerator. Crop coordinates,
rescaling, round OCR and card-count OCR are unchanged.

Raw text, word bounding rectangles and saved OCR-input crops for this real
frame and the four prior window/fullscreen HUD fixtures are recorded in
`evidence/remaining-ocr-probe/crop-probe.json`. The original crop for this
frame is `[1248, 301, 294, 38]`, resized to `[618, 80]`.

Verification uses the existing six OCR tests, one native regression for
this real frame, and two focused fraction-parser tests covering fragmented
numerators/denominators, label digits, `0 / 45`, `45 / 45` and invalid or
ambiguous fractions. The command and output are in
`evidence/remaining-ocr-probe/native-ocr-tests.log`.

Result on the patched code: **9 passed, 0 failed**; 20 non-OCR tests were
filtered out. The native frame reads remaining `15`, round `3` and counts
`[0, 0, 2]`; existing window/fullscreen and larger-viewport OCR assertions
also pass.

This fixture establishes the displayed fields in this actual MuMu/WGC
frame. It does not establish exhaustive OCR behavior for all game states
or other installed Windows OCR language configurations.
