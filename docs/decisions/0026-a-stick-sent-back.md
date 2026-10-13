# ADR 0026: A stick sent back

**Status:** proposed, on hold — 2026-10-13: the design only, after the
first big real-world transfer. Reviewed by Fable and Sonnet; their
findings are folded in.

## Context

A friend who received a stick wanted to send something back: "drop my own
files into the stick I see, and they reach you". The user wants it before
the first public release: with no public users yet, a protocol change now
splits no one.

A stick has one owner, who alone writes it (CLAUDE.md: no write rights for
readers in v1). Two writers would bring what this design has kept out:
conflicts, deletions that reach the other side, rights to grant and take
back, and a mess of two people's files in one folder ("my music and his
paintings"). The other way round is already there: anyone can make a stick
and send its link. What is missing is the trip of the link: the receiver
must copy it, find a way to send it, and the sender must paste it, while
the two apps already talk.

The reader already talks to the owner: signed, encrypted reports
(`ourstick/report/1`, ADR 0015). They cannot carry a link: a report is at
most 512 bytes, it must never carry a secret (the owner keeps reports in
plain SQLite), and its server answers "taken" before it opens the report:
a 0.1.x owner drops a report of another form silently. A stick back needs
an answer that means "kept".

## Decision

### The receiver's side

1. **A stick back is a stick of the receiver's own**, made and shared like
   any other: one owner, its own folder, link, key and node key (ADR
   0009). Nothing of the stick it answers changes; the two never mix.
2. **"Send something back…"** in a received stick's view (the app only):
   it makes a new stick, named by the receiver (prefilled with "<the
   answered stick's name> — back"; never the computer's name), and opens
   it to drop files and folders. Nothing is sent by itself: once it has
   files, the view shows "Send it to the sender" (primary), with what it
   holds ("12 files, 340 MB") and "The sender sees this stick's link, as
   anyone you send a link to: with it, your computer's address." Pressed,
   the app offers the link (3).
3. **The offer is kept and tried until answered.** A new table `sent_back`
   (migration 0026): the answered stick, the stick back, the state
   (`trying`, `kept`, `already`, `older`, `not_here`, `no_longer`,
   `ended`), since when, and the counter of its last try. A task of its
   own (not the receive's: a paused stick still offers) tries at once,
   then as a receive waits (`after_error`: 15 s doubling to 5 minutes),
   across restarts, for 30 days. Each try sends the stick back's link as it
   is then (its address hints change). The received stick's view says how
   it goes: "Sending “Paintings” to the sender… (they are away)", "Sent:
   the sender can look at it.", or, with `Copy link` to send it another
   way: "The sender's OurStick is older and cannot take it.", "The sender
   cannot take it there.", "The sender no longer shares this stick.",
   "The sender did not answer for 30 days."
4. **It ends with what it answers.** The offer ends (its row goes) when
   the stick back is removed or given a new link (a new one offered
   instead, if the user asks), or the answered stick is forgotten (its
   reader key goes with it, ADR 0015). One stick back per answered stick:
   sending another replaces it (the owner keeps the latest per reader).

### The wire

5. **A protocol of its own, `ourstick/offer/1`**, to the owner's node of
   the answered stick, as a report reaches it.
   - **Request:** the answered `StickId` in clear (32 bytes), then
     `0x01 ‖ nonce(24) ‖ XChaCha20-Poly1305(reader_id(32) ‖ sig(64) ‖ body)`,
     the key `blake3::derive_key("OurStick 2026-10-13 offer
     XChaCha20-Poly1305 key", StickKey)`, the associated data `0x01 ‖
     StickId`. The body (postcard): `counter: u64`, the stick back's
     whole ticket (at most 1 060 bytes, ADR 0022), `name: Option<String>`
     (as in reports, at most 40 characters, only one the receiver typed).
     The signature, with the reader's key for the answered stick
     (`ReaderKey`), covers `"OurStick 2026-10-13 offer signature" ‖
     0x01 ‖ StickId ‖ body`, the ticket and so the stick back's id
     included. New contexts: an offer is never taken for a report. At most
     2 KiB in all (about 1.4 KiB at most in use).
   - **Answer**, one byte on the same stream, sent only once the offer is
     stored for good (row and secret): `0 kept`, `1 already` (this stick
     back, from this reader, is kept, received or one of the owner's
     own), `2 unknown_stick` (not shared here any more), `3 not_here` (a
     program that does not take offers, as the CLI), `4 full`, `5 busy`
     (the owner could not store it now: try again). Unknown answers and
     no answer: try again. `0`, `1`, `2`, `3`: the reader stops.
   - **An older owner** does not list the protocol: the TLS handshake
     fails (no application protocol). The transport maps that failure to
     a new `TransportError::NotSpoken`, never `Unreachable`, and
     `after_error` stops on it: "older", at once, never a 30-day wait.
     Tested against an endpoint without the protocol.
   - **Limits before opening:** at most 4 offers at once per owner node,
     2 KiB read at most, 10 s per stream; logs name only the answered
     StickId and the reader id, never a ticket, a name or bytes (a test
     checks it, as for reports).

### The owner's side

6. **An offer is kept, never fetched unasked.** The handler answers only
   after: the stick is still shared here; the stick back is not one of
   the owner's own, received, or waited for (`check_link`: then
   `already`); a row in `offers` (answered stick, reader id, stick back
   id, name, counter, state, first and last time) is written as
   `storing`; the ticket is put in the secret store as
   `SecretName::Offered { stick, reader }` (per answered stick and reader:
   two offers never share a secret); the row is marked `kept`. If the
   secret cannot be written, the row goes and the answer is `busy`. A
   counter not above the kept one is `already`.
7. **Tombstones, as reports keep hidden rows:** `No thanks` and `Get it`
   delete the secret and mark the row `declined` or `taken`, kept 30 days
   with its counter, so a lost answer or a replay never brings an offer
   back. A kept offer not looked at ends after 30 days (secret deleted,
   row `ended`). At start, a row still `storing`, or one whose state says
   its secret is gone, has the secret deleted again (a crash between the
   two steps leaves nothing behind).
8. **Limits that hold against a forwarded link** (a reader id is only a
   key anyone with the link can make): one offer per reader per stick (a
   later one replaces it); at most 20 kept per stick; when full, a reader
   that has a report line (it really received the stick) takes the place
   of the oldest offer from a reader without one; else `full`. At most 10
   new offers an hour per stick (then `full`). The owner has `Clear
   offers` and "Take no sticks back" for each stick (then `not_here`,
   until turned on again or the link changes). `full` is never silent on
   the receiver's side (3): it says so and tries again later.
9. **In the window:** on the reader's line in the stick's view, as reports
   name it ("Nikos (Windows PC)", or "A Windows PC"): "sent you a stick
   back"; and in `Received`, an entry "Offered: Stick 1a2b3c4d" (unlike a
   waited-for link's words) with `Look at it` / `No thanks`. Its words are
   honest: "Someone with the link of “Holiday photos” sent you a stick
   back. A name is what they typed: anyone with the link could send
   one." A reader with a report line is said to have received the stick;
   one without, "never got this stick here". `Look at it` says "Looking
   connects to their computer, as any link does." and opens the add-link
   preview by the offer (the backend reads the secret; the window never
   holds the link). `Get it` (or `Wait for it`, ADR 0025) makes it a
   received stick: `Ticket(back)` is put, the stick is joined (or
   waited for) and the offer marked `taken`, then `Offered` deleted; a
   leftover is deleted at start (7).
10. **With the stick:** removing a stick, or giving it a new link, deletes
    its offers' secrets and rows (`delete_stick_rows`, and the secrets in
    the app's remove path). The macOS move of keys (ADR 0021) lists
    `Offered` secrets as optional.
11. **The CLI** does not take offers: its endpoint answers `not_here`
    (the handler answers so whenever no program takes offers). `ourstick
    get` and `ls` do not change.

## Consequences

- A receiver answers with their own files in a few clicks; the sender sees
  it where they look already, and decides.
- Each side keeps its files in its own folder: no conflict to solve.
- A stranger with a forwarded link can offer sticks: never fetched
  unasked, said honestly, limited (8), and the owner can turn offers off.
- The sender learns the stick back's link, so the receiver's address, as
  with any link; the receiver is told before sending (2).
- An older sender is told apart at once, with the manual way offered.
- New places for a secret: the offer in flight (sealed), `Offered` in the
  owner's secret store. Never in a table or a log.
- Migration 0026 makes an older app refuse the store (`NewerDatabase`), as
  any migration.

## Docs

`docs/protocol.md` (the offer: format, sealing, answers, limits),
`docs/schema.md` (`offers`, `sent_back`), `docs/gui-spec.md` (Send
something back…, the offer on the reader's line and in Received),
`docs/test-vectors` (an offer), the FAQ (Syncthing: "the other person can
send a stick back").

## Tests

- Core: an offer sealed and opened (wrong key, signature, size, version);
  test vectors; the owner's rules: one per reader, 20 per stick and who
  takes a place, 10 an hour, a lower counter, tombstones after No thanks
  and Get it, expiry, the start-time cleanup; `NotSpoken` from an endpoint
  without the protocol; `not_here` with no program taking offers; no
  ticket in logs.
- Integration, in memory: kept; the owner away, then back; a lost answer
  then a retry after No thanks (stays declined); the stick removed while
  an offer is in flight (`unknown_stick`, no secret left); two readers
  offering the same stick back; a secret store that fails (`busy`, then
  kept).
- By hand: Linux ↔ Windows (VM), and macOS: send back, look at it, get it;
  No thanks; an older (0.1.0) sender; a CLI sender.

## Alternatives considered

- **Readers write into the same stick:** two writers, conflicts and
  deletions on both sides, rights to manage; the mess the user feared.
- **The link inside a report (a report version 2):** too big for 512
  bytes, a secret in plain SQLite, and silently lost by a 0.1.x owner,
  whose report server says "taken" before it reads.
- **Fetching a stick back at once:** anyone with a forwarded link could
  fill the sender's disk.
- **Offering by itself once the stick has files:** the receiver may have
  dropped the wrong folder or not be done; one click is the consent.
