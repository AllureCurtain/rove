#!/usr/bin/env python3
"""Bounded, opt-in PTY smoke for the local fake-model TUI.

The harness intentionally uses only the Python standard library.  It is a
Unix PTY gate today; Windows reports a structured skip because the standard
library does not expose ConPTY.  A skip is returned with exit code 77 and is
never reported as a passing interoperability check.
"""

import argparse
import errno
import json
import os
import re
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

try:
    import fcntl
    import termios
except ImportError:
    fcntl = None
    termios = None


SKIP_EXIT = 77
DEFAULT_TIMEOUT_SECONDS = 20.0
DEFAULT_BUILD_TIMEOUT_SECONDS = 120.0
MAX_OUTPUT_BYTES = 1024 * 1024
ANSI_SEQUENCE = re.compile(
    rb"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\))"
)


def emit(status, reason, **fields):
    payload = {"status": status, "reason": reason}
    payload.update(fields)
    print(json.dumps(payload, sort_keys=True))
    if status == "skipped":
        return SKIP_EXIT
    return 0 if status == "passed" else 1


def parse_args():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--run",
        action="store_true",
        help="run the opt-in gate (ROVE_TUI_PTY_SMOKE=1 has the same effect)",
    )
    parser.add_argument("--binary", type=Path, help="prebuilt rove executable")
    parser.add_argument(
        "--skip-build",
        action="store_true",
        help="skip the cargo build when --binary is not found",
    )
    parser.add_argument(
        "--timeout-seconds", type=float, default=DEFAULT_TIMEOUT_SECONDS
    )
    parser.add_argument(
        "--build-timeout-seconds", type=float, default=DEFAULT_BUILD_TIMEOUT_SECONDS
    )
    return parser.parse_args()


def safe_child_environment(state_dir):
    environment = os.environ.copy()
    sensitive_fragments = ("_API_KEY", "_TOKEN", "_SECRET", "_PASSWORD")
    for name in list(environment):
        upper_name = name.upper()
        if upper_name.endswith(sensitive_fragments) or upper_name in {
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "ROVE_API_TOKEN",
        }:
            environment.pop(name, None)
    environment.update(
        {
            "ROVE_MODEL": "fake",
            "ROVE_STATE_DIR": str(state_dir),
            "RUST_LOG": "error",
            "TERM": "xterm-256color",
        }
    )
    return environment


def resolve_binary(args, repo_root):
    candidate = args.binary
    if candidate is None and os.environ.get("ROVE_TUI_BINARY"):
        candidate = Path(os.environ["ROVE_TUI_BINARY"])
    if candidate is None:
        target_dir = os.environ.get("CARGO_TARGET_DIR")
        if target_dir:
            candidate = Path(target_dir) / "debug" / "rove"
        else:
            candidate = repo_root / "target" / "debug" / "rove"
    candidate = candidate.expanduser()
    if os.name == "nt" and candidate.suffix.lower() != ".exe":
        candidate = candidate.with_suffix(".exe")
    if candidate.exists():
        return candidate.resolve(), None
    if args.skip_build:
        return None, "rove binary is missing and --skip-build was requested"
    cargo = shutil.which("cargo")
    if cargo is None:
        return None, "cargo is unavailable and no prebuilt rove binary was supplied"
    try:
        completed = subprocess.run(
            [cargo, "build", "--bin", "rove"],
            cwd=repo_root,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=max(1.0, args.build_timeout_seconds),
            check=False,
        )
    except subprocess.TimeoutExpired:
        return None, "cargo build exceeded the bounded build timeout"
    if completed.returncode != 0:
        return None, "cargo build failed for the rove binary"
    if not candidate.exists():
        return None, "cargo build completed but the rove binary was not found"
    return candidate.resolve(), None


def set_window_size(fd, columns, rows):
    fcntl.ioctl(
        fd,
        termios.TIOCSWINSZ,
        struct.pack("HHHH", rows, columns, 0, 0),
    )


def visible_text(output):
    without_ansi = ANSI_SEQUENCE.sub(b"", output)
    return bytes(ch for ch in without_ansi if ch in b"\n\r\t" or ch >= 0x20).decode(
        "utf-8", errors="replace"
    )


def pump(master_fd, output, query_replied):
    """Read available PTY bytes and answer the kitty capability query.

    Crossterm probes keyboard enhancement by writing ``CSI ? u`` to the PTY.
    A PTY is not a terminal emulator, so answering with primary-device
    attributes makes the child take its documented Unavailable path instead
    of waiting for the probe timeout.
    """

    try:
        readable, _, _ = select.select([master_fd], [], [], 0.05)
    except (OSError, ValueError):
        return False, query_replied
    if not readable:
        return True, query_replied
    try:
        data = os.read(master_fd, 65536)
    except OSError as error:
        if error.errno in (errno.EIO, errno.EBADF):
            return False, query_replied
        raise
    if not data:
        return False, query_replied
    if len(output) + len(data) > MAX_OUTPUT_BYTES:
        raise RuntimeError("TUI PTY output exceeded the bounded capture limit")
    output.extend(data)
    if not query_replied and b"\x1b[?u" in output:
        os.write(master_fd, b"\x1b[?1;2c")
        query_replied = True
    return True, query_replied


def wait_for_output(process, master_fd, output, predicate, deadline, query_replied):
    while time.monotonic() < deadline:
        open_pty, query_replied = pump(master_fd, output, query_replied)
        if predicate(output):
            return query_replied
        if process.poll() is not None and not open_pty:
            return query_replied
    return query_replied


def stop_child(process):
    if process.poll() is not None:
        return
    try:
        process.send_signal(signal.SIGTERM)
        process.wait(timeout=2)
    except (subprocess.TimeoutExpired, OSError):
        try:
            process.kill()
            process.wait(timeout=2)
        except (subprocess.TimeoutExpired, OSError):
            pass


def run_unix_smoke(binary, args):
    output = bytearray()
    master_fd = slave_fd = None
    process = None
    workspace = Path(tempfile.mkdtemp(prefix="rove-tui-pty-"))
    state_dir = workspace / ".rove-state"
    state_dir.mkdir()
    try:
        master_fd, slave_fd = os.openpty()
        set_window_size(slave_fd, 80, 24)
        baseline_termios = termios.tcgetattr(slave_fd)
        environment = safe_child_environment(state_dir)

        def make_controlling_tty():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        process = subprocess.Popen(
            [str(binary), "tui", "--model", "fake"],
            cwd=workspace,
            env=environment,
            stdin=slave_fd,
            stdout=slave_fd,
            stderr=slave_fd,
            close_fds=True,
            preexec_fn=make_controlling_tty,
        )
        deadline = time.monotonic() + max(3.0, args.timeout_seconds)
        query_replied = False
        query_replied = wait_for_output(
            process,
            master_fd,
            output,
            lambda captured: all(
                marker in visible_text(captured)
                for marker in ("Transcript", "Activity", "Composer")
            ),
            deadline,
            query_replied,
        )
        if not all(
            marker in visible_text(output)
            for marker in ("Transcript", "Activity", "Composer")
        ):
            return emit("failed", "TUI did not produce a nonblank frame before timeout")
        first_frame_bytes = len(output)

        set_window_size(slave_fd, 40, 12)
        os.kill(process.pid, signal.SIGWINCH)
        resize_start = len(output)
        query_replied = wait_for_output(
            process,
            master_fd,
            output,
            lambda captured: (
                len(captured) > first_frame_bytes
                and "ws:- |" in visible_text(captured[resize_start:])
                and "Composer" in visible_text(captured[resize_start:])
            ),
            min(deadline, time.monotonic() + 5.0),
            query_replied,
        )
        resized_text = visible_text(output[resize_start:])
        if (
            len(output) <= first_frame_bytes
            or "ws:- |" not in resized_text
            or "Composer" not in resized_text
            or process.poll() is not None
        ):
            return emit(
                "failed",
                "TUI did not redraw the expected narrow layout after the PTY resize",
            )

        os.write(master_fd, b"\x11")
        query_replied = wait_for_output(
            process,
            master_fd,
            output,
            lambda _captured: process.poll() is not None,
            min(deadline, time.monotonic() + 5.0),
            query_replied,
        )
        if process.poll() is None:
            return emit("failed", "TUI did not exit after Ctrl+Q within the bound")
        if process.returncode != 0:
            return emit("failed", "TUI exited with a non-zero status")

        # Drain the restore sequences and let the child close its terminal fd.
        drain_deadline = time.monotonic() + 0.5
        while time.monotonic() < drain_deadline:
            open_pty, query_replied = pump(master_fd, output, query_replied)
            if not open_pty:
                break

        restored_termios = termios.tcgetattr(slave_fd)
        if restored_termios != baseline_termios:
            return emit("failed", "TUI did not restore the PTY termios state")

        required_sequences = {
            b"\x1b[?1049h": "alternate-screen enter",
            b"\x1b[?1049l": "alternate-screen leave",
            b"\x1b[?2004h": "bracketed-paste enable",
            b"\x1b[?2004l": "bracketed-paste disable",
            b"\x1b[?25l": "cursor hide",
            b"\x1b[?25h": "cursor show",
        }
        missing = [name for sequence, name in required_sequences.items() if sequence not in output]
        if missing:
            return emit("failed", "terminal restore sequence missing: " + ", ".join(missing))
        return emit(
            "passed",
            "Unix PTY frame, resize, clean exit, and restore checks passed",
            frame_bytes=first_frame_bytes,
            output_bytes=len(output),
            keyboard_probe_answered=query_replied,
            platform=sys.platform,
        )
    except (OSError, RuntimeError, ValueError) as error:
        return emit("failed", "PTY smoke harness error: " + str(error))
    finally:
        if process is not None:
            stop_child(process)
        for fd in (master_fd, slave_fd):
            if fd is not None:
                try:
                    os.close(fd)
                except OSError:
                    pass
        shutil.rmtree(workspace, ignore_errors=True)


def main():
    args = parse_args()
    if not args.run and os.environ.get("ROVE_TUI_PTY_SMOKE") != "1":
        return emit("skipped", "opt-in gate; pass --run or set ROVE_TUI_PTY_SMOKE=1")
    if args.timeout_seconds <= 0 or args.build_timeout_seconds <= 0:
        return emit("failed", "timeouts must be positive")
    if os.name != "posix":
        return emit(
            "skipped",
            "Windows ConPTY is not exposed by this standard-library harness; use a native ConPTY runner",
            platform=sys.platform,
        )
    if fcntl is None or termios is None or not hasattr(os, "openpty"):
        return emit("skipped", "Python pty/termios modules are unavailable on this Unix host")

    repo_root = Path(__file__).resolve().parents[1]
    binary, error = resolve_binary(args, repo_root)
    if binary is None:
        return emit("failed", error)
    return run_unix_smoke(binary, args)


if __name__ == "__main__":
    sys.exit(main())
