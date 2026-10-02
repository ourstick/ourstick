# 0010: Files this computer cannot save under their names

Status: Accepted (2026-10-02)

## Context

A stick is made on one system and received on another. A name the
sender's system allows may be one the receiver's refuses: on Windows
`< > : " | ? *`, a trailing dot or space, `CON`, `COM1` and the other names
Windows keeps for itself; anywhere, a name longer than 255 (bytes, or
UTF-16 units on Windows), or two names that are one file on a disk that
ignores letter case or Unicode normalization (`Photo.jpg` and `photo.jpg`).
A file already in the receiver's folder can also be in the way of a path
(version 1 has the file `a`, version 2 adds `a/b`).

Today `check_manifest_paths` refuses the whole manifest for one such name
(`DownloadError::UnsafePath` / `PathCollision`): nothing is received, and
the receiver waits for the owner's next version (`Waiting::CannotTake`).
Found in the manual test of 2026-10-02: a stick from a Mac with one file
named `…*"¨><?…` gave a Windows receiver none of its 20 files. A Mac user
puts `?` or `:` in a name easily; the people OurStick is for cannot tell
why nothing comes, and should not have to ask anyone to rename anything.

## Decision

1. **Every file comes.** A file whose name this computer cannot save is
   saved under a name it can: the smallest change that makes it savable,
   chosen by the receiver, with no question asked.
2. **The change is small and predictable**, per name (file or folder), by
   one function, `safe_name(name, rules)`:
   - on Windows, each character Windows does not allow becomes `_`
     (`What a view?.jpg` → `What a view_.jpg`; `???.txt` → `___.txt`), and
     a name Windows keeps for itself gets `_` after its stem
     (`CON.txt` → `CON_.txt`);
   - on Windows, a trailing dot or space is dropped (`_` if nothing is
     left; `stick_names::folder_name` drops them everywhere, as today, since
     a stick's folder may be copied to Windows);
   - everywhere: `c:` at the start becomes `c_`, a first name OurStick uses
     itself (any starting with `.ourstick-`, such as `.ourstick-partial`
     and `.ourstick-received`) gets `_` in front.
     (`c:` everywhere, because a stick must not hold a drive on any
     receiver; any other `:` only on Windows, where it is not allowed.)
   - a name too long (255 UTF-16 units on Windows, 255 bytes elsewhere) is
     cut on a character boundary, keeping its extension (unless the
     extension takes more than half the room), with room left for a ` (n)`; after the
     cut the rules above run again.
   `safe_name` is idempotent and its result always passes
   `component_problem`; if no rule can make it so, the name is `_`.
   `stick_names::folder_name` uses it too: one rule for every name OurStick
   writes (it keeps its own `/` and `\` replacement and its `Stick` fallback
   for an empty name, `.` or `..`, which `safe_name` never sees).
3. **Then, ` (n)` where two names would be one file here**, as
   `drive::free_name_where` does (`photo.jpg` → `photo (2).jpg`; the folder
   `a` → `a (2)`): with another entry of the stick, with a name already
   recorded for this folder, or with what is already in the folder: a file
   where a folder must go (for a recorded folder too: its files move), and,
   for a changed name, anything at that place. A name that tries 1 000
   numbers and finds no place (every place in the way, e.g. a folder of the
   stick that is now a link elsewhere) is left out and told. Two names are
   one file here if they are equal once folded as this folder's disk folds
   them: letter case only where the folder ignores it (asked once per round
   with a name inside the folder, or inside the nearest folder above it
   that is there, in the other case; when nothing tells, folded), Unicode
   normalization on macOS only (a platform rule, not asked). A Linux folder on a case-sensitive disk keeps
   `photo.jpg` and `Photo.jpg` as they are.
4. **Who keeps its place:** first the names already recorded (their places
   stay theirs), then every entry whose name needs no change, then the rest
   in manifest order (byte-sorted, so stable). A genuine `a_.jpg` in the
   same version as `a?.jpg` keeps its name and `a?.jpg` becomes
   `a_ (2).jpg`; a genuine `a_.jpg` that arrives after `a?.jpg` was saved
   as `a_.jpg` becomes `a_ (2).jpg` itself.
5. **A folder renamed once is renamed for all its files**, now and in later
   versions: `Trip?/` becomes `Trip_/`, and a file the sender adds to
   `Trip?/` next month goes into `Trip_/` too.
6. **A name, once chosen, stays.** The receiver records where it saved each
   file; a later version that changes the file writes it there. Every
   recorded place is taken for the round whether or not its file is still
   on disk, so a place is never given to two entries; rows of files no
   longer in the stick whose file is gone are forgotten first, before the
   round's names are chosen and before anything is recorded. Recorded places
   that this round's folding makes one (rows made on a case-sensitive disk,
   the folder now on one that is not): the byte-first keeps its place, the
   other gets a fresh ` (n)` and its row is updated, logged.
7. **Nothing is overwritten or deleted.** If the sender later renames the
   file, the new name comes as a new file; the copy under the changed name
   stays, like any file no longer in the stick (as today).
8. **The receiver is told, briefly**, from the latest round's plan (only
   names in the stick now: a renamed copy the sender's own rename left
   behind is no longer listed); after a restart, until a round has planned
   (the sender may be away), from what is recorded. Under the received stick's state:

   > 414 files have slightly different names here than at the sender:
   > Day 1/What a view?.jpg → Day 1/What a view_.jpg
   > Notes/CON.txt → Notes/CON_.txt
   > Trip?/ → Trip_/ (412 files)

   The first 10 lines, then "Show all". Each name isolated (U+2068/U+2069)
   and escaped. The preview says the same before Get it, computed the same
   way ("414 files will have slightly different names here"). The heading
   counts files; the lines are names (a renamed folder is one line).
   `get` prints one line per name ("“a?.jpg” is saved here as
   “a_.jpg”"), escaped, and a summary line, and exits 0.
9. **Safety does not change.** Every path written is built from savable
   names and checked again before it is joined to the folder; a path that
   still fails (a bug), or a file of the user's in the way of its folder,
   leaves that one file out (`FileLeftOut`, told in `get` and logged), and
   the round goes on: a round never fails for one file's place. Paths that could leave the folder are refused when the manifest is
   decoded (`\`, NUL, `.`, `..`, empty components), as today; nothing is
   written through a symbolic link in the folder: a folder of the stick
   where a link is gets ` (2)`, a file whose place is a link is left out.

Known limits: a folder on a Windows-formatted disk mounted on Linux or
macOS gets the rules of the system it runs on, so a `?` there still fails
when written (an error of the round, as today); paths longer than Windows'
260 characters are left to the system's long-path support.

## In the code

- `download/saved_names.rs` (new): `safe_name`, and `saved_paths(manifest,
  recorded, folder, rules) -> SavedPaths`: one walk, parents before
  children, a stick-prefix → folder-prefix map seeded from the recorded
  rows (each row's two paths must have equally many parts; a row that does
  not is ignored and logged); each candidate checked against the round's
  taken set (files and every folder prefix, folded, both directions: `a`
  and `A/b`) and against the disk (a file where a folder must go is in the
  way; a folder or nothing is fine), ` (n)` until free. Pure but for the
  disk lookups, which `plan_folder` passes in; tested everywhere with the
  rules forced.
- `check_manifest_paths` keeps only what decode does not already refuse
  and is no longer platform-dependent; `component_problem` stays as the
  final check of each written path; `PathCollision` goes; the partial and
  marker names are compared with the same folding as everything else.
- Store: `DownloadedFile.saved_as: String`, always filled (one stored form:
  equal to `path` when unchanged). Migration 0018 adds the column, sets it
  to `path` for every row, and adds a unique index on `(stick_id, saved_as)`
  (`ADD COLUMN saved_as TEXT NOT NULL DEFAULT ''`, `UPDATE … SET saved_as =
  path`, `CREATE UNIQUE INDEX …`; the table is `STRICT, WITHOUT ROWID`);
  `ON CONFLICT … DO UPDATE` sets it too. New: `forget_downloaded_file`
  (one row; a re-numbered row is forgotten and recorded again once its file
  is at the new place) and `saved_elsewhere`. `docs/schema.md` says what each
  path is.
- Every place that touches the disk uses the saved path, every place that
  names the stick's entry uses the stick path: `plan_folder` (target, stamp,
  content hash, found rows), `no_longer_in_stick` (stick paths compared,
  saved paths tested on disk), `receive_file`'s early check, `written_here`,
  `place`, `copy_file`, `record` (partial files are keyed on the blob hash
  and need no change). An entry's own place holding a file of its content is "already
  here", so a crash between placing and recording does not rename it again.
- The file events name each file where it is saved here (its place in
  the folder); `DownloadEvent::SavedAs { path, saved_as, files }`, once per
  renamed name (a folder once) before the files come, links the stick's
  names to them. `DownloadReport.saved_as` counts the files. The round is
  complete as before. The records keep the stick path.
- Windows' own rules are not forced in the integration tests (the reader
  takes this system's); they are unit-tested in `saved_names` and
  `saved_paths`, and met for real when the tests run on Windows.
- `Preview.saved_as` from the same `saved_paths` call, with the rows of
  that folder (none for a new folder; a CLI `--out` to another folder
  starts from none there too).
- `DownloadError::UnsafePath` remains for a symbolic link in the way;
  `Waiting::CannotTake` for a manifest that cannot be read, its words and
  doc comment narrowed; the docs of `check_manifest_paths` and
  `target_path` narrowed.
- `transfer.rs`'s `reader_refuses_unsafe_names_before_writing_anything`
  becomes "a name this computer refuses is saved under a savable name".
- No protocol change.

## In the window and the CLI

- The notice comes from each round's `SavedAs` events, kept per stick by
  the receiver (`Receivers::saved_as`, reset when a round plans, in
  `told.rs`); before a round has planned, from `Store::saved_elsewhere`
  (filtered in the database) grouped by `download::saved_elsewhere`.
  Words in `SavedAsNote.svelte`, names escaped in `receiving::words`. `ReceivedView.svelte` shows
  it; the stick list does not. `AddLink.svelte` shows the preview's;
  `api.ts` gets the types. `receiving::words` drops its `UnsafePath` and
  `PathCollision` wording; `told.rs` and `progress.rs` take the new event.

Owners leave every name starting with `.ourstick-` out of their sticks
(any letter case), as receivers take such names for OurStick's own: a
user's `.ourstick-notes` is not shared.

## Consequences

- A receiver gets every file, always, and is told where the few renamed
  ones went.
- A sender never has to rename anything for a receiver.
- One more column in `downloaded_files`; a folder holds files whose names
  differ slightly from the sender's, by OurStick's choice, recorded.
- A sender's later rename leaves the earlier copy behind (rule 7), as any
  file no longer in the stick.

## Order of work

1. Core: `safe_name` (every rule; Greek, emoji and RTL names; `???.txt`;
   cutting with the extension kept and room for ` (n)`; idempotent and
   `component_problem`-clean for any input), shared with
   `stick_names::folder_name` (its tests updated); tests.
2. Core: `saved_paths` (recorded places first, then unchanged names; folder renamed for all its
   files; numbering under the length budget; `a` and `A/b`; a file in the
   way; recorded names kept, recorded places taken even with their file
   gone and forgotten first; two recorded places that fold equal; case kept
   on a case-sensitive folder; NFC/NFD on macOS); tests and
   `every_script`.
3. Store: migration 0018 (an upgrade from 0017 with rows), `saved_as` in
   `DownloadedFile`, `SqliteStore`, `MemoryStore`, `saved_elsewhere`,
   `forget_downloaded_file`; tests.
4. Core: plan / preview / fetch / place / copy / record / partial / events /
   report with saved paths. Tests with Windows' rules forced (through
   `plan_folder_by`) and integration tests with two peers on this system: `a?.jpg`, `CON.txt`, `Trip?/x` and a genuine `a_.jpg` all
   come; a next version changing `a?.jpg` updates its file; a next version
   adding `Trip?/y` puts it into `Trip_/`; a genuine `a_.jpg` added after
   `a?.jpg` was saved as `a_.jpg` becomes `a_ (2).jpg`; the file `a` then `a/b`; a stopped
   round resumed with a renamed file; the user deleting a renamed copy;
   `no_longer_in_stick` with a renamed file; `transfer.rs` updated.
5. CLI `get` lines (escaped), `progress.rs`.
6. App: the notice (store, words, ReceivedView, Show all), the preview's
   (AddLink, api.ts), `told.rs`, `words.rs` cleaned.
7. docs/protocol.md (the reader's behaviour), docs/schema.md, docs/todo.md
   entry closed; status Accepted.
