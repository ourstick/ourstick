# ADR 0006: Relay policy — never stop, never ask; limit the relay's speed

**Status:** accepted, version 2 — 2026-09-29. Replaces version 1 of the same
day (a 100 MB relay budget per download, then a question; see "History").

## Context
OurStick uses iroh. When two peers cannot reach each other directly, iroh
carries their QUIC traffic through a relay server. Today these are the public
relays run by n0. They are free to us, carry no guarantee of capacity or
availability, and are not meant to move people's films and photo libraries.
Relays of our own would cost bandwidth in proportion to what users share.

Field tests (2026-09-28/29) between a PC on a home connection, a phone on
4G/5G and a MacBook on the phone's hotspot found a direct path within about 2
seconds every time, over IPv6 and over IPv4 through carrier NAT. A relay-only
test (direct paths disabled on purpose) moved data through the n0 relay at
about 1 MB/s.

Version 1 stopped a large download after 100 MB through the relay and asked
the user whether to go on. That breaks the promise of the product, a USB stick
in a link: a stick does not stop halfway and ask questions. The desktop app
(stage 1, `docs/gui-spec.md`) has no receiver window to ask in, either.

## Decision
1. **Never stop, never ask.** If there is no direct path, the transfer starts
   and continues through the relay at once. iroh keeps looking for a direct
   path the whole time; when one opens, the data moves there by itself.
2. **Say so, and keep saying it.** While data arrives through the relay, the
   reader shows a message that stays visible: "No direct connection found,
   continuing through the relay, up to 10 Mbps on the free relays" (the number
   is the configured limit). The likely reasons (this network, what the owner
   offered) are explained once, as before.
3. **A speed limit on relay bytes only.** The reader limits the rate of data
   that arrives through a relay to a configured limit, **10 Mbps** by default.
   Data over a direct path is never limited. When a direct path opens, the
   limit stops applying at once.
   The limit is enforced by the reader: it slows down taking data from the
   connection, and QUIC flow control slows the sender. Readers are our own
   software. On our own relays the server enforces a limit too (iroh-relay's
   `ClientRateLimit`), since a modified reader could ignore its own.
4. **Relays are configurable.** The transport takes its relays from its
   configuration: the public n0 relays by default, and optionally further relay
   URLs next to them. Nothing in the policy is tied to n0.
5. **HTTP(S) proxies.** The endpoint takes a proxy from `HTTP_PROXY`,
   `http_proxy`, `HTTPS_PROXY` or `https_proxy` (in that order) for its relay
   connections. iroh 1.x supports this (`Endpoint::builder().proxy_from_env()`),
   but only when asked, so `bind()` asks.
6. **Removed:** the question, `--allow-relay`, the 100 MB budget, and the 5 s
   and 15 s waits for a direct path.

## Why
- A slow transfer that finishes is what a stick does; a stopped one that asks a
  question the user cannot judge is not.
- A rate limit bounds what a relay costs per second, whatever the size of the
  share, and keeps the free relays usable for everyone.
- 10 Mbps is about what the n0 relay gave in the field test anyway (~1 MB/s),
  so the limit costs almost nothing today and protects our own relays later.
- Direct paths open within 1–2 seconds in every test so far, so almost every
  transfer never meets the limit.

## Consequences
- On a network where hole punching fails, a large transfer is slow but
  finishes: 10 GB takes about 2 h 15 min at 10 Mbps, and all of it crosses the
  relay. Volume is not bounded, only rate.
- **Our own relays are a prerequisite for the first public release.**
  `relay.ourstick.com` comes first, with the n0 relays as a fallback, so that
  public users do not put their bulk data on n0's service. iroh chooses a home
  relay by measured latency, not by list order; how to prefer ours (only ours
  plus n0 as a fallback, or a latency bias) is decided when it is set up.
- The server-side limit on our relays is what really bounds cost; the reader's
  limit only keeps honest readers polite.

## Tests (for the implementation)
- A direct path is found: no limit, no relay message.
- Relay only: the transfer finishes, its relay rate stays at or under the
  limit, and the message is shown.
- A direct path appears in the middle: the rest arrives directly and the limit
  no longer applies.

## Future direction (not in v1)
- **Free tier** on our relays: the same as the n0 relays under this policy,
  including the rate limit.
- **Cloud tier (paid):** no limit on relay speed, and the cloud seed that
  CLAUDE.md defers.

## History
- **Version 1 (2026-09-29):** relays introduce, direct paths carry data. A 5 s
  wait for a direct path, then up to 100 MB per download through the relay;
  after that a 15 s wait and a question (default no, `--allow-relay` for
  scripts). Replaced the same day by version 2, above.
