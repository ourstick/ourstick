#!/usr/bin/env python3
"""Runs `ourstick share` on a folder under a pseudo-terminal, for
measurements: answers the key file's passphrase from a file, and writes
everything the owner prints to OUT (mode 0600: it holds the stick's link,
which must never be shown or logged; shred it afterwards).

    owner.py EXE DATA_DIR FOLDER PASSPHRASE_FILE OUT [--rescan]

The passphrase file is made (12 random bytes, hex, mode 0600) if missing.
Ends when the owner ends; stop the owner with SIGINT (its pid is this
process's child).
"""
import os
import pty
import select
import sys
import time


def main():
    exe, data_dir, folder, phrase_file, out_path = sys.argv[1:6]
    rescan = "--rescan" in sys.argv[6:]
    if not os.path.exists(phrase_file):
        fd = os.open(phrase_file, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.write(fd, os.urandom(12).hex().encode())
        os.close(fd)
    phrase = open(phrase_file).read().strip()
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["OURSTICK_DATA_DIR"] = data_dir
        os.environ.setdefault("RUST_LOG", "warn")
        args = ["ourstick", "share", "--name", "Measurement",
                "--key-file", os.path.join(data_dir, "keys.file"), folder]
        if rescan:
            args.insert(2, "--rescan")
        os.execv(exe, args)
    out = os.open(out_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    seen = b""
    answered = 0
    while True:
        try:
            ready, _, _ = select.select([fd], [], [], 1)
            if not ready:
                continue
            data = os.read(fd, 65536)
            if not data:
                break
            os.write(out, data)
            seen += data
            last = seen.rstrip().lower().splitlines()[-1] if seen.strip() else b""
            # The passphrase is asked once for an existing key file, twice
            # for a new one.
            if answered < 2 and last.endswith(b":") and b"passphrase" in last:
                time.sleep(1.5)
                os.write(fd, (phrase + "\n").encode())
                answered += 1
                seen = b""
        except OSError:
            break


if __name__ == "__main__":
    main()
