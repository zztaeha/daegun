import collections, re, sys

INSTRUCTIONS = 7
CALLER_OF = None
ROW = re.compile(r"<row>(.*?)</row>", re.S)
FRAME = re.compile(r'<frame id="\d+" name="([^"]+)"')

def _key(row, ids):
    m = re.search(r'<sample-time id="(\d+)"[^>]*>(\d+)</sample-time>|<sample-time ref="(\d+)"/>', row)
    stamp = None
    if m:
        if m.group(2):
            stamp = int(m.group(2)); ids[("t", m.group(1))] = stamp
        else:
            stamp = ids.get(("t", m.group(3)))
    m = re.search(r'<thread id="(\d+)".*?<tid id="\d+"[^>]*>(\d+)</tid>|<thread ref="(\d+)"/>', row, re.S)
    thread = None
    if m:
        if m.group(2):
            thread = m.group(2); ids[("th", m.group(1))] = thread
        else:
            thread = ids.get(("th", m.group(3)))
    return stamp, thread

def stacks(path):
    text = open(path, errors="replace").read()
    ids = {}
    out = collections.defaultdict(list)
    for match in ROW.finditer(text):
        row = match.group(1)
        stamp, thread = _key(row, ids)
        if stamp is None or thread is None:
            continue
        own = [n for n in FRAME.findall(row) if "daegun" in n]
        if not own:
            continue
        pick = own[0]
        if CALLER_OF:
            outer = [n for n in own if CALLER_OF not in n]
            if CALLER_OF not in own[0] or not outer:
                continue
            pick = outer[0]
        out[thread].append((stamp, re.sub(r"::h[0-9a-f]{16}$", "", pick)))
    for series in out.values():
        series.sort()
    return out

def counters(path):
    text = open(path, errors="replace").read()
    ids = {}
    series = collections.defaultdict(list)
    for match in ROW.finditer(text):
        row = match.group(1)
        stamp, thread = _key(row, ids)
        m = re.search(r"<pmc-events[^>]*>([\d ]+)</pmc-events>", row)
        if stamp is None or thread is None or not m:
            continue
        series[thread].append((stamp, [int(v) for v in m.group(1).split()]))
    return series

def main():
    import bisect
    global CALLER_OF
    if "--caller-of" in sys.argv:
        CALLER_OF = sys.argv[sys.argv.index("--caller-of") + 1]
    frames = stacks(sys.argv[1])
    by_function = collections.Counter()
    total = attributed = 0
    for thread, series in counters(sys.argv[2]).items():
        series.sort()
        for (t0, a), (t1, b) in zip(series, series[1:]):
            gap = (t1 - t0) / 1e9
            if not 0 < gap < 0.01:
                continue
            delta = b[INSTRUCTIONS] - a[INSTRUCTIONS]
            if delta < 0:
                continue
            total += delta
            series = frames.get(thread)
            if not series:
                continue
            times = [t for t, _ in series]
            lo = bisect.bisect_left(times, t0)
            hi = bisect.bisect_right(times, t1)
            inside = series[lo:hi]
            if not inside:
                continue
            share = delta / len(inside)
            for _, name in inside:
                by_function[name] += share
            attributed += delta
    if not total:
        print("no counter deltas – was the recording made with backtraces enabled?"); sys.exit(1)
    what = f"callers of {CALLER_OF!r}" if CALLER_OF else "daegun frames"
    print(f"instructions retired: {total:,}   attributed to {what}: "
          f"{attributed:,} ({100 * attributed / total:.1f}%)\n")
    for name, n in by_function.most_common(20):
        short = name.replace("daegun::daeshaper::", "").replace("daegun::", "")
        print(f"  {100 * n / attributed:>5.1f}%  {n:>14,.0f}  {short[:78]}")

main()
