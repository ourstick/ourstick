# 0015: Who has it

Status: Accepted (2026-10-04), after the design reviews and their confirmation

## Context

A sender cannot tell when a receiver is done. Today the owner sees only
what happens now (`ReaderActivity`, ADR 0007: how many readers download,
at what rate, whether they wait for their files to be got ready). When the
bar on the other computer reaches the end, nothing on the sender's side
says so. So the sender cannot know when it is safe to close the computer,
to take a folder off the stick, or to remove the stick, and asks the
receiver by phone. The user asked for it after the tree test of ADR 0012
Amendment 1 (2026-10-04): "whether the receiver has downloaded the whole
stick, so that they know it is safe to close the PC or delete the stick".

What the owner can see today is not enough. A connection's NodeId names no
computer: readers make a new iroh key every round (`random_key()`). And the
blobs fetched do not tell the cases apart: a reader that left something out
(ADR 0012) fetches less and is done; a reader whose files were already
there fetches nothing and is done; a reader that stops half way fetches less
and is not done. Only the reader knows, so the reader has to say it.

Things that hold, from earlier decisions:

- There are no accounts. Whoever has the link is a reader; a link can be
  forwarded.
- The owner does not learn what a reader leaves out (ADR 0012).
- Secrets live only in the secret store; never in a plain file, never in
  a log.
- Text from peers is escaped before it is shown.

## Decision

1. **A reader tells the owner how far it is**, in a small report, on a new
   protocol `ourstick/report/1`, over the connection its round already has.
   It sends one when a round has planned, about every 30 seconds while it
   downloads, and when the round ends, each with a deadline of 5 seconds.
   The last state that could not go (most likely the round's end, as its
   connection closes) is kept and sent once more, on a fresh connection,
   when the reader next asks the owner for its record (a received stick
   is checked for changes while the app runs), and with every later round.
   Each report says:
   - the version it is about (the record's `seq`);
   - its state: `downloading` with a whole percent (0–99) of what it gets,
     `up_to_date`, `waiting` with one reason of a fixed list
     (`sender_away`, `no_room`, `folder_missing`, `other`), `paused`, or
     `forgotten`;
   - **`everything`**: whether it gets the whole stick (nothing left out).
     No bytes, no file counts, no names: the percent is of what it gets,
     so neither tells how much was left out. ADR 0012's "the owner does not
     learn what a reader leaves out" holds, made exact: the owner learns
     *that* something is left out, never what or how much (protocol.md
     says so).
   - **the kind of computer** (`macos`, `windows`, `linux`, `other`) and
     **a name only if the receiver typed one** (rule 3);
   - a **counter** that only grows: milliseconds of the reader's clock, or
     the last one sent plus one if the clock went back. The last one sent is
     kept with the received stick (a column of `joined_sticks`), so it
     grows across restarts too.
2. **Each reader speaks only for itself.** When the app receives a stick
   it makes, once, a **reader key** for that stick (Ed25519), kept in the
   secret store with the stick's ticket and deleted with it; its public
   key is the **reader id**. It is deleted with the ticket when the stick
   is forgotten (`received::finish`), and made at the next round for a
   stick received before this ADR. A new key per received stick, never one
   per device: two senders cannot tell that the same computer has both their
   sticks. Every report is signed with it. So someone else with the link
   can add lines of their own, but can never change another reader's line.
   A report whose counter is not above the last one kept for that reader is
   ignored (an old report sent again changes nothing).
3. **Only someone with the link can report or read a report.** The signed
   report is sealed with XChaCha20-Poly1305 under a key derived from the
   StickKey (BLAKE3 `derive_key`, context
   `OurStick 2026-10-04 report XChaCha20-Poly1305 key`), with a random
   24-byte nonce and the version byte and StickId as associated data. On
   the wire: the StickId in clear (32 bytes, so the owner knows which key
   to try, as `ourstick/record/1` asks; the owner takes that stick's
   `SecretName::StickKey` from its secret store), then version, nonce and sealed
   report, at most 512 bytes in all, read with that limit. What does not
   open, is too big, or names a stick the owner does not share is dropped,
   with a debug line that names neither bytes nor keys. A reader with the
   link can still lie about itself (say "done" when it is not): its word,
   shown as such, never a proof.
4. **The name is the receiver's own, given only if they choose to.** The
   computer's own name (`hostname`: on a Mac "Name's MacBook Pro", on Linux
   `login-System-Product-Name`) is never sent: a link is often forwarded or
   posted, and the computer's name usually holds a real name or a login,
   which would go to whoever holds the link, also someone the receiver does
   not know. What is sent without asking is the kind of computer, which
   every download from a website tells anyway. The receiver may type a name
   in one place the app shows but never asks about: a line at the foot of
   the received sticks, "Senders see you as: a Mac. `Change…`" (gui-spec
   changes: it is a line, not a settings page). At most 40 characters, the
   same character rules as a manifest's names (no control or format
   characters); the owner escapes it before showing it. Kept in the
   `device` row (a new column, `receiver_name`), the one place for it.
5. **The owner keeps the latest report per reader** in SQLite
   (`reader_reports`, index on stick and time): stick, reader id, counter,
   seq, state, percent, reason, `everything`, kind, name, and when *the
   owner* received it (its own clock: the time it shows). At most 200
   readers per stick. A reader not yet kept takes the place of the one
   silent longest if silent for 30 days, else of a hidden line, else is
   refused: a flood of made-up readers cannot push real ones off the list,
   it only fills the free places (the cost: while 200 made-up lines keep speaking, a real new
   reader gets no line; `Clear this list` empties it at once). Lines older
   than 90 days are dropped when a report is kept and
   never shown. Removing a stick, or giving it a new link (a new StickId
   and key), leaves its reports behind: they are deleted with it.
6. **What the sender sees**, under the stick, one line per computer, the
   latest first (at most 5, then "and 12 more"). "Up to date" counts only
   on the latest *complete* version the owner has published (not a partial
   one of ADR 0007, not one being signed):
   - "A Mac has all of it, up to date." — `up_to_date`, `everything`,
     the latest complete seq. A computer is named by its kind; only if its
     receiver typed a name is that name shown before it: "Maria (Mac) has
     all of it, up to date." Two computers of one kind with no name are
     told apart by when they were first heard from: "A Mac (since 3 Oct)".
   - "A Windows PC has what it chose, up to date." — `everything` false.
   - "A Mac has everything so far; you are still reading your folder." —
     up to date with a partial version ("has what it chose so far" without
     `everything`).
   - "A Mac is downloading: 60%."
   - "A Linux PC had all of it at 14:30; not what changed since." — an
     older seq; the time is when the owner heard it.
   - "A Mac is waiting: no room there." — `waiting`, with its reason.
   When every computer heard from has all of it, or all it chose, up to
   date: "Everyone who received it is up to date." With none:
   "No one has received it yet." Never "safe to delete": what was received
   stays with the receivers whatever the sender does, and the sender
   decides.
7. **A receiver that forgets the stick says so** if the owner can be
   reached at that moment (state `forgotten`, a deadline of 3 seconds): the
   owner drops that line. Otherwise the line stays until 90 days pass or
   the sender removes it (`Remove from this list`, or `Clear this list` for
   all). A removed line keeps its counter, hidden: only a newer report
   brings it back, never an old one sent again (unless a full list later
   gave its place to a new reader, rule 5, or 90 days passed). Forgetting never waits for
   the network beyond that.
8. **The app only, in v1.** `ourstick get` keeps no secrets (it opens no
   secret store), so it has no reader key and sends no reports; the
   sender sees it only in the live activity, as today. **No switch to stop
   reporting in v1**: the owner already sees, live, that someone downloads
   and how fast; the report adds how far, and that it ended (left open,
   below).

## In the code

- `crates/ourstick-core/src/report/` (new): `ReaderReport`, `ReportState`,
  `WaitReason`, `ComputerKind`; postcard form; `seal(report, reader_key,
  stick_key)` and `open(bytes, stick_id, stick_key) -> Result<Opened,
  ReportError>` (thiserror): checks size, AEAD with the AAD, the signature,
  the name's characters.
- Transport: `PeerConnection` gains `report(&[u8])`, best effort with a
  short deadline, on the connection the round has; `IrohPeerConnection`
  opens `ourstick/report/1` on it (new `report_protocol.rs`, beside
  `record_protocol.rs`). `Publisher` gains `take_reports()`, a channel of
  raw reports with their StickId, like `take_demand()`; the owner's side
  opens them with the stick's key (the transport holds no StickKey). The
  in-memory fakes (`tests/common/owner_in_memory.rs`) do the same. Reports
  are not `Demand` and not `ReaderActivity` (ADR 0007): a separate channel.
- Store: migration 0020: `reader_reports` with its index and a `hidden`
  flag, `device.receiver_name`, and `joined_sticks.report_counter`. `Store` gains `keep_report` (the counter, the 200
  and 30-day rule, the 90 days), `reports(stick, now)` (filters by age),
  `hide_report(stick, reader)`, `hide_reports(stick)`, the name and
  `next_report_counter`; both stores, the trait
  tests, a migration test. The reader key: `SecretName::ReaderKey(stick)`,
  made when the stick is added (and, for sticks received before, at their
  next round), deleted with the ticket in `received::finish`; protocol.md's
  "Key storage" lists it.
- `download/`: a `Reporter` (reader key, stick key, kind, name, a `Clock`
  for the 30 s) the round calls when it plans, as it goes, and when it
  ends; the CLI passes none.
- App: the owner's side lists the reports under its stick (event
  `readers-changed`), with the words of rule 6 and `Remove from this
  list`; the line "Senders see you as…" under the received sticks; Forget
  sends `forgotten` first.
- Docs, each with the step that needs it: `protocol.md` (the ALPN in the
  table, a section `ourstick/report/1` with its layout, fields, limits and
  key, the sentence of ADR 0012 made exact, what an owner does with a
  report), `schema.md`, `gui-spec.md` (the list under a stick, the line
  under the received ones; "no settings page" stays true).
- Tests: seal and open (a wrong key, a changed byte, too big, another
  stick, a bad signature, another reader's id: refused; no panic on any
  input); the counter (an old report again ignored); the 200 rule (a
  flood refused, a silent reader replaced after 30 days); 90 days; the
  30 s with a test clock; a report that cannot go does not hold up the
  download; two peers on one machine: "has all of it, up to date" after a
  download, "has what it chose" with something left out, an older seq after
  the sender adds a file, "everything so far" with a partial version, the
  line gone after the reader forgets the stick, and the last state sent
  again after its report was lost; `forgotten` to an owner that cannot be
  reached gives up after 3 seconds; a stream longer than 512 bytes is not
  read past the limit; a counter kept across a restart with the clock set
  back; no key or ticket in the log lines of any of these.

## Consequences

- The sender knows when every receiver is done, in the receiver's own
  words, without asking them.
- The owner learns a little more than today: that a computer of a given
  kind has the stick and how far it is, whether it left something out (not
  what, not how much), and a name only if the receiver gave one.
- Someone with the link can add made-up lines (at most up to 200, never
  pushing a real reader off, but then keeping new real readers off until
  the sender clears the list) and can lie about itself, never about
  another. The list is a help to the sender, never a proof.
- One more protocol on the owner's endpoint; reports are small (under 512
  bytes every 30 s per downloading reader).
- v1 grows by one thing that CLAUDE.md's "what v1 does" does not list yet:
  it is added there once this ADR is accepted.

## Considered and not chosen

- **The computer's name, sent by itself:** the sender knows where they
  sent the link, but a link is forwarded, and the computer's name often
  carries a real name or a login (rule 4).
- **Only "done" at the end of a round** (the least the owner would learn):
  it answers "has it" but not "how far", "waiting for room", or "has only
  part", which is what makes the sender wait or call.
- **Guessing from the blobs fetched:** cannot tell "left something out" or
  "already had it" from "stopped half way".
- **The reader's iroh NodeId as its identity:** a new one every round; and
  one kept per device would let two senders match their lists.
- **Reports in clear, or sealed but unsigned:** in clear, whoever knows a
  link's public part could fill the list; unsigned, anyone with the link
  could change another reader's line (say a reader is done when it is not).

## Open questions

- Should a receiver be able to download without the sender knowing (a
  switch "Tell the sender how far I am", on by default)? Not in v1.
- A seed (later, out of v1) is a reader: it would report like one.

## Order of work

Each step ends with `cargo test` green and its docs.

1. Core: `report/` (seal, open, sign, the counter), tests with bad input.
2. Store: migration 0020, both stores, the 200/30/90 rules, trait and
   migration tests; schema.md.
3. Transport: `ourstick/report/1`, `PeerConnection::report`,
   `Publisher::take_reports`, the fakes; protocol.md.
4. Reader: the reader key, the `Reporter` in the app's rounds, Forget's
   `forgotten`; two peers tests.
5. Owner: the list under the stick, its words, `Remove from this list`;
   the line "Senders see you as…"; gui-spec.md.
6. A manual test Linux → Mac and Linux → Windows VM: "has all of it, up to
   date"; a file added, "not what changed since"; up to date again; Forget,
   the line gone.
