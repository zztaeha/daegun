#!/usr/bin/env python3
USAGE = """    syllable-equivalence.py --save <machine> <path>     freeze the current machine
    syllable-equivalence.py --check <machine> <path>    compare against a frozen one

Exit status is 1 on any difference, so it drops straight into a shell loop."""


import json
import os
import re
import sys
from collections import deque

HERE = os.path.dirname(os.path.abspath(__file__))
TABLES = os.path.join(HERE, "..", "..", "..", "src", "daecore", "src", "daeshaper", "generated", "syllable_tables.rs")

MACHINES = ("INDIC", "KHMER", "MYANMAR", "USE")

def load(name):
    src = open(TABLES, encoding="utf-8").read()

    m = re.search(rf"static {name}_TRANSITIONS: \[\[u16; (\d+)\]; \d+\] = \[(.*?)\n\];", src, re.S)
    if not m:
        raise SystemExit(f"no {name}_TRANSITIONS in {TABLES}")
    ncat = int(m.group(1))
    rows = re.findall(r"\[([^\]]*)\]", m.group(2))
    table = []
    for row in rows:
        cells = [c.strip() for c in row.split(",") if c.strip()]
        if len(cells) == ncat:
            table.append([-1 if c == "DEAD" else int(c) for c in cells])

    a = re.search(rf"static {name}_ACCEPTS: \[u8; \d+\] = \[(.*?)\];", src, re.S)
    cells = [c.strip() for c in a.group(1).replace("\n", " ").split(",") if c.strip()]
    accepts = {i: (-1 if c == "NONE" else int(c)) for i, c in enumerate(cells)}

    return {"ncat": ncat, "table": table, "accepts": {k: v for k, v in accepts.items() if v >= 0}}

def kind(machine, state):
    if state < 0:
        return None
    return machine["accepts"].get(state, machine["accepts"].get(str(state)))

def compare(a, b):
    if a["ncat"] != b["ncat"]:
        return ("alphabet", a["ncat"], b["ncat"])

    seen = {(0, 0)}
    queue = deque([((0, 0), [])])
    while queue:
        (sa, sb), path = queue.popleft()
        ka, kb = kind(a, sa), kind(b, sb)
        if ka != kb:
            return (path, ka, kb)
        for c in range(a["ncat"]):
            na = a["table"][sa][c] if sa >= 0 else -1
            nb = b["table"][sb][c] if sb >= 0 else -1
            step = (na, nb)
            if step not in seen and (na >= 0 or nb >= 0):
                seen.add(step)
                queue.append((step, path + [c]))
    return None

def main(argv):
    if len(argv) < 4 or argv[1] not in ("--save", "--check"):
        raise SystemExit(USAGE)
    mode, name, path = argv[1], argv[2].upper(), argv[3]
    if name not in MACHINES:
        raise SystemExit(f"unknown machine {name}; expected one of {', '.join(MACHINES)}")

    current = load(name)
    if mode == "--save":
        json.dump(current, open(path, "w"))
        print(f"{name}: saved {len(current['table'])} states, {current['ncat']} categories")
        return 0

    saved = json.load(open(path))
    result = compare(saved, current)
    if result is None:
        print(f"{name}: EQUIVALENT – identical on every input of every length")
        return 0
    if result[0] == "alphabet":
        print(f"{name}: different alphabets, {result[1]} vs {result[2]}")
        return 1
    path_, was, now = result
    print(f"{name}: DIFFER on the shortest input {path_ or '(empty)'}")
    print(f"  saved accepts: {was}")
    print(f"  now accepts:   {now}")
    return 1

if __name__ == "__main__":
    sys.exit(main(sys.argv))
