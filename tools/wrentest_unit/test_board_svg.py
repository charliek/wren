"""The golden SVG exports parse as XML outside Rust and carry one text run per wrapped line."""

import re
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

GOLDEN = Path(__file__).resolve().parents[2] / "crates" / "wren-board" / "golden"
SVG = "{http://www.w3.org/2000/svg}"


class BoardSvgTests(unittest.TestCase):
    def test_every_golden_svg_matches_its_scene(self):
        files = sorted((GOLDEN / "svg").glob("*.svg"))
        self.assertGreater(len(files), 0)
        for path in files:
            with self.subTest(path.name):
                root = ET.parse(path).getroot()
                self.assertEqual(root.tag, f"{SVG}svg")
                scene = (GOLDEN / "scene" / f"{path.stem}.scene").read_text(encoding="utf-8")
                lines = [line for line in scene.split("\n") if line.startswith("  ")]
                paths = [line for line in scene.split("\n") if re.match(r"^(node|edge) \S+ path ", line)]
                texts = root.findall(f"{SVG}text")
                self.assertEqual(len(texts), len(lines), "one <text> per wrapped line")
                self.assertEqual(len(root.findall(f"{SVG}path")), len(paths), "one <path> per path item")
                for text in texts:
                    self.assertEqual(text.get("font-family"), "Inter, system-ui, sans-serif")
                    float(text.get("x"))
                    float(text.get("y"))


if __name__ == "__main__":
    unittest.main()
