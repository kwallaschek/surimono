#!/usr/bin/env bash
# Chrome reference renders (reference.yml). The fixture tooling lives in fixtures/tools/
# (owned by the conformance researcher); this only calls it.
set -euo pipefail
cd "$(dirname "$0")/../.."
if [ ! -x fixtures/tools/reference.sh ]; then echo "fixtures/tools/reference.sh not there yet; nothing to render"; exit 0; fi
fixtures/tools/reference.sh "${CI_JOB_OUT:-ci-out/reference}"
