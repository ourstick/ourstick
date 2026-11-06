# 0032. Received files are from the internet

Date: 2026-11-04. Status: accepted (found by the security review before the
repository went public; Giorgos asked for every finding to be fixed).

## Context

A browser marks every file it downloads: Windows keeps a `Zone.Identifier`
stream with the file (the "Mark of the Web"), macOS a `com.apple.quarantine`
attribute. With the mark, Windows has SmartScreen look at a program before
it runs and Office opens a document in Protected View, without macros;
macOS has Gatekeeper check a program and asks before it opens.

OurStick wrote received files with no mark. A program, a shortcut, a script
or a document with macros from a stick then ran or opened with none of
these questions: the hole archivers had and closed (7-Zip and WinRAR, which
did not pass the mark on to what they unpacked).

## Decision

1. Every file a reader writes into its folder gets the mark a browser
   gives a download, before it is moved into place:
   - Windows: the stream `Zone.Identifier` with `ZoneId=3` (the internet).
     Writing it changes the file's time, so it comes before the time is set.
   - macOS: `com.apple.quarantine` as Safari writes it (`0081`, the time,
     `OurStick`), set with the `setxattr` system call through `rustix`
     (no unsafe code of our own). It was `/usr/bin/xattr` at first, one
     process per file, which a stick of millions of files would have paid
     for in hours (changed 2026-11-05, with Giorgos's yes to `rustix`).
   - Linux and Android have no such mark.
2. Best effort: a disk that cannot hold the mark (FAT, exFAT) keeps the
   file without it, and the download goes on.
3. Not the app's `LSFileQuarantineEnabled`: that would mark every file the
   app makes, its database and its disk image among them, and the files
   in the image would look downloaded to their own owner.

## Consequences

- Opening a received program, script or Office file asks first, as one
  from a browser does. Photos, videos and other plain documents open as
  before.
- A file copied from another of the stick's files (ADR 0014) is marked
  too.
- Files received before this version have no mark.

## Amendment 1: folders too (2026-11-06)

A received folder that is an app bundle (a `Tool.app` inside a stick) had
its files marked, but not the bundle itself. Now every folder a download
makes inside the stick's folder is put in quarantine as soon as it is
made, as Archive Utility does with what it unpacks; never through a link
(`lsetxattr`). macOS only: Windows has no mark for a folder, Linux and
Android none at all. The stick's own folder is made by the app and left
unmarked: its name never ends like a system bundle (`stick_names.rs` adds
a `_`), and since an app may take any other extension for its document
package (`Holiday.pages`), "Open folder" shows a received folder whose
name has an extension in Finder, selected, rather than opening it. A
folder that was there before is left as it is.

A received stick moved to another disk keeps its marks: a file through
`std::fs::copy`, which copies them, a folder by taking the mark its
original has. Folders received before this version have none.
