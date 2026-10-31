#!/usr/bin/env bash
# Pushes job results to the ci-results branch under runs/<sha>/<workflow>/<job>/,
# because the fleet's machine can read git but not the GitHub Actions API.
#
# Usage:
#   scripts/ci/publish-results.sh --from <dir>
#       The workflow's own publish job (docs/ci/*.yml): the only job with a
#       write token, run from a checkout of the default branch after the build
#       jobs. <dir> holds the downloaded ci-out-<job>[-<matrix>] artifacts, each
#       a build job's ci-out/ with job.json (scripts/ci/job-status.sh). All of
#       them go out in one commit, plus needs.json (the jobs' results as GitHub
#       reports them, from $NEEDS).
#   scripts/ci/publish-results.sh <job-status>
#       An always() step at the end of a build job (the workflows before the
#       docs/ci proposal): publishes ci-out/ for this job.
#
# Every git command runs with hooks and fsmonitor off, so nothing an earlier
# step left in .git runs here, and the token goes only to fetch and push, in
# GIT_CONFIG_* environment variables: never on a command line, never in
# .git/config. Symbolic links on ci-results check out as plain files
# (core.symlinks=false), and results are copied as regular files only, never
# symlinks or .git* entries, so publishing writes nowhere but runs/<sha>/.
# In --from mode a job's status is GitHub's (needs.json) whenever that isn't
# success: a build job runs code under test and could rewrite its job.json.
set -uo pipefail
if [ -z "${GITHUB_ACTIONS:-}" ]; then echo "not in GitHub Actions; skipping"; exit 0; fi

from=""; status="unknown"
case "${1:-}" in
  --from) from="${2:?--from needs a directory}" ;;
  *) status="${1:-unknown}" ;;
esac

git_safe() {
  git -c core.hooksPath=/dev/null -c core.fsmonitor=false -c core.symlinks=false \
    -c user.name=surimono-ci -c user.email=41898282+github-actions[bot]@users.noreply.github.com "$@"
}
# With persist-credentials: false on checkout, the token reaches only the step
# that runs this, as GITHUB_TOKEN; it is handed to git for fetch and push.
# With the token persisted by checkout (old workflows), don't add a second one.
auth_header=""
if [ -n "${GITHUB_TOKEN:-}" ] && [ -z "$(git_safe config --get-all http.https://github.com/.extraheader 2>/dev/null)" ]; then
  basic="$(printf 'x-access-token:%s' "$GITHUB_TOKEN" | base64 | tr -d '\n')"
  echo "::add-mask::$basic"
  auth_header="AUTHORIZATION: basic $basic"
fi
git_auth() {
  if [ -n "$auth_header" ]; then
    GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0="http.https://github.com/.extraheader" \
      GIT_CONFIG_VALUE_0="$auth_header" git_safe "$@"
  else
    git_safe "$@"
  fi
}

# A JSON string body: backslash and quote escaped, control characters dropped
# (branch names may contain " and \, never control characters).
esc() {
  local s
  s="$(printf '%s' "$1" | tr -d '\000-\037')"
  s="${s//\\/\\\\}"
  printf '%s' "${s//\"/\\\"}"
}

sha="${PR_HEAD_SHA:-}"; [ -n "$sha" ] || sha="$GITHUB_SHA"
workflow="${GITHUB_WORKFLOW// /-}"
base="runs/${sha}/${workflow}"

# status.json for one job: <dir> <job> <matrix> <status> [<reported> <needs-result>]
write_status() {
  cat > "$1/status.json" <<JSON
{"workflow":"$(esc "$GITHUB_WORKFLOW")","job":"$(esc "$2")","matrix":"$(esc "$3")","status":"$(esc "$4")","reported":"$(esc "${5:-$4}")","needs_result":"$(esc "${6:-}")","sha":"$(esc "$sha")","ref":"$(esc "${GITHUB_HEAD_REF:-${GITHUB_REF_NAME:-}}")","event":"$(esc "${GITHUB_EVENT_NAME:-}")","run_id":"$(esc "${GITHUB_RUN_ID:-}")","attempt":"$(esc "${GITHUB_RUN_ATTEMPT:-}")","finished_at":"$(date -u +%Y-%m-%dT%H:%M:%SZ)"}
JSON
}

# Copies <src> into <dest>: directories and regular files only. Symbolic
# links are skipped, never followed (one could point at a runner secret), and
# so are .git* entries (a .git directory, .gitattributes, .gitmodules), which
# would act on later publishes.
copy_tree() {
  local src="$1" dest="$2" f
  mkdir -p "$dest"
  [ -d "$src" ] || return 0
  (cd "$src" && find . -name '.git*' -prune -o -type f -print0) |
    while IFS= read -r -d '' f; do
      mkdir -p "$dest/$(dirname "$f")"
      cp -- "$src/$f" "$dest/$f"
    done
}

# GitHub's result for a job from $NEEDS (toJSON(needs)), or "".
needs_result() {
  NEEDS_JOB="$1" python3 -I -c '
import json, os
try:
    needs = json.loads(os.environ.get("NEEDS") or "{}")
    print((needs.get(os.environ["NEEDS_JOB"]) or {}).get("result", ""))
except Exception:
    print("")
' 2>/dev/null | tr -cd 'a-z'
}

# A field of a job.json written by job-status.sh, or "" (no jq needed).
field() {
  sed -n "s/.*\"$2\":\"\\([A-Za-z0-9._-]*\\)\".*/\\1/p" "$1" 2>/dev/null | head -n 1
}

# Copies the results into the ci-results worktree <work>.
stage() {
  local work="$1"
  if [ -z "$from" ]; then
    local dest="$base/${GITHUB_JOB}${MATRIX_KEY:+-$MATRIX_KEY}"
    copy_tree "${CI_OUT:-ci-out}" "$work/$dest"
    write_status "$work/$dest" "$GITHUB_JOB" "${MATRIX_KEY:-}" "$status"
    msg="results: ${workflow}/${GITHUB_JOB} ${status} @ ${sha:0:9}"
    return
  fi
  local n=0 dir name job matrix st reported needed
  for dir in "$from"/ci-out-*/; do
    [ -d "$dir" ] || continue
    name="${dir%/}"; name="${name##*/ci-out-}"
    # Artifact names come from the workflow; refuse anything that isn't a
    # plain path segment.
    if [[ ! "$name" =~ ^[A-Za-z0-9._-]+$ ]] || [ "$name" = . ] || [ "$name" = .. ]; then
      echo "skipping artifact with an unexpected name: $name" >&2
      continue
    fi
    job="$(field "$dir/job.json" job)"; job="${job:-${name%%-*}}"
    matrix="$(field "$dir/job.json" matrix)"
    reported="$(field "$dir/job.json" status)"
    case "$reported" in success | failure | cancelled) ;; *) reported=unknown ;; esac
    # GitHub's result is per job (all matrix legs together). When it isn't
    # success, no leg is published as success on the build job's word.
    needed="$(needs_result "$job")"
    st="$reported"
    if [ -n "$needed" ] && [ "$needed" != success ]; then st="$needed"; fi
    copy_tree "$dir" "$work/$base/$name"
    write_status "$work/$base/$name" "$job" "$matrix" "$st" "$reported" "$needed"
    n=$((n + 1))
  done
  mkdir -p "$work/$base"
  printf '%s\n' "${NEEDS:-{\}}" > "$work/$base/needs.json"
  msg="results: ${workflow} (${n} jobs) @ ${sha:0:9}"
}

work="$(mktemp -d)"
msg=""
for attempt in 1 2 3 4 5 6; do
  git_safe worktree remove --force "$work" >/dev/null 2>&1 || true
  rm -rf "$work"
  if git_auth fetch --quiet --depth 1 origin "+refs/heads/ci-results:refs/remotes/origin/ci-results" 2>/dev/null; then
    git_safe worktree add --quiet --detach "$work" origin/ci-results
  else
    git_safe worktree add --quiet --detach "$work" HEAD
    (cd "$work" && git_safe checkout --quiet --orphan ci-results-new && git_safe rm -rfq --ignore-unmatch . && rm -rf ./*)
  fi
  # With core.symlinks=false nothing checks out as a link; refuse to write
  # into a tree that has one anyway.
  if [ -n "$(find "$work" -path "$work/.git" -prune -o -type l -print -quit)" ]; then
    echo "the ci-results worktree contains a symbolic link; not publishing" >&2
    exit 0
  fi
  stage "$work"
  (cd "$work" && git_safe add -A && git_safe commit --quiet -m "$msg" &&
    git_auth push --quiet origin HEAD:refs/heads/ci-results) && { echo "published to ci-results:$base"; exit 0; }
  sleep $((attempt * 4))
done
echo "could not publish results" >&2
exit 0
