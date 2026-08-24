"""
Shared async UCI-engine wrapper for Bitwark's Python test suite.

One interface drives *any* UCI engine — bitwark itself or the stockfish
oracle binary — so tests can cross-check the two without caring which is
which.

Usage:
    from common import UciEngine, BITWARK_BIN

    async with UciEngine(BITWARK_BIN) as eng:
        await eng.send("uci")
        lines = await eng.wait_for("uciok")
"""

from __future__ import annotations

import asyncio
import os
from pathlib import Path
from typing import Callable

# Repository root (this file lives in <root>/tools/).
REPO_ROOT = Path(__file__).resolve().parent.parent

# Default engine paths; overridable via environment so CI or other machines
# can point at different builds.
BITWARK_BIN = Path(os.environ.get("BITWARK_BIN", REPO_ROOT / "target" / "release" / "bitwark"))
STOCKFISH_BIN = Path(os.environ.get("STOCKFISH_BIN", REPO_ROOT / "stockfish"))


def _line_token(line: str) -> str:
    """First whitespace token of a protocol line (empty string if blank)."""
    return line.split()[0] if line.strip() else ""


class UciEngine:
    """Async wrapper around a UCI engine subprocess."""

    def __init__(self, path: str | Path):
        self.path = Path(path)
        self.proc: asyncio.subprocess.Process | None = None
        # Captured stderr — an engine should never write to it, but when a
        # Rust panic happens the message lands here and test failures get
        # *vastly* easier to diagnose.
        self.stderr_lines: list[str] = []
        self._stderr_task: asyncio.Task | None = None

    # ---- lifecycle ----
    async def start(self) -> UciEngine:
        self.proc = await asyncio.create_subprocess_exec(
            str(self.path),
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        self._stderr_task = asyncio.create_task(self._drain_stderr())
        return self

    async def _drain_stderr(self) -> None:
        assert self.proc and self.proc.stderr
        while True:
            raw = await self.proc.stderr.readline()
            if not raw:
                return
            self.stderr_lines.append(raw.decode(errors="replace").rstrip("\n"))

    async def close(self, timeout: float = 5.0) -> None:
        """Polite shutdown: `quit`, wait, then kill if needed."""
        if self.proc is None:
            return
        if self.proc.returncode is None:
            try:
                await self.send("quit")
            except Exception:
                pass
            try:
                await asyncio.wait_for(self.proc.wait(), timeout)
            except asyncio.TimeoutError:
                self.proc.kill()
                await self.proc.wait()
        if self._stderr_task:
            await self._stderr_task

    async def __aenter__(self) -> UciEngine:
        return await self.start()

    async def __aexit__(self, *exc) -> None:
        await self.close()

    # ---- writing ----
    async def send(self, line: str) -> None:
        assert self.proc and self.proc.stdin
        self.proc.stdin.write((line + "\n").encode())
        await self.proc.stdin.drain()

    # ---- reading ----
    async def _readline_raw(self) -> str:
        assert self.proc and self.proc.stdout
        raw = await self.proc.stdout.readline()
        if not raw:
            tail = self.stderr_lines[-5:] if self.stderr_lines else []
            raise EOFError(f"{self.path.name} closed stdout (stderr tail: {tail})")
        return raw.decode().rstrip("\n")

    async def read_line(self, timeout: float = 10.0) -> str:
        async with asyncio.timeout(timeout):
            return await self._readline_raw()

    async def read_until(self, predicate: Callable[[str], bool], timeout: float = 10.0) -> list[str]:
        """Collect lines until `predicate(line)` is true; returns all collected lines."""
        lines: list[str] = []
        async with asyncio.timeout(timeout):
            while True:
                line = await self._readline_raw()
                lines.append(line)
                if predicate(line):
                    return lines

    async def wait_for(self, token: str, timeout: float = 10.0) -> list[str]:
        """Read until a line whose first whitespace token equals `token`.

        Works for exact lines like `uciok` and for prefixed lines like
        `bestmove e2e4`.
        """
        return await self.read_until(lambda line: _line_token(line) == token, timeout)

    async def read_silence(self, duration: float = 0.3) -> list[str]:
        """Wait `duration` seconds; return any lines that arrived (expected [])."""
        got: list[str] = []
        try:
            async with asyncio.timeout(duration):
                while True:
                    got.append(await self._readline_raw())
        except TimeoutError:
            pass
        return got

    # ---- helpers ----
    async def handshake(self) -> list[str]:
        await self.send("uci")
        return await self.wait_for("uciok")

    async def isready(self) -> list[str]:
        await self.send("isready")
        return await self.wait_for("readyok")

    @property
    def alive(self) -> bool:
        return self.proc is not None and self.proc.returncode is None
