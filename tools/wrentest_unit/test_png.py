"""
Unit tests for tools/wrentest/png.py.

Uses only stdlib so the test runner is:
    python -m unittest discover -s tools/wrentest_unit -v

Each test exercises a distinct code path in the PNG decoder; the fixtures
are tiny (1–4 pixels) so the suite runs in milliseconds.
"""
from __future__ import annotations

import struct
import unittest
import zlib

from tools.wrentest import png as _png


# ---------------------------------------------------------------------------
# Minimal PNG construction helpers
# ---------------------------------------------------------------------------

def _chunk(type_: bytes, data: bytes) -> bytes:
    crc = zlib.crc32(type_ + data) & 0xFFFF_FFFF
    return struct.pack(">I", len(data)) + type_ + data + struct.pack(">I", crc)


def _ihdr(
    width: int,
    height: int,
    bit_depth: int = 8,
    color_type: int = 6,
    compression_method: int = 0,
    filter_method: int = 0,
    interlace: int = 0,
) -> bytes:
    payload = struct.pack(
        ">IIBBBBB",
        width, height, bit_depth, color_type,
        compression_method, filter_method, interlace,
    )
    return _chunk(b"IHDR", payload)


def _idat(raw_rows: list[bytes]) -> bytes:
    """Compress the concatenation of raw_rows into a single IDAT chunk."""
    return _chunk(b"IDAT", zlib.compress(b"".join(raw_rows)))


def _iend() -> bytes:
    return _chunk(b"IEND", b"")


def _row(*pixels: tuple[int, int, int, int]) -> bytes:
    """Return a NoFilter row (filter byte 0x00 + concatenated RGBA pixels)."""
    return bytes([0]) + b"".join(bytes(px) for px in pixels)


def _make_png(
    pixels: list[tuple[int, int, int, int]],
    width: int,
    height: int,
    **ihdr_kwargs,
) -> bytes:
    rows = [
        _row(*pixels[y * width : (y + 1) * width]) for y in range(height)
    ]
    return _png.PNG_MAGIC + _ihdr(width, height, **ihdr_kwargs) + _idat(rows) + _iend()


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

class TestPNGDecodeValid(unittest.TestCase):
    """Happy-path decoding."""

    def test_1x1_all_channels(self):
        data = _make_png([(255, 0, 128, 64)], 1, 1)
        img = _png.read_png(data)
        self.assertEqual(img.width, 1)
        self.assertEqual(img.height, 1)
        self.assertEqual(img.pixel(0, 0), (255, 0, 128, 64))

    def test_2x2_pixel_grid(self):
        pixels = [
            (255,   0,   0, 255), (  0, 255,   0, 255),
            (  0,   0, 255, 255), (255, 255,   0, 255),
        ]
        data = _make_png(pixels, 2, 2)
        img = _png.read_png(data)
        self.assertEqual(img.width, 2)
        self.assertEqual(img.height, 2)
        self.assertEqual(img.pixel(0, 0), (255,   0,   0, 255))
        self.assertEqual(img.pixel(1, 0), (  0, 255,   0, 255))
        self.assertEqual(img.pixel(0, 1), (  0,   0, 255, 255))
        self.assertEqual(img.pixel(1, 1), (255, 255,   0, 255))

    def test_wren_dark_background_color(self):
        """Pixel at the Wren dark-theme editor background color round-trips cleanly."""
        bg = (0x14, 0x16, 0x1B, 0xFF)
        data = _make_png([bg], 1, 1)
        img = _png.read_png(data)
        self.assertEqual(img.pixel(0, 0), bg)

    def test_wren_light_background_color(self):
        """Pixel at the Wren light-theme editor background color round-trips cleanly."""
        bg = (0xF7, 0xF8, 0xFA, 0xFF)
        data = _make_png([bg], 1, 1)
        img = _png.read_png(data)
        self.assertEqual(img.pixel(0, 0), bg)

    def test_transparent_pixel(self):
        data = _make_png([(0, 0, 0, 0)], 1, 1)
        img = _png.read_png(data)
        self.assertEqual(img.pixel(0, 0), (0, 0, 0, 0))

    def test_multiple_idat_chunks(self):
        """Concatenating multiple IDAT chunks before decompressing must work."""
        raw_row = bytes([0, 10, 20, 30, 40])  # filter 0 + one RGBA pixel
        compressed = zlib.compress(raw_row)
        # Split compressed data across two IDAT chunks.
        mid = len(compressed) // 2
        idat1 = _chunk(b"IDAT", compressed[:mid])
        idat2 = _chunk(b"IDAT", compressed[mid:])
        data = _png.PNG_MAGIC + _ihdr(1, 1) + idat1 + idat2 + _iend()
        img = _png.read_png(data)
        self.assertEqual(img.pixel(0, 0), (10, 20, 30, 40))


class TestPNGDecodeMagicAndHeader(unittest.TestCase):
    """Magic bytes and overall structure."""

    def test_empty_input(self):
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(b"")

    def test_too_short_for_magic(self):
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(_png.PNG_MAGIC[:5])

    def test_wrong_magic_first_byte(self):
        corrupted = b"\x00" + _png.PNG_MAGIC[1:]
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(corrupted + b"\x00" * 50)

    def test_wrong_magic_entirely(self):
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(b"JFIF" + b"\x00" * 100)

    def test_magic_constant_is_correct(self):
        self.assertEqual(_png.PNG_MAGIC, b"\x89PNG\r\n\x1a\n")


class TestPNGDecodeChunks(unittest.TestCase):
    """Chunk-level validation: lengths, CRC, required chunks."""

    def test_truncated_chunk_header(self):
        # Only magic + 3 bytes of a chunk header (need 8).
        data = _png.PNG_MAGIC + b"\x00\x00\x00"
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_chunk_data_truncated(self):
        # Chunk header claims 100 bytes of data but file ends immediately.
        data = _png.PNG_MAGIC + struct.pack(">I", 100) + b"IHDR" + b"\x00" * 10
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_crc_mismatch_in_ihdr(self):
        raw = _make_png([(0, 0, 0, 255)], 1, 1)
        corrupted = bytearray(raw)
        # Flip one byte inside the IHDR data to invalidate its CRC.
        corrupted[12] ^= 0xFF
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(bytes(corrupted))

    def test_missing_ihdr(self):
        data = _png.PNG_MAGIC + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_missing_idat(self):
        data = _png.PNG_MAGIC + _ihdr(1, 1) + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)


class TestPNGDecodeIHDR(unittest.TestCase):
    """IHDR field validation."""

    def test_unsupported_bit_depth_16(self):
        raw_row = bytes([0]) + bytes(8)  # filter + 2 bytes/channel × 4 channels
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1, bit_depth=16) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_unsupported_bit_depth_4(self):
        raw_row = bytes([0, 0])
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1, bit_depth=4) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_unsupported_color_type_rgb(self):
        # color_type 2 = RGB (no alpha)
        raw_row = bytes([0, 0, 0, 0])  # filter + 3 channels
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1, color_type=2) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_unsupported_color_type_greyscale(self):
        raw_row = bytes([0, 0])
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1, color_type=0) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_interlaced_rejected(self):
        raw_row = bytes([0]) + bytes(4)
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1, interlace=1) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_zero_width_rejected(self):
        # Craft a valid-looking IHDR with width=0 (even though the PNG spec
        # forbids it; we still parse the chunk first, then validate).
        payload = struct.pack(">IIBBBBB", 0, 1, 8, 6, 0, 0, 0)
        ihdr = _chunk(b"IHDR", payload)
        raw_row = bytes([0]) + bytes(4)
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + ihdr + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_invalid_compression_method(self):
        raw_row = bytes([0]) + bytes(4)
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1, compression_method=1) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_invalid_filter_method(self):
        raw_row = bytes([0]) + bytes(4)
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1, filter_method=1) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)


class TestPNGDecodePixelData(unittest.TestCase):
    """Pixel data decompression and filter-byte validation."""

    def test_nonzero_filter_type_rejected(self):
        # Row with filter type 1 (Sub) instead of 0 (None).
        raw_row = bytes([1]) + bytes(4)  # filter=1, one RGBA pixel
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_filter_type_4_paeth_rejected(self):
        raw_row = bytes([4]) + bytes(4)  # filter=4 (Paeth)
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_wrong_raw_data_length_too_short(self):
        # Compress fewer bytes than a 1×1 RGBA image requires.
        idat = _chunk(b"IDAT", zlib.compress(bytes(3)))
        data = _png.PNG_MAGIC + _ihdr(1, 1) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_wrong_raw_data_length_too_long(self):
        # Compress more bytes than a 1×1 RGBA image requires.
        idat = _chunk(b"IDAT", zlib.compress(bytes(100)))
        data = _png.PNG_MAGIC + _ihdr(1, 1) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_corrupt_idat_compression(self):
        # Replace IDAT payload with garbage.
        idat = _chunk(b"IDAT", b"\x00\x01\x02\x03\x04\x05")
        data = _png.PNG_MAGIC + _ihdr(1, 1) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_all_second_row_pixels_accessible(self):
        """In a 2×2 image row indexing must address the second row correctly."""
        pixels = [
            (10, 20, 30, 40), (50, 60, 70, 80),
            (90, 100, 110, 120), (130, 140, 150, 160),
        ]
        data = _make_png(pixels, 2, 2)
        img = _png.read_png(data)
        self.assertEqual(img.pixel(0, 1), (90, 100, 110, 120))
        self.assertEqual(img.pixel(1, 1), (130, 140, 150, 160))


class TestPNGBoundedDecompression(unittest.TestCase):
    """Bounded decompression: zip bombs, truncated streams, extra stream data."""

    def _make_1x1_png_with_idat(self, compressed: bytes) -> bytes:
        """PNG for a 1×1 image with a custom IDAT payload (expected_raw=5)."""
        return _png.PNG_MAGIC + _ihdr(1, 1) + _chunk(b"IDAT", compressed) + _iend()

    def test_zip_bomb_declared_small(self):
        # Compressed stream that expands far beyond the declared 1×1 image size.
        # With max_length=expected_raw+1=6, decompression is capped and the
        # excess is detected via len(raw) > expected_raw.
        large_payload = bytes(50_000)  # 50 KB of zeros → compresses to ~60 bytes
        compressed = zlib.compress(large_payload)
        data = self._make_1x1_png_with_idat(compressed)
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_truncated_zlib_stream(self):
        # Remove the last 4 bytes (adler32 checksum) from a valid stream.
        # The deflate blocks are intact but the zlib wrapper is incomplete,
        # so dobj.eof remains False.
        raw_row = bytes([0]) + bytes(4)
        compressed = zlib.compress(raw_row)
        self.assertGreater(len(compressed), 4, "compressed too short to truncate")
        data = self._make_1x1_png_with_idat(compressed[:-4])
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_extra_data_after_zlib_stream(self):
        # A valid zlib stream followed by extra bytes goes into dobj.unused_data.
        raw_row = bytes([0]) + bytes(4)
        compressed = zlib.compress(raw_row) + b"\x00\x00\x00\x00"
        data = self._make_1x1_png_with_idat(compressed)
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)


class TestPNGStructureValidation(unittest.TestCase):
    """Structural rules: IEND presence, chunk ordering, unknown critical chunks."""

    def test_missing_iend(self):
        raw_row = bytes([0]) + bytes(4)
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1) + idat  # no IEND
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_iend_nonzero_length_rejected(self):
        # IEND must have exactly zero bytes of data.
        bad_iend = _chunk(b"IEND", b"\x00")
        raw_row = bytes([0]) + bytes(4)
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1) + idat + bad_iend
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_duplicate_ihdr_rejected(self):
        raw_row = bytes([0]) + bytes(4)
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1) + _ihdr(1, 1) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_ihdr_must_be_first_chunk(self):
        # An ancillary chunk before IHDR violates required chunk ordering.
        # 'tEXt' has a lowercase first letter so it is ancillary, but it still
        # must not appear before IHDR.
        text_payload = b"Comment\x00hello"
        anc = _chunk(b"tEXt", text_payload)
        raw_row = bytes([0]) + bytes(4)
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + anc + _ihdr(1, 1) + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_unknown_critical_chunk_rejected(self):
        # 'TEST' has an uppercase first letter → critical; must be rejected.
        crit = _chunk(b"TEST", b"\x00")
        raw_row = bytes([0]) + bytes(4)
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1) + crit + idat + _iend()
        with self.assertRaises(_png.PNGDecodeError):
            _png.read_png(data)

    def test_ancillary_chunk_allowed(self):
        # 'tEXt' has a lowercase first letter → ancillary; must be ignored.
        text_payload = b"Comment\x00hello"
        anc = _chunk(b"tEXt", text_payload)
        raw_row = bytes([0]) + bytes(4)
        idat = _chunk(b"IDAT", zlib.compress(raw_row))
        data = _png.PNG_MAGIC + _ihdr(1, 1) + anc + idat + _iend()
        img = _png.read_png(data)
        self.assertEqual(img.width, 1)


class TestPNGPixelBounds(unittest.TestCase):
    """PNG.pixel must reject negative and out-of-bounds coordinates."""

    def setUp(self):
        data = _make_png([(10, 20, 30, 40)], 1, 1)
        self.img = _png.read_png(data)

    def test_negative_x_rejected(self):
        with self.assertRaises(IndexError):
            self.img.pixel(-1, 0)

    def test_negative_y_rejected(self):
        with self.assertRaises(IndexError):
            self.img.pixel(0, -1)

    def test_x_at_width_rejected(self):
        with self.assertRaises(IndexError):
            self.img.pixel(self.img.width, 0)

    def test_y_at_height_rejected(self):
        with self.assertRaises(IndexError):
            self.img.pixel(0, self.img.height)

    def test_valid_origin_still_works(self):
        self.assertEqual(self.img.pixel(0, 0), (10, 20, 30, 40))


if __name__ == "__main__":
    unittest.main()
