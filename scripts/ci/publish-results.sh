#!/usr/bin/env bash
# Pushes ci-out/ to the ci-results branch under runs/<sha>/<workflow>/, because the
# fleet's machine can read git but not the GitHub Actions API.
# Usage (always step): scripts/ci/publish-results.sh <job-status>
set -uo pipefail
status="${1:-unknown}"
src="${CI_OUT:-ci-out}"
if [ -z "${GITHUB_ACTIONS:-}" ]; then echo "not in GitHub Actions; skipping"; exit 0; fi
if [ ! -d "$src" ]; then mkdir -p "$src"; fi
sha="${PR_HEAD_SHA:-}"; [ -n "$sha" ] || sha="$GITHUB_SHA"
workflow="${GITHUB_WORKFLOW// /-}"
dest="runs/${sha}/${workflow}/${GITHUB_JOB}${MATRIX_KEY:+-$MATRIX_KEY}"
git config --global user.name "surimono-ci"
git config --global user.email "41898282+github-actions[bot]@users.noreply.github.com"
# With persist-credentials: false on checkout (docs/ci proposal), the write
# token reaches only this step, as GITHUB_TOKEN; send it for this push only.
# With the token persisted by checkout, don't add a second header.
auth=()
if [ -n "${GITHUB_TOKEN:-}" ] && [ -z "$(git config --get-all http.https://github.com/.extraheader 2>/dev/null)" ]; then
  basic="$(printf 'x-access-token:%s' "$GITHUB_TOKEN" | base64 | tr -d '\n')"
  echo "::add-mask::$basic"
  auth=(-c "http.https://github.com/.extraheader=AUTHORIZATION: basic $basic")
fi
work="$(mktemp -d)"
for attempt in 1 2 3 4 5 6; do
  git worktree remove --force "$work" >/dev/null 2>&1 || true
  rm -rf "$work"
  if git ${auth[@]+"${auth[@]}"} fetch --quiet --depth 1 origin "+refs/heads/ci-results:refs/remotes/origin/ci-results" 2>/dev/null; then
    git worktree add --quiet --detach "$work" origin/ci-results
  else
    git worktree add --quiet --detach "$work" HEAD
    (cd "$work" && git checkout --quiet --orphan ci-results-new && git rm -rfq --ignore-unmatch . && rm -rf ./*)
  fi
  mkdir -p "$work/$dest"
  cp -R "$src"/. "$work/$dest/" 2>/dev/null || true
  cat > "$work/$dest/status.json" <<JSON
{"workflow":"${GITHUB_WORKFLOW}","job":"${GITHUB_JOB}","matrix":"${MATRIX_KEY:-}","status":"${status}","sha":"${sha}","ref":"${GITHUB_HEAD_REF:-$GITHUB_REF_NAME}","event":"${GITHUB_EVENT_NAME}","run_id":"${GITHUB_RUN_ID}","attempt":"${GITHUB_RUN_ATTEMPT}","finished_at":"$(date -u +%Y-%m-%dT%H:%M:%SZ)"}
JSON
  (cd "$work" && git add -A && git commit --quiet -m "results: ${workflow}/${GITHUB_JOB} ${status} @ ${sha:0:9}" && git ${auth[@]+"${auth[@]}"} push --quiet origin HEAD:refs/heads/ci-results) && { echo "published to ci-results:$dest"; exit 0; }
  sleep $((attempt * 4))
done
echo "could not publish results" >&2
exit 0
