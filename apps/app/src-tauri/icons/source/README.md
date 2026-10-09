# Icon sources

OurStick's own drawings (2026-10-09): an upright stick, its
connector below, a white arrow going out. Not derived from any icon theme.

- `icon.svg`: the app icon. Rendered at 1024 px with a transparent background
  (`../../icon-source.png`), then `npm run tauri icon src-tauri/icon-source.png`
  in `apps/app` makes `../32x32.png` … `../icon.icns`, `../icon.ico` (keep only
  the files already in `icons/`).
- `tray.svg`: the tray glyph, one colour, 64 px: black for
  `tray-template.png` (macOS tints it), white for `tray-light.png`, grey
  `#8a8a8a` for the base of `tray-grey-active.png`. The `-active` ones add a
  green dot (`#2ea44f`, r 10.5 at 51,11) in a clear ring (r 13.5).
