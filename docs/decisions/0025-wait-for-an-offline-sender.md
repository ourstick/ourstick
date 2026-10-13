# ADR 0025: Wait for a sender who is offline

**Status:** accepted — 2026-10-13. Reviewed by Fable and Sonnet; their
findings are folded in.

## Context

A friend pasted a link while its sender's computer was off. "Look at it"
waited half a minute (`PREVIEW_WAIT`) and said: "The sender's computer is
off, or OurStick is not open there. Try again when it is." Nothing was
kept: he had to remember the link, come back, paste it again and try
again, perhaps many times, with no way to know when the sender is on.

A stick already received does not have this problem: its receive waits
for the sender by itself (`Waiting::Sender`; `download/receive/policy.rs`
decides, 15 s doubling to 5 minutes, also after a restart) and goes on
when the sender is back. Only the first step, the preview, gives up.

The preview cannot simply be skipped: a received stick has its folder
(`JoinedStick::output_dir`, used throughout the download and the store)
from the start, and the folder is named after the stick
(`Downloads/<name>`), whose name is only in the encrypted manifest, which
only the sender can give. Making that folder optional would touch every
reader of it; a separate, short-lived state before the received stick
touches nothing of it.

## Decision

1. **"Wait for it" where "Try again" was.** `preview_link` returns a typed
   outcome, not only words: `SenderAway` when the preview timed out, or
   failed as a receive would wait for the sender (`after_error` says
   `Waiting::Sender`: a sender just gone is told at once, without the
   half minute). Then
   the add-link view says "The sender's computer is off, or OurStick is
   not open there." with `Cancel` and `Wait for it` (primary). Any other
   failure is shown as today. A bad link, one of this device's own sticks,
   one already received, or one already waiting is refused before (see 3).
2. **A waiting link is kept in the store, its ticket in the secret
   store.** A new table `waiting_links` (migration
   `0025_waiting_links.sql`): `stick_id` (primary key), `added_at_ms`.
   The folder's parent is not kept: at timeout it is always the default,
   resolved when the stick comes (`parent_folder`), as HOME may move. The
   ticket is kept as a received stick's (`keep_ticket`, after
   `check_secret_store` and `ensure_device`); the row is written after
   the ticket, and if the row fails the ticket is deleted. Never in the
   table, a file or a log. `joined_sticks` does not change.
3. **One stick, one state, one lock.** `check_link` also refuses a
   waiting stick ("This link is waiting for its sender already.";
   `ReceivedError::Waiting`), for the app and the CLI. A waiting link is
   held in `Receivers` like a received stick: its download lock is taken
   when it starts waiting (so `ourstick get` of the same link says the app
   has it), and its wait runs as the `Held`'s task with its `Cancel`.
   Wait, Remove, the change into a received stick and Quit all go
   under the one `held` mutex, so they never overlap; Quit cancels and
   awaits it with the others (12 s at most).
4. **The wait is in the core, with the receive's cadence.** A new
   `download::wait_for_owner(fetcher, ticket, cancel, on_event)` connects
   to the sender, as a probe, with `after_error` deciding each next step,
   so its pauses (15 s to 5 minutes) and its words (`waiting_words`:
   sender away, no longer shared, damaged…) are the receive's. One
   fetcher per wait, kept across attempts and shut down when it ends.
   The ticket is read from the secret store once, when the wait starts.
   Once a connection works, the full preview runs once; it touches none
   of the add-link view's state (`Receivers::previewed`).
5. **When the sender answers, it becomes a received stick, with
   everything ticked:** under the `held` mutex, after checking that Quit
   has not begun and the row is still there, the app makes its folder as
   `add_received` does (`folder_for(name, parent_folder(None))`, or the
   folder of a CLI receive of the same stick, as `preview_link` chooses
   it: one function for both), records it as a received stick and deletes
   the waiting row in one store transaction (`join_waiting`), and starts
   the receive, as after `Get it` with `All`. If it does not fit, the
   receive waits for room, as any received stick does ("Not enough room
   on this computer: it needs up to X…"), and `Choose what to get…`
   un-ticks. If the change fails (the secret store locked, the folder
   cannot be made…), the row stays, the entry says why, and the next
   attempt tries again.
6. **A wait ends after 30 days.** A link nobody answers (the stick was
   removed, or given a new link) stops waiting; the entry says so ("The
   sender did not answer for 30 days.") and keeps only `Remove`. The days
   are counted from `added_at_ms`, shown in the entry.
7. **In the window:** the waiting link is in `Received` at once, as
   "Stick 1a2b3c4d" (`display_name` without a name) and "Waiting for the
   sender". Its view: "It comes by itself when the sender's computer is on
   and OurStick is open there. Keep OurStick open: minimize it rather
   than quit." and "Waiting since 13 October", with `Forget this stick…`
   (the word received sticks use; it removes the row, then the ticket).
   No folder, Pause or Choose. Once it is a received stick, its name and
   the usual view replace these.
8. **Elsewhere:** the macOS move of keys to the data-protection keychain
   (ADR 0021, `secrets_to_move`) and the choice of a key store for a
   device without a row (`keys.rs`) count waiting links' tickets as
   received sticks' are. The Quit dialog does not list waiting links:
   nothing of them is lost by quitting; they wait again at the next start.

## Consequences

- No one has to remember a link: pasted once, it comes.
- What comes is decided without the receiver seeing the list: all of it,
  maybe days later, with the window closed. They chose to wait for it;
  the size is checked before anything is written, and `Choose what to
  get…` un-ticks. A system notification when it starts coming would tell
  them; it needs `tauri-plugin-notification`, a new dependency, so it is
  a separate decision.
- Each attempt that reaches the sender shows on the sender's side as a
  reader for a moment (the preview fetches the manifest), as today's
  preview does once.
- An older app or CLI opening a store with migration 0025 refuses it
  (`NewerDatabase`), as with any migration.
- The CLI does not change: `ourstick get` still stops at once when the
  sender is off, and refuses a link the app is waiting for.
- The folder is under `Downloads`; no `Choose…` while waiting.

## Docs

`docs/schema.md` (the table), `docs/gui-spec.md` (the add-link view, the
waiting entry), `docs/todo.md`.

## Tests

- Core: `wait_for_owner` with an owner started late returns its preview;
  with an unknown stick it waits with "no longer shared"; cancelled, it
  ends at once. The store: a waiting link is added, listed and removed;
  `join_waiting` adds the received stick and removes the row together;
  `check_link` refuses a waiting stick; the secrets to move include its
  ticket.
- App (units): the typed `SenderAway`; the folder chosen for a waiting
  link (default, and after a CLI receive).
- By hand, Linux and Windows (VM): sender off, Wait for it, sender on: it
  comes without a click; the same across a restart of the receiver; a
  second paste of the link is refused; Forget during a wait; Quit during
  an attempt does not hang; a stick that does not fit waits for room.

## Alternatives considered

- **A received stick at once, with no folder until the first manifest:**
  reuses the receive's wait, but `output_dir` is read by the download,
  the store and the CLI; every one would need a "no folder yet" case.
- **Ask the receiver to look again when the sender is back** (a "Ready to
  look at" state): safer about size, but a second visit the friend did
  not want; the size check and `Choose what to get…` cover it.
- **Keep retrying in the add-link view while it is open:** lost when the
  window closes or the computer restarts.
- **No end to a wait:** a dead link would wake the network for ever.
