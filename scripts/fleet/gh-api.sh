#!/usr/bin/env bash
# Minimal GitHub REST call for the fleet (repo-scoped endpoints only; the proxy adds auth).
# Usage: scripts/fleet/gh-api.sh GET|POST|PATCH|PUT <path under /repos/OWNER/REPO> [json-body-file|-]
set -euo pipefail
REPO="${SURIMONO_REPO:-kwallaschek/surimono}"
method="$1"; path="$2"; body="${3:-}"
args=(-sS -m 60 -X "$method" -H "Accept: application/vnd.github+json" -H "X-GitHub-Api-Version: 2022-11-28")
if [ -n "$body" ]; then args+=(-H "Content-Type: application/json" --data-binary "@$body"); fi
curl "${args[@]}" "https://api.github.com/repos/${REPO}${path}"
