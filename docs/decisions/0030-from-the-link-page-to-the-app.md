# 0030. From the link page to the app

Date: 2026-11-01. Status: accepted (reviewed by Fable and Sonnet, approved by Giorgos).

## Context

A computer that has OurStick gets a link in a mail or a chat. Today the
link opens `ourstick.com/s/`, which says "Get OurStick and open it" and
"Copy the link, then press Receive in OurStick". The user copies it, goes
to OurStick, and presses Receive.

Two things went wrong in Giorgos's own test (2026-11-01, Firefox on
Linux, the app already running):

1. The page offers a download to someone who already has the app. A page
   cannot learn which programs a computer has (browsers keep that from
   every site), so it cannot know.
2. OurStick was open on a received stick from before. The link was on
   the clipboard, but nothing happened until Receive was pressed: the
   clipboard is read only on that press, and when the window comes back
   while Receive's empty field waits (ADR 0028 §4, amended). That rule
   says "never on another screen".

Rules that hold (CLAUDE.md): the ticket lives only in the fragment, never
in a query or a path; **never a custom scheme `ourstick://` that carries a
key**; a ticket is never logged. A link handed to a program on its command
line is also seen by every other program that lists processes, so it
never goes there.

What the systems give, without a new dependency (checked in the sources
of Tauri 2.12, its CLI, tao 0.37, tauri-plugin-single-instance 2.5 and
tauri-plugin-deep-link 2.6; the systems' and browsers' behaviour is to be
seen on the devices):

- **Registration.** The Tauri CLI could write it from
  `plugins.deep-link.desktop.schemes` in `tauri.conf.json`, but a
  `plugins` key there makes `generate_context!` need `serde_json` as a
  dependency of the app: not taken. By hand instead: our `.deb` desktop
  template gets `MimeType=x-scheme-handler/ourstick;`, `Exec=… %u` (the
  system starts the app with the address only with a field code) and
  `StartupNotify=true` (a launcher makes an activation token only then);
  our NSIS hooks write `HKCU\Software\Classes\ourstick` (`URL:OurStick`,
  `URL Protocol`, `DefaultIcon`, `shell\open\command` =
  `"$INSTDIR\OurStick.exe" "%1"`) after each install and update, and
  delete it on an uninstall that is not an update; macOS:
  `CFBundleURLTypes` in our `Info.plist`, which Tauri already merges.
- **Arrival.** On Linux and Windows the address is an argument: of the
  first process on a cold start, or forwarded by the single-instance
  plugin to the running app, which receives **only** the arguments and
  the folder, not the environment. On macOS it comes as
  `RunEvent::Opened { urls }`, on a cold start before the page has loaded.
- **Focus.** GNOME lets a window come to the front only with an
  activation token, which the browser gives the process it starts (in
  `XDG_ACTIVATION_TOKEN` or `DESKTOP_STARTUP_ID`); the running app does
  not get it through the plugin, so without more it shows "OurStick is
  ready" instead. Windows lets a process that is not in front only flash
  its taskbar button, but Tauri's `set_focus` (tao) takes the foreground
  by itself when it is refused. On Wayland the clipboard cannot be read
  by a window without focus.
- **Browsers.** The first time, a browser asks ("Open this link with
  OurStick?", with "always"). Opening a scheme from a frame needs a
  recent click (a few seconds). Without the app, Firefox shows its "not
  understood" page in the frame, not in place of ours; Chrome and Edge on
  Windows may offer the Store. Our site's CSP is `default-src 'none'`, so
  a frame needs `frame-src ourstick:`.
- **macOS** (from macOS 26) may ask the user once whether OurStick may
  read the clipboard without a paste; Receive's press reads it already.

## Decision

### 1. An address that opens the app and carries nothing

The app registers the scheme `ourstick` on Linux, Windows and macOS. The
**only** address it acts on is `ourstick:receive`, also written
`ourstick://receive` and either with a trailing `/`, compared on the raw
text (as read lossily from the arguments), ignoring ASCII case. It
carries no key, no stick, no sender: it says only "show OurStick's
Receive". One function decides (`receiving::scheme::is_receive`); the
first start, the second start and macOS's `Opened` all go through it,
and the other arguments are not looked at for this (an address cannot
pass `--autostarted`).

Any other `ourstick:` address is refused whole: not read as a link, never
logged, shown or put into an error (someone may have put a key in it);
the app comes to the front and does nothing else. Arrivals closer than
3 seconds to the last are dropped, so a page cannot keep raising the
window. Nothing logs arguments; `tao`, which logs opened addresses at
`trace`, is capped at `debug` in our log filter.

On `ourstick:receive` the running app comes to the front and opens
Receive through the path an opened link takes today (`receiving::opened`,
the `link-opened` event; a cold start asks with `waiting()` once the page
is up), with nothing in it: Receive then reads the clipboard itself, when
it has focus, as when pressed (one reader, ADR 0028 §4). It opens from
Home, Send, About and the view of a stick, sent or received. It does not
open over a dialog, over a Receive past "Paste the link" (finding the
sender, choosing, downloading), or while the app quits: there the window
only comes to the front.

To come to the front: on Linux the second process hands its activation
token to the running app before it exits (our own call on the session
bus, before Tauri starts, made only for `ourstick:receive`; the token is
handed to `windows::Activation` as the tray's is); on Windows Tauri's
`set_focus` does it, with no `unsafe` of ours. Where this fails, the
system's "OurStick is ready" brings it, and Receive reads the clipboard
when it gets focus.

### 2. A link found by itself is opened by one press

A link that Receive finds without the user pressing Receive (after
`ourstick:receive`, or by §4) shows "✓ We found a link you copied" and
**Open it**; only that press makes OurStick look for the sender. Looking
tells the sender's computer that this one exists and where it is, and the
name shown is the sender's own words; any page can put a link on the
clipboard with one click and then open `ourstick:receive`. A press of
Receive in the app looks at once, as today.

### 3. The link page: "Open in OurStick"

Step 2 of `ourstick.com/s/` gets **Open in OurStick** in place of "Copy
the link". In the same click it starts copying the link and opens
`ourstick:receive` from a hidden frame (the copy takes effect when asked;
the app reads only once it has focus), so that the page stays where
nothing opens it. If the copy fails: "Could not copy. Copy the address
from the address bar." as today. Under the button: "Nothing opened? Get
OurStick above, open it and press Receive. The link is already copied."
Step 1 says "Don't have OurStick yet?". The CSP gets `frame-src
ourstick:`. The page still never sends, stores or shows the link.

### 4. A new link found when the window comes back (Linux and Windows)

ADR 0028 §4 is amended: each time the main window gains focus after
having lost it, **on Home**, the clipboard is read once. A link opens
Receive (with §2's press) only if it is new:

- not the one last seen: the app keeps, in memory only (never logged,
  gone when it quits), the BLAKE3 hash of the last link's StickId, set by
  every read that found a link, Receive's press and §1 included; the
  StickId, because the same link copied again with a stray "." is the
  same;
- not a stick this computer receives, and not its own: then nothing
  happens, and it is remembered as seen.

The first focus after the app starts is not "coming back", so a link
from days ago does not open Receive at every start. Never on another
screen, never from the tray, never in the background, never on a timer.
`ourstick:receive` is not held back by "last seen": the user asked.

**Not on macOS** for now: there each such read may make macOS ask. It
stays as ADR 0028 says, and is decided again with what the Mac shows
(macOS 15.4 can tell whether the clipboard holds an address without
reading it; later). Phones keep ADR 0027's "Paste the link" and get links
through App Links.

## Consequences

- One click from the mail to "We found a link you copied" and one more to
  look at it, for someone who has the app; a browser question, once.
- No link on a command line, in a process list or in a log: the address
  carries nothing, the link travels by the clipboard, as today. What an
  `ourstick:` address holds is a stranger's text and is treated so.
- The site changes (needs Giorgos's yes to deploy). The app needs a new
  release; an old app with the new page: nothing opens, and the words
  under the button still lead to Receive.
- Uninstall removes the scheme (the installer; the `.deb` and the Mac app
  take their registration with them).
- No new dependency and no `unsafe`: neither the deep-link plugin nor
  `serde_json`; the registration is a few lines in each system's own
  files, and the Linux call uses `zbus`, already a dependency of the
  app. (A first draft had `AllowSetForegroundWindow`, an `unsafe` call
  and a `windows-sys` feature: not needed, see above.)
- Later, perhaps: Universal Links on the Mac (Safari opens the link in
  the app, no page, no clipboard), as App Links on Android.

## Order of work

1. `receiving::scheme::is_receive` with tests: the four forms in any
   case, longer and shorter ones, a fragment, a key-like tail, junk,
   non-UTF-8, `--autostarted` beside an address; the 3-second gap.
2. The desktop template (`MimeType`, `%u`, `StartupNotify`), the NSIS
   hooks, `Info.plist`; the generated `.desktop` and the installer's keys
   checked in the built packages; the log filter for `tao`.
3. Arrival: first start, second start (the token on Linux; on Windows
   seen to come in front without help), macOS `Opened`; Receive opened
   through `receiving::opened` with nothing, the screens that hold it
   back; §2's "Open it".
4. §4: the read on Home when focus comes back, the hash, "already have
   it"; not on macOS.
5. The link page and its CSP; ReceiveView's "Clicked the link and a web
   page opened?" words, the home page's and the FAQ's "copy the link and
   press Receive", `docs/gui-spec.md`, ADR 0028 §4's note.
6. Tested on this PC (GNOME on Wayland, Firefox as a snap and Chrome,
   "always" on and off), in the Windows VM (a fresh install and an update
   over the old one; Edge and Firefox) and on the Mac (a signed test
   build; Safari); each with and without the app running, and without
   the app installed. The site deployed after Giorgos's yes.

## Seen on the devices

**Windows** (2026-11-01, the VM, Windows 11 24H2, Edge): the installer,
as an update over 0.2.0, wrote the keys. `ourstick:receive` with the app
closed started it on Receive, in front; with the app running on Home it
opened Receive. A link from another computer on the clipboard, back on
Home after the window lost focus: Receive opened with "✓ We found a link
you copied" and `Open it`, and only that press found the sender; the same
link again opened nothing. In Edge the link page's button made Edge ask
"This site is trying to open OurStick." (`Open`, `Cancel`): so the hidden
frame and `frame-src ourstick:` work; `Open` brought OurStick in front of
Edge, on Receive.

**macOS** (2026-11-01, macOS 26.7.1, a signed test build, Safari): Tauri
merged `CFBundleURLTypes`. `ourstick:receive` with the app closed started
it, in front; with it running, `Opened` brought it in front on Receive.
With a link from another computer on the clipboard Receive showed "✓ We
found a link you copied" and `Open it`, and macOS asked nothing about the
clipboard for that read; `Open it` found the sender. In Safari the link
page's button brought OurStick in front of Safari, on Receive.

Still to see: Firefox and Chrome on Windows, and Linux (GNOME, Wayland,
the token), on this PC once its big transfer is done.
