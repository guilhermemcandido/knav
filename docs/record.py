#!/usr/bin/env python3
"""Records a scripted knav session as an asciicast (docs/demo.cast), then turns it into
docs/demo.gif and the pictures in docs/ (needs `agg` and `ffmpeg`, and a cluster).

    cargo build --release && python3 docs/record.py
"""
import codecs, fcntl, json, os, pty, select, signal, struct, subprocess, sys, termios, time

ROWS, COLS = 40, 140
ENTER, ESC, DOWN = "\r", "\x1b", "\x1b[B"
# (what to send, seconds to wait after, name of a picture to keep from that moment)
SCRIPT = [
    ("", 0.7, "loading"), ("", 3.3, "home"),
    # A throwaway keypress first: the pty seems to drop the very first key sent
    # after a long idle wait, so warm it up with a no-op round trip (Down then
    # Up lands back on Resources, same as where "home" was captured) before
    # the real navigation below, or ":" ends up silently eaten.
    (DOWN, 1.1, None), ("\x1b[A", 1.1, None),
    (":", 0.4, None), ("pods", 0.4, None), (ENTER, 2.2, "pods"),
    # The sidebar only toggles cleanly from inside a list; from the Overview
    # itself it renders wrong, so it's shown here rather than from "home".
    ("b", 1.6, "sidebar"),
    (DOWN, 0.4, None), (DOWN, 0.4, None), ("i", 2.2, "info"), ("i", 0.6, None),
    ("R", 2.4, "related"), (ESC, 0.8, None),
    ("/", 0.4, None), ("work", 0.5, None), (ENTER, 1.6, "search"),
]


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    env = dict(os.environ, TERM="xterm-256color", XDG_CONFIG_HOME="/tmp/knav-demo")
    pid, fd = pty.fork()
    if pid == 0:
        os.execve(os.path.join(root, "target/release/knav"), ["knav"], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
    start = time.time()
    events, marks = [], {}
    # A read can end in the middle of a multi-byte character.
    decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def pump(seconds):
        end = time.time() + seconds
        while time.time() < end:
            ready, _, _ = select.select([fd], [], [], 0.02)
            if ready:
                try:
                    data = os.read(fd, 65536)
                except OSError:
                    return False
                events.append([round(time.time() - start, 3), "o", decoder.decode(data)])
        return True

    for keys, wait, name in SCRIPT:
        for key in ([keys] if keys.startswith("\x1b") or len(keys) <= 1 else list(keys)):
            os.write(fd, key.encode())
            pump(0.09)
        if not pump(wait):
            break
        if name:
            marks[name] = round(time.time() - start, 2)
    # Killed rather than quit normally: knav dims the screen on its way out,
    # and that fade is the last thing agg would otherwise hold on the still frame.
    os.kill(pid, signal.SIGKILL)
    docs = os.path.join(root, "docs")
    with open(os.path.join(docs, "demo.cast"), "w") as cast:
        cast.write(json.dumps({"version": 2, "width": COLS, "height": ROWS, "env": {"TERM": "xterm-256color"}}) + "\n")
        for event in events:
            cast.write(json.dumps(event) + "\n")
    gif = os.path.join(docs, "demo.gif")
    subprocess.run(["agg", "--quiet", "--font-size", "16", "--theme", "dracula", "--last-frame-duration", "2", os.path.join(docs, "demo.cast"), gif], check=True)
    # Each picture is the last frame of the recording cut at its moment (every frame is written over the last).
    for name, at in marks.items():
        part = os.path.join(docs, "part.cast")
        with open(part, "w") as cast:
            cast.write(json.dumps({"version": 2, "width": COLS, "height": ROWS}) + "\n")
            for event in events:
                if event[0] <= at:
                    cast.write(json.dumps(event) + "\n")
        tmp = os.path.join(docs, "part.gif")
        subprocess.run(["agg", "--quiet", "--font-size", "16", "--theme", "dracula", "--last-frame-duration", "1", part, tmp], check=True)
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", tmp, "-update", "1", os.path.join(docs, f"{name}.png")], check=True)
        os.remove(part)
        os.remove(tmp)
    print("marks", marks)


if __name__ == "__main__":
    sys.exit(main())
