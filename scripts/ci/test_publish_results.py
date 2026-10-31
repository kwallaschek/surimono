"""Tests for publish-results.sh and job-status.sh (run by scripts/ci.sh):
python3 -B -I -m unittest discover -s scripts/ci -p 'test_*.py'

Each test makes a bare repository as "origin" and a clone as the job's
checkout, runs the scripts with GitHub Actions' environment, and reads the
ci-results branch back."""

import json
import os
import shutil
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
PUBLISH = os.path.join(HERE, "publish-results.sh")
JOB_STATUS = os.path.join(HERE, "job-status.sh")
SHA = "0123456789abcdef0123456789abcdef01234567"


def git(cwd, *args):
    return subprocess.run(
        ["git", *args], cwd=cwd, check=True, capture_output=True, text=True
    ).stdout


@unittest.skipUnless(shutil.which("git") and shutil.which("bash"), "needs git and bash")
class Publish(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.origin = os.path.join(self.tmp, "origin.git")
        self.repo = os.path.join(self.tmp, "repo")
        git(self.tmp, "init", "-q", "--bare", self.origin)
        git(self.tmp, "init", "-q", "-b", "main", self.repo)
        git(self.repo, "-c", "user.name=t", "-c", "user.email=t@e", "commit", "-q", "--allow-empty", "-m", "init")
        git(self.repo, "remote", "add", "origin", self.origin)
        git(self.repo, "push", "-q", "origin", "main")
        # Code an earlier step could plant: hooks (also through
        # core.hooksPath) that would run inside the publishing git commands.
        self.marker = os.path.join(self.tmp, "hook-ran")
        hooks = os.path.join(self.tmp, "evil-hooks")
        os.makedirs(hooks)
        for d in (os.path.join(self.repo, ".git", "hooks"), hooks):
            for name in ("pre-commit", "post-commit", "pre-push", "post-checkout", "reference-transaction"):
                path = os.path.join(d, name)
                with open(path, "w") as f:
                    f.write(f"#!/bin/sh\necho {name} >> {self.marker}\n")
                os.chmod(path, 0o755)
        git(self.repo, "config", "core.hooksPath", hooks)

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def env(self, **extra):
        env = {
            "PATH": os.environ["PATH"],
            "HOME": self.tmp,
            "GIT_CONFIG_NOSYSTEM": "1",
            "GITHUB_ACTIONS": "true",
            "GITHUB_SHA": SHA,
            "GITHUB_WORKFLOW": "ci",
            "GITHUB_JOB": "linux",
            "GITHUB_REF_NAME": "main",
            "GITHUB_EVENT_NAME": "push",
            "GITHUB_RUN_ID": "7",
            "GITHUB_RUN_ATTEMPT": "1",
        }
        env.update(extra)
        return env

    def run_script(self, script, *args, **env):
        return subprocess.run(
            ["bash", script, *args], cwd=self.repo, env=self.env(**env), capture_output=True, text=True
        )

    def published(self, path):
        git(self.repo, "-c", "core.hooksPath=/dev/null", "fetch", "-q", "origin", "ci-results")
        return git(self.repo, "-c", "core.hooksPath=/dev/null", "show", f"FETCH_HEAD:{path}")

    def test_in_job_mode_publishes_ci_out_with_safe_json(self):
        os.makedirs(os.path.join(self.repo, "ci-out", "linux"))
        with open(os.path.join(self.repo, "ci-out", "linux", "log.txt"), "w") as f:
            f.write("ok\n")
        weird = 'we"ird\\branch$(id)'
        r = self.run_script(PUBLISH, "failure", GITHUB_HEAD_REF=weird, PR_HEAD_SHA=SHA)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("published", r.stdout)
        base = f"runs/{SHA}/ci/linux"
        self.assertEqual(self.published(f"{base}/linux/log.txt"), "ok\n")
        status = json.loads(self.published(f"{base}/status.json"))
        self.assertEqual(status["ref"], weird)
        self.assertEqual(status["status"], "failure")
        self.assertEqual(status["job"], "linux")
        self.assertFalse(os.path.exists(self.marker), "a planted hook ran")
        self.assertNotIn("extraheader", git(self.repo, "config", "--list"))

    def test_publish_job_mode_takes_every_artifact(self):
        results = os.path.join(self.tmp, "results")
        for name, job, matrix, st in [
            ("test-macos-15", "test", "macos-15", "success"),
            ("ruby-ruby-3.2", "ruby", "ruby-3.2", "failure"),
        ]:
            out = os.path.join(results, f"ci-out-{name}")
            os.makedirs(os.path.join(out, name))
            with open(os.path.join(out, name, "log.txt"), "w") as f:
                f.write(name)
            r = self.run_script(JOB_STATUS, st, CI_OUT=out, GITHUB_JOB=job, MATRIX_KEY=matrix)
            self.assertEqual(r.returncode, 0, r.stderr)
        # A status a build job could fake beyond the three real ones is "unknown".
        out = os.path.join(results, "ci-out-soak")
        os.makedirs(out)
        with open(os.path.join(out, "job.json"), "w") as f:
            f.write('{"job":"soak","matrix":"","status":"great\\"}')
        # Names that aren't one path segment are skipped.
        os.makedirs(os.path.join(results, "ci-out-a b"))
        r = self.run_script(
            PUBLISH, "--from", results, GITHUB_WORKFLOW="full", GITHUB_JOB="publish",
            NEEDS='{"test":{"result":"success"}}',
        )
        self.assertEqual(r.returncode, 0, r.stderr)
        base = f"runs/{SHA}/full"
        self.assertEqual(self.published(f"{base}/test-macos-15/test-macos-15/log.txt"), "test-macos-15")
        s = json.loads(self.published(f"{base}/test-macos-15/status.json"))
        self.assertEqual((s["job"], s["matrix"], s["status"]), ("test", "macos-15", "success"))
        s = json.loads(self.published(f"{base}/ruby-ruby-3.2/status.json"))
        self.assertEqual(s["status"], "failure")
        s = json.loads(self.published(f"{base}/soak/status.json"))
        self.assertEqual(s["status"], "unknown")
        self.assertEqual(json.loads(self.published(f"{base}/needs.json")), {"test": {"result": "success"}})
        ls = git(self.repo, "ls-tree", "--name-only", "FETCH_HEAD", f"{base}/")
        self.assertNotIn("a b", ls)
        self.assertFalse(os.path.exists(self.marker), "a planted hook ran")

    def test_outside_actions_it_does_nothing(self):
        env = self.env()
        del env["GITHUB_ACTIONS"]
        r = subprocess.run(["bash", PUBLISH, "success"], cwd=self.repo, env=env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0)
        self.assertIn("skipping", r.stdout)


if __name__ == "__main__":
    unittest.main()
