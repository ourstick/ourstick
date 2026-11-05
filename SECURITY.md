# Security

OurStick encrypts files on the sender's computer and sends them peer to
peer; the design is in [docs/protocol.md](docs/protocol.md) and the
decisions behind it in [docs/decisions](docs/decisions). It has not yet been
reviewed by an independent auditor.

## Reporting a vulnerability

Please write to **hi@ourstick.com** with "Security" in the subject, or use
GitHub's private reporting (the repository's Security tab, **Report a
vulnerability**), and do not open a public issue. Say what you found, how to reproduce it, which system
you used, and when you downloaded OurStick (or which release).

Never send a real sharing link: a link carries a stick's key. If a link is
part of the problem, describe it, or make a test stick and send its link
after removing the stick.

I will try to answer within a week. Once a fix is released, it is named in
the release notes, with credit to you unless you prefer not.

## Supported versions

Only the latest release gets fixes. From 0.3.0 on, the desktop app looks
once a day for a new version (ADR 0033): it reads one small file,
`https://ourstick.com/version.json`, signed with the release key
(`release-keys/`), and sends nothing else; Cloudflare, which serves the
site, sees the computer's address and the time, as for any visit. A
security fix shows as an amber line in the app. It can be turned off in
About. It installs nothing: you download the new version yourself.
Releases are also on
[GitHub Releases](https://github.com/ourstick/ourstick/releases): choose
Watch → Custom → Releases there to be told, or follow
`https://github.com/ourstick/ourstick/releases.atom`. Phones are updated by
their stores.

## What OurStick protects, and what it does not

What it is built to hold against:

- **Anyone on the network, relays included:** they see that two computers
  talk, never the files, their names or their sizes. Everything is
  encrypted on the sender's computer before it leaves
  ([docs/protocol.md](docs/protocol.md)).
- **Anyone who knows a sender's node id,** which is in every link's public
  part: they can connect, but what they may hold of the sender (requests,
  transfers, connections, memory) is bounded
  ([ADR 0031](docs/decisions/0031-what-one-peer-may-hold.md)). With the
  public part alone they can fetch the encrypted list of files, and learn
  when it changes and about how big it is, never a name.
- **A sender whose link you opened:** what it sends is checked before it is
  written. It cannot write outside the folder you chose, replace a file of
  yours, or fill your disk past the space you agreed to, and every file you
  receive carries the mark of a download from the internet on Windows and
  macOS ([ADR 0032](docs/decisions/0032-received-files-are-from-the-internet.md)).
  It does learn which of the paths it names already exist in that folder.
- **A former reader after a new link:** the new link has new keys; nothing
  of the stick reaches the old one again.

What it does not hold against, by design or for now:

- **Someone who has your link.** A link is the key: whoever has it can take
  the files, can keep the sender's transfers busy, and can fill the
  sender's list of who has the stick (200 lines) with made-up readers, so
  that new ones show only once those fall silent. The remedy is a new
  link, which ends every old one.
- **Programs running as you on your own computer.** On Linux and Windows
  they can read OurStick's keys from the system's key store, as they can
  read your files; on Linux they can also use the app's tray menu and its
  session D-Bus names (they could, say, turn "Start at login" on or off).
  macOS keeps the keys to OurStick itself. A program that can change files
  in a folder you share or receive into can also race OurStick's checks.
  On a Mac shared with another account, that account can keep OurStick
  from starting (the app finds its running copy through a fixed place in
  `/tmp`).
- **What you open.** A received file is what its sender made. The mark of
  the web makes Windows and macOS ask before a program runs, but a
  shortcut or an Internet Shortcut file (`.lnk`, `.url`) can reach out to
  the network when a folder showing it is opened, as with any download.
- **Signed downloads:** the Windows installer and the Linux package are
  not signed yet; the macOS app is signed and notarized. Their SHA-256 sums
  are signed with minisign from 0.2.1 on, with a key kept off every build
  machine (`release-keys/`), so a changed file on the site or on GitHub
  Releases shows when the sums are checked against the key in the
  repository; the app does not check them itself.
