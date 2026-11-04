#!/usr/bin/env bash
# Checks every finding in this directory against quickjs-ng. Clones it into a
# temporary directory, builds it twice (ASan for the crashes, release for the
# time limits) and prints one line per reproducer. Nothing outside the temporary
# directory is touched. Needs git, cmake, a C compiler with AddressSanitizer
# and 10 to 15 minutes.
#
#   ./repro.sh                 current master
#   ./repro.sh <commit>        that commit
#   QUICKJS_DIR=/path ./repro.sh   an existing checkout (it is not modified)
#
# REPRODUCED means the bug is there; "not reproduced" means this reproducer no
# longer shows it, so read the finding's code before filing.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
ref="${1:-master}"
cc="${CC:-cc}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# step WHAT COMMAND...: runs COMMAND; on failure says which step and why, and stops.
step() {
  local what="$1"; shift
  if ! "$@" >"$work/step.log" 2>&1; then
    echo "repro.sh: $what failed:" >&2
    tail -n 30 "$work/step.log" >&2
    exit 2
  fi
}

if [ -n "${QUICKJS_DIR:-}" ]; then
  src="$QUICKJS_DIR"
else
  src="$work/quickjs"
  step "cloning quickjs-ng" git clone -q https://github.com/quickjs-ng/quickjs.git "$src"
  step "checking out $ref" git -C "$src" checkout -q "$ref"
fi
echo "quickjs-ng $(git -C "$src" rev-parse HEAD) ($(git -C "$src" log -1 --format=%cs))"

echo "building (ASan and release)..."
jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)"
step "configuring the ASan build" cmake -S "$src" -B "$work/asan" -DCMAKE_BUILD_TYPE=Debug \
  -DCMAKE_C_COMPILER="$cc" -DQJS_ENABLE_ASAN=ON -DCMAKE_C_FLAGS="-O1 -D__SANITIZE_ADDRESS__"
step "the ASan build" cmake --build "$work/asan" -j "$jobs" --target qjs
step "configuring the release build" cmake -S "$src" -B "$work/rel" -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_C_COMPILER="$cc"
step "the release build" cmake --build "$work/rel" -j "$jobs" --target qjs qjs_exe
step "compiling harness/fail-nth.c" "$cc" -g -O1 -fsanitize=address -I"$src" "$here/harness/fail-nth.c" \
  "$work/asan/libqjs.a" -lm -lpthread -ldl -o "$work/fail-nth"
step "compiling harness/time-limit.c" "$cc" -g -O2 -I"$src" "$here/harness/time-limit.c" \
  "$work/rel/libqjs.a" -lm -lpthread -ldl -o "$work/time-limit"

export ASAN_OPTIONS=detect_leaks=0:allocator_may_return_null=1:abort_on_error=0

# quickjs.c keeps a small-block arena unless __SANITIZE_ADDRESS__ is defined. gcc
# predefines it under -fsanitize=address and clang doesn't, so the ASan build above
# defines it. With the arena on, most allocations never reach the harness and the
# sweeps below would find nothing, which would read as "bug gone". syntax.js makes
# about 3,000 allocations with the arena off and 140 with it on.
allocs="$("$work/fail-nth" count "$here/cases/oom/syntax.js" 2>/dev/null || true)"
if ! [[ "$allocs" =~ ^[0-9]+$ ]] || [ "$allocs" -lt 600 ]; then
  echo "repro.sh: the ASan build still has the small-block arena on: cases/oom/syntax.js made '$allocs' allocations through the harness, expected at least 600. The results would be meaningless." >&2
  exit 2
fi

# oom FINDING CASE PATTERN: refuses each allocation of cases/oom/CASE.js in turn
# and looks for PATTERN (a sanitizer frame or an assertion) in a crash report.
oom() {
  local finding="$1" script="$here/cases/oom/$2.js" pattern="$3" n report
  for n in $("$work/fail-nth" sweep "$script" 2>/dev/null); do
    report="$("$work/fail-nth" run "$n" "$script" 2>&1 || true)"
    if grep -Eq "$pattern" <<<"$report"; then
      printf 'REPRODUCED      %-3s %-26s refused allocation %s: %s %s\n' "$finding" "$2" "$n" \
        "$(grep -Em1 'ERROR: AddressSanitizer|Assertion' <<<"$report" | sed -E 's/^==[0-9]+==//; s/ on (unknown )?address.*//; s/.*(Assertion .*failed).*/\1/')" \
        "$(grep -Eom1 '^(READ|WRITE) of size [0-9]+|caused by a (READ|WRITE) memory access' <<<"$report" | sed -E 's/caused by a (READ|WRITE) memory access/\1/')"
      return
    fi
  done
  printf 'not reproduced  %-3s %-26s (%s)\n' "$finding" "$2" "$pattern"
}

# slow FINDING CASE [LIMIT_MS]: runs cases/time/CASE.js under a time limit.
slow() {
  local finding="$1" out rc
  out="$("$work/time-limit" "${3:-250}" 15 "$here/cases/time/$2.js" 2>&1)"
  rc=$?
  if [ "$rc" -eq 1 ] || [ "$rc" -eq 3 ]; then
    printf 'REPRODUCED      %-3s %-26s %s\n' "$finding" "$2" "$out"
  else
    printf 'not reproduced  %-3s %-26s %s\n' "$finding" "$2" "$out"
  fi
}

echo
echo "out-of-memory paths, one allocation refused"
oom 00 compiler 'resolve_variables|resolve_scope_var'
oom 00 compiler 'cpool_idx >= 0'
oom 01 bind 'js_bound_function_finalizer|js_bound_function_mark'
oom 02 resolve-labels 'in resolve_labels'
oom 02 parser-offsets 'first_reloc == NULL'
oom 03 parser-offsets 'js_parse_class'
oom 03 loops 'js_parse_for_in_of'
oom 03 syntax 'in js_parse_destructuring_element'
oom 04 atoms 'free_bytecode_atoms'
oom 05 promise-allsettled 'list_empty\(&rt->gc_obj_list\)'
oom 05 promise-any 'list_empty\(&rt->gc_obj_list\)'
oom 06 regexp-groups 'JS_NewAtom'
oom 07 parameter-destructuring 'in JS_CallInternal'
oom 08 array-subclass 'JS_NewObjectFromShape'

echo
echo "out of memory from JavaScript alone (release build, qjs --memory-limit)"
# js_scan FINDING NAME FIRST STEP LAST SCRIPT: runs SCRIPT, with $k the pad size.
js_scan() {
  local finding="$1" name="$2" first="$3" step="$4" last="$5" script="$6" crashes=0 runs=0 k
  for k in $(seq "$first" "$step" "$last"); do
    runs=$((runs + 1))
    "$work/rel/qjs" --memory-limit 8M -e "${script//\$k/$k}" >/dev/null 2>&1
    [ $? -ge 128 ] && crashes=$((crashes + 1))
  done 2>/dev/null
  if [ "$crashes" -gt 0 ]; then
    printf 'REPRODUCED      %-3s %-26s %s of %s pad sizes end in a signal\n' "$finding" "$name" "$crashes" "$runs"
  else
    printf 'not reproduced  %-3s %-26s 0 of %s pad sizes\n' "$finding" "$name" "$runs"
  fi
}
js_scan 01 bind-from-js 5900000 16384 6400000 \
  'const pad = new Uint8Array($k); const args = new Array(30000).fill(0); (function () {}).bind(null, ...args)'
js_scan 08 array-subclass-from-js 6000000 5003 6800000 \
  'const pad = new Uint8Array($k); const keep = []; for (;;) { class A extends Array {} keep.push(Object.create(A.prototype), new A()); }'

echo
echo "from JavaScript alone (release build, qjs with default limits)"
# js_once FINDING CASE: runs cases/js/CASE.js and looks for a signal.
js_once() {
  local rc
  "$work/rel/qjs" "$here/cases/js/$2.js" >/dev/null 2>&1
  rc=$?
  if [ "$rc" -ge 128 ]; then
    printf 'REPRODUCED      %-3s %-26s ended in signal %s\n' "$1" "$2" "$((rc - 128))"
  else
    printf 'not reproduced  %-3s %-26s exit %s\n' "$1" "$2" "$rc"
  fi
} 2>/dev/null
js_once 07 generator-parameters

echo
echo "time limits, 250 ms unless noted; the handler is written to answer at once"
slow -  loop-baseline
for c in swallow-promise swallow-asyncgen swallow-disposable; do slow 09 "$c"; done
for c in array-join array-fill array-reverse array-copywithin array-concat string-raw; do slow 10 "$c"; done
for c in granular-json granular-indexof granular-string; do slow 11 "$c"; done
for c in search-naive search-replaceall bigint-tostring sort-typed; do slow 12 "$c"; done
slow 13 regexp-delete
slow 14 parse-source
slow 14 parse-locals
slow 14 regexp-compile
slow 14 json-parse
