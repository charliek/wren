"""
Unit tests for tools/wrentest/client.py.

All tests use real Unix domain sockets with a fake-server thread – no Rust
binary is required.  Coverage:

* Normal round-trip (success and application-level error responses)
* Fragmented response (server sends the JSON in multiple pieces)
* Wrong response ID → ProtocolError, connection closed
* Response frame exceeding MAX_FRAME_BYTES → ProtocolError, connection closed
* Server timeout (no response within deadline) → TransportError, connection closed
* Server sends EOF before completing the response → TransportError
* Malformed JSON in response → ProtocolError, connection closed
* Missing ``ok`` field in response → ProtocolError, connection closed
* Waiting: client blocks until a delayed but valid response arrives

Command to run only these tests::

    uv run --locked --group test \\
        python -m unittest discover -s tools/wrentest_unit -p test_client.py -v
"""

from __future__ import annotations

import json
import shutil
import socket
import tempfile
import threading
import time
import unittest
from pathlib import Path
from typing import Any

from tools.wrentest.client import (
    MAX_FRAME_BYTES,
    Client,
    ProtocolError,
    RemoteError,
    TransportError,
)


# ── fake-server helpers ───────────────────────────────────────────────────────


class FakeServer:
    """
    A minimal Unix-domain-socket server for testing.

    Creates its own private temp directory (mode 0700) and binds a listening
    socket there.  Call :meth:`accept_once` from a background thread to block
    until a client connects, then communicate through the returned socket.

    Call :meth:`close` in ``tearDown`` to remove the temp directory.
    """

    def __init__(self) -> None:
        self._tmpdir = Path(tempfile.mkdtemp(prefix="wren-unit-"))
        self._tmpdir.chmod(0o700)
        self.path = str(self._tmpdir / "s.sock")
        self._server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self._server.bind(self.path)
        self._server.listen(1)
        self._server.settimeout(5.0)

    def accept_once(self) -> socket.socket:
        """Accept one connection and return the connected socket."""
        conn, _ = self._server.accept()
        return conn

    def close(self) -> None:
        """Shut down the server and remove its temp directory."""
        try:
            self._server.close()
        except OSError:
            pass
        shutil.rmtree(str(self._tmpdir), ignore_errors=True)


def _send_raw(conn: socket.socket, data: bytes) -> None:
    """Send all *data* bytes through *conn*."""
    view = memoryview(data)
    offset = 0
    while offset < len(view):
        sent = conn.send(view[offset:])
        if sent == 0:
            break
        offset += sent


def _recv_line(conn: socket.socket, timeout: float = 3.0) -> bytes:
    """Read one newline-terminated line from *conn* (for fake servers)."""
    deadline = time.monotonic() + timeout
    buf = b""
    while b"\n" not in buf:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("FakeServer: _recv_line timed out")
        conn.settimeout(min(remaining, 1.0))
        chunk = conn.recv(4096)
        if not chunk:
            raise EOFError("FakeServer: connection closed while reading")
        buf += chunk
    return buf


def _make_ok(request_id: int, result: Any) -> bytes:
    """Return a NDJSON ok-response frame."""
    return (json.dumps({"id": request_id, "ok": True, "result": result}) + "\n").encode()


def _make_err(request_id: int, error: str) -> bytes:
    """Return a NDJSON error-response frame."""
    return (json.dumps({"id": request_id, "ok": False, "error": error}) + "\n").encode()


def _server_thread(server: FakeServer, handler) -> threading.Thread:
    """
    Start a daemon thread that accepts one connection and runs *handler*.

    *handler* is called with the connected ``socket.socket``.  The connection
    is closed in a ``finally`` block after *handler* returns or raises.
    """

    def run() -> None:
        conn = server.accept_once()
        try:
            handler(conn)
        finally:
            conn.close()

    t = threading.Thread(target=run, daemon=True)
    t.start()
    return t


# ── test cases ────────────────────────────────────────────────────────────────


class TestClientRoundTrip(unittest.TestCase):
    """Normal protocol round-trips: success and application-level errors."""

    def setUp(self) -> None:
        self.server = FakeServer()

    def tearDown(self) -> None:
        self.server.close()

    def _start_server(self, handler) -> threading.Thread:
        return _server_thread(self.server, handler)

    # ── success ───────────────────────────────────────────────────────────

    def test_success_response_delivers_result(self) -> None:
        """A well-formed ok:true response delivers the result value."""

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            _send_raw(conn, _make_ok(req["id"], {"protocol": "v1", "ready": True}))

        t = self._start_server(serve)
        with Client(self.server.path, default_timeout=5.0) as client:
            result = client.call("identify")
        self.assertEqual(result, {"protocol": "v1", "ready": True})
        t.join(timeout=2.0)

    def test_null_result_is_valid(self) -> None:
        """ok:true with result:null is accepted."""

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            _send_raw(conn, _make_ok(req["id"], None))

        self._start_server(serve)
        with Client(self.server.path, default_timeout=5.0) as client:
            result = client.call("app.quit")
        self.assertIsNone(result)

    # ── application-level error ───────────────────────────────────────────

    def test_error_response_raises_protocol_error_with_message(self) -> None:
        """ok:false causes ProtocolError containing the server's error string."""

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            _send_raw(conn, _make_err(req["id"], "feature not implemented"))

        self._start_server(serve)
        with Client(self.server.path, default_timeout=5.0) as client:
            with self.assertRaises(ProtocolError) as ctx:
                client.call("editor.set", {"text": "hello"})
        self.assertIn("feature not implemented", str(ctx.exception))

    # ── request fields ────────────────────────────────────────────────────

    def test_request_carries_op_name_and_params(self) -> None:
        """The client encodes the op and params correctly in the JSON frame."""
        received: dict[str, Any] = {}

        def serve(conn: socket.socket) -> None:
            received.update(json.loads(_recv_line(conn)))
            _send_raw(conn, _make_ok(received["id"], {}))

        self._start_server(serve)
        with Client(self.server.path, default_timeout=5.0) as client:
            client.call("editor.find", {"query": "hello", "direction": "forward"})
        self.assertEqual(received["op"], "editor.find")
        self.assertEqual(received["params"], {"query": "hello", "direction": "forward"})

    def test_request_ids_are_unique_and_ascending(self) -> None:
        """Each call uses a unique, monotonically increasing request ID."""
        ids: list[int] = []

        def serve(conn: socket.socket) -> None:
            for _ in range(3):
                req = json.loads(_recv_line(conn))
                ids.append(req["id"])
                _send_raw(conn, _make_ok(req["id"], None))

        self._start_server(serve)
        with Client(self.server.path, default_timeout=5.0) as client:
            client.call("a")
            client.call("b")
            client.call("c")
        self.assertEqual(len(ids), 3)
        self.assertEqual(ids, sorted(set(ids)), "IDs must be strictly ascending")


class TestClientProtocol(unittest.TestCase):
    """
    Protocol error and transport error paths.

    Each test verifies the correct exception type AND that the connection is
    closed after the error.
    """

    def setUp(self) -> None:
        self.server = FakeServer()

    def tearDown(self) -> None:
        self.server.close()

    def _start_server(self, handler) -> threading.Thread:
        return _server_thread(self.server, handler)

    # ── fragmented response ───────────────────────────────────────────────

    def test_fragmented_response_is_reassembled(self) -> None:
        """
        A response split across multiple TCP segments is reassembled correctly.
        The client must buffer partial data and not return early.
        """

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            resp = _make_ok(req["id"], {"text": "hello world"})
            mid = max(1, len(resp) // 2)
            _send_raw(conn, resp[:mid])
            time.sleep(0.05)
            _send_raw(conn, resp[mid:])

        self._start_server(serve)
        with Client(self.server.path, default_timeout=5.0) as client:
            result = client.call("editor.get")
        self.assertEqual(result, {"text": "hello world"})

    # ── wrong response ID ─────────────────────────────────────────────────

    def test_wrong_id_raises_protocol_error_and_closes_connection(self) -> None:
        """
        A response whose 'id' field does not match the sent request ID causes
        ProtocolError, and the connection is closed immediately.
        """

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            wrong_id = req["id"] + 1000
            _send_raw(conn, _make_ok(wrong_id, None))

        self._start_server(serve)

        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(ProtocolError) as ctx:
            client.call("identify")

        self.assertIn("mismatch", str(ctx.exception).lower())
        self.assertIsNone(client._sock, "connection must be closed after ProtocolError")

    # ── oversized frame ───────────────────────────────────────────────────

    def test_oversized_response_raises_protocol_error_and_closes(self) -> None:
        """
        A response frame exceeding MAX_FRAME_BYTES raises ProtocolError
        without buffering the entire oversized payload, and closes the connection.
        """

        def serve(conn: socket.socket) -> None:
            _recv_line(conn)  # discard the request
            # Send MAX_FRAME_BYTES+1 bytes of data, then a newline.
            # The newline arrives after the size limit is exceeded.
            oversized = b"x" * (MAX_FRAME_BYTES + 1) + b"\n"
            conn.settimeout(10.0)
            conn.sendall(oversized)

        self._start_server(serve)

        client = Client(self.server.path, default_timeout=10.0)
        client.connect()
        with self.assertRaises(ProtocolError) as ctx:
            client.call("identify")

        self.assertIn("large", str(ctx.exception).lower())
        self.assertIsNone(client._sock)

    # ── send timeout ──────────────────────────────────────────────────────

    def test_timeout_raises_transport_error_and_closes(self) -> None:
        """
        When the server does not respond within the deadline, TransportError
        is raised promptly and the connection is closed.
        """

        def serve(conn: socket.socket) -> None:
            _recv_line(conn)  # read request but never respond
            time.sleep(10.0)  # outlasts the client deadline

        self._start_server(serve)

        client = Client(self.server.path, default_timeout=0.4)
        client.connect()
        start = time.monotonic()
        with self.assertRaises(TransportError) as ctx:
            client.call("identify")
        elapsed = time.monotonic() - start

        self.assertIn("timeout", str(ctx.exception).lower())
        # Must finish near the deadline, not wait for the server sleep.
        self.assertLess(elapsed, 3.0, "client must time out near the deadline")
        self.assertIsNone(client._sock)

    # ── EOF before complete response ──────────────────────────────────────

    def test_eof_mid_response_raises_transport_error(self) -> None:
        """
        The server closing the connection before the response is complete
        raises TransportError with an EOF/closed message.
        """

        def serve(conn: socket.socket) -> None:
            _recv_line(conn)
            # Send an incomplete JSON object – no closing brace or newline.
            _send_raw(conn, b'{"id": 1, "ok": true')
            # conn closed in finally block → EOF to client

        self._start_server(serve)

        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(TransportError) as ctx:
            client.call("identify")

        self.assertIn("eof", str(ctx.exception).lower())

    # ── malformed JSON ────────────────────────────────────────────────────

    def test_malformed_json_raises_protocol_error_and_closes(self) -> None:
        """A response that is not valid JSON raises ProtocolError."""

        def serve(conn: socket.socket) -> None:
            _recv_line(conn)
            _send_raw(conn, b"not valid { json\n")

        self._start_server(serve)

        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(ProtocolError) as ctx:
            client.call("identify")

        self.assertIn("json", str(ctx.exception).lower())
        self.assertIsNone(client._sock)

    # ── missing ok field ─────────────────────────────────────────────────

    def test_missing_ok_field_raises_protocol_error_and_closes(self) -> None:
        """A response without an 'ok' field (or with ok:null) raises ProtocolError."""

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            # Intentionally omit the 'ok' field.
            resp = json.dumps({"id": req["id"], "result": "whatever"}) + "\n"
            _send_raw(conn, resp.encode())

        self._start_server(serve)

        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(ProtocolError) as ctx:
            client.call("editor.undo")

        self.assertIn("ok", str(ctx.exception).lower())
        self.assertIsNone(client._sock)

    # ── connect refused ───────────────────────────────────────────────────

    def test_connect_to_nonexistent_socket_raises_transport_error(self) -> None:
        """Connecting to a path that does not exist raises TransportError."""
        client = Client("/tmp/wren-unit-no-such-socket-xyzzy.sock", default_timeout=2.0)
        with self.assertRaises(TransportError):
            client.connect()
        self.assertIsNone(client._sock)


class TestClientWait(unittest.TestCase):
    """Tests for blocking behaviour and multi-call session management."""

    def setUp(self) -> None:
        self.server = FakeServer()

    def tearDown(self) -> None:
        self.server.close()

    def _start_server(self, handler) -> threading.Thread:
        return _server_thread(self.server, handler)

    def test_client_blocks_until_delayed_response_arrives(self) -> None:
        """
        The client waits the full delay (within the deadline) and returns the
        correct result once the server responds.
        """
        DELAY = 0.3

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            time.sleep(DELAY)
            _send_raw(conn, _make_ok(req["id"], "done"))

        self._start_server(serve)

        start = time.monotonic()
        with Client(self.server.path, default_timeout=5.0) as client:
            result = client.call("editor.redo")
        elapsed = time.monotonic() - start

        self.assertEqual(result, "done")
        # Must have waited at least most of the delay.
        self.assertGreaterEqual(elapsed, DELAY * 0.8)

    def test_multiple_sequential_calls_share_one_connection(self) -> None:
        """Multiple calls on the same Client reuse the connection in order."""
        results: list[Any] = []

        def serve(conn: socket.socket) -> None:
            for i in range(5):
                req = json.loads(_recv_line(conn))
                _send_raw(conn, _make_ok(req["id"], i))

        self._start_server(serve)

        with Client(self.server.path, default_timeout=5.0) as client:
            for _ in range(5):
                results.append(client.call("ping"))

        self.assertEqual(results, list(range(5)))

    def test_call_without_connect_raises_transport_error(self) -> None:
        """Calling call() before connect() raises TransportError immediately."""
        client = Client(self.server.path, default_timeout=5.0)
        with self.assertRaises(TransportError):
            client.call("identify")


class TestClientEnvelope(unittest.TestCase):
    """Strict response-envelope validation beyond the basic field checks."""

    def setUp(self) -> None:
        self.server = FakeServer()

    def tearDown(self) -> None:
        self.server.close()

    def _start_server(self, handler) -> threading.Thread:
        return _server_thread(self.server, handler)

    # ── id type ───────────────────────────────────────────────────────────

    def test_boolean_true_id_rejected(self) -> None:
        """id:true (== 1 in Python) is rejected because type(True) is bool."""

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            resp = json.dumps({"id": True, "ok": True, "result": None}) + "\n"
            _send_raw(conn, resp.encode())

        self._start_server(serve)
        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(ProtocolError) as ctx:
            client.call("identify")
        self.assertIn("mismatch", str(ctx.exception).lower())
        self.assertIsNone(client._sock)

    # ── ok type ───────────────────────────────────────────────────────────

    def test_ok_integer_one_rejected(self) -> None:
        """ok:1 is rejected because type(1) is int, not bool."""

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            resp = json.dumps({"id": req["id"], "ok": 1, "result": None}) + "\n"
            _send_raw(conn, resp.encode())

        self._start_server(serve)
        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(ProtocolError) as ctx:
            client.call("identify")
        self.assertIn("ok", str(ctx.exception).lower())
        self.assertIsNone(client._sock)

    # ── success-response shape ────────────────────────────────────────────

    def test_success_without_result_key_rejected(self) -> None:
        """ok:true but missing 'result' key is a protocol error."""

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            resp = json.dumps({"id": req["id"], "ok": True}) + "\n"
            _send_raw(conn, resp.encode())

        self._start_server(serve)
        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(ProtocolError) as ctx:
            client.call("identify")
        self.assertIn("result", str(ctx.exception).lower())
        self.assertIsNone(client._sock)

    def test_success_with_extra_fields_rejected(self) -> None:
        """ok:true with an extra key is a protocol error."""

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            resp = json.dumps({
                "id": req["id"], "ok": True, "result": None, "extra": 1,
            }) + "\n"
            _send_raw(conn, resp.encode())

        self._start_server(serve)
        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(ProtocolError):
            client.call("identify")
        self.assertIsNone(client._sock)

    # ── error-response shape ──────────────────────────────────────────────

    def test_error_with_non_string_error_rejected(self) -> None:
        """ok:false with a non-string 'error' is a protocol error."""

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            resp = json.dumps({
                "id": req["id"], "ok": False, "error": 42,
            }) + "\n"
            _send_raw(conn, resp.encode())

        self._start_server(serve)
        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(ProtocolError):
            client.call("identify")
        self.assertIsNone(client._sock)

    def test_error_with_extra_fields_rejected(self) -> None:
        """ok:false with extra keys is a protocol error."""

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            resp = json.dumps({
                "id": req["id"], "ok": False, "error": "nope", "detail": "x",
            }) + "\n"
            _send_raw(conn, resp.encode())

        self._start_server(serve)
        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(ProtocolError):
            client.call("identify")
        self.assertIsNone(client._sock)

    # ── response-level type ───────────────────────────────────────────────

    def test_response_array_rejected(self) -> None:
        """A JSON array response is not a valid response object."""

        def serve(conn: socket.socket) -> None:
            _recv_line(conn)
            _send_raw(conn, b"[1,2,3]\n")

        self._start_server(serve)
        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(ProtocolError):
            client.call("identify")
        self.assertIsNone(client._sock)

    # ── duplicate JSON keys ───────────────────────────────────────────────

    def test_duplicate_json_keys_rejected(self) -> None:
        """Duplicate keys in the response JSON are rejected."""

        def serve(conn: socket.socket) -> None:
            _recv_line(conn)
            _send_raw(conn, b'{"id":1,"ok":true,"result":null,"ok":false}\n')

        self._start_server(serve)
        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(ProtocolError):
            client.call("identify")
        self.assertIsNone(client._sock)

    # ── NaN / Infinity ────────────────────────────────────────────────────

    def test_nan_in_response_rejected(self) -> None:
        """NaN in response JSON is not valid."""

        def serve(conn: socket.socket) -> None:
            _recv_line(conn)
            _send_raw(conn, b'{"id":1,"ok":true,"result":NaN}\n')

        self._start_server(serve)
        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(ProtocolError):
            client.call("identify")
        self.assertIsNone(client._sock)

    # ── invalid UTF-8 ─────────────────────────────────────────────────────

    def test_invalid_utf8_response_rejected(self) -> None:
        """Non-UTF-8 bytes in the response are a protocol error."""

        def serve(conn: socket.socket) -> None:
            _recv_line(conn)
            _send_raw(conn, b'{"id":1,"ok":true,"result":"\xff\xfe"}\n')

        self._start_server(serve)
        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        with self.assertRaises(ProtocolError) as ctx:
            client.call("identify")
        self.assertIn("utf", str(ctx.exception).lower())
        self.assertIsNone(client._sock)


class TestClientConnectionLifecycle(unittest.TestCase):
    """RemoteError keeps connection; outgoing oversize keeps connection."""

    def setUp(self) -> None:
        self.server = FakeServer()

    def tearDown(self) -> None:
        self.server.close()

    def _start_server(self, handler) -> threading.Thread:
        return _server_thread(self.server, handler)

    def test_remote_error_keeps_connection_open(self) -> None:
        """After RemoteError (ok:false), the next call still works."""

        def serve(conn: socket.socket) -> None:
            # First call: error response
            req = json.loads(_recv_line(conn))
            _send_raw(conn, _make_err(req["id"], "not ready yet"))
            # Second call: success response
            req = json.loads(_recv_line(conn))
            _send_raw(conn, _make_ok(req["id"], "done"))

        self._start_server(serve)
        client = Client(self.server.path, default_timeout=5.0)
        client.connect()

        with self.assertRaises(RemoteError):
            client.call("editor.set", {"text": "hello"})
        self.assertIsNotNone(client._sock, "connection must stay open")

        result = client.call("ping")
        self.assertEqual(result, "done")
        client.close()

    def test_remote_error_is_protocol_error_subclass(self) -> None:
        """RemoteError subclasses ProtocolError for backward compatibility."""
        self.assertTrue(issubclass(RemoteError, ProtocolError))

    def test_outgoing_oversize_keeps_connection_open(self) -> None:
        """An outgoing frame that exceeds MAX_FRAME_BYTES raises
        ProtocolError but does not close the connection."""

        def serve(conn: socket.socket) -> None:
            req = json.loads(_recv_line(conn))
            _send_raw(conn, _make_ok(req["id"], "ok"))

        self._start_server(serve)
        client = Client(self.server.path, default_timeout=5.0)
        client.connect()

        with self.assertRaises(ProtocolError) as ctx:
            client.call("op", {"data": "x" * MAX_FRAME_BYTES})
        self.assertIn("large", str(ctx.exception).lower())
        self.assertIsNotNone(client._sock, "connection must stay open")

        result = client.call("ping")
        self.assertEqual(result, "ok")
        client.close()

    def test_outgoing_oversize_not_remote_error(self) -> None:
        """Outgoing oversize is ProtocolError, NOT RemoteError."""
        accepted = threading.Event()
        conn_box: list[socket.socket | None] = [None]

        def _accept_and_signal() -> None:
            try:
                conn_box[0] = self.server.accept_once()
            except OSError:
                pass
            accepted.set()

        t = threading.Thread(target=_accept_and_signal, daemon=True)
        t.start()

        client = Client(self.server.path, default_timeout=5.0)
        client.connect()
        accepted.wait(timeout=2.0)

        with self.assertRaises(ProtocolError) as ctx:
            client.call("op", {"data": "x" * MAX_FRAME_BYTES})
        self.assertNotIsInstance(ctx.exception, RemoteError)

        client.close()
        if conn_box[0] is not None:
            conn_box[0].close()
        t.join(timeout=2.0)


if __name__ == "__main__":
    unittest.main()
