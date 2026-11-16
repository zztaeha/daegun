"""Each row of latency.c's output held to its Rust twin's: C's best within 10 ns of Rust's, or 5% past 200 ns.
A side's time is the median of its runs, a row's gap the median over layouts split by `--`, as code layout moves rows.

Usage: latency-delta.py rust.txt c.txt [rust.txt c.txt ...] [-- rust.txt c.txt ...]"""

import itertools
import re
import statistics
import sys

ROW = re.compile(r"^  (\S.*?)\s+([0-9.]+) ns\s+median\s+([0-9.]+) ns$")


def rows(paths):
    runs = {}
    for path in paths:
        for m in filter(None, (ROW.match(line.rstrip()) for line in open(path, encoding="utf-8"))):
            runs.setdefault(m.group(1), []).append(float(m.group(2)))
    return {name: statistics.median(times) for name, times in runs.items()}


def main(argv):
    groups = [list(group) for dash, group in itertools.groupby(argv, lambda a: a == "--") if not dash]
    if not groups or any(len(group) % 2 for group in groups):
        print(__doc__)
        return 1
    layouts = [(rows(group[0::2]), rows(group[1::2])) for group in groups]
    problems = [f"  ONE SIDE  {name} is timed only in {'Rust' if name in rust else 'C'}"
                for rust, c in layouts for name in sorted(set(rust) ^ set(c))]
    names = sorted(set.intersection(*(set(rust) & set(c) for rust, c in layouts)))
    for name in names:
        gaps = [c[name] - rust[name] for rust, c in layouts]
        delta = statistics.median(gaps)
        rust_time = statistics.median(rust[name] for rust, _ in layouts)
        bound = max(10.0, 0.05 * rust_time)
        each = " ".join(f"{gap:+.1f}" for gap in gaps)
        print(f"  {name:<26} Rust {rust_time:9.1f} ns   {delta:+8.1f} ns   ({each})")
        if delta > bound:
            problems.append(f"  SLOWER  {name}: C takes {delta:.1f} ns over Rust, past {bound:.1f}")
    if problems or not names:
        print("issues")
        print("\n".join(problems) or "  no rows")
        return 1
    print(f"{len(names)} rows, over {len(layouts)} layouts each C call within its bound over Rust's")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
