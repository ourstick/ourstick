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
     `OurStick`), set with `/usr/bin/xattr`. OurStick takes no new
     dependency for it; the cost is one short process per file, beside
     the file's own sync.
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
