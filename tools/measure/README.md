# Measuring OurStick

Tools for the measurements the ADRs ask for (memory, threads, time). They
run the release CLI on test folders of your own; nothing here is part of
OurStick.

- `owner.py`: `ourstick share` of a folder under a pseudo-terminal, the
  key file's passphrase answered from a file. Its output holds the stick's
  link: it goes to a 0600 file, never to the screen; shred it after.
- `reader-memory.sh EXE STICK_DIR [OWNER_EXE]`: owner and reader of
  `STICK_DIR/src` on this machine (Linux), the reader writing to RAM; logs
  the reader's RSS, threads and files every 5 s, then its peak and time.
  Drop the owner's files from the system's cache first for a cold disk
  (`posix_fadvise(POSIX_FADV_DONTNEED)` on each file needs no root).
- The plan's heap for 100 000 files: the ignored test in
  `crates/ourstick-core/src/download/folder/gauge_tests.rs`, under
  heaptrack (the command is at its top; a build with symbols needs
  `CARGO_PROFILE_RELEASE_STRIP=false`).

Keep test folders off the system disk. Share copies, never a folder you
cannot lose: the app writes a marker into a folder it shares
(`.ourstick-drive`, see `docs/protocol.md`).
