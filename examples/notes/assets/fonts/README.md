# Browser document fonts

GPUI's Wasm canvas renderer cannot use installed browser/system fonts. These
unmodified fonts provide Japanese/CJK text and color emoji fallbacks, loaded
once per Web application. Native builds use host fonts and do not embed these
15.3 MB of font data. These fonts do not cover every Unicode script.

Sources, downloaded 2026-10-24:

- [Noto Sans JP Regular](https://github.com/notofonts/noto-cjk/blob/main/Sans/SubsetOTF/JP/NotoSansJP-Regular.otf)
  SHA-256: `dff723ba59d57d136764a04b9b2d03205544f7cd785a711442d6d2d085ac5073`
- [Noto Color Emoji](https://github.com/googlefonts/noto-emoji/blob/main/2D/fonts/NotoColorEmoji.ttf)
  SHA-256: `15671215ab769fdc7162a045d56fd7d7e477c51b04e6b3c761d914d8fdd6cc44`

Both use the SIL Open Font License. Their separate copyright/license notices are
included here and combined in `web/font-licenses.txt` for browser distribution.
