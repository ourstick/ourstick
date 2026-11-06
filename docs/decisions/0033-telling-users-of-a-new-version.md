# 0033. Telling users of a new version

**Status:** accepted — 2026-11-05. Reviewed by Fable and Sonnet (their
points are folded in below); accepted by Giorgos with the safeguards for a
user who turns it off (rules 10 and 10a), and his yes to `reqwest`,
`serde_json` and `semver` in the core.

## Context

OurStick does not check for updates (SECURITY.md, FAQ, privacy page). New
versions are on the site and on GitHub Releases, and a user learns of one
only by looking. That was fine while few people used it. It is not fine for
a security fix: the review before the repository went public fixed problems
a stranger with a link could use (ADR 0031, 0032), and a user still on an
older version is open to them until they happen to download a new one.

What each system gives today:

- **Android and iOS:** the stores update the app, usually within days, when
  the user's settings allow it. Nothing to do in the app. (An APK taken from
  the site before the app is on Play gets nothing; see Consequences.)
- **Windows and macOS:** nothing. The installers are a `.exe` and a `.dmg`
  from the site.
- **Linux:** a `.deb` (amd64) from the site; `apt upgrade` does not know it.

What OurStick has, and keeps (CLAUDE.md, "Τι ΔΕΝ κάνει η v1"): no accounts,
so no email to send; no server of its own (the site is static, on
Cloudflare Pages); no analytics (the site sets no cookies and loads nothing
from elsewhere).

The keys to sign releases exist (minisign, made 2026-11-05, kept by
Giorgos, never on a build machine; `release-keys/`). The release key
(`2DDEFEB70EAC0BBB`) signs `SHA256SUMS` from 0.2.1 on.

## Decision

Three steps, each its own release. Only the first is decided here; the
second and third are written down so that the first does not stand in their
way.

### 1. A notice in the app (desktops only)

**What is read.**

1. Once a day (at a random minute, not at every user's login second), and
   at start if there was no good look in the last day, the app reads one
   file: `https://ourstick.com/version.json`. It holds the notice and its
   signature together, so that a cache can never serve one without the
   other.
2. The request carries nothing about the computer: no version, no id, no
   cookie, no query, no `User-Agent` with a version, no `Referer`, no
   `If-None-Match` or `If-Modified-Since`. HTTPS only, no redirect, at most
   64 KiB read, 15 seconds. It goes through the HTTP(S) proxy the
   environment names, as the relays do (ADR 0006).
3. The signature is checked against the public key built into the core
   before anything in the notice is parsed, and the signature's key id must
   be that key's. The signature is a minisign one of the legacy kind
   (`Ed`, the notice's bytes signed as they are); any other form is
   refused, so a notice signed the wrong way is seen at the release, not
   ignored by every app.
4. **The notice** names, for `windows`, `macos` and `linux` (the `.exe`,
   the `.dmg` and the amd64 `.deb`): `newest`, the newest version, and
   `security`, the oldest version that is safe; and `serial`, a whole
   number that only grows, and `made`, the day it was made. It names **no
   page and no address**: the app opens its own fixed download page for
   its system (ADR 0024 rule 11: the window never passes a URL). A notice
   with a version that is not plain `X.Y.Z`, a `security` above `newest`,
   nothing for this system, or a `serial` not a whole number is ignored
   whole. Unknown fields are allowed (a later version may add some).
5. **Old notices.** The highest `serial` seen is kept, with the id of the
   key that signed it, in the database (`settings`); a notice with a lower
   `serial` is ignored, so an old signed one served again cannot hide a
   newer one, and is not a good look either: About's "last looked" stays
   as it was and says an older notice came, so an old file served again
   for days is seen as a notice withheld. A new key starts its own count.
   A notice withheld cannot be seen; that is today's state, and About says
   when the app last looked.

**What is shown.**

6. Versions are compared as versions (`0.10.0` is after `0.9.0`), never as
   text. Nothing is shown when this version is the newest, or newer (a test
   build, or the minutes after a release before the notice is made).
7. A newer version: Home shows a line at its foot, beside the version, in
   the footer's muted style, and the tray menu the same line above About:
   "OurStick 0.3.0 is out · Get it".
8. This version older than `security`: the line reads "0.3.0 fixes a
   security problem in this version · Get it", in the amber of the app's
   warnings, in the tray too; it is told once in the window the first time
   it is seen, also when the app was started at login with no window (as a
   due thing makes such a window show, ADR 0024 rule 3); then it stays at
   the foot until the app is updated. Never a dialog, never a sound, never
   a system notification.
9. `Get it` opens the app's fixed download page for its system in the
   browser. The app downloads and installs nothing at this step.

**What the user controls and is told.**

10. About has "Look for new versions", on by default, with one sentence:
    "Once a day OurStick asks ourstick.com which version is newest. It
    sends nothing about you or your files." and a line "Last looked: today"
    (or "Could not look since 3 days: no connection to ourstick.com").
    Turned off, it says under the switch, in the amber of warnings: "With
    this off, you may miss a security fix. Check ourstick.com yourself now
    and then." (worded so on 2026-11-06). Off, the app reads nothing. A newer version it knew of is
    forgotten, but not a security line: one already seen stays at Home's
    foot until this version is no longer below `security`, as knowing it
    tells no one anything. Turning it off is never a way to stop being
    told that the version in use is unsafe.
10a. **Off, an old version still says so, with no network.** The app knows
    the day it was released (built in by the release step; a debug build
    has none and shows nothing). With "Look for new versions" off and that
    day more than 90 days ago, Home's foot shows, in the muted style:
    "This version is from March 2027. A newer one may fix security
    problems · ourstick.com", where `ourstick.com` opens the fixed download
    page. It is not shown while looking is on (the notice says better), and
    it asks nothing of the network. A clock set before the release day
    shows nothing.
11. A device that already has OurStick is told once, in the window, at the
    first start of the version that brings this: "OurStick now looks for a
    new version once a day, so you won't miss a security fix. You can turn
    this off in About." (worded so on 2026-11-06, About saying that it
    sends nothing about you; ADR 0024's told-once way.)
12. The privacy page, the FAQ and SECURITY.md say it: once a day the app
    asks ourstick.com for one small file and sends nothing else; Cloudflare,
    which serves the site, sees the computer's address and the time, as for
    any visit, and keeps its logs by its own policy; the file is served with
    no analytics, scripts or cookies, like the rest of the site.
13. A debug build, a build with `OURSTICK_DATA_DIR` and the tests never
    read the real file; a debug build may be pointed at a test file and a
    test key, a release build never reads such a setting.

**How it is built** (CLAUDE.md).

14. The core decides: verify, parse, compare, keep the highest serial,
    choose what to show. The reading goes through a trait with an HTTP
    implementation and an in-memory one for the tests (rule 2). What is
    kept is in SQLite (rule 3). The app only shows, and opens its fixed
    page with a command that takes no argument.
15. Dependencies: an HTTP client and a JSON parser are already in
    Cargo.lock through iroh and the workspace (`reqwest`, `serde_json`), and
    `semver` too; they become dependencies of the core (Giorgos's yes,
    2026-11-05), which adds no crate. The minisign check is written in the
    core with `ed25519-dalek`, which it has (about 40 lines).
16. Phones do not compile it.

### 2. Installing with one press (Windows, macOS) — later, its own ADR

Tauri's updater downloads the new installer, checks its signature, installs
it and starts the app again. It is what closes the gap the notice leaves:
the notice sends users to installers that are not signed on Windows and
Linux, from the same place as their sums (SECURITY.md), so a compromise of
the site alone cannot forge the notice but can swap what it points to.
Step 2's own ADR decides its key (see Keys), how its file relates to
`version.json` (Tauri's updater reads its own `latest.json`, whose JSON it
does not check; hence the different name here), and how signing stays off
the build machine (`tauri signer sign` after the build). Linux: an APT
repository or Flathub.

### 3. Refusing what is too old — open

A security problem in the protocol itself may need a new version to stop
talking to old ones ("The other computer must update OurStick"). Part of the
mechanism exists (the manifest has a format version, and readers refuse one
they do not know); a version that peers tell each other at connection is
the missing piece. Not decided here. It is also the net for a user who
turned the notice off (rule 10): the peer says "update", so a protocol
problem reaches them whatever their setting.

## Keys

- **The release key** signs `SHA256SUMS` and the notice. Its public key is in
  the repository (one file, which the core's build reads: one source), on
  the site and in the README.
- **The recovery key**, made at the same time and kept on paper, apart,
  never used: its public key is built into the core too. A notice it signs
  is accepted only to say that the release key changed and that the new
  version must be installed by hand. Without it, a lost or stolen release
  key would leave exactly the users on a vulnerable version out of reach.
- **Stolen release key, step 1:** the thief can sign a false notice: a
  false "0.9.0 is out", or a false security line on every computer until
  the key is replaced (a nag, not a hazard). It cannot make the app install
  anything, nor send anyone anywhere but the app's own download page. It
  could also push the serial very high, so that the real key's notices are
  ignored; the recovery key's notice is how the app hears of a new key
  then, and the new key starts its own count.
- **Step 2:** a stolen installer key would be code on every computer. Step
  2's ADR decides whether installers get their own key (likely) rather
  than this one.
- **Lost key:** the recovery key's notice, then a version with a new release
  key, installed by hand.

## Consequences

- A user learns of a security fix within a day of its release, in the app,
  without an account and without the app telling anyone anything about
  them.
- The app makes one request a day that it did not make before, the first it
  makes to ourstick.com itself; users are told once, and can turn it off.
- A release has one more file to sign and publish, `version.json`, signed by
  Giorgos at the key (never on a build machine): a notice, and so a security
  notice, goes out only when he is at it. It goes up after the installers
  and the release notes, never before.
- The notice cannot reach an app that cannot reach ourstick.com; About says
  so, and a user who sees "Could not look" knows to check by hand.
- Phones are left to their stores. Until the app is on Play, an APK taken
  from the site gets no notice.

## Alternatives considered

- **GitHub's API or Releases feed:** not signed by us, cannot carry a
  security floor, and puts a third party in every install, every day.
- **A DNS TXT record:** no signature without DNSSEC checked in the app;
  every resolver on the way sees and keeps it.
- **No network at all**, a line that says how old this version is: costs
  nothing in privacy, but cannot tell of a security problem, which is the
  reason for this ADR.
- **Tauri's updater in check-only mode:** its `latest.json` is not signed,
  only the installers are; this notice is signed whole.
- **Two files (`latest.json` and `latest.json.minisig`):** a cache can serve
  one without the other; one file cannot.
- **A URL in the notice, checked:** a parser's rule (host, user, port,
  `/s/#…`) where the app's own fixed page needs none.

## Not decided

- Step 2 (installing, and its key) and the Linux repository.
- Step 3 (a protocol version, refusing old peers).
- Whether a security notice links to what the problem was (an advisory on a
  fixed page of the site) or only says that there is one.
- What a user whose system is too old for the new version is told (a
  minimum system per platform, or a FAQ entry).
- A second place to look when the site cannot be reached (a GitHub release
  asset; GitHub would then see the address).
- An administrator's way to turn it off outside the app (not for v1).
- Whether the CLI ever looks (`ourstick --version` stays local; at most an
  explicit command later).

## Order of work

1. ~~The two keys (Giorgos); `SHA256SUMS` signed.~~ Done 2026-11-05
   (`release-keys/`, `tools/release/sign.sh`).
2. `version.json` made and signed by the release step, after the
   installers and notes; it refuses a serial not above the last and a
   `security` above `newest`. `_headers`: `Content-Type: application/json`,
   `Cache-Control: max-age=600`.
3. The core: the check, the trait, the settings migration, the schedule
   (daily with a random minute; after a failure again in an hour, never
   more than once an hour), with the tests below.
4. The app: the line on Home and in the tray, the told-once windows, About's
   setting with its warning and "Last looked", the old-version line of rule
   10a and its release day built in, the fixed download page per system;
   `docs/gui-spec.md`, `docs/schema.md`, the privacy page, the FAQ,
   SECURITY.md, and CLAUDE.md's list of what v1 does (Giorgos unlocks it).
5. Tried on all three systems with a test key and a test file, including a
   start at login with no window, a proxy, a captive portal (HTML instead of
   the file) and no network at start.

## Tests

- A good notice: checked, parsed, the line decided.
- Refused: a bad signature, another key, a truncated signature, a prehashed
  (`ED`) signature, a changed notice under an old good signature.
- A lower serial ignored; a higher one kept, per key, across a restart; the
  recovery key's notice accepted only as a key change.
- Versions: equal, older, newer, a test build newer than `newest`, a version
  that is not `X.Y.Z` (the notice is ignored), `security` above `newest`.
- The security line: older than `security` shows it, equal does not.
- Reading (in-memory source): HTML from a captive portal, a timeout, 404 and
  500, a file over 64 KiB; a failure does not count as a look.
- The schedule with a fake clock: due at start, not due, due a day later,
  again an hour after a failure, a clock moved back.
- Off: nothing is read (a counting source sees no call); a newer version
  is forgotten, a security line already seen stays until this version is
  safe.
- The old-version line (rule 10a): off and 91 days after the release day
  shows it, 89 does not, on shows it never, no release day (debug) never,
  a clock before the release day never.
- The log never holds the file's contents.
