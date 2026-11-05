import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import recipe_manifest as rm

GOOD = """Intro.

```toml
other = "1"
```

```toml
[dependencies]
surimono = { git = "https://github.com/kwallaschek/surimono", branch = "main" }

[patch.crates-io]
rquickjs-sys = { git = "https://github.com/kwallaschek/surimono", branch = "main" }
```

And ```toml in a sentence.
"""


class RecipeBlock(unittest.TestCase):
    def test_picks_the_block_with_both_entries(self):
        block = rm.recipe_block(GOOD)
        self.assertTrue(block.startswith("[dependencies]\n"))
        self.assertEqual(len(rm.recipe_lines(block)), 2)

    def test_a_block_without_the_patch_is_not_the_recipe(self):
        with self.assertRaises(ValueError):
            rm.recipe_block(GOOD.replace("[patch.crates-io]\nrquickjs-sys", "[other]\nrquickjs-sys"))
        with self.assertRaises(ValueError):
            rm.recipe_block(GOOD.replace("rquickjs-sys = ", "other = "))

    def test_without_patch_drops_the_whole_section(self):
        out = rm.without_patch(rm.recipe_block(GOOD))
        self.assertIn("surimono = ", out)
        self.assertNotIn("rquickjs-sys", out)
        self.assertNotIn("patch", out)


class Main(unittest.TestCase):
    def run_main(self, readme, guide, *flags):
        with tempfile.TemporaryDirectory() as d:
            a, b = Path(d, "a.md"), Path(d, "b.md")
            a.write_text(readme, encoding="utf-8")
            b.write_text(guide, encoding="utf-8")
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                return rm.main(["x", str(a), str(b), "file:///r", *flags])

    def test_same_recipe_passes(self):
        self.assertEqual(self.run_main(GOOD, GOOD.replace("Intro.", "fulgur too.")), 0)

    def test_a_guide_that_differs_fails(self):
        self.assertEqual(self.run_main(GOOD, GOOD.replace('branch = "main"', 'rev = "abc"')), 1)

    def test_a_recipe_for_another_repository_fails(self):
        other = GOOD.replace("kwallaschek/surimono", "someone/else")
        self.assertEqual(self.run_main(other, other), 1)


if __name__ == "__main__":
    unittest.main()
