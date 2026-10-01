# Sparse-board visual regression verification

Verified on 2026-09-30 from `app/src-tauri`, using process-local settings:

```powershell
$env:CARGO_HOME='D:\Tool\BA扫雷\.tools\cargo'
$env:RUSTUP_HOME='D:\Tool\BA扫雷\.tools\rustup'
$env:PATH='D:\Tool\BA扫雷\.tools\cargo\bin;' + $env:PATH
cargo test --release --test vision_sparse_board --test vision_recognition --test vision_dynamic
```

Exit code: **0**. The release build completed in 1m 11s. Target results:

| Target | Executed | Failed | Distinct target-specific tests | Shared internal tests |
| --- | ---: | ---: | ---: | ---: |
| `vision_sparse_board` | 11 | 0 | 7 new | 4 |
| `vision_recognition` | 21 | 0 | 17 existing | 4 |
| `vision_dynamic` | 11 | 0 | 7 existing | 4 |

Total: **43 passing executions, 35 distinct cases**. Each target includes
`vision.rs` via `#[path]`, which includes the same four dynamic module tests.
Those four run three times, contributing eight duplicate executions.

The seven new distinct cases passed:

- `real_ten_covers_are_observed_fresh_with_two_finished_cards`: original
  1924 by 1142 frame, exact 45 labels, eight exact completed rectangles,
  two Finish cards, and fresh untyped gun/phone occupancy. A second fresh
  recognition with absent board OCR and `[99,99,99]` remaining-piece inputs
  preserves all labels/rectangles.
- `real_thirteen_to_ten_transition_keeps_pixel_labels_and_phone_reference`:
  exact 45 labels before/after, only 18/27/36 change, five cached typed
  completed phones, and the same phone reference fingerprint.
- `one_real_cover_outside_fixed_samples_uses_opened_pixel_geometry`: exact
  45 derived labels with only covered index 16 and seven gray objects.
- `two_real_covers_outside_fixed_samples_use_opened_pixel_geometry`: exact
  45 derived labels with only covered indices 16/20 and seven gray objects.
- `one_selected_cover_outside_fixed_samples_keeps_sparse_geometry`: both
  the real lime selection tile and blue selection tile, placed at index 16
  in the one-cover derivative, preserve all 45 labels and seven gray objects.
- `sparse_board_with_a_half_cell_manual_shift_is_rejected`: range is within
  the coarse manual geometry tolerance; recognition returns `present=false`,
  no board, all uncertain cells, and no object/candidate metadata.
- `sparse_board_hidden_by_large_popup_is_rejected_even_with_valid_counts`:
  synthetic full-board popup keeps the real header/cards visible; both
  remaining-count 10 and 0 return the same strict rejection state.

Code state during this run (SHA-256):

- `src/vision.rs`:
  `f815751812592b37d4db7e862e462dcbc2de18de07f026ce08c0b3a9cf00d1cf`.
- `src/vision_dynamic.rs`:
  `89785a43cc1071b95aedad49f7b1f40c243fbf093cd2050093ce3eb0b4103a3f`.
- `tests/vision_sparse_board.rs`:
  `ff917357c232107329f4d60742deb21faf72b3e952d1ea66757e18732c877205`.

The test fixture's byte-copy hash matches its original:
`ada44b55db5ee0b362b49e026b581586b73b236d24f934f18aeedf1d0d365a53`.
See `vision-sparse-board-provenance.md` for labels and synthetic derivations.
Only existing dead-code warnings were emitted; no test failed. These checks
establish visual recognition regression behavior, not OCR accuracy, solver
recommendations, newly captured native gameplay, or delivery-binary acceptance.
