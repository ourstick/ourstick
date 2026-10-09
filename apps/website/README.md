# ourstick.com

The static site: the home page with the downloads, the page a sharing link
opens without the app (`/s/<public>#<secret>`), privacy, and the third-party
licences. Served by Cloudflare Pages. Nothing on it loads from other sites:
the link page holds a stick's key in its address, after `#`, which any
foreign script could read (`site/_headers`).

## Deploying

1. Build the three installers: the `.dmg` with `tools/macos/make-dmg.sh` on
   the Mac, the NSIS installer in the Windows VM (`docs/windows-dev.md`), the
   `.deb` with `npm run tauri build -- --bundles deb` in `apps/app`.
2. `apps/website/make.sh <dmg> <setup.exe> <deb>` puts everything in
   `apps/website/dist`.
3. `npx wrangler pages deploy apps/website/dist --project-name ourstick`
   (the first time, `npx wrangler login`, and the custom domain `ourstick.com`
   added to the project in the dashboard).

Once, in the Cloudflare dashboard for the `ourstick.com` zone, keep off
everything that injects scripts into pages: **Rocket Loader**, **Web
Analytics** (automatic setup), **Zaraz**, and **Email Address Obfuscation**.
The CSP blocks them anyway, and Rocket Loader would then stop the pages' own
scripts.

## Checking a deploy

- `https://ourstick.com/s/abc#xyz` shows "Someone sent you a stick", and
  `https://ourstick.com/s/abc` shows "This link is not whole".
- The browser's console shows no CSP errors, and the response headers have
  the `Content-Security-Policy` of `_headers`.
- `THIRD-PARTY-LICENSES.html` has its styles.
