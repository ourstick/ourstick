#!/bin/bash
# The memory, threads and time of a reader of a whole stick, owner and
# reader on this machine (Linux: it reads /proc). The reader's data and
# files go to RAM (/dev/shm), so it does not share the owner's disk.
#
#   reader-memory.sh EXE STICK_DIR [OWNER_EXE]
#
# STICK_DIR holds `src/` (the folder shared); the owner's state is kept in
# STICK_DIR/owner, so a second run is not a first catalogue. Samples every
# 5 s go to STICK_DIR/reader-memory.log, then the peak and the time. The
# stick's link is never printed, and every output holding it is shredded.
set -u
EXE=$1; S=$2; OWNER_EXE=${3:-$1}
HERE=$(dirname "$(readlink -f "$0")")
R=/dev/shm/ourstick-measure-$$
umask 077
mkdir -p "$S/owner" "$R/data" "$R/out"
python3 "$HERE/owner.py" "$OWNER_EXE" "$S/owner" "$S/src" "$S/owner/passphrase" "$R/share.out" &
driver=$!
until [ -f "$R/share.out" ] && tr '\r' '\n' < "$R/share.out" | grep -aq "Read all\|were as last time"; do
  kill -0 $driver 2>/dev/null || { echo "the owner ended"; exit 1; }
  sleep 1
done
owner=$(pgrep -P $driver)
link=$(grep -ao 'https://ourstick.com/s/[^[:space:]]*' "$R/share.out" | head -1)
# The link on stdin ("-"), never on the command line, which ps shows.
OURSTICK_DATA_DIR="$R/data" RUST_LOG=warn /usr/bin/time -v "$EXE" get - --out "$R/out" \
  <<<"$link" > "$R/get.out" 2> "$R/get.err" &
timer=$!
unset link
sleep 2
reader=$(pgrep -P $timer)
LOG=$S/reader-memory.log; : > "$LOG"
while kill -0 "$reader" 2>/dev/null; do
  echo "$(date +%T) rss_kb=$(awk '/VmRSS/{print $2}' /proc/$reader/status 2>/dev/null)" \
    "threads=$(ls /proc/$reader/task 2>/dev/null | wc -l)" \
    "files=$(find "$R/out" -type f | wc -l)" >> "$LOG"
  sleep 5
done
wait $timer
kill -INT "$owner"; while kill -0 "$owner" 2>/dev/null; do sleep 1; done
grep -aE "Done in" "$R/get.out" | sed 's|https://[^ ]*|<link>|g' >> "$LOG"
grep -E "Maximum resident|Elapsed" "$R/get.err" >> "$LOG"
shred -u "$R/share.out" "$R/get.out" "$R/get.err"
rm -rf "$R"
tail -3 "$LOG"
