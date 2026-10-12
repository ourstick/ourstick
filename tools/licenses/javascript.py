"""Adds the JavaScript in the app's window to THIRD-PARTY-LICENSES.html:
the packages whose code Vite puts into apps/app/dist (ADR 0024 §12), each
with its version, the licence it is used under and that licence's text.

    python3 tools/licenses/javascript.py THIRD-PARTY-LICENSES.html apps/app/node_modules
"""

import html
import json
import pathlib
import sys

# (package, licence used, licence file, the part of it that is the
# package's own licence: up to this line, or the whole file).
PACKAGES = [
    ("svelte", "MIT", "LICENSE.md", None),
    ("@tauri-apps/api", "MIT (offered as Apache-2.0 OR MIT)", "LICENSE-MIT", None),
    # Only Vite's module preload helper is in the bundle.
    ("vite", "MIT", "LICENSE.md", "# Licenses of bundled dependencies"),
]

MARK = "<!-- javascript -->"


def section(modules: pathlib.Path) -> str:
    parts = [
        MARK,
        '<h2 id="javascript">JavaScript in the app\'s window</h2>',
        "<p>The window's code includes these packages, from npm "
        "(https://www.npmjs.com/package/NAME).</p>",
    ]
    for name, licence, file, end in PACKAGES:
        package = modules / name
        version = json.loads((package / "package.json").read_text())["version"]
        text = (package / file).read_text()
        if end is not None:
            text = text.split(end)[0]
        parts.append(f"<h3>{html.escape(name)} {html.escape(version)}: {html.escape(licence)}</h3>")
        parts.append(f"<pre>{html.escape(text.strip())}</pre>")
    return "\n".join(parts) + "\n"


def main() -> None:
    page_path, modules = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    # newline="": the licences' own line ends (some \r\n) kept as they are.
    with page_path.open(newline="") as file:
        page = file.read()
    if MARK in page:
        page = page[: page.index(MARK)] + "</body>\n</html>\n"
    page = page.replace("</body>", section(modules) + "</body>", 1)
    with page_path.open("w", newline="") as file:
        file.write(page)


main()
