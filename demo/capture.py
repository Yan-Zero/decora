#!/usr/bin/env python3
"""Run the local demo and capture its terminal session as asciicast v2."""

import io
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
OUTPUT_DIR = Path(r"E:\source\video\projects\decora-kiroween\assets\capture")
COLS = 100
ROWS = 32


def main() -> int:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    # Match the UTF-8 capture on Windows consoles as well as in the files.
    if isinstance(sys.stdout, io.TextIOWrapper):
        sys.stdout.reconfigure(encoding="utf-8", newline="")
    env = os.environ.copy()
    env["COLUMNS"] = str(COLS)
    env["TERM"] = "xterm-256color"

    real_run = env.get("DEMO_REAL") == "1" and env.get("DEMO_MOCK_ONLY") != "1"
    stem = "demo_real" if real_run else "demo"
    cast_path = OUTPUT_DIR / f"{stem}.cast"
    text_path = OUTPUT_DIR / f"{stem}.txt"
    started = time.monotonic()
    with cast_path.open("w", encoding="utf-8", newline="\n") as cast, text_path.open(
        "w", encoding="utf-8", newline="\n"
    ) as plain:
        header = {
            "version": 2,
            "width": COLS,
            "height": ROWS,
            "timestamp": int(time.time()),
            "title": "Decora demo",
            "env": {"TERM": "xterm-256color", "SHELL": "/bin/bash"},
        }
        cast.write(json.dumps(header, ensure_ascii=False, separators=(",", ":")) + "\n")
        cast.flush()

        proc = subprocess.Popen(
            [env.get("DEMO_BASH", "bash"), "demo/run.sh"],
            cwd=str(ROOT),
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            bufsize=0,
        )
        assert proc.stdout is not None
        decoder = __import__("codecs").getincrementaldecoder("utf-8")("replace")
        pending = ""
        try:
            while True:
                chunk = proc.stdout.read(4096)
                if not chunk:
                    break
                text = decoder.decode(chunk)
                if not text:
                    continue
                # Keep terminal output on CRLF so terminal renderers do not staircase.
                pending += text
                normalized = pending.replace("\r\n", "\n").replace("\r", "\n")
                if normalized.endswith("\n"):
                    pending = ""
                else:
                    pending = normalized.rsplit("\n", 1)[-1]
                    normalized = normalized[: len(normalized) - len(pending)]
                if normalized:
                    terminal_text = normalized.replace("\n", "\r\n")
                    sys.stdout.write(terminal_text)
                    sys.stdout.flush()
                    plain.write(normalized)
                    plain.flush()
                    cast.write(
                        json.dumps(
                            [time.monotonic() - started, "o", terminal_text],
                            ensure_ascii=False,
                            separators=(",", ":"),
                        )
                        + "\n"
                    )
                    cast.flush()
            remainder = decoder.decode(b"", final=True) + pending
            if remainder:
                sys.stdout.write(remainder)
                sys.stdout.flush()
                plain.write(remainder)
                plain.flush()
                cast.write(
                    json.dumps(
                        [time.monotonic() - started, "o", remainder],
                        ensure_ascii=False,
                        separators=(",", ":"),
                    )
                    + "\n"
                )
                cast.flush()
        finally:
            proc.stdout.close()
        return proc.wait()


if __name__ == "__main__":
    raise SystemExit(main())
