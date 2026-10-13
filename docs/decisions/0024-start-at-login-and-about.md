# ADR 0024: Start at login, and an About window

**Status:** accepted — 2026-10-12. Reviewed by Fable and Sonnet; their
findings are folded in.

## Context

A stick is served only while the app runs. After a restart of the
computer nothing starts it: every stick stops, and its receivers see
"Waiting for the sender" until the sender opens OurStick by hand. Most
people will not think of it; a USB stick does not need it.

The app also says little about itself: macOS has the system's "About
OurStick" panel, Windows and Linux nothing. Its version is at the foot of
the sidebar (0.1.1), but nothing points to the website, the privacy page,
the licences (`THIRD-PARTY-LICENSES.html` is installed beside the app and
nothing opens it; OurStick's own Apache-2.0 text is not shipped at all),
or how to report a problem.

`tauri-plugin-autostart` 2.7 (on `auto-launch` 0.6) was read first. It
writes the program's path unquoted into Windows' `Run` value and into the
Linux entry's `Exec=` (a path with a space, as in
`C:\Users\Maria Papadopoulou\…`, breaks, and an unquoted `Run` path is a
known Windows weakness), puts an unescaped path into the macOS plist's
XML, and on every enable overwrites a "Disabled" the user set in Task
Manager. What it does is three small files or values, which the app can
write itself, as it writes registry entries (ADR 0023) and bookmarks.

## Decision

### Start at login

1. **On by default**, for a new device and for one updated from 0.1.0. The
   choice is kept in the store, in a new one-row table `settings`
   (`start_at_login`, `start_at_login_told`), not in `device`: that row
   exists only once a first key is written, and the choice must hold
   before. No row means the defaults: on, not told. The CLI ignores it.
2. **The first start that turns it on tells it once**, in the window: "OurStick
   now opens when you log in, so that your sticks are shared after a
   restart. Turn it off in the tray or in About." (`start_at_login_told`
   then 1). On macOS this explains the system's own notice that a login
   item was added.
3. **Started at login (`--autostarted`), the app opens no window**: the
   main window is created hidden (`"visible": false` in the config) and
   `setup` shows it unless the start was `--autostarted` and nothing is
   due first: the keys' move (ADR 0021), macOS's Local Network step (ADR
   0020), the notice of rule 2. Sticks are plugged in and shared as at
   any start. A second start while the app runs (single instance) does
   nothing if its arguments hold `--autostarted`, and shows the window
   otherwise, as now. On macOS a click on the Dock icon shows the window
   (`RunEvent::Reopen`; today it does nothing). On Linux, if the tray
   cannot be registered (no StatusNotifier host yet at login), the app
   tries again for 30 seconds, then shows its window rather than run
   unseen.
4. **Who writes the entry.** At every start, the app makes the system's
   entry match the choice, but only when it is the installed app:
   - a release build (`cfg!(debug_assertions)` off), without
     `OURSTICK_DATA_DIR` (the entry starts the default data folder only),
     not from an AppImage (`APPIMAGE` set; it is not shipped);
   - its executable in an install location: Windows, not under a `target`
     folder; macOS, inside an `.app` under `/Applications` or
     `~/Applications` (not a disk image under `/Volumes`, not an App
     Translocation path); Linux, `/usr/bin/ourstick-app`.
   Any other build neither writes nor removes it; its tray check shows the
   choice but changes only the store. A failure is logged and never stops
   the start.
5. **Written only when it differs.** If the entry is missing or names
   another path, it is written; if it is ours already, it is left as it
   is. If the choice is off, our entry is removed.
6. **The system's own "off" is kept.** A user may turn the item off in the
   system's settings rather than in OurStick: Task Manager
   (`StartupApproved\Run\OurStick` disabled), GNOME (`X-GNOME-Autostart-
   enabled=false`) or KDE (`Hidden=true`) in our file, System Settings →
   Login Items on macOS. The app never undoes that at a start. The tray
   check shows the choice as the system sees it where it can be read
   (Windows, Linux): off if the system disabled it. Turning the check on
   in OurStick is the user's own act, and then clears the system's off
   (Windows: `StartupApproved` enabled; Linux: the file written anew);
   macOS's own switch cannot be read or set without `unsafe`, so there
   the check shows OurStick's choice and About says where the system's
   switch is.
7. **Per system:**
   - **Windows:** `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`,
     value `OurStick` = `"<path of ourstick-app.exe>" --autostarted`, the
     path in quotes, through one `.reg` file and `reg import /reg:64`
     (ADR 0023's helper); read back with `reg query`.
   - **macOS:** `~/Library/LaunchAgents/com.ourstick.OurStick.plist`:
     `Label` = `com.ourstick.OurStick`, `ProgramArguments` = the
     executable inside the bundle and `--autostarted` (XML-escaped),
     `RunAtLoad` true, `LimitLoadToSessionType` = `Aqua`,
     `ProcessType` = `Interactive`, `AssociatedBundleIdentifiers` =
     `com.ourstick.OurStick` (on macOS 13 and later the Login Items list
     should then name OurStick); never `KeepAlive`, so that Quit stays
     quit.
   - **Linux:** `$XDG_CONFIG_HOME/autostart/ourstick.desktop` (else
     `~/.config/autostart/`): `Type=Application`, `Name=OurStick`,
     `Icon=ourstick-app`, `Exec=` the path and `--autostarted` quoted by
     the Desktop Entry rules in both layers (an argument with a reserved
     character in `"…"`, with `"`, `` ` ``, `$`, `\` escaped by `\`, and
     `%` as `%%`; then every `\` doubled as a string value),
     `X-GNOME-Autostart-Delay=3`; no `X-GNOME-Autostart-enabled` key.
8. **The tray's "Start at login"** check (above Quit) and About's change
   the store and the entry at once, off the main thread.
9. **Uninstalling:** the NSIS uninstaller deletes the `Run` value and its
   `StartupApproved\Run` value (our hook, not on an update; Tauri's own
   step also deletes the first). Only the uninstalling Windows user's;
   another user's entry names a program that is gone, which Windows skips.
   A Mac app dragged to the Bin, or the `.deb` removed, leaves the
   LaunchAgent or the autostart file: the system skips it at login. The
   site's FAQ gets "How do I uninstall OurStick?", with those files.

### About

10. **An About window** (label `about`, one at a time, not resizable),
    from the tray ("About OurStick", above "Start at login"), from the
    version line at the foot of the sidebar, and on macOS from the app
    menu's "About OurStick" (in place of the system panel, so that the
    three systems show the same). It shows:
    - the icon, "OurStick", the version (`getVersion`, as the sidebar),
      "A USB stick in a link.";
    - the bundles' `copyright` string, read from the app's configuration
      (one place: `tauri.conf.json`);
    - links: ourstick.com, Privacy, "OurStick's licence" (Apache-2.0),
      "Licences of the libraries in OurStick", "Report a problem"
      (`mailto:hi@ourstick.com`);
    - the "Start at login" check, the same as the tray's, and on macOS a
      line on where the system's own switch is.
11. **Links are opened by the app, not by the page**: one command takes a
    fixed name (`website`, `privacy`, `licence`, `libraries`, `report`)
    and opens its fixed target; the page never passes a URL or a path.
    The two licence files are installed resources (`LICENSE` is added
    beside `THIRD-PARTY-LICENSES.html`), opened with the system's default
    program, the site's copy if the file is missing. The `about` window
    is added to the capabilities, with the same permissions as the
    others.
12. **The licences shipped are completed:** `THIRD-PARTY-LICENSES.html`
    gains the JavaScript the app's page bundles (`@tauri-apps/api` and
    whatever else `npm` puts in `dist`, MIT), and the `.deb` gets
    `/usr/share/doc/ourstick/copyright`. NOTICE files of Apache-2.0
    dependencies are checked to be reproduced.

## Consequences

- After a restart, sticks are shared again without anyone opening the
  app.
- A migration adds `settings`; `docs/schema.md` follows.
- `docs/gui-spec.md`: auto-start at login leaves "Out of scope"; the tray's
  items and the About window are described.
- No new dependency, no `unsafe`.

## Tests

- Pure: the Windows `.reg` content and its removal (a path with spaces, a
  Greek user name); reading `StartupApproved` and `reg query`; the plist
  (`Label`, a path with `&` and `<`); the Linux entry (both escaping
  layers: spaces, `"`, `$`, `\`, `%`) and reading the system's "off"
  from it; "written only when it differs"; which executables may write
  (debug, `OURSTICK_DATA_DIR`, `APPIMAGE`, `target`, `/Volumes`, App
  Translocation, the install locations); whether a start shows the window
  (`--autostarted` with nothing due, with the keys' move, the Local
  Network step or the notice due); the single-instance answer to
  `--autostarted`; the store's defaults with no row.
- By hand, on each system: install, restart the computer, the sticks are
  shared, no window opens; the check off, restart, nothing starts; on
  again. Windows: a user name with a space; Task Manager's Disabled kept
  at a start, cleared by the check. macOS: from the DMG (nothing
  written), from `/Applications`, Login Items, the Dock click, "Reopen
  windows when logging back in". Linux (GNOME): the tray at login, the
  Startup Applications switch kept. About: every link, the check in step
  with the tray's. The update from 0.1.0: the notice once.

### Tried

- Windows 11 VM, 2026-10-12, the app driven through UI Automation:
  installed, it wrote `Run` with the quoted path; the notice showed once;
  About's check off removed the value, on wrote it and Task Manager's
  switch as enabled; Task Manager's Disabled (written by hand) was kept at
  a start and the check showed off; after a restart the app started with
  `--autostarted` and shared the stick, with its window open only while
  the notice was unread, and none after; a second start with
  `--autostarted` did nothing, a plain one showed the window; the NSIS
  uninstall removed `Run` and Task Manager's value and kept the data;
  `LICENSE.txt` and the libraries' licences are installed. About first
  opened blank (a window made from a synchronous command on Windows), and
  Tauri leaves the bundle's copyright out of the app's configuration (read
  by build.rs instead): both fixed. Not tried there: the tray's items
  (the same code as About's), the links opening a browser.
- macOS and Linux: by hand, before the release.

## Alternatives considered

- **`tauri-plugin-autostart`:** a new dependency with the faults above.
- **`SMAppService` on macOS 13+** (the item goes with the app, and its
  switch can be read): needs the Objective-C runtime through `unsafe`.
- **Off by default, asked once:** most people would not turn it on, and
  their sticks would stop at every restart.
- **No main window at all until it is opened** (made on the first Open):
  no window at login on any system, but every place that sends to the
  main window would have to allow for its absence; a hidden window does
  the same with no change there.

## Amendment 1: the check on Quit too (accepted, 2026-10-13)

A friend wanted OurStick not to start with his computer, and found no way:
the tray and About are where few look. The Quit dialog (from the window's
✕, the tray, the Dock or ⌘Q) now has the same check, as About has it: it
changes the choice at once, whichever button follows. One choice, shown
in three places.
