# ADR 0006: Relay policy — relays introduce, direct paths carry data

**Status:** accepted — 2026-09-29

## Context
OurStick uses iroh. When two peers cannot reach each other directly, iroh
carries their QUIC traffic through a relay server. In v1 these are the public
relays run by n0 (CLAUDE.md: no infrastructure of our own). They are free to
us, carry no guarantee of capacity or availability, and are not meant to move
people's films and photo libraries. Carrying data through relays of our own
would cost bandwidth in proportion to everything users share. The lessons from
a previous sharing project say the same: decide the relay policy and its costs
early (lesson 5), and let every network failure say why.

Field tests (2026-09-28/29) between a PC on a home connection and a phone on
4G/5G, and a MacBook on the phone's hotspot, found a direct path within about
2 seconds every time, over IPv6 and IPv4 through carrier NAT. The relay was
needed only to introduce the peers and coordinate hole punching (a few KB).
Earlier "relay" readings in those tests were a display bug, since fixed.

## Decision
1. **Relays introduce, direct paths carry data.** A relay is used to find the
   owner, coordinate hole punching, and fetch the signed record and the
   manifest (kilobytes).
2. **Small transfers may use the relay without asking.** If what is left to
   download is at most 100 MB, the reader does not wait for a direct path and
   does not ask.
3. **Larger transfers wait for a direct path.** Before any file data is
   fetched, the reader waits up to 15 seconds for a direct path. If none opens,
   it stops, explains the likely reason (this device's network, what the owner
   offered), and asks whether to continue through the relay. The default answer
   is no. `--allow-relay` answers yes for non-interactive use; without a
   terminal and without the flag the answer is no.
4. **A direct path lost mid-transfer is retried, then asked about.** If the
   data starts arriving through the relay during a larger transfer, the reader
   pauses, reconnects and waits again for a direct path (step 3). Only if that
   fails does it ask. Nothing received is lost: downloads resume.
5. **The relay is configurable.** The transport takes its relays from its
   configuration (the public n0 relays by default, or a list of relay URLs), and
   the question to the user names the relay that would be used. Nothing in the
   policy is tied to n0.

## Why
- The public relays are someone else's service. Using them for introductions is
  what they are for; using them for bulk data is not something to build on.
- Asking, with the reason, is honest: the user decides whether a slow relay
  transfer is acceptable, instead of it happening silently.
- 100 MB keeps small shares (documents, a few photos) working everywhere without
  questions, while films and libraries cannot end up on a relay by accident.

## Consequences
- On networks where hole punching fails (strict corporate firewalls, some
  IPv4-only mobile networks), a large download stops after 15 seconds and asks.
  How often this happens is unknown; more field tests on other networks will
  tell.
- Up to about 4 MiB can arrive through the relay before a lost direct path is
  noticed (the reader checks which path the data arrives on every 4 MiB).
- A declined relay leaves a resumable download.

## Future direction (not in v1)
Relays of our own as a **paid tier**, with no limit on size or speed. Then the
question in step 3 becomes "continue through OurStick Cloud" for subscribers,
and the relay configuration (step 5) points at those relays. This goes together
with the cloud seed that CLAUDE.md already defers. No code for our own relays is
written now.
