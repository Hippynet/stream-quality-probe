#!/usr/bin/env python3
"""Fake Icecast/Shoutcast server for testing streamprobe.

    sh tests/make_fixtures.sh
    python3 tests/fake_icecast.py [fixtures_dir] [port]      # default tests/fixtures 18765
    streamprobe http://127.0.0.1:18765/faults -t 115          # start within ~2s of the server

Serves an MP3 file as a live stream (looped, paced at the bitrate, 64 KB burst),
with ICY metadata. Fault injection by wall-clock offset from server start.

  /faults      faults.mp3, 'ICY 200 OK' status, titles per segment,
               stall at STALL_AT for STALL_LEN s, drop connection at DROP_AT
  /upscaled    upscaled.mp3 @192k (low bandwidth source)
  /mono        monostream.mp3 @64k (1 channel)
  /wrongtype   faults.mp3 served as audio/aac
  /badmeta     advertises icy-metaint 16000 but inserts metadata every 8000
               bytes, so metadata bytes end up inside the audio (frame_sync)
  /hls/live.m3u8  live HLS: a sliding 3-segment window over hls/seg*.ts
  /garbage     200 audio/mpeg, then bytes that aren't audio (no_audio)
  /listen.pls  playlist -> /faults
  /page        HTML page
  anything else 404
"""
import socket
import socketserver
import sys
import threading
import time

import os
DIR = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "fixtures")
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 18765
STALL_AT, STALL_LEN, DROP_AT = 78.0, 8.0, 100.0
START = time.time()
TITLES = [(0, "Test - Stereo"), (15, "Test - Dual mono"), (30, "Test - Out of phase"),
          (45, "Test - Silence"), (51, "Test - Left only"), (66, "Test - Clipping"),
          (78, "Test - Stereo again")]
MOUNTS = {"/faults": ("faults.mp3", 128, "audio/mpeg", True),
          "/upscaled": ("upscaled.mp3", 192, "audio/mpeg", False),
          "/mono": ("monostream.mp3", 64, "audio/mpeg", False),
          "/wrongtype": ("faults.mp3", 128, "audio/aac", False),
          "/badmeta": ("faults.mp3", 128, "audio/mpeg", False)}
HLS_SEG = 4  # seconds per segment (make_fixtures.sh)
DATA = {}
dropped = set()


def title_at(off):
    t = TITLES[0][1]
    for s, name in TITLES:
        if off % 93 >= s:
            t = name
    return t


class H(socketserver.BaseRequestHandler):
    def handle(self):
        s = self.request
        req = b""
        while b"\r\n\r\n" not in req:
            c = s.recv(4096)
            if not c:
                return
            req += c
        line = req.split(b"\r\n")[0].decode()
        path = line.split()[1]
        meta = b"icy-metadata: 1" in req.lower()
        if path == "/listen.pls":
            body = f"[playlist]\nNumberOfEntries=1\nFile1=http://127.0.0.1:{PORT}/faults\n"
            s.sendall(f"HTTP/1.0 200 OK\r\nContent-Type: audio/x-scpls\r\n\r\n{body}".encode())
            return
        if path == "/page":
            s.sendall(b"HTTP/1.0 200 OK\r\nContent-Type: text/html\r\n\r\n"
                      b"<html><head><title>Icecast Status</title></head><body>hi</body></html>")
            return
        if path == "/garbage":
            s.sendall(b"HTTP/1.0 200 OK\r\nContent-Type: audio/mpeg\r\n\r\n")
            try:
                while True:
                    s.sendall(bytes((i * 7) % 251 for i in range(4000)))
                    time.sleep(0.25)
            except OSError:
                return
        if path.startswith("/hls/"):
            self.hls(s, path)
            return
        if path not in MOUNTS:
            s.sendall(b"HTTP/1.0 404 File Not Found\r\nContent-Type: text/html\r\n\r\n"
                      b"<b>The file you requested could not be found</b>")
            return
        fname, kbps, ctype, faults = MOUNTS[path]
        data = DATA[fname]
        rate = kbps * 1000 // 8
        metaint = 16000
        real_metaint = 8000 if path == "/badmeta" else metaint
        status = "ICY 200 OK" if faults else "HTTP/1.0 200 OK"
        hdr = (f"{status}\r\nContent-Type: {ctype}\r\nicy-name: Fake Test FM\r\n"
               f"icy-genre: Testing\r\nicy-br: {kbps}\r\nServer: FakeIcecast 0.1\r\n")
        if meta:
            hdr += f"icy-metaint: {metaint}\r\n"
        s.sendall((hdr + "\r\n").encode())
        # live position, minus 64 KB burst
        pos = int((time.time() - START) * rate) - 65536
        pos = max(0, pos)
        since_meta = 0
        stalled = False
        last_title = None
        sent_until = time.time()

        def send_audio(chunk):
            nonlocal since_meta, last_title
            out = b""
            while chunk:
                n = min(len(chunk), real_metaint - since_meta) if meta else len(chunk)
                out += chunk[:n]
                chunk = chunk[n:]
                since_meta += n
                if meta and since_meta == real_metaint:
                    since_meta = 0
                    t = title_at(time.time() - START) if faults else "Some Artist - Some Track"
                    if t != last_title:
                        last_title = t
                        m = f"StreamTitle='{t}';".encode()
                        m += b"\x00" * ((16 - len(m) % 16) % 16)
                        out += bytes([len(m) // 16]) + m
                    else:
                        out += b"\x00"
            s.sendall(out)

        try:
            send_audio((data[pos % len(data):] + data)[:65536])
            pos += 65536
            while True:
                off = time.time() - START
                if faults and not stalled and STALL_AT <= off < STALL_AT + 1:
                    stalled = True
                    time.sleep(STALL_LEN)          # network stall: data delayed
                if faults and off >= DROP_AT and path not in dropped:
                    dropped.add(path)
                    s.close()
                    return
                n = rate // 10
                i = pos % len(data)
                chunk = (data[i:] + data)[:n]
                send_audio(chunk)
                pos += n
                sent_until += 0.1
                d = sent_until - time.time()
                if d > 0:
                    time.sleep(d)
        except (BrokenPipeError, ConnectionResetError, OSError):
            return

    def hls(self, s, path):
        name = path.split("?")[0].rsplit("/", 1)[-1]
        if name == "live.m3u8":
            nseg = len([f for f in os.listdir(f"{DIR}/hls") if f.endswith(".ts")])
            now = int((time.time() - START) // HLS_SEG)
            first = max(0, min(now, nseg - 1) - 2)
            last = min(now, nseg - 1)
            body = f"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{HLS_SEG}\n#EXT-X-MEDIA-SEQUENCE:{first}\n"
            for i in range(first, last + 1):
                body += f"#EXTINF:{HLS_SEG}.0,\nseg{i:03d}.ts\n"
            if now >= nseg - 1:
                body += "#EXT-X-ENDLIST\n"
            data, ctype = body.encode(), "application/vnd.apple.mpegurl"
        else:
            try:
                data, ctype = open(f"{DIR}/hls/{os.path.basename(name)}", "rb").read(), "video/mp2t"
            except OSError:
                s.sendall(b"HTTP/1.0 404 Not Found\r\n\r\n")
                return
        s.sendall(f"HTTP/1.0 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {len(data)}\r\n\r\n".encode() + data)


for f in {m[0] for m in MOUNTS.values()}:
    DATA[f] = open(f"{DIR}/{f}", "rb").read()
socketserver.ThreadingTCPServer.allow_reuse_address = True
socketserver.ThreadingTCPServer.request_queue_size = 64  # the e2e suite opens ~8 at once
srv = socketserver.ThreadingTCPServer(("127.0.0.1", PORT), H)
srv.daemon_threads = True
print(f"fake icecast on {PORT}", flush=True)
srv.serve_forever()
