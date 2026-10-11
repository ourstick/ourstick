# ADR 0023: On Windows, the stick by its name in File Explorer

**Status:** accepted — 2026-10-11. Reviewed by Fable and Sonnet; their
findings are folded in. Tried in the Windows 11 VM the same day (see
"Tests").

## Context

On Windows a stick is the folder `%USERPROFILE%\OurStick\<name>`, shown as
a drive letter with `subst` (`docs/gui-spec.md`). A `subst` letter has no
label of its own: File Explorer calls it after the disk it points into,
"Windows (O:)". Both per-user `DefaultLabel` keys were tried in the VM on
2026-10-02 and Explorer ignored them; a real volume (a VHDX) would carry a
name, but attaching one needs administrator rights. So the stick's name,
the one thing that tells it apart, is nowhere in Explorer.

Microsoft documents how a folder appears at the root of Explorer's
navigation pane under a name and an icon of its own, the way OneDrive does
("Integrate a Cloud Storage Provider",
https://learn.microsoft.com/en-us/windows/win32/shell/integrate-cloud-storage):
a shell folder registered in `HKEY_CURRENT_USER` (no administrator rights),
whose instance is shell32's folder shortcut
(`{0E5AAE11-A475-4c5b-AB00-C66DE400274E}`) pointed at a target folder. The
sync-root registration on the same page is for cloud providers that sync
placeholders (HKLM, a provider, a SID); OurStick is a plain folder and does
not need it.

## Decision

1. **Each mounted stick is an entry at the root of Explorer's navigation
   pane**, named as the stick, with OurStick's icon, opening the stick's
   folder: the Windows counterpart of the Linux bookmark. The drive letter
   stays as well, for now: it is a path any program can type, and the app's
   idea of "mounted" on Windows rests on it (`is_mounted`, `letter_of`, the
   stick's remembered letter). Dropping it, with "mounted" becoming "the
   entry exists" as on Linux, is a possible later step.
2. **The entry**, all under `HKCU\Software\Classes\CLSID\{C}` and
   `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer`, with the
   values of Microsoft's page. The class is written a second time, the
   same, under `HKCU\Software\Classes\WOW6432Node\CLSID\{C}`, where
   32-bit programs look: without it their Open and Save dialogs show a
   blank item (seen in the VM).
   - `CLSID\{C}`: default the stick's name; `System.IsPinnedToNameSpaceTree`
     = 1; `SortOrderIndex` = 0x42.
   - `CLSID\{C}\DefaultIcon`: the running executable's path, index 0
     (`REG_SZ`), so an update or a move is followed at the next mount.
   - `CLSID\{C}\InProcServer32`: `%SystemRoot%\system32\shell32.dll`
     (`REG_EXPAND_SZ`; the only value that is expanded).
   - `CLSID\{C}\Instance`: `CLSID` = `{0E5AAE11-A475-4c5b-AB00-C66DE400274E}`.
   - `CLSID\{C}\Instance\InitPropertyBag`: `Attributes` = 0x11,
     `TargetFolderPath` = the stick's folder as a full path, **`REG_SZ`**:
     a folder may hold `%` (a stick called "100% sure"), which a
     `REG_EXPAND_SZ` would expand into another place.
   - `CLSID\{C}\ShellFolder`: `FolderValueFlags` = 0x28, `Attributes` =
     0xF080004D (no SFGAO_CANDELETE or CANRENAME: the entry itself cannot
     be deleted or renamed by mistake; its contents can).
   - `Explorer\Desktop\NameSpace\{C}`: default the stick's name.
   - `Explorer\HideDesktopIcons\NewStartPanel`: `{C}` = 1.
3. **The class id `{C}` is derived from the stick id**, never stored: the
   first 16 bytes of BLAKE3(`"OurStick navigation pane entry" ‖ StickId`),
   written as a braced GUID. The same stick has the same entry at every
   start; two sticks of the same name have two entries.
4. **The list of what OurStick put there**:
   `HKCU\Software\OurStick\NavigationPane\<D>`, one value per entry, named
   `{C}`, holding the stick id in hex (a public key, already in the public
   part of every link and allowed in logs by ADR 0021; no secret).
   `<D>` is the first 8 bytes of BLAKE3 of the app's data folder, in hex,
   so two data folders of one Windows user (a development build beside the
   installed one, `OURSTICK_DATA_DIR`) never clean up each other's
   entries. The list is written **before** the entry, and removed **after**
   it, so a crash between the two never leaves an entry no list names.
5. **Written in one go with `reg import`**: each change writes one `.reg`
   file (UTF-16LE with a BOM, "Windows Registry Editor Version 5.00")
   into the data folder and runs `reg import <file> /reg:64` once, then
   deletes the file. Its escaping is fully specified (`\\`, `\"` inside
   strings; `hex(2):` for the one `REG_EXPAND_SZ`), so a name with `"`,
   `\`, `%`, a leading `/` or `-` arrives whole, and a test can compare the
   file's exact content. A removal is the same file with
   `[-HKEY_CURRENT_USER\...]` sections and `"{C}"=-` values. `/reg:64`
   keeps the 64-bit view whatever the app's own build. Every program the
   app runs on Windows (`reg`, and the existing `subst` and `attrib`) is
   started with `CREATE_NO_WINDOW` (through std's `CommandExt`, no `unsafe`),
   so no console window flashes.
6. **When:**
   - *mount* (the app's start for every app stick, Mount, a new stick, a
     stick moved into `~/OurStick`): the letter as today, then the entry,
     written in full each time (a moved executable or a key deleted by hand
     is mended by it);
   - *eject*: the letter goes, then the entry, **whether or not a letter
     still showed it** (after a logoff the letter is gone but the entry is
     not);
   - *rename*: the entry is **rewritten in place** (same `{C}`, the new name
     and folder) after the folder moved; if the folder could not move, it
     is rewritten with the old name and folder, as the letter is given back;
   - *new link*: once `give_new_link` succeeds, the old id's entry is
     removed and the stick, under its new id, is mounted (its letter is
     already there; the new entry is written);
   - *remove*: through eject, as today, before the folder goes.
   A failure to write or remove the entry is logged and told the way a
   bookmark that could not be made is told on Linux (`unplugged`); it never
   stops the mount, the letter or sharing.
7. **Left behind**, at the start, before mounting: each value of this data
   folder's list is checked: its name must be a braced GUID, its data a
   stick id, and deriving `{C}` from that id must give the name back;
   anything else is skipped and logged, never deleted beyond the list
   value itself. A checked entry whose stick is not an app stick of this
   store (`kind = app`) is removed: the entry, then the list value. A
   stick that is still an app stick keeps its entry; the mount that follows
   rewrites it.
8. **Uninstalling** (NSIS, a `NSIS_HOOK_POSTUNINSTALL` hook in
   `bundle.windows.nsis.installerHooks`): for every subkey of
   `Software\OurStick\NavigationPane`, every value name that is a braced
   GUID has its four places deleted (both views of the class, the
   namespace key, the desktop value); then `Software\OurStick` goes. Not
   on an update: Tauri's NSIS installer runs the old uninstaller with
   `/UPDATE` first, and the hook checks `$UpdateMode`. The `.msi` has no
   hook: an entry left by it points at a
   folder the uninstall keeps (the sticks are the user's), so it still
   opens, with a blank icon; the next install cleans what no stick owns.
   Another Windows account's entries are its own HKCU: they stay, the same
   way.
9. **No shell notification.** Explorer windows already open may show a
   change only when opened again; `SHChangeNotify` would need `unsafe`,
   which this ADR does not ask for. If the VM shows it matters, it is a
   separate question for Giorgos.

## Consequences

- Explorer shows "Holiday photos" with the OurStick icon at the top level of
  its pane, opening the stick; `O:` stays, as "Windows (O:)".
- One `reg import` per mount, eject or rename (tens of milliseconds each);
  at the start one per app stick, off the main thread as mounting is today.
- `docs/gui-spec.md`, `docs/windows-dev.md` (the reg keys to look at) and the
  site's Windows picture change; the todo item "The Windows drive's name"
  closes.

## Tests

- Pure, on every system:
  - the class id: stable, a braced GUID, different for another stick;
  - the exact `.reg` content of an entry and of a removal, for names with
    `"`, `\`, `%`, `%PATH%`, a leading `/` and `-`, a Greek name, a folder
    path with spaces; the list value written before the entry and removed
    after it;
  - the list subkey of a data folder (stable, different for another);
  - the start's clean-up for a list: a foreign or malformed value, an id
    whose `{C}` does not match, a stick that is still an app stick, and one
    that is not; only the last is removed.
- In the Windows VM (Windows 11 LTSC 24H2), 2026-10-11, the app driven
  through UI Automation, all as expected unless said:
  - a new stick appears in the pane by name and icon and opens its folder;
    copying in, deleting and renaming files through it work, and the
    receiver gets them;
  - Eject removes it, also after a logoff; Mount brings it back;
  - Rename shows the new name; a rename refused (a file open) keeps the
    old one;
  - New link: one entry, still the same name;
  - Remove takes it away; a Greek name and "100% sure";
  - after killing the app, an entry of ours for a stick not in the store
    and a foreign list value were planted; the next start removed the
    entry, and only the list value of the foreign one (its key stayed);
  - an Open dialog of a 32-bit program: it showed a blank item until the
    class was written for 32-bit programs too (§2); no icon on the Desktop;
  - an NSIS update (with and without `/UPDATE`) keeps the entries; an NSIS
    uninstall removes all of them and keeps the stick's folder and the
    app's data.
  - Not tried: Eject after a logoff (the code removes the entry without
    looking for a letter), "Show all folders", and whether a console
    window flashed before (`CREATE_NO_WINDOW` is set either way).
  - A rename refused by Windows could not be caused on purpose (Windows
    renames a folder with a file open in it); the refusal seen when the
    folder had gone (renamed by hand) kept the entry as it was.

## Alternatives considered

- **A VHDX per stick:** a real volume with a name, but attaching one needs
  administrator rights, and its size would need managing (ADR 0008).
- **`DefaultLabel` under `DriveIcons`:** ignored by Explorer for a `subst`
  letter (tried, 2026-10-02).
- **A mapped network drive** (`net use` to the administrative share): it
  can be labelled, but needs the `C$` share and the user's credentials.
- **A shortcut in Quick Access:** pinning needs the shell API, and it shows
  in "Home", not at the pane's root.
- **`reg add` per value:** ten processes per mount, and `reg.exe`'s own
  escaping of data on top of the command line's; one `.reg` file is one
  process and one fully specified format.
