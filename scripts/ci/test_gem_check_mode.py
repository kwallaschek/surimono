"""Tests for gem_check_mode in lib.sh (E2-11): which native gems native-gem.sh
installs and tests on the runner, in Alpine, or not at all. Run by
scripts/ci.sh: python3 -B -I -m unittest discover -s scripts/ci -p 'test_*.py'"""

import os
import subprocess
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
LIB = os.path.join(HERE, "lib.sh")


def mode(platform, host):
    out = subprocess.run(
        ["bash", "-c", f'set -euo pipefail; . "{LIB}"; gem_check_mode "$1" "$2"', "_",
         platform, host],
        cwd=HERE, env={"PATH": "/usr/bin:/bin"}, capture_output=True, text=True, check=True)
    return out.stdout.strip()


class GemCheckModeTest(unittest.TestCase):
    def test_the_runners_own_platform_is_tested_here(self):
        self.assertEqual(mode("x86_64-linux", "x86_64-linux"), "host")
        self.assertEqual(mode("x86_64-linux", "x86_64-linux-gnu"), "host")
        self.assertEqual(mode("arm64-darwin", "arm64-darwin"), "host")

    def test_a_musl_gem_of_the_runners_cpu_is_tested_in_alpine(self):
        self.assertEqual(mode("x86_64-linux-musl", "x86_64-linux"), "alpine")
        self.assertEqual(mode("x86_64-linux-musl", "x86_64-linux-gnu"), "alpine")
        self.assertEqual(mode("aarch64-linux-musl", "aarch64-linux"), "alpine")

    def test_on_a_musl_host_the_musl_gem_is_its_own(self):
        self.assertEqual(mode("x86_64-linux-musl", "x86_64-linux-musl"), "host")

    def test_other_cpus_and_systems_get_content_checks_only(self):
        self.assertEqual(mode("aarch64-linux-musl", "x86_64-linux"), "none")
        self.assertEqual(mode("aarch64-linux", "x86_64-linux"), "none")
        self.assertEqual(mode("x86_64-darwin", "x86_64-linux"), "none")
        self.assertEqual(mode("arm64-darwin", "x86_64-linux"), "none")
        # A glibc gem is never tested on a musl host.
        self.assertEqual(mode("x86_64-linux", "x86_64-linux-musl"), "none")


if __name__ == "__main__":
    unittest.main()
