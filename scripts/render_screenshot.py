#!/usr/bin/env python3
"""Render a captured streamprobe dashboard as a PNG (the README screenshots).

1. Capture the dashboard in a pseudo-terminal of a fixed size, e.g.:

       script -q dash.tty sh -c 'stty cols 120 rows 46; streamprobe <URL> -t 50'

   (On Linux: script -q -c "stty cols 120 rows 46; streamprobe <URL> -t 50" dash.tty)

2. Render the last full frame:

       python3 scripts/render_screenshot.py dash.tty docs/images/dashboard.png \\
           --title "streamprobe — a live stream" --cols 120 --rows 46

The frame's ANSI colours become a terminal-style HTML page, which headless
Chrome/Chromium screenshots at 2x on a transparent background. Set CHROME to
the browser binary if it isn't found automatically. Standard library only.
"""
import argparse
import html
import os
import re
import shutil
import subprocess
import sys
import tempfile

COLOURS = {"31": "red", "32": "green", "33": "yellow", "35": "magenta", "36": "cyan"}
FONT_PX, LINE_HEIGHT = 13, 1.28

CSS = """
html,body{margin:0;background:transparent}
.win{margin:24px;display:inline-block;border-radius:10px;background:#0d1117;
  box-shadow:0 8px 28px rgba(0,0,0,.35),0 0 0 1px rgba(255,255,255,.08) inset;overflow:hidden}
.bar{height:30px;background:#161b22;display:flex;align-items:center;padding:0 12px;gap:8px;
  font:12px -apple-system,Helvetica,sans-serif;color:#8b949e}
.dot{width:12px;height:12px;border-radius:50%%}.t{flex:1;text-align:center;margin-right:52px}
pre{margin:0;padding:14px 18px 16px;font:%(px)dpx/%(lh)s Menlo,"DejaVu Sans Mono",monospace;
  color:#d0d7de;width:%(cols)dch;white-space:pre}
.bold{font-weight:bold;color:#f0f6fc}.dim{color:#7d8590}
.red{color:#ff7b72}.green{color:#56d364}.yellow{color:#e3b341}.magenta{color:#d2a8ff}.cyan{color:#79c0ff}
.bold.red{color:#ffa198}.bold.cyan{color:#a5d6ff}
"""


def last_frame(raw: str) -> str:
    raw = raw.split("\x1b[?1049l")[0]  # before the dashboard leaves the alternate screen
    frames = [f for f in re.split(r"\x1b\[H|\x1b\[1;1H", raw) if "LOUDNESS" in f]
    if not frames:
        sys.exit("no dashboard frame found in the capture (was it run in a terminal, without --no-ui?)")
    return frames[-1].replace("\r", "")


def to_html_lines(frame: str) -> list:
    lines, cur, buf, pos = [], [], "", 0
    st = {"b": False, "d": False, "c": None}

    def span(text):
        if not text:
            return ""
        cls = [c for c in ("bold" if st["b"] else "", "dim" if st["d"] else "", st["c"] or "") if c]
        t = html.escape(text)
        return f'<span class="{" ".join(cls)}">{t}</span>' if cls else t

    for m in re.finditer(r"\x1b\[([0-9;?]*)([A-Za-z])|\n", frame):
        buf += frame[pos:m.start()]
        pos = m.end()
        cur.append(span(buf))
        buf = ""
        if m.group(0) == "\n":
            lines.append("".join(cur))
            cur = []
            continue
        if m.group(2) != "m":
            continue  # erase-line and other non-colour sequences
        for code in (m.group(1) or "0").split(";"):
            if code in ("0", ""):
                st.update(b=False, d=False, c=None)
            elif code == "1":
                st["b"] = True
            elif code == "2":
                st["d"] = True
            elif code == "22":
                st.update(b=False, d=False)
            elif code == "39":
                st["c"] = None
            elif code in COLOURS:
                st["c"] = COLOURS[code]
    cur.append(span(buf + frame[pos:]))
    lines.append("".join(cur))
    return lines


def find_chrome() -> str:
    for c in (os.environ.get("CHROME"), "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
              shutil.which("google-chrome"), shutil.which("chromium"), shutil.which("chromium-browser")):
        if c and os.path.exists(c):
            return c
    sys.exit("Chrome/Chromium not found: set CHROME to the browser binary")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("capture", help="pseudo-terminal capture (from `script`)")
    ap.add_argument("out", help="PNG to write")
    ap.add_argument("--title", default="streamprobe")
    ap.add_argument("--cols", type=int, default=120)
    ap.add_argument("--rows", type=int, default=46)
    a = ap.parse_args()

    lines = to_html_lines(last_frame(open(a.capture, "rb").read().decode("utf-8", "replace")))
    lines = (lines + [""] * a.rows)[: a.rows]
    while lines and not re.sub(r"<[^>]+>", "", lines[-1]).strip():
        lines.pop()  # no empty terminal rows at the bottom

    page = (
        '<!doctype html><html><head><meta charset="utf-8"><style>'
        + CSS % {"px": FONT_PX, "lh": LINE_HEIGHT, "cols": a.cols}
        + '</style></head><body><div class="win"><div class="bar">'
        + "".join(f'<span class="dot" style="background:{c}"></span>' for c in ("#ff5f57", "#febc2e", "#28c840"))
        + f'<span class="t">{html.escape(a.title)}</span></div><pre>'
        + "\n".join(lines)
        + "</pre></div></body></html>"
    )
    width = int(a.cols * FONT_PX * 0.602 + 36 + 48 + 8)  # Menlo advance ≈ 0.602 em, padding, margins
    height = int(len(lines) * FONT_PX * LINE_HEIGHT + 110)
    with tempfile.TemporaryDirectory() as tmp:
        page_path = os.path.join(tmp, "frame.html")
        open(page_path, "w").write(page)
        subprocess.run(
            [find_chrome(), "--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-first-run",
             "--no-default-browser-check", f"--user-data-dir={os.path.join(tmp, 'profile')}",
             "--default-background-color=00000000", "--force-device-scale-factor=2",
             f"--window-size={width},{height}", f"--screenshot={os.path.abspath(a.out)}", "file://" + page_path],
            check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
    print(f"{a.out}: {len(lines)} rows")


if __name__ == "__main__":
    main()
