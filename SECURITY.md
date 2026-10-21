# Security

OurStick encrypts files on the sender's computer and sends them peer to
peer; the design is in [docs/protocol.md](docs/protocol.md) and the
decisions behind it in [docs/decisions](docs/decisions). It has not yet been
reviewed by an independent auditor.

## Reporting a vulnerability

Please write to **hi@ourstick.com** with "Security" in the subject, or use
GitHub's private reporting (the repository's Security tab, **Report a
vulnerability**), and do not open a public issue. Say what you found, how to reproduce it, which system
you used, and when you downloaded OurStick (or which release).

Never send a real sharing link: a link carries a stick's key. If a link is
part of the problem, describe it, or make a test stick and send its link
after removing the stick.

I will try to answer within a week. Once a fix is released, it is named in
the release notes, with credit to you unless you prefer not.

## Supported versions

Only the latest release gets fixes. New releases are announced on
[GitHub Releases](https://github.com/ourstick/ourstick/releases): choose
Watch → Custom → Releases there to be told, or follow
`https://github.com/ourstick/ourstick/releases.atom`. OurStick does not
check for updates by itself.
