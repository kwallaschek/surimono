#!/usr/bin/env bash
# Frees disk in the shared target dir: for every workspace-crate artifact and test
# binary (which accumulate one copy per feature set / dependency change), keeps the
# newest two and deletes the rest. crates.io dependency artifacts are untouched, so
# nothing expensive (Stylo, QuickJS) is rebuilt. Safe while agents work; at worst a
# running build relinks.
set -euo pipefail
dir="${CARGO_TARGET_DIR:-/home/claude/target}/debug/deps"
[ -d "$dir" ] || exit 0
cd "$dir"
python3 - <<'PY'
import os, re, collections, stat
names=[n for n in os.listdir('.') if re.match(r'^(lib)?surimono[_-]', n)]
for n in os.listdir('.'):
    try: st=os.stat(n)
    except FileNotFoundError: continue
    if stat.S_ISREG(st.st_mode) and (st.st_mode & stat.S_IXUSR) and '.' not in n: names.append(n)
groups=collections.defaultdict(list)
for n in set(names):
    m=re.match(r'^(.*?)-([0-9a-f]{16})(.*)$', n)
    key=(m.group(1), m.group(3)) if m else (n, '')
    try: groups[key].append((os.path.getmtime(n), n))
    except FileNotFoundError: pass
freed=0
for items in groups.values():
    items.sort(reverse=True)
    for _, n in items[2:]:
        try: freed+=os.path.getsize(n); os.remove(n)
        except FileNotFoundError: pass
print(f"freed {freed/1e9:.1f} GB")
PY
df -h / | tail -1
