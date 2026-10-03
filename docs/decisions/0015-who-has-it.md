# 0015: Who has it

Status: Proposed (2026-10-04)

## Context

A sender cannot tell when a receiver is done. Today the owner sees only
what happens now (`ReaderActivity`: how many readers download, at what
rate, whether they wait for their files to be got ready). When the bar on
the other computer reaches the end, nothing on the sender's side says so.
So the sender cannot know when it is safe to close the computer, to take
a folder off the stick, or to remove the stick, and asks the receiver by
phone. The user asked for it after the tree test of ADR 0012 Amendment 1
(2026-10-04): "whether the receiver has downloaded the whole stick, so
that they know it is safe to close the PC or delete the stick".

What the owner can know today: a connection's NodeId (the reader's iroh
key, which says nothing about who it is) and which blobs it fetches. That
is not enough: a reader that left something out (ADR 0012) fetches less
and is still done; a reader whose files were already there (a folder
received before) fetches nothing and is done; and a reader that stops half
way fetches less and is not done. Only the reader knows, so the reader has
to say it.

Things that hold, from earlier decisions:

- There are no accounts (CLAUDE.md). Whoever has the link is a reader; a
  link can be forwarded.
- The owner never learns what a reader leaves out (ADR 0012; protocol.md
  says so of choosing what to get).
- Text from peers is escaped before it is shown (lessons in CLAUDE.md).
- A ticket or a key is never logged.

## Decision

1. **A reader tells the owner how far it is**, in a small report, on a new
   protocol `ourstick/report/1` beside `ourstick/record/1`. It sends one
   when a round has planned (how much it will get), about every 30 seconds
   while it downloads, and when the round ends (up to date, or why it
   stopped). Each report says:
   - which version (the record's `seq`) it is about;
   - bytes and files it has of what it gets, out of how many;
   - its state: `downloading`, `up_to_date`, `waiting` (sender away, no
     room, folder unplugged), `paused`;
   - one flag, **`everything`**: true when it gets the whole stick, false
     when it left something out. Only the flag: never what, nor how much
     (ADR 0012 holds: the owner learns that something is left out, not
     what);
   - **what kind of computer** it is (`macos`, `windows`, `linux`, or
     `other`), and **a name only if the receiver typed one** (rule 3).
   - a **reader id**: 16 random bytes made when the stick is added and kept
     with it, so the owner counts one computer once across rounds and
     restarts. A new id per received stick, never one per device: two
     senders cannot tell that the same computer has both their sticks. A
     reader's iroh key plays no part: readers make a new one for every
     round (`random_key()`), so it names no computer.
2. **Only someone with the link can report.** The report is sealed with
   ChaCha20-Poly1305 under a key derived from the StickKey (BLAKE3
   `derive_key`, context `"ourstick report v1"`), with a random nonce. An
   owner opens it with the same key and drops, silently but for a debug
   line, what does not open, is too big (at most 512 bytes), or names
   another stick. Someone with only the public part of a link (the path of
   `https://ourstick.com/s/…`, without the fragment) can neither report nor
   read a report. A reader with the link can still lie (say "done" when it
   is not): this is the reader's word, shown as such, never a proof.
3. **The name is the receiver's own, given only if they choose to.** The
   app has, in its settings, "Your name for senders (optional)", empty at
   first and never asked for by a popup or a step. The computer's own
   name (`hostname`, "Anna's MacBook Pro", `login-System-Product-Name`)
   is never sent: a link is often forwarded or posted, and the computer's
   name usually holds the user's real name or login, which would then go
   to whoever holds the link, also someone the receiver does not know.
   What is sent without asking is the kind of computer, which every
   download from a website tells anyway. A name is at most 40 characters;
   the owner escapes it before showing it, as every text from a peer. The
   CLI sends no name.
4. **The owner keeps the latest report per reader** in SQLite (a new table
   `reader_reports`): stick, reader id, seq, bytes and files done and in
   all, state, `everything`, kind, name, when it was received. A report
   with a lower seq than the one kept for that reader is ignored (the
   reader moved on). At most 200 readers per stick are kept, the ones not
   heard from longest dropped first; a reader not heard from in 90 days is
   dropped. Removing a stick removes its reports. A new link (a new key)
   starts with none: reports sealed for the old key do not open.
5. **What the sender sees**, under the stick, one line per computer,
   newest first (at most 5 shown, then "and 12 more"):
   - "Giorgos (Mac) has all of it, up to date." — `up_to_date`,
     `everything`, the seq the owner publishes now.
   - "A Windows PC has what it chose, up to date." — `everything` false.
   - "A Mac is downloading: 60%." — with its state.
   - "A Linux PC had all of it at 14:30; it does not have what changed
     since." — an older seq (the sender added or changed files after).
   - "A Mac is waiting: no room there." — `waiting`, with its reason.
   When every computer heard from in the last 90 days has all of it, up to
   date, the stick says once more, plainly: "Everyone who received it has
   all of it." With none yet: "No one has received it yet." The words
   never say "safe to delete": what was received stays with the receivers
   whatever the sender does (ADR 0012), and the sender decides.
6. **A receiver that forgets the stick says so**, if the owner can be
   reached at that moment (a report with state `forgotten`): the owner
   drops that reader's line. If it cannot be reached, the line stays until
   the 90 days, or until the sender removes it (`Remove from this list` on
   the line). Nothing waits for it: forgetting never blocks on the
   network.
7. **No switch to stop reporting in v1.** The owner already sees, live,
   that someone downloads and how fast (ADR 0007); the report adds how far,
   and that it ended. Whether a receiver should be able to download
   without the owner knowing is a larger question (a seed, later, would be
   a reader too); it is left open (below).

## In the code

- `crates/ourstick-core/src/report/` (new): `ReaderReport` (the fields of
  rule 1), its postcard form, `seal(report, stick_key)` and
  `open(bytes, stick_key) -> Result<ReaderReport, ReportError>` (thiserror),
  with the size limit and the escaping of the name for showing
  (`untrusted::escape`).
- Transport: the trait `Fetcher` gains `report(&ReaderReport)`, best effort
  with a short deadline (a report that cannot go is dropped, never
  retried, never in the way of a download); `IrohFetcher` opens
  `ourstick/report/1` (one stream, the sealed report, no answer but the
  stream's end). `Publisher` gains a `ReportServer`, which opens the
  report and hands it to the owner's side through a channel; the
  in-memory fake for tests does the same.
- `download/`: the round sends its reports (planned, every 30 s, ended)
  through a small `Reporter` that knows the reader id, the kind and the
  name; the CLI's `get` uses it too, with no name.
- Store: a migration adds `joined_sticks.reader_id` (16 random bytes, made
  for every received stick, also ones received before) and the table
  `reader_reports`; `Store` gains `keep_report`, `reports(stick)`,
  `forget_report(stick, reader)`; the 200 and 90 days are applied when a
  report is kept. The optional name is kept in a new one-row table of
  this device's settings (same migration).
- App: the owner's side lists the reports under its stick (event
  `readers-changed`), with the words of rule 5 and `Remove from this
  list`; the settings get the name field; Forget sends `forgotten` first,
  with a deadline of 3 seconds.
- Docs: `protocol.md` (the ALPN, the sealed report, its fields and
  limits), `schema.md`, `gui-spec.md`.
- Tests: sealing and opening (a wrong key, a changed byte, too big, another
  stick: refused, no panic on any input); a lower seq ignored; the limits
  of 200 and 90 days; two peers on one machine: the owner learns "has all
  of it, up to date" after a download, "has what it chose" with something
  left out, an older seq after the sender adds a file, and the line goes
  when the reader forgets the stick.

## Consequences

- The sender knows when every receiver is done, in the receiver's own
  words, without asking them.
- The owner learns a little more than today: that a computer of a given
  kind has the stick, whether it left something out (not what), and a
  name only if the receiver gave one.
- A reader with the link can send false reports, or many (at most 200 lines
  per stick are kept, so the list cannot grow without end). The list is a
  help to the sender, never a proof.
- One more protocol on the owner's endpoint, and reports are small (a few
  hundred bytes every 30 s per downloading reader).

## Considered and not chosen

- **The computer's name, sent by itself** (the user's other idea): the
  sender sent the link and may well know where; but a link is forwarded,
  and the computer's name often carries a real name or login. Not sent;
  the receiver may type a name of their own (rule 3).
- **Guessing from the blobs fetched:** cannot tell "left something out" or
  "already had it" from "stopped half way".
- **The reader's iroh NodeId as its identity:** readers make a new key
  every round, so it would count one computer many times; and a key kept
  per device would be the same for every stick it receives, so two
  senders could match their lists. A reader id per stick does neither.
- **Reports in clear text:** whoever knows the public part of a link (it
  may be in a URL's path in a server log) could fill the list.

## Open questions

- Should a receiver be able to download without the sender knowing (a
  switch "Tell the sender how far I am", on by default)? Not in v1 (rule
  7).
- A seed (later, out of v1) is a reader: it would report like one, and the
  sender would see "a seed has all of it".

## Order of work

1. Core: `report/` with seal and open, tests (including bad input).
2. Transport: `ourstick/report/1`, `Fetcher::report`, the `ReportServer`;
   the migration and the store; two peers test.
3. Reader: the `Reporter` in the round (CLI and app); the name setting.
4. Owner: the list under the stick, its words, `Remove from this list`;
   Forget sends `forgotten`.
5. Docs; a manual test Linux → Mac and Linux → Windows VM: the sender sees
   "has all of it, up to date", then adds a file and sees "does not have
   what changed since", then the line once the receiver has it again.
