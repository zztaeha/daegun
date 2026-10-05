import statistics
import subprocess
import sys
import os
import re

HERE = os.path.dirname(os.path.abspath(__file__))
ROW = re.compile(r"^\s*([\d.]+)%\s+([\d,]+)\s+(.+)$")

def one(d):
    out = subprocess.run(
        [sys.executable, os.path.join(HERE, "pmu-attribute.py"),
         f"{d}/time-profile.xml", f"{d}/kdebug-counters-with-time-sample.xml"],
        capture_output=True, text=True).stdout
    shares, total = {}, None
    for line in out.splitlines():
        if "instructions retired" in line:
            total = int(line.split(":")[1].split()[0].replace(",", ""))
        m = ROW.match(line)
        if m:
            shares[m.group(3).strip()] = float(m.group(1))
    return total, shares

def main():
    dirs = sys.argv[1:]
    if len(dirs) < 2:
        sys.exit("give at least two recordings – one run proves nothing, which is the point")

    totals, runs = [], []
    for d in dirs:
        t, s = one(d)
        if t is None:
            print(f"  ({d}: no output – a failed recording?)", file=sys.stderr)
            continue
        totals.append(t)
        runs.append(s)

    lo, hi = min(totals), max(totals)
    print(f"{len(runs)} runs   instructions retired {lo:,}..{hi:,}"
          f"   spread {100 * (hi - lo) / lo:.1f}%\n")

    names = set().union(*runs)
    rows = []
    for n in names:
        vals = [r.get(n, 0.0) for r in runs]
        rows.append((statistics.median(vals), min(vals), max(vals), n))
    rows.sort(reverse=True)

    print(f"{'median':>8} {'range':>16}   function")
    for med, mn, mx, n in rows[:24]:
        steady = mn > 0 and mx <= mn * 2
        print(f"{med:7.1f}% {mn:6.1f}–{mx:5.1f}%  {'*' if steady else ' '} {n}")
    print("\n  * present in every run and within a factor of two – the only rows worth a hypothesis.")

if __name__ == "__main__":
    main()
