# ourstick.com

The static site: the home page with the downloads, the page a sharing link
opens without the app (`/s/#<public>.<secret>`), privacy, and the third-party
licences. Served by Cloudflare Pages. Nothing on it loads from other sites:
the link page holds a whole ticket, the stick's key included, in its
address after `#`, which any foreign script could read (`site/_headers`).

## Deploying

1. Build the three installers: the `.dmg` with `tools/macos/make-dmg.sh` on
   the Mac, the NSIS installer in the Windows VM (`docs/windows-dev.md`), the
   `.deb` with `npm run tauri build -- --bundles deb` in `apps/app`.
2. `OURSTICK_RELEASE_KEY=<the release key's secret file> apps/website/make.sh
   <dmg> <setup.exe> <deb>` puts everything in `apps/website/dist`, with
   `SHA256SUMS.txt` signed (it asks for the key's passphrase;
   `release-keys/README.md`). The GitHub Release gets the same:
   `tools/release/sign.sh SHA256SUMS-<version>.txt`, and its `.minisig`
   goes up with the installers.
   After the installers and the release notes are up, never before:
   `tools/release/version-json.sh <newest> <security>` writes the signed
   notice the app reads (ADR 0033) to `site/version.json`; commit it, run
   `make.sh` again and deploy. `crates/ourstick-core/release-day.txt` holds
   the release's day, set before the installers are built.
3. `npx wrangler pages deploy apps/website/dist --project-name ourstick`
   (the first time, `npx wrangler login`, and the custom domain `ourstick.com`
   added to the project in the dashboard).

Once, in the Cloudflare dashboard for the `ourstick.com` zone, keep off
everything that injects scripts into pages: **Rocket Loader**, **Web
Analytics** (automatic setup), **Zaraz**, and **Email Address Obfuscation**.
The CSP blocks them anyway, and Rocket Loader would then stop the pages' own
scripts.

## Checking a deploy

- `https://ourstick.com/s/#abc.xyz` shows "Someone sent you files", and
  `https://ourstick.com/s/#abc` shows "This link is not whole".
- The browser's console shows no CSP errors, and the response headers have
  the `Content-Security-Policy` of `_headers`.
- `THIRD-PARTY-LICENSES.html` has its styles.
- The link page (`/s/`) loads no `/cdn-cgi/` script: Cloudflare's script
  features stay off (above).

## Media

`site/media/ourstick-demo-10.mp4`: the demo (1:42, with a voice-over; a
new version gets a new number, as browsers keep a video for hours), made
with Remotion from screen recordings of the app on two computers (a test
stick of generated pictures) and one generated picture of a hand giving a
USB stick. The voice is Kokoro TTS (voice af_heart). It says OurStick is
for Mac, Windows and Linux, and shows "A Windows PC has all of it". The
link it shows has the real form (`https://ourstick.com/s/#<public>.<secret>`)
but random characters: not a ticket, and no ticket is ever shown, as a link
holds the sender's addresses and the key. Its music is J. S. Bach's
Aria from the Goldberg Variations, played by Kimiko Ishizaka, dedicated to
the public domain (CC0) by the Open Goldberg Variations project:
https://commons.wikimedia.org/wiki/File:Kimiko_Ishizaka_-_J.S._Bach-_-Open-_Goldberg_Variations,_BWV_988_(Piano)_-_01_Aria.mp3
(https://opengoldbergvariations.org). `ourstick-demo-poster-6.jpg` is a
frame of its first shot.

The font, `site/fonts/UbuntuSans.woff2`, is Ubuntu Sans 1.006 by Canonical
(github.com/canonical/Ubuntu-Sans-fonts, release v1.006, the file
`webfont/UbuntuSans[wdth,wght].woff2`, unchanged but for its name), under
the Ubuntu Font Licence 1.0 (`site/fonts/UbuntuSans-LICENCE.txt`, shipped
beside it). The demo video's titles use the same release's
`UbuntuSans-Bold.ttf` and `UbuntuSans-Regular.ttf`.
