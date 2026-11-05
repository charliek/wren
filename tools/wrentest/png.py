"""
Minimal PNG decoder for RGBA8, non-interlaced, no-filter images.

Supports only the subset that the Wren smoke-capture path produces:
  - RGBA (color type 6), 8-bit depth
  - No interlacing (interlace_method = 0)
  - NoFilter (every row's filter byte is 0x00)

No third-party libraries; only stdlib ``struct`` and ``zlib``.
"""
from __future__ import annotations

import struct
import zlib

PNG_MAGIC = b"\x89PNG\r\n\x1a\n"

# Maximum raw image size we will decompress (100 MiB).
_MAX_RAW_BYTES = 100 * 1024 * 1024


class PNGDecodeError(ValueError):
    """Raised when the PNG data is invalid or uses an unsupported feature."""


class PNG:
    """Decoded RGBA8 image."""

    __slots__ = ("width", "height", "_data")

    def __init__(self, width: int, height: int, data: bytes) -> None:
        self.width = width
        self.height = height
        self._data = data  # flat RGBA bytes, row-major

    def pixel(self, x: int, y: int) -> tuple[int, int, int, int]:
        """Return the (R, G, B, A) tuple at column x, row y (0-indexed)."""
        if x < 0 or x >= self.width or y < 0 or y >= self.height:
            raise IndexError(
                f"pixel ({x}, {y}) out of bounds for {self.width}x{self.height} image"
            )
        offset = (y * self.width + x) * 4
        d = self._data
        return (d[offset], d[offset + 1], d[offset + 2], d[offset + 3])


def read_png(data: bytes) -> PNG:
    """Parse *data* as a PNG file and return a :class:`PNG` object.

    Raises :class:`PNGDecodeError` if the data is malformed or uses any
    PNG feature outside the supported subset.
    """
    if len(data) < 8:
        raise PNGDecodeError("too short to be a PNG file")
    if data[:8] != PNG_MAGIC:
        raise PNGDecodeError("invalid PNG magic bytes")

    pos = 8
    ihdr_data: bytes | None = None
    idat_parts: list[bytes] = []
    seen_ihdr = False
    seen_iend = False

    while pos < len(data):
        # Each chunk: 4-byte length, 4-byte type, N-byte data, 4-byte CRC.
        if pos + 8 > len(data):
            raise PNGDecodeError(f"truncated chunk header at offset {pos}")
        length = struct.unpack_from(">I", data, pos)[0]
        pos += 4
        chunk_type = data[pos : pos + 4]
        pos += 4

        if pos + length + 4 > len(data):
            raise PNGDecodeError(
                f"chunk {chunk_type!r} data truncated (need {length + 4} bytes "
                f"at offset {pos}, have {len(data) - pos})"
            )
        chunk_data = data[pos : pos + length]
        pos += length

        expected_crc = struct.unpack_from(">I", data, pos)[0]
        pos += 4
        actual_crc = zlib.crc32(chunk_type + chunk_data) & 0xFFFFFFFF
        if actual_crc != expected_crc:
            raise PNGDecodeError(
                f"CRC mismatch in {chunk_type!r} chunk: "
                f"expected 0x{expected_crc:08x}, got 0x{actual_crc:08x}"
            )

        # PNG spec: IHDR must be the first chunk.
        if not seen_ihdr and chunk_type != b"IHDR":
            raise PNGDecodeError(
                f"expected IHDR as first chunk, got {chunk_type!r}"
            )

        if chunk_type == b"IHDR":
            if seen_ihdr:
                raise PNGDecodeError("duplicate IHDR chunk")
            if length != 13:
                raise PNGDecodeError(f"IHDR must be 13 bytes, got {length}")
            seen_ihdr = True
            ihdr_data = chunk_data
        elif chunk_type == b"IDAT":
            idat_parts.append(chunk_data)
        elif chunk_type == b"IEND":
            if length != 0:
                raise PNGDecodeError("IEND chunk must have zero length")
            seen_iend = True
            break
        else:
            # Critical chunks (uppercase first letter) that we don't recognise
            # must be rejected; ancillary chunks are silently skipped.
            if chunk_type[0] & 0x20 == 0:
                raise PNGDecodeError(f"unknown critical chunk {chunk_type!r}")

    if not seen_iend:
        raise PNGDecodeError("missing IEND chunk")
    if not idat_parts:
        raise PNGDecodeError("missing IDAT chunk")

    # --- parse IHDR ---
    # ihdr_data is always set when seen_ihdr is True (enforced above).
    assert ihdr_data is not None
    width, height = struct.unpack_from(">II", ihdr_data)
    bit_depth          = ihdr_data[8]
    color_type         = ihdr_data[9]
    compression_method = ihdr_data[10]
    filter_method      = ihdr_data[11]
    interlace          = ihdr_data[12]

    if width == 0 or height == 0:
        raise PNGDecodeError(f"zero image dimension ({width}x{height})")
    if bit_depth != 8:
        raise PNGDecodeError(
            f"unsupported bit depth {bit_depth} (only 8-bit RGBA is supported)"
        )
    if color_type != 6:
        raise PNGDecodeError(
            f"unsupported color type {color_type} (only RGBA=6 is supported)"
        )
    if compression_method != 0:
        raise PNGDecodeError(
            f"unsupported IHDR compression method {compression_method} (only 0 is defined)"
        )
    if filter_method != 0:
        raise PNGDecodeError(
            f"unsupported IHDR filter method {filter_method} (only 0 is defined)"
        )
    if interlace != 0:
        raise PNGDecodeError("interlaced PNG is not supported")

    # Each row: 1 filter byte + width*4 pixel bytes.
    row_stride = 1 + width * 4
    expected_raw = height * row_stride
    if expected_raw > _MAX_RAW_BYTES:
        raise PNGDecodeError(
            f"image {width}x{height} would require {expected_raw} bytes "
            f"(limit {_MAX_RAW_BYTES})"
        )

    # --- decompress IDAT (bounded) ---
    # max_length=expected_raw+1 caps output without calling unbounded flush().
    # Checks after decompress verify no excess output, no extra stream data,
    # and that the zlib stream ended cleanly (dobj.eof).
    compressed = b"".join(idat_parts)
    dobj = zlib.decompressobj()
    try:
        raw = dobj.decompress(compressed, expected_raw + 1)
    except zlib.error as exc:
        raise PNGDecodeError(f"IDAT decompression failed: {exc}") from exc

    if len(raw) > expected_raw:
        raise PNGDecodeError(
            f"IDAT decompressed data exceeds expected {expected_raw} bytes"
        )
    if dobj.unused_data:
        raise PNGDecodeError("extra data after IDAT zlib stream end")
    if not dobj.eof:
        # Stream was truncated or is still in progress (checksum not verified).
        raise PNGDecodeError("IDAT zlib stream is truncated or incomplete")
    if len(raw) != expected_raw:
        raise PNGDecodeError(
            f"decompressed data length {len(raw)} != expected {expected_raw} "
            f"(for {width}x{height} RGBA8)"
        )

    # --- unfilter scanlines (only filter type 0 = None is supported) ---
    pixels = bytearray(width * height * 4)
    for y in range(height):
        row_start = y * row_stride
        filter_type = raw[row_start]
        if filter_type != 0:
            raise PNGDecodeError(
                f"unsupported filter type {filter_type} at row {y} "
                "(only NoFilter=0 is supported)"
            )
        src = row_start + 1
        dst = y * width * 4
        pixels[dst : dst + width * 4] = raw[src : src + width * 4]

    return PNG(width, height, bytes(pixels))
