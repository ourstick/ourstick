# 0012: Choosing what to get of a received stick

Status: Proposed (2026-10-02)

## Context

A received stick comes whole. In the manual test of 2026-10-02 a Mac was
sent the sender's "OurStick", which held a 104 GB folder of films: the
preview said it does not fit, and there was no way to take the other
things on it (a few music folders, some documents). The receiver window
(step 7b) has no choice of what to get; `docs/gui-spec.md` names it step
7b-iv, and CLAUDE.md's v1 receiver program includes it ("λίστα sticks που
έλαβε, επιλογή φακέλων, πρόοδος").

## Decision

1. **The choice is by top-level entry**: each folder and file directly on
   the stick, as the preview already lists them (name, size). Ticked or not.
   Deeper choices (a folder inside a folder) are not in v1.
2. **What is left out is recorded, not what is taken.** A stick's choice is
   the set of top-level names the user un-ticked, each exactly as the stick
   names it (the manifest's bytes: a Mac's names may be in another Unicode
   form than a Windows one's; never the name it is saved under here, which
   ADR 0010 may change). Anything the sender adds later comes, as today; a
   top-level folder the sender renames (or copies to a system that writes
   names in another form) is a new name and comes too. A mother who
   un-ticked "Films" once does not have to think about the music folder
   added next month, and the window says so (rule 5).
3. **Where:**
   - in the preview (`Add a sharing link` → `Look at it`): every entry has a
     tick (a real, labelled check box), all ticked, largest first, with `All`
     and `None`; un-ticking updates "Needs up to X · fits" at once; `Get it`
     takes what is ticked. If nothing is ticked, `Get it` is off and says
     why ("Tick at least one thing."). Every top-level entry is listed, not
     only the first 200 (each one must be choosable).
   - on a received stick: `Choose what to get…` in its `⋯` menu, the same
     list with the ticks as chosen, and `Save`. It works with the sender
     away: the list is the latest round's entries (kept by the receiver)
     together with every name already un-ticked, even one not on the stick
     now ("not on the stick now", still un-ticked), so a Save never drops a
     choice it did not show. Save sends the whole set.
4. **Un-ticking something already here leaves its files where they are**
   (nothing is ever deleted by OurStick, as everywhere): it is no longer kept
   up to date, and its places stay its (ADR 0010: its recorded rows keep
   their places, so a re-tick finds `Films_/x` where it was, and nothing
   else is put there meanwhile). Ticking it again brings what is missing or
   changed.
5. **Words:** the preview's heading "What to get:", each line with its tick,
   name (isolated and escaped) and size; under the list "Needs up to X ·
   fits" for the ticked ones, and "New things the sender adds will come
   too." On a received stick with something un-ticked, under its state:
   "Not getting: Films, Old photos." (the first three, then "and N more"),
   and for one already here "Not updating: Films (it stays in the folder)".
   Saving a choice that un-ticks something already here says so: "Films
   stays in your folder, but is no longer updated. OurStick never deletes
   anything." A "no room" wait names what is coming, so a renamed big
   folder can be un-ticked right there (`Choose what to get…`).
6. **The command line gets everything, as today.** A choice belongs to the
   window's sticks (`kind = app`); `ourstick get` never writes it (a round
   reads it for every stick, and a stick of `get`'s has none),
   and refuses a stick the window receives, as today, in the words it has
   ("OurStick (the app) receives this stick, into …").

## In the code

- Store: migration 0019, `joined_left_out (stick_id, name)` (primary key
  both, `name` the first path component byte-exact as in the manifest;
  foreign key to `joined_sticks`; `received::forget` deletes its rows
  explicitly, as it does `downloaded_files`). `Store::left_out(stick_id)`,
  `Store::set_left_out(stick_id, &[String])` (replaces the set, one
  transaction). Both stores, tests.
- Core download: `download_over` reads `Store::left_out` every round, beside
  the recorded files (a choice saved while waiting counts at the next
  round). `saved_paths` takes the set of un-chosen entries (by index):
  their manifest paths are skipped in the unchanged and choose passes
  (their place is their recorded one, or none; `saved[i]` is the path, as
  for a left-out entry; not in `renamed`), while their recorded rows are
  kept and take their places as every row does. `plan_folder` gives them
  `FilePlan::NotChosen`: not fetched, not in `Planned` (so the space ledger
  and the bar leave them out), not "left out" or "changed here", not in
  `no_longer_in_stick` (they are in the stick; only the "Not getting" line
  tells). A displaced row of an un-chosen entry (folding, ADR 0010 rule 6)
  is forgotten and not recorded again until it is ticked: its copy stays
  where it was. `DownloadEvent::NotChosen { name, files, bytes }` once per
  top-level name; `DownloadReport.not_chosen` (files). Completion follows
  as today: every chosen file is here. Batches need no change (an un-chosen
  folder's files are contiguous in the byte-sorted manifest; a batch with
  no `Download` is never used).
- `Preview.entries` gain `to_write` per top-level entry (what that entry
  would write into the folder, files already there left out: the per-file
  plans grouped, also for a folder not made yet), so the window sums the
  ticked ones without asking again; `Preview` keeps its totals and lists
  every top-level entry. Its `saved_as` notice is computed with all ticked;
  the first round's `SavedAs` events are what counts.
- The receive loop tells the window each round's top-level entries
  (`DownloadEvent::Contents { entries }` after the manifest is read), which
  the app keeps per stick, as it keeps the renamed names (ADR 0010).
- App: `add_received` takes the un-ticked names and stores them right after
  `take_stick` records the stick and before its first round starts;
  `set_left_out(stick_id, names)` command, which stores the whole set, then
  stops and starts the stick's receive under `Receivers::held` (as Pause and
  Resume do; a file being placed finishes); `ReceivedEntry` gains
  `left_out` (with which are already here) and the latest top entries.
  `told.rs` and the CLI's `progress.rs` match the new events. Svelte: ticks in
  `AddLink.svelte`, `ChooseWhat.svelte` for the received stick, the
  "Not getting" line in `ReceivedView.svelte`.

## Consequences

- A stick too big for this computer can still be received in part.
- One more table; a choice is per stick and per device.
- A sender's reorganisation of top-level folders (a rename, or a copy to a
  system that writes names in another Unicode form) can bring back
  something the user did not want. Acceptable: the "no room" wait names it,
  the user un-ticks it again, and nothing is lost or overwritten.

- Known gap: the stick is recorded, then its choice is stored, in two
  writes. If the app stops between them (a crash, a power cut), the next
  start receives the stick with everything ticked. A choice that fails to
  be stored while the app runs undoes the whole add instead. Closing the gap
  would need one store write for both.

## Order of work

1. Store: migration 0019, `left_out` / `set_left_out`, both stores; tests
   (an upgrade from 18; forgetting the stick takes its rows; an empty set
   means everything is got, never "nothing").
2. Core: `saved_paths` with un-chosen entries, `FilePlan::NotChosen`,
   `NotChosen` and `Contents` events, `DownloadReport.not_chosen`,
   per-entry `to_write` in the preview; tests (unit: plan with un-chosen
   entries keeping their recorded places, a genuine name added meanwhile
   numbered; renamed names under an un-chosen folder not reported; a file
   and a folder of one name; a mixed batch; preview sums, also for a new
   folder; integration with two peers: three top-level folders, one
   un-ticked, two come; the sender adds a fourth, it comes; ticked again,
   the first comes too; un-ticked after it came, its files stay and its
   places are kept; a choice saved while the stick waits counts at the
   next round).
3. App: storage of the choice at add, the command, the views; tests of the
   Rust parts.
4. Svelte: ticks in the preview with the sum, the received stick's
   `Choose what to get…` and "Not getting" line; svelte-check.
5. Docs: gui-spec.md (the receiver side), protocol.md (the reader's
   behaviour; nothing on the wire changes), schema.md, todo.md; status
   Accepted.
