#!/bin/sh
# Stand-in for streamprobe in the session tests. The "URL" argument picks a mode:
#   replay:FILE  print FILE (next to this script) line by line, exit 2
#   die          print a start line, exit 9 (no final line)
#   error        print an error line, exit 3
#   stubborn     ignore SIGINT, print a start line, sleep
here=$(dirname "$0")
case "$1" in
  replay:*)
    while IFS= read -r line; do printf '%s\n' "$line"; sleep 0.01; done <"$here/${1#replay:}"
    exit 2 ;;
  die)
    printf '{"type":"start","version":"t","url":"die"}\n'
    echo "fake: dying" >&2
    exit 9 ;;
  error)
    printf '{"type":"error","code":"ffmpeg_missing","message":"ffmpeg not found"}\n'
    exit 3 ;;
  stubborn)
    trap '' INT
    printf '{"type":"start","version":"t","url":"stubborn"}\n'
    sleep 30 & wait ;;
esac
