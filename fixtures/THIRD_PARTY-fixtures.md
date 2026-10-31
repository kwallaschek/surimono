# Third-party material in fixtures/

Test-only files under `fixtures/`, with version, licence and source. They are
never compiled into the crates or the gem. The root `THIRD_PARTY.md` lists them
too.

## Fonts (`fixtures/fonts/`)

Pinned per file in `fixtures/fonts/fonts.toml` (source URL at a commit SHA,
sha256, size); `python3 fixtures/fonts/verify.py` checks them.

| Font | Files | Version | Licence | Source |
|---|---|---|---|---|
| Noto Sans (unhinted, static) | NotoSans-Regular.ttf, NotoSans-Bold.ttf | 2.015 | SIL OFL 1.1 (`fixtures/fonts/OFL.txt`) | https://github.com/notofonts/notofonts.github.io at 412a7edb2c8c535d1066157e5b17e661fd5591fe |
| Noto Sans JP (static subset OTF) | NotoSansJP-Regular.otf, NotoSansJP-Bold.otf | 2.004 | SIL OFL 1.1 (`fixtures/fonts/OFL.txt`) | https://github.com/notofonts/noto-cjk at f8d157532fbfaeda587e826d4cd5b21a49186f7c |
