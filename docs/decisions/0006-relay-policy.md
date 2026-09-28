# ADR 0006: Relay policy — relays introduce, direct paths carry data

**Status:** accepted — 2026-09-29, revised the same day: a relay budget per
download replaces the up-front 15 s wait for larger transfers, which left the
download idle.

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
2. **A short wait first.** When file data is about to be fetched and there is
   no direct path yet, the reader waits up to 5 seconds for one (in the field
   tests it appeared within 1–2 seconds). Then it starts, over whatever path
   exists.
3. **A relay budget of 100 MB per download.** Up to 100 MB of file data may
   arrive through the relay without asking, while iroh keeps looking for a
   direct path; once one opens, the data moves there by itself. Transfers of at
   most 100 MB therefore never ask.
4. **When the budget is used up**, without a direct path, the reader stops
   fetching, reconnects and waits up to 15 seconds for a direct path. If none
   opens, it explains the likely reason (this device's network, what the owner
   offered) and asks whether to continue through the relay. The default answer
   is no. `--allow-relay` answers yes for non-interactive use; without a
   terminal and without the flag the answer is no. After the budget is used up,
   data arriving through the relay again (a lost direct path) leads to the same
   wait and question.
   A direct path lost earlier, while budget is left, needs nothing special: the
   data continues through the relay within the budget until iroh finds a direct
   path again.
5. **The relay is configurable.** The transport takes its relays from its
   configuration: the public n0 relays by default, and optionally further relay
   URLs **next to** n0's (never instead of them). The question to the user names
   the relays that would be used. Nothing in the policy is tied to n0.

## Why
- The public relays are someone else's service. Using them for introductions is
  what they are for; using them for bulk data is not something to build on.
- Asking, with the reason, is honest: the user decides whether a slow relay
  transfer is acceptable, instead of it happening silently.
- 100 MB keeps small shares (documents, a few photos) working everywhere without
  questions, while films and libraries cannot end up on a relay by accident.
- A budget instead of an up-front wait keeps the download moving: data flows
  through the relay during the seconds a direct path needs, instead of idling.

## Consequences
- A large download can put up to 100 MB on the relay; in practice only the few
  MB of the first seconds, since direct paths open within 1–2 seconds.
- On networks where hole punching fails (strict corporate firewalls, some
  IPv4-only mobile networks), a large download stops after its first 100 MB and
  15 more seconds, and asks.
  How often this happens is unknown; more field tests on other networks will
  tell.
- The budget is checked every 4 MiB, so up to about 4 MiB more than 100 MB can
  arrive through the relay before the reader stops.
- The budget counts per run of `get`: a resumed download starts a new budget.
  Keeping it per stick in the Store is possible if that is ever abused.
- A declined relay leaves a resumable download.

## Future direction (not in v1)
A relay of our own, e.g. `relay.ourstick.com`, added **next to** the n0 relays
through the configuration of step 5, not replacing them. Two tiers on it:

- **Free tier:** does exactly what the n0 relays do under this policy:
  introductions, hole punching coordination, the record and manifest, and
  transfers up to 100 MB.
- **Cloud tier (paid):** "continue through the relay" for larger transfers, with
  no limit on size or speed. The question in step 3 becomes "continue through
  OurStick Cloud" for subscribers.

This goes together with the cloud seed that CLAUDE.md already defers. CLAUDE.md
(no infrastructure of our own in v1) is unchanged, and no code for our own relay
is written now.
