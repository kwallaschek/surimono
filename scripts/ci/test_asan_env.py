"""Tests for find_llvm_symbolizer and asan_env in lib.sh (P1-PKG-17), run by
scripts/ci.sh: python3 -B -I -m unittest discover -s scripts/ci -p 'test_*.py'"""

import os
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
LIB = os.path.join(HERE, "lib.sh")


def fake_tool(directory, name, works=True):
    """An executable that answers --version like llvm-symbolizer, or fails."""
    path = os.path.join(directory, name)
    with open(path, "w") as f:
        f.write("#!/bin/sh\necho 'LLVM version 99.0.0'\n" if works else "#!/bin/sh\nexit 1\n")
    os.chmod(path, 0o755)
    return path


def run_lib(script, path_dirs, **env):
    """Runs `script` in bash after sourcing lib.sh. GITHUB_ACTIONS is unset so the
    apt fallback never runs; PATH is the fake dirs plus the system's tools."""
    full_env = {"PATH": os.pathsep.join(path_dirs + ["/usr/bin", "/bin"])}
    full_env.update(env)
    out = subprocess.run(
        ["bash", "-c", f'set -euo pipefail; . "{LIB}"; {script}'],
        cwd=HERE, env=full_env, capture_output=True, text=True, check=True)
    return out.stdout


class FindLlvmSymbolizer(unittest.TestCase):
    def test_caller_path_wins(self):
        with tempfile.TemporaryDirectory() as tmp:
            on_path = os.path.join(tmp, "bin")
            os.mkdir(on_path)
            fake_tool(on_path, "llvm-symbolizer")
            chosen = fake_tool(tmp, "llvm-symbolizer-77")
            got = run_lib("find_llvm_symbolizer", [on_path], ASAN_SYMBOLIZER_PATH=chosen)
            self.assertEqual(got.strip(), chosen)

    def test_broken_caller_path_falls_through_to_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            on_path = os.path.join(tmp, "bin")
            os.mkdir(on_path)
            want = fake_tool(on_path, "llvm-symbolizer")
            broken = fake_tool(tmp, "llvm-symbolizer-broken", works=False)
            got = run_lib("find_llvm_symbolizer", [on_path], ASAN_SYMBOLIZER_PATH=broken)
            self.assertEqual(got.strip(), want)

    def test_prints_only_a_path(self):
        # asan_env captures stdout; nothing else may go there.
        with tempfile.TemporaryDirectory() as tmp:
            want = fake_tool(tmp, "llvm-symbolizer")
            got = run_lib("find_llvm_symbolizer", [tmp])
            self.assertEqual(got, want + "\n")


class AsanEnv(unittest.TestCase):
    def test_exports_flags_and_symbolizer(self):
        with tempfile.TemporaryDirectory() as tmp:
            sym = fake_tool(tmp, "llvm-symbolizer")
            got = run_lib(
                'asan_env >/dev/null; for v in RUSTFLAGS RUSTDOCFLAGS ASAN_OPTIONS '
                'LSAN_OPTIONS ASAN_SYMBOLIZER_PATH; do printf "%s=%s\\n" "$v" "${!v}"; done',
                [tmp])
            env = dict(line.split("=", 1) for line in got.splitlines())
            self.assertEqual(env["RUSTFLAGS"], "-Zsanitizer=address")
            self.assertEqual(env["RUSTDOCFLAGS"], "-Zsanitizer=address")
            self.assertEqual(set(env["ASAN_OPTIONS"].split(":")), {"detect_leaks=1", "symbolize=1"})
            self.assertEqual(env["LSAN_OPTIONS"], "symbolize=1")
            self.assertEqual(env["ASAN_SYMBOLIZER_PATH"], sym)


if __name__ == "__main__":
    unittest.main()
