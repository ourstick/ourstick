# ADR 0020: The Local Network permission on macOS

**Status:** proposed — 2026-10-08.

## Context

Since macOS 15, an app must be allowed to reach devices on the local
network ("Allow "OurStick" to find devices on local networks?"). The first
signed build asked on its first start (2026-10-08, macOS 26.7). Declined,
macOS refuses every datagram the app sends to an address on an attached
network: two computers on one Wi-Fi cannot reach each other directly, so a
transfer between them goes through a relay, at a fraction of the speed,
and nothing tells the user why. Transfers over the internet are not
affected (their direct paths go to public addresses).

macOS decides; an app cannot make the permission required, ask a second
time, or read its state. Once declined, it can only be allowed in System
Settings → Privacy & Security → Local Network. The CLI is not affected the
same way: run from Terminal it is Terminal's permission, over ssh none.

## Decision

Tell the user before macOS asks, and say so plainly whenever a declined
permission is what makes a transfer slow, with the way to fix it, until it
is fixed. Nothing changes in the protocol or the transfers.

1. **Our own words in macOS's question.** `NSLocalNetworkUsageDescription`
   in the bundle's `Info.plist` (Tauri merges `src-tauri/Info.plist`):
   "OurStick sends sticks directly to computers on the same network. Without
   this, they go through a relay server, much more slowly."
2. **A step before macOS asks.** At the app's first start on macOS, before
   any sharing or receiving starts networking, one screen: "OurStick will
   ask to find devices on your local network. Allow it: sticks then go
   straight to computers on the same network, much faster." `Continue`
   then runs the probe (rule 3), which makes macOS ask at that moment
   rather than in the middle of something else. Shown once (a row in
   SQLite, not a file); not on Linux or Windows.
3. **A probe: is it declined?** In `ourstick-core`, behind a trait
   (`LocalNetwork::access() -> Access { Allowed, Declined, Unknown }`), the
   macOS implementation sends one UDP datagram from an unbound socket to the
   default gateway (or to the first private IPv4 of an attached network)
   on the discard port; `EHOSTUNREACH` / `ENETUNREACH` right away means
   declined, success means allowed (or not decided yet: the send itself
   makes macOS ask), anything else unknown. Other systems: always
   `Allowed`. It is run at most every 10 seconds, in `spawn_blocking`.
   Tests use a fake.
4. **Same network?** A peer is on this computer's network when one of its
   IPv4 address hints is private (RFC 1918) and in the same /24 as one of
   this endpoint's own private IPv4 addresses, or one of its IPv6 hints is
   in the same /64 as one of ours (ULA or global). /24 and /64 are a
   heuristic: a wider netmask is missed (no message), and a wrong match
   alone shows nothing (rule 5 needs the other two). Reading the
   interfaces' real prefixes would need `netwatch` as a direct dependency;
   not now.
5. **When it is said.** On a receiver (the app's received sticks view, a
   transfer in progress), and on an owner (the activity line), when all
   three hold for 10 seconds: the peer is on the same network (rule 4),
   the path to it is the relay (`PathKind::Relay`), and the probe says
   declined. The message stays while the three hold, and goes once the
   permission is allowed (the probe says so) or the path becomes direct:
   > This is slow: the other computer is on your network, but OurStick
   > is not allowed to reach it. `Open Settings`
   `Open Settings` opens
   `x-apple.systempreferences:com.apple.preference.security?Privacy_LocalNetwork`
   (through the opener plugin). It replaces, for that peer, the generic
   relay message (ADR 0006 v2, "Reader paths"): it is more precise.
6. **The CLI** prints one line, once, under the same three conditions,
   naming Terminal: "macOS does not let Terminal reach this network: allow
   it in System Settings → Privacy & Security → Local Network."

## To verify on the Mac first (the permission is declined there now)

- That the probe (rule 3) sees `EHOSTUNREACH` in the signed app, and
  success once allowed; and whether the gateway is the right target on a
  network with client isolation.
- That the Settings URL opens the Local Network list on macOS 26.
- That a running app, once allowed, reaches the peer directly without a
  restart (iroh tries direct paths again by itself), and how soon.
- That the probe at "Continue" (rule 2) makes macOS ask, and that rule 1's
  text is what it shows. Asking again needs a new bundle identifier or a
  reset of the privacy database; note how.
- That two Macs (or a Mac and the Linux PC) on one Wi-Fi with the
  permission declined really go through the relay, and with it allowed go
  direct: the message must match what happens.

## Tests

The probe's result mapped from each error (a fake socket); same-network
matching (private /24, a public address, a /16 neighbour not matched,
IPv6 /64, no hints); the message's conditions as a small state machine
over time (10 seconds, cleared by allowed or direct, not shown for a
public peer); the first-start step shown once (SQLite row). The rest by
hand, as above.

## Alternatives considered

- **Ask again ourselves.** macOS does not allow it.
- **Only the first-start step.** A user who declines, or changes it
  later, is back to slow transfers with no reason given.
- **Only the message during a transfer.** It comes after the user has
  already said no, often without reading; the step before it makes the
  "yes" likely in the first place.
- **The message whenever the permission is declined.** Most transfers go
  over the internet, where it does not matter; telling then is noise.
