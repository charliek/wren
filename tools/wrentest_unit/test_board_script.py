"""The board helpers that need no app: the acceptance script's notation
(discovery/boards.md §5.20) and the saved-file line diff."""

import unittest

from tools.wrentest.boards import blocks, line_diff, script_steps


class ScriptTests(unittest.TestCase):
    def test_keys_in_backticks_and_text_between_them(self):
        self.assertEqual(
            script_steps("`U Tab` Start, `Esc ⇧⌘← ↵` No `Esc ←`"),
            [("key", "u"), ("key", "tab"), ("type", "Start"),
             ("key", "escape"), ("key", "left", "shift", "mod"), ("key", "enter"),
             ("type", "No"), ("key", "escape"), ("key", "left")],
        )

    def test_a_group_repeats_and_a_letter_after_c_is_the_query(self):
        self.assertEqual(
            script_steps("`Esc`, `↑` eight times, `→`, `C` end `↵`"),
            [("key", "escape")] + [("key", "up")] * 8
            + [("key", "right"), ("key", "c"), ("type", "end"), ("key", "enter")],
        )

    def test_the_whole_acceptance_script_presses_232_keys(self):
        """§5.20: 153 label characters, 9 query letters and 70 chords."""
        text = """
        `U Tab` Start, `Tab` Receive order, `Esc D Tab` Valid?
        `Esc ⌘→` Reject order, `Esc ⇧⌘← ↵` No `Esc ←`
        `Tab` Check stock, `Esc ⇧⌘↑ ↵` Yes, `Esc D Tab` In stock?
        `Esc ⌘→` Backorder, `Esc ⇧⌘← ↵` No, `Tab` Notify customer, `Esc C` che `↵`, `← ↓`
        `Tab` Charge card, `Esc ⇧⌘↑ ↵` Yes, `Esc D Tab` Payment ok?
        `Esc ⌘→` Retry payment, `Esc ⇧⌘← ↵` No, `Esc → C` cha `↵`, `← ↓`
        `Tab` Pack, `Esc ⇧⌘↑ ↵` Yes, `Tab` Ship, `Tab` Send receipt, `Esc U Tab` End
        `Esc`, `↑` eight times, `→`, `C` end `↵`
        """
        steps = script_steps(text)
        keys = [step for step in steps if step[0] == "key"]
        typed = "".join(step[1] for step in steps if step[0] == "type")
        self.assertEqual(len(keys), 70)
        self.assertEqual(len(typed), 153 + 9)
        self.assertEqual(len(keys) + len(typed), 232)

    def test_an_unknown_key_is_an_error(self):
        with self.assertRaises(ValueError):
            script_steps("`Hyper`")


class LineDiffTests(unittest.TestCase):
    def test_only_the_changed_lines(self):
        before = b'schema = 1\n\n[node.a]\nat = [0, 0]\nz = "a"\n'
        after = b'schema = 1\n\n[node.a]\nat = [20, 0]\nz = "a"\n'
        self.assertEqual(line_diff(before, after), ["- at = [0, 0]", "+ at = [20, 0]"])
        self.assertEqual(line_diff(before, before), [])

    def test_blocks_names_each_element(self):
        text = 'schema = 1\n\n[node.a]\nz = "a"\n\n[edge.e]\nz = "b"\n\n[node.a.ext]\n'
        self.assertEqual(blocks(text), ["[node.a]", "[edge.e]", "[node.a.ext]"])


if __name__ == "__main__":
    unittest.main()
