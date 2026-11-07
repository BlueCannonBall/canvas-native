#!/usr/bin/env python3
"""Run all release tests on a newly allocated Xvfb, never the shared display."""
import os
from pathlib import Path
import selectors
import subprocess
import tempfile


def main():
    root = Path(__file__).resolve().parent.parent
    read_fd, write_fd = os.pipe()
    with tempfile.TemporaryFile(dir="/tmp/opencode") as log:
        server = subprocess.Popen(
            ["Xvfb", "-displayfd", str(write_fd), "-screen", "0", "800x600x24",
             "-nolisten", "tcp"],
            pass_fds=(write_fd,), stdout=log, stderr=log,
        )
        os.close(write_fd)
        try:
            with os.fdopen(read_fd) as pipe, selectors.DefaultSelector() as selector:
                selector.register(pipe, selectors.EVENT_READ)
                if not selector.select(timeout=10):
                    raise RuntimeError("Timed out allocating an isolated X11 display")
                number = pipe.readline().strip()
            if not number.isdigit() or number == "0":
                raise RuntimeError("No safe isolated X11 display allocated")
            env = os.environ.copy()
            env["DISPLAY"] = ":" + number
            env["CANVAS_NATIVE_X11_TEST_DISPLAY"] = ":" + number
            env.pop("WAYLAND_DISPLAY", None)
            print("Running tests on isolated X11 display :" + number, flush=True)
            subprocess.run(
                ["cargo", "test", "--release", "--offline", "--", "--include-ignored"],
                cwd=root, env=env, check=True,
            )
        finally:
            server.terminate()
            server.wait(timeout=10)


if __name__ == "__main__":
    main()
