"""The committed board fixtures are the ones tools/board_fixtures.py writes."""

import tomllib
import unittest

from tools import board_fixtures, board_format


class BoardFixtureTests(unittest.TestCase):
    def test_committed_fixtures_match_the_generator(self):
        stale = board_fixtures.stale(board_fixtures.fixtures())
        self.assertEqual([str(p.relative_to(board_fixtures.ROOT)) for p in stale], [],
                         "run python3 tools/board_fixtures.py write and review the diff")

    def test_outputs_are_canonical_and_inputs_are_not(self):
        self.assertEqual(board_fixtures.problems(board_fixtures.committed(board_fixtures.fixtures())), [])

    def test_every_output_is_toml_1_0_that_the_reference_writes_back(self):
        """The Rust writer reproduces these bytes (wren-board's golden tests); here a stock
        TOML 1.0 parser reads each, and the reference writer writes the same bytes."""
        outputs = sorted(board_fixtures.GOLDEN.glob("*.out.wren-board"))
        samples = [p for p in sorted(board_fixtures.SAMPLES.glob("*.wren-board"))
                   if p.name != "Hand written.wren-board"]
        self.assertGreater(len(outputs), 10)
        for path in outputs + samples:
            text = path.read_bytes().decode("utf-8")
            with self.subTest(path.name):
                tomllib.loads(text)
                self.assertEqual(board_format.dump(board_format.load(text)), text)

    def test_every_input_has_its_output(self):
        names = {p.name for p in board_fixtures.GOLDEN.iterdir()}
        for name in names:
            if name.endswith(".in.wren-board"):
                self.assertIn(name.replace(".in.", ".out."), names)

    def test_checkout_flow_is_the_keyboard_models_layout(self):
        text = (board_fixtures.SAMPLES / "Checkout flow.wren-board").read_text(encoding="utf-8")
        board = board_format.load(text)
        nodes, edges = board["nodes"], board["edges"]
        self.assertEqual((len(nodes), len(edges)), (15, 17))
        shapes = sorted(n.get("shape", "rect") for n in nodes.values())
        self.assertEqual(shapes, ["diamond"] * 3 + ["pill"] * 2 + ["rect"] * 10)
        labels = sorted(label["text"] for e in edges.values() for label in e.get("labels", []))
        self.assertEqual(labels, ["No"] * 3 + ["Yes"] * 3)
        boxes = [(x, y, x + w, y + h) for (x, y), (w, h) in ((n["at"], n["size"]) for n in nodes.values())]
        for i, a in enumerate(boxes):
            self.assertTrue(all(c % 20 == 0 for c in a))
            for b in boxes[i + 1:]:
                self.assertFalse(a[0] < b[2] and b[0] < a[2] and a[1] < b[3] and b[1] < a[3], (a, b))
        for edge in edges.values():
            self.assertIn(edge["from"]["node"], nodes)
            self.assertIn(edge["to"]["node"], nodes)
        z = [r["z"] for r in list(nodes.values()) + list(edges.values())]
        self.assertEqual(len(set(z)), len(z))

    def test_a_label_with_a_carriage_return_is_written_as_it_reads_back(self):
        for typed in ("A\r\nB", "A\rB", "\r"):
            m = {"schema": 1, "nodes": {"a": {"text": typed, "kind": "shape", "z": typed}},
                 "edges": {"e": {"kind": "edge", "labels": [{"text": typed}], "text": typed}}}
            with self.subTest(typed=typed):
                written = board_format.dump(m)
                self.assertEqual(board_format.dump(board_format.load(written)), written)
                back = board_format.load(written)
                lines = typed.replace("\r\n", "\n").replace("\r", "\n")
                self.assertEqual(back["nodes"]["a"]["text"], lines)
                self.assertEqual(back["edges"]["e"]["labels"][0]["text"], lines)
                # Only labels: a CR in any other string is kept.
                self.assertEqual(back["nodes"]["a"]["z"], typed)
                self.assertEqual(back["edges"]["e"]["text"], typed)

    def test_hand_written_loads_and_is_not_canonical(self):
        text = (board_fixtures.SAMPLES / "Hand written.wren-board").read_text(encoding="utf-8")
        self.assertNotIn("\r", text)
        self.assertNotEqual(board_format.dump(board_format.load(text)), text)
        self.assertEqual(len(tomllib.loads(text)["node"]), 3)


if __name__ == "__main__":
    unittest.main()
