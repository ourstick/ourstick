# ADR 0020: The Local Network permission on macOS

**Status:** accepted — 2026-10-08. Reviewed by two other models against
our code and iroh 1.2; this version takes in their changes. Built; what
differs is under "As built".

## Context

Since macOS 15, an app must be allowed to reach devices on the local
network ("Allow "OurStick" to find devices on local networks?"). The first
signed build asked on its first start (2026-10-08, macOS 26.7). Declined,
macOS refuses what the app sends to an address on an attached network: two
computers on one Wi-Fi cannot reach each other directly (a direct path
needs both sides to send), so a transfer between them goes through a
relay, at a fraction of the speed, and nothing tells the user why.
Transfers over the internet are not affected (their direct paths go to
public addresses), nor are Tailscale's 100.64.0.0/10 addresses (a `utun`
interface, not a local network), nor two programs on one Mac (loopback).

macOS decides; an app cannot make the permission required, ask a second
time, or read its state. Once declined, it can only be allowed in System
Settings → Privacy & Security → Local Network. The permission belongs to
the responsible process, keyed by the bundle identifier
(`com.ourstick.OurStick`) and signature: WKWebView's helpers count as the
app; the CLI, run from Terminal, has Terminal's; over ssh, none applies;
`cargo tauri dev` has the terminal's, so this is tested in the signed
bundle only.

What asks, today: iroh's bind. Its portmapper (UPnP, NAT-PMP, PCP) talks to
the router as soon as an endpoint is bound, and `sharing::start_app` binds
the sticks' endpoints in `setup()` when any stick exists. So macOS asks at
the app's first start, whatever the app is showing then.

## Decision

Explain it before macOS asks, and say so plainly whenever a declined
permission is what makes a transfer slow, with the way to fix it, until it
is fixed. The protocol does not change.

1. **Our own words in macOS's question.** A new `src-tauri/Info.plist`
   (Tauri merges it into the bundle's) with
   `NSLocalNetworkUsageDescription`: "OurStick sends sticks directly to
   computers on the same network. Without this, they go through a relay
   server, much more slowly."
2. **A step before macOS asks.** On macOS, at the first start of a version
   with this ADR (a fresh install, or an upgrade: an earlier answer cannot
   be read, rule 3 then tells), the window opens on one screen before
   anything else: "OurStick will ask to find devices on your local network.
   Allow it: sticks then go straight to computers on the same network,
   much faster." `Continue`. The app binds no endpoint (`start_app` and
   the receiving side wait) until `Continue`; so macOS asks right after
   it, for the bind. Remembered in the `device` table (a new column,
   `local_network_explained`, with its migration and `docs/schema.md`),
   meaning only "the step was shown", never the permission's state. Not
   on Linux or Windows.
3. **Is it declined? A probe.** In `ourstick-core`, behind a trait
   (`LocalNetwork::access() -> Access`) with a fake for tests, using
   `std::net::UdpSocket` only (no new dependency): a socket connected to
   another address of this computer's own private IPv4 /24 (`.253`, or
   `.252` if that is its own; not the router: macOS lets every app reach
   it), sends twice, 100 ms apart, to port 9 (discard). macOS refuses a
   connected socket's sends with `EPIPE` ("Broken pipe") when the
   permission is declined or not answered yet, at once, every time; that
   is `Declined`. Allowed, the sends go (`Ok`), and for an address no
   computer has they fail later with `EHOSTDOWN` or `EHOSTUNREACH` (the
   router's ARP gave up): those, and "connection refused", are
   `NotDeclined`, never `Declined`. An unconnected `send_to` cannot tell
   the two apart (`EHOSTUNREACH` both ways), so it is not used. No private
   IPv4 (IPv6-only) or any other error: `Unknown`. A VPN's full tunnel
   whose own address is private probes the tunnel, which macOS does not
   gate: `NotDeclined`, a message missed, never a false one.
   Other systems: always `NotDeclined`. At most every 10 seconds, in
   `spawn_blocking`. Not answered yet looks the same as declined, so the
   app's first-start step (rule 2) is what keeps the message from showing
   while macOS's question is still open: the facts of rule 5 start only
   after it.
4. **Same network?** Each side asks iroh for the other's addresses,
   `Endpoint::remote_info(peer).addrs()`: what iroh learned of the peer
   over the relay (its candidate direct addresses, perhaps stale), a
   snapshot, asked again at each 10-second check. On the reader this
   includes the ticket's hints; on the owner it is the only source (the
   ticket carries the owner's hints only). A peer is on this computer's
   network when one of its addresses is private IPv4 (RFC 1918) in the
   same /24 as one of this computer's own, or IPv6 (ULA or global, not
   link-local) in the same /64; this computer's own addresses excluded
   (the same Mac). /24 and /64 are a heuristic: a wider network is missed
   (no message), and a wrong match alone shows nothing (rule 5 needs the
   other two). The interfaces' real prefixes would come from `netwatch`
   (already in `Cargo.lock` through iroh), but as a direct dependency it
   needs approval; not asked for now.
5. **When it is said, and where.** In the core, as facts, not words: for
   a connection, "local network declined here" holds when, for 10
   seconds, the peer is on the same network (rule 4), the path to it is
   the relay (`PathKind::Relay`), and the probe says `Declined`. The
   reader's progress and the owner's `ReaderActivity` carry it; the words,
   the button and the delay's display belong to the app and the CLI (as
   `serving/events.rs::relay_only_message` does). The app shows, on the
   receiving view of that stick or the owner's activity line:
   > This is slow: the other computer is on your network, but OurStick
   > is not allowed to reach it. `Open Settings`
   > (System Settings → Privacy & Security → Local Network)
   `Open Settings` opens
   `x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork`
   through the opener plugin (its capability allowing that scheme); the
   path under it stays in case the link fails. It takes the place of the
   generic relay message for that peer. It goes when the probe says
   `NotDeclined` or the path becomes direct; if the path stays on the
   relay, the generic message returns (the other side may be the one
   declined). Allowed while a connection is on the relay, a new
   connection goes direct; the one going on may stay on the relay until
   the next transfer, unless the app calls `Endpoint::network_change()`
   when the probe turns `NotDeclined` (to be tried on the Mac). The
   message names no text from the peer.
6. **Only the declined side can know.** A Mac that declined, sharing to a
   Linux or Windows reader on the same network: the reader's probe says
   `NotDeclined`, so it shows the generic relay message; the owner shows
   this one. Both Macs declined: each shows it; one allowing clears its
   own. `docs/protocol.md` ("Reader paths") says that a Mac's declined
   permission, on either side, looks like a firewall to the other, and
   the generic message (ADR 0006 v2, `relay_reasons`) adds "If the other
   computer is a Mac, it may need to allow OurStick on its local network."
   when the peer is on the same network.
7. **The CLI** prints one line, once, under the same facts (on `get`, and
   on `share` from `ReaderActivity`), naming Terminal: "macOS does not let
   Terminal reach this network: allow it in System Settings → Privacy &
   Security → Local Network."

## Seen on the Mac (2026-10-08, macOS 26.7, the signed bundle)

A build that ran the probe every 2 seconds, from macOS's question to the
answer "Don't Allow" and then allowing it in Settings, against
`192.168.1.253` (no computer there) and the router `192.168.1.1`:

| | connected `send` to .253 | `send_to` .253 | the router |
|---|---|---|---|
| question open | `EPIPE`, `EPIPE` | `EHOSTUNREACH` | goes |
| declined | `EPIPE`, `EPIPE` | `EHOSTUNREACH` | goes |
| allowed | `Ok`, `Ok`; then `EHOSTDOWN`/`EHOSTUNREACH` in turns (no ARP answer) | `Ok`, then the same | goes |

Allowed in Settings, the running app could send at once (the next probe,
within 2 seconds): no restart. Settings listed OurStick twice (the
installed app and the test build, one bundle identifier at two paths),
each with its own switch.

The signed app (afecc37), the Mac receiving 2 GB from the Linux PC on one
Wi-Fi (2026-10-08), the Mac's permission declined before the app started:

- With Tailscale up on both, the data went over Tailscale's 100.x
  addresses at 14.5 MB/s (a `utun` interface is no local network), after
  a first connection that found nothing for 50 seconds ("The sender's
  computer is off"). With the router's hairpin (the owner's public
  address), it can go direct too.
- With Tailscale down on the PC: through the relay at about 1 MB/s, and
  after about 15 seconds the received stick showed "This is slow: the
  sender is on your network, but OurStick is not allowed to reach it."
  with Open Settings. Five minutes later, still declined and with nothing
  touched, a direct path 192.168.1.20 → 192.168.1.30 opened (the owner had
  sent first, holepunching; macOS seems to let a reply through) and the
  message went by itself. Not proven; to see again.

Still to see: whether macOS 26 shows rule 1's text at all (every question
seen so far showed macOS's own, from a build before the text or an
unsigned one, which macOS also names by its executable, "ourstick-app");
it asks once per app, so on a Mac that never ran OurStick; Open Settings pressed; that the bind is what makes macOS ask and rule 1's text is
what it shows; that the Settings link opens that list; that a connection
already on the relay goes direct once allowed (with and without
`network_change()`); the Linux PC and the Mac on one Wi-Fi, declined and
allowed; and how to ask again for a test.

## As built

- The probe, the same-network test and the ten-second facts are in
  `local_network/`; iroh's side (`transport/iroh/local_network.rs`)
  watches each connection, the owner's (`handler.rs`, its endpoint now
  bound before the handler) and the reader's (`fetcher.rs`). The probe is
  shared per endpoint and asked at most every ten seconds.
- A connection already on the relay when the permission is allowed goes
  direct with the download's next look for a direct path (ADR 0006's
  relay watch connects again after a minute, then less often); no
  `network_change()` call.
- Rule 6, the generic message: `relay_reasons` names a Mac's Local
  Network first when the owner is on this network, by the ticket's hints
  (the CLI's `get`). The owner's firewall message is unchanged: it cannot
  tell which reader is on its network when it is not itself declined.
- The step is remembered in `device.local_network_explained`; on a first
  launch, before the device exists, it is kept for the run and written
  once the device is set up.
- One place decides when the app may bind: `local_network::may_bind`,
  awaited before every endpoint (a stick's publisher, a received stick's
  fetcher, a link's preview), open once the step is done or not due. So
  the tray's Start sharing or Mount, before the step, waits too. Until
  "Continue", an upgraded Mac shares nothing: the window opens on the
  step at start.
- One probe for the whole program (each stick has its own endpoint, ADR
  0009), one answer per look; each look counts at its tick's time, so the
  message comes ten seconds after the facts begin, at the second look.
- The same-network test also compares public IPv4 addresses when both
  sides' are known: two homes on the same router defaults (192.168.1.x)
  are not one network.
- The owner's message only while someone downloads; a reader's watchers
  end with its connection (closed, or dropped on an error).
- A reader that connects again (to look for a direct path) starts its
  facts afresh: the message goes for ten seconds, then comes back.
- Not covered by the two-peer tests: on one machine a peer's addresses
  are this computer's own, never "on this network", so the iroh glue
  (`watch_declined`, `on_this_network`) is checked by hand on the Mac.

## Tests

The probe's result mapped from each error, and two-in-a-row agreement (a
fake); same-network matching (private /24, a public address, a /16
neighbour not matched, IPv6 /64, link-local ignored, our own address
excluded, 100.64/10 not matched, no addresses); the facts over time (10
seconds, cleared by `NotDeclined` or a direct path, not for a peer on
another network); the step shown once and the endpoints bound only after
it (the `device` column). The rest by hand, as above.

## Alternatives considered

- **Ask again ourselves.** macOS does not allow it.
- **Only the first-start step.** A user who declines, or changes it
  later, is back to slow transfers with no reason given.
- **Only the message during a transfer.** It comes after the user has
  already said no, often without reading; the step makes the "yes"
  likely in the first place.
- **The message whenever the permission is declined.** Most transfers go
  over the internet, where it does not matter; telling then is noise.
- **The reader's addresses in its signed progress report (ADR 0015)**, for
  the owner's rule 4: a protocol change, and the reader's private
  addresses sent on purpose; `remote_info` already has them.
- **Detect only from iroh's failed sends.** No probe to prove, but it
  depends on what iroh exposes; kept as the fallback of rule 3.
