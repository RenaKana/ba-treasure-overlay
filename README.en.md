# BA Treasure Assistant

[简体中文](README.md) | [繁體中文](README.zh-TW.md) | [日本語](README.ja.md) | English

A Windows treasure-hunting assistant for Blue Archive that helps you search more quickly and comfortably.

It calculates the hit probability for each tile based on the current board and remaining items, then displays the probabilities as an overlay in the game.

## Features

- **Probability display**: See the hit probability for each tile on the game board.
- **Item footprint marking**: Identify an item's full footprint from revealed fragments, mark it with a gold outline, and exclude it from the remaining probability calculation.
- **Confirmed item tracking**: Update observations of confirmed items as more tiles within their footprints are opened during the same round.
- **Manual and automatic refresh**: Refresh after opening a tile, or enable automatic updates as the board changes.
- **Highest-probability highlight**: Turn on “Highlight highest” to emphasize the tile with the highest hit probability.
- **Manual correction**: Correct a tile's recognized state, item dimensions, and remaining quantity.

## Download

Download the Windows 64-bit ZIP from [GitHub Releases](https://github.com/RenaKana/ba-treasure-overlay/releases/latest), extract it, and run `ba-treasure-overlay.exe`. Close the previous assistant before updating.

## How to use

Runs on **64-bit Windows 10 / 11**. See the [detailed user guide](app/docs/desktop.md) for required components and common issues.

1. Open the treasure board in the game, then run `ba-treasure-overlay.exe`.
2. Select the game window in the assistant and click Connect.
3. Switch back to the game and click the refresh button beside the board to view the probability hints.
4. Refresh after each tile is opened. With automatic mode enabled, results update on their own.

If tile appearances are not recognized, click **Select samples**, choose one unopened tile for each appearance from the 45-cell board, and save. Canceling or a failed save keeps the existing samples.

Saved samples stay on this PC across rounds and restarts. After an event change or recognition issue, check the game screen and update them manually; clearing samples restores automatic recognition.

Use the control at the top of the assistant window to change the interface language and zoom level. The zoom setting is saved and affects only the assistant window; it does not change the overlay coordinates on the game board.

Probabilities are for reference and do not guarantee a hit every time. Once a partially revealed item's full footprint is identified, the assistant marks it and excludes it from the remaining probability calculation. Unrecognized images or ambiguous footprints pause recommendations. If the recognized board differs from the actual game, correct it manually and refresh again. Image recognition and probability calculations run locally, and the assistant does not open tiles automatically.

## Client compatibility

| Game environment | Status |
| --- | --- |
| Chinese version · MuMu emulator | Tested in-game, including recognition at different window sizes |
| Global version · Steam PC | The event was tested at 4K resolution and in 16:9 and 4:3 game windows |
| Japanese version · PC client | Window capture is confirmed; in-event recognition and probability display have not been verified |

Compatibility may vary with Windows display scaling, fullscreen mode, and new events. If recognition fails, try switching to windowed mode. See the [compatibility notes](app/docs/pc-compatibility.md) for details.

## Feedback

When reporting an issue, include the program version, game client, game language, and steps to reproduce it. A screenshot can help; please cover your account name, UID, and other personal information before sharing.

## Acknowledgments and license

This project is based on [terry-u16/schale-inventory-management](https://github.com/terry-u16/schale-inventory-management). It adds Windows game-window recognition and in-game probability display to the original calculation features. Thanks to the original author for their open-source work.

This is an unofficial tool and is not affiliated with the game publisher. The upstream [MIT license](LICENSE) and original author notices are retained. Rights to game screenshots, icons, and other assets remain with their respective owners. See the [third-party notices](THIRD_PARTY_NOTICES.md) for details.
