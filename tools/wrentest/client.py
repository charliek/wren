"""Bounded, persistent NDJSON client for the Wren Unix test socket.

Protocol/transport failures close the connection. A valid ``ok: false`` reply
raises RemoteError but keeps it usable. Calls never reconnect or retry: after a
failed mutation its outcome may be unknown.
"""

from __future__ import annotations

import json
import math
import socket
import threading
import time
from pathlib import Path
from typing import Any

__all__ = [
    "MAX_FRAME_BYTES", "DEFAULT_TIMEOUT", "ProtocolError", "RemoteError",
    "OperationError", "TransportError", "Client",
]

# Includes the terminating newline, in both directions.
MAX_FRAME_BYTES = 1024 * 1024
DEFAULT_TIMEOUT = 30.0


class ProtocolError(Exception):
    """Malformed JSON, invalid response envelope, or an oversized frame."""


class RemoteError(ProtocolError):
    """A valid operation rejection; unlike protocol failures, does not close IPC.

    The ProtocolError base is retained for callers of the original client.
    """


OperationError = RemoteError


class TransportError(Exception):
    """Socket failure or deadline expiry. ``eof`` identifies a peer close."""

    def __init__(self, message: str, *, eof: bool = False) -> None:
        super().__init__(message)
        self.eof = eof


def _seconds(value: float) -> float:
    if isinstance(value, bool) or not math.isfinite(value) or value <= 0:
        raise ValueError("timeout must be finite and positive")
    return value


def _object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key {key!r}")
        result[key] = value
    return result


def _invalid_constant(value: str) -> None:
    raise ValueError(f"invalid JSON constant {value}")


class Client:
    """One explicit connection, with serialised calls and absolute deadlines.

    ``connect(timeout=...)`` and ``call(..., timeout=...)`` each have a finite
    time budget. A call's budget covers serialisation, sending and receiving,
    not just individual socket reads. There is no automatic reconnect.
    """

    def __init__(
        self, socket_path: str | Path, default_timeout: float = DEFAULT_TIMEOUT,
    ) -> None:
        self._path = str(socket_path)
        self._default_timeout = _seconds(default_timeout)
        self._sock: socket.socket | None = None
        self._buf = b""
        self._next_id = 1
        self._lock = threading.Lock()

    @property
    def connected(self) -> bool:
        """Whether this client has an open socket (not a peer-liveness probe)."""
        return self._sock is not None

    @property
    def socket_path(self) -> Path:
        return Path(self._path)

    def connect(self, timeout: float | None = None) -> None:
        seconds = _seconds(self._default_timeout if timeout is None else timeout)
        if self.connected:
            return
        deadline = time.monotonic() + seconds
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            sock.settimeout(self._remaining(deadline, "connect"))
            sock.connect(self._path)
            self._remaining(deadline, "connect")
        except (OSError, TransportError) as exc:
            sock.close()
            raise TransportError(f"cannot connect to {self._path!r}: {exc}") from exc
        self._sock = sock
        self._buf = b""

    def close(self) -> None:
        if self._sock is not None:
            try:
                self._sock.close()
            finally:
                self._sock = None
        self._buf = b""

    def __enter__(self) -> Client:
        self.connect()
        return self

    def __exit__(self, _exc_type: object, _exc_val: object, _tb: object) -> None:
        self.close()

    def call(
        self, op: str, params: dict[str, Any] | None = None,
        timeout: float | None = None,
    ) -> Any:
        seconds = _seconds(self._default_timeout if timeout is None else timeout)
        deadline = time.monotonic() + seconds
        if not self._lock.acquire(timeout=seconds):
            raise TransportError("timeout waiting for another call")
        try:
            if self._sock is None:
                raise TransportError("not connected; call connect() first")
            self._remaining(deadline, "call")
            # Frame-building errors leave the connection usable.
            request_id, frame = self._build_frame(
                op, {} if params is None else params,
            )
            # Exchange errors (except RemoteError) close the connection.
            try:
                return self._exchange(request_id, op, frame, deadline)
            except RemoteError:
                raise
            except (ProtocolError, TransportError):
                self.close()
                raise
        finally:
            self._lock.release()

    def _build_frame(
        self, op: str, params: dict[str, Any],
    ) -> tuple[int, bytes]:
        if not isinstance(op, str) or not op or not isinstance(params, dict):
            raise ProtocolError("request requires a nonempty string op and object params")
        request_id = self._next_id
        self._next_id += 1
        try:
            frame = (json.dumps(
                {"id": request_id, "op": op, "params": params}, allow_nan=False,
            ) + "\n").encode("utf-8")
        except (TypeError, ValueError, UnicodeError, RecursionError) as exc:
            raise ProtocolError(f"invalid request JSON: {exc}") from exc
        if len(frame) > MAX_FRAME_BYTES:
            raise ProtocolError(f"outgoing frame too large: {len(frame)} bytes")
        return request_id, frame

    def _exchange(
        self, request_id: int, op: str, frame: bytes, deadline: float,
    ) -> Any:
        assert self._sock is not None
        self._send_all(frame, deadline)
        response = self._recv_line(deadline)
        try:
            obj = json.loads(
                response, object_pairs_hook=_object, parse_constant=_invalid_constant,
            )
        except (ValueError, RecursionError) as exc:
            raise ProtocolError(f"invalid JSON in response: {exc}") from exc
        self._remaining(deadline, "call")
        if not isinstance(obj, dict):
            raise ProtocolError("response is not a JSON object")
        if type(obj.get("id")) is not int or obj["id"] != request_id:
            raise ProtocolError(
                f"response ID mismatch: sent {request_id}, got {obj.get('id')!r}"
            )
        if type(obj.get("ok")) is not bool:
            raise ProtocolError("response missing or invalid 'ok' field")
        if obj["ok"]:
            if set(obj) != {"id", "ok", "result"}:
                raise ProtocolError("success response requires only id, ok, result")
            return obj["result"]
        if set(obj) != {"id", "ok", "error"} or not isinstance(obj["error"], str):
            raise ProtocolError("error response requires only id, ok, string error")
        raise RemoteError(f"server error for op {op!r}: {obj['error']}")

    @staticmethod
    def _remaining(deadline: float, operation: str) -> float:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TransportError(f"{operation} timeout")
        return remaining

    def _send_all(self, data: bytes, deadline: float) -> None:
        assert self._sock is not None
        offset = 0
        while offset < len(data):
            self._sock.settimeout(self._remaining(deadline, "send"))
            try:
                sent = self._sock.send(data[offset:])
            except OSError as exc:
                raise TransportError(f"send error or timeout: {exc}") from exc
            if sent == 0:
                raise TransportError("connection closed during send", eof=True)
            offset += sent

    def _recv_line(self, deadline: float) -> str:
        assert self._sock is not None
        while True:
            self._remaining(deadline, "receive")
            nl = self._buf.find(b"\n")
            if nl >= 0:
                if nl + 1 > MAX_FRAME_BYTES:
                    raise ProtocolError(f"incoming frame too large: {nl + 1} bytes")
                line, self._buf = self._buf[:nl], self._buf[nl + 1:]
                try:
                    return line.decode("utf-8", errors="strict")
                except UnicodeError as exc:
                    raise ProtocolError(f"invalid UTF-8 response: {exc}") from exc
            if len(self._buf) >= MAX_FRAME_BYTES:
                raise ProtocolError(f"incoming frame too large (limit {MAX_FRAME_BYTES})")
            self._sock.settimeout(self._remaining(deadline, "receive"))
            try:
                chunk = self._sock.recv(min(65536, MAX_FRAME_BYTES - len(self._buf)))
            except OSError as exc:
                raise TransportError(f"receive error or timeout: {exc}") from exc
            if not chunk:
                raise TransportError(
                    "connection closed (EOF) before complete response", eof=True,
                )
            self._buf += chunk
