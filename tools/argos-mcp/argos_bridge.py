"""Client for the ARGOS window's agent bridge, shared by the MCP server and the loop.

The window writes ``agent-bridge.json`` (port and token) into its session
folder: ``ARGOS_SESSION_DIR``, else ``~/Documents/ARGOS Sessions``. Each request
is one JSON line and gets one JSON line back.
"""
import hashlib
import json
import os
import pathlib
import socket
import tempfile
import threading
import time

BRIDGE_FILE = "agent-bridge.json"
HERE = pathlib.Path(__file__).resolve().parent


class BridgeError(RuntimeError):
    """ARGOS refused the request, or the window is not reachable."""


def session_dir():
    configured = os.environ.get("ARGOS_SESSION_DIR")
    if configured:
        return pathlib.Path(configured)
    home = os.environ.get("HOME") or os.environ.get("USERPROFILE")
    if home:
        return pathlib.Path(home) / "Documents" / "ARGOS Sessions"
    return pathlib.Path(tempfile.gettempdir())


def exit_with_argos(poll_s=2.0):
    """When ARGOS started this process (ARGOS_AGENT_CMD), exit once ARGOS is gone.

    ARGOS sets ``ARGOS_AGENT_PARENT``. If the window quits without stopping us,
    we are re-parented, so a changed parent pid means ARGOS has closed.
    """
    if not os.environ.get("ARGOS_AGENT_PARENT") or not hasattr(os, "getppid"):
        return None
    parent = os.getppid()

    def watch():
        while True:
            time.sleep(poll_s)
            if os.getppid() != parent:
                print("ARGOS closed; stopping", flush=True)
                os._exit(0)

    thread = threading.Thread(target=watch, name="argos-parent", daemon=True)
    thread.start()
    return thread


def bridge_path():
    return pathlib.Path(os.environ.get("ARGOS_AGENT_BRIDGE") or session_dir() / BRIDGE_FILE)


class Bridge:
    """One connection to the window. Reconnects (and re-reads the token) on failure."""

    def __init__(self, path=None, engine="agent", timeout=15.0):
        self.path = pathlib.Path(path) if path else bridge_path()
        self.engine = engine
        self.timeout = timeout
        self._lock = threading.Lock()
        self._sock = self._file = None
        self._token = ""
        self._next = 0
        self.hello = None

    def _connect(self):
        try:
            info = json.loads(self.path.read_text())
        except (OSError, ValueError) as error:
            raise BridgeError(
                f"ARGOS is not running here, or its agent bridge is off (no {self.path}): {error}"
            ) from None
        self._token = info["token"]
        try:
            self._sock = socket.create_connection((info.get("host", "127.0.0.1"), info["port"]),
                                                  timeout=self.timeout)
        except OSError as error:
            raise BridgeError(f"Cannot reach ARGOS on port {info['port']}: {error}") from None
        self._file = self._sock.makefile("rwb")
        self.hello = self._send("hello", {"engine": self.engine})

    def _close(self):
        for part in (self._file, self._sock):
            try:
                part and part.close()
            except OSError:
                pass
        self._sock = self._file = None

    def _send(self, op, fields):
        self._next += 1
        line = json.dumps({"id": self._next, "token": self._token, "op": op, **fields})
        self._file.write(line.encode() + b"\n")
        self._file.flush()
        reply = self._file.readline()
        if not reply:
            raise ConnectionError("ARGOS closed the connection")
        reply = json.loads(reply)
        if not reply.get("ok"):
            raise BridgeError(reply.get("error", "ARGOS refused the request"))
        reply.pop("ok", None)
        reply.pop("id", None)
        return reply

    def call(self, op, **fields):
        """Send one request. Raises BridgeError with ARGOS's reason when refused."""
        fields = {key: value for key, value in fields.items() if value is not None}
        with self._lock:
            for attempt in (0, 1):
                try:
                    if self._file is None:
                        self._connect()
                    return self._send(op, fields)
                except (OSError, ConnectionError, ValueError) as error:
                    self._close()
                    if attempt:
                        raise BridgeError(f"Lost the connection to ARGOS: {error}") from None

    def close(self):
        with self._lock:
            self._close()


def rules_text():
    return (HERE / "rules.md").read_text()


def topics_text():
    return (HERE / "topics.md").read_text()


def system_prompt():
    """Fixed per match, so it caches: the game rules and what can be read."""
    return rules_text() + "\n\n" + topics_text()


def state_digest(state):
    """Short hash of a field state, logged with each tick so it can be replayed."""
    stable = {key: value for key, value in state.items() if key != "time_s"}
    return hashlib.sha256(json.dumps(stable, sort_keys=True).encode()).hexdigest()[:16]


def tick_prompt(state):
    """The per-tick user message: the field now, and what to do with it."""
    return (
        "Here is the field as ARGOS sees it now (JSON). Decide what, if anything, "
        "the operator should do next. Call propose once with up to 3 recommendations, "
        "most urgent first. If the fleet needs nothing, do not call propose; say "
        "\"no recommendation\" and why in one line.\n\n"
        + json.dumps(state, separators=(",", ":"))
    )
