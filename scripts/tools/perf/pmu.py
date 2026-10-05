import collections, os, re, sys

CYCLES, INSTRUCTIONS = 2, 7
ROW = re.compile(r"<row>(.*?)</row>", re.S)

def _samples(path):
    text = open(path, errors="replace").read()
    ids, cores, out = {}, {}, []
    for match in ROW.finditer(text):
        row = match.group(1)
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
        m = re.search(r'<core id="(\d+)"[^>]*>(\d+)</core>|<core ref="(\d+)"/>', row)
        core = None
        if m:
            if m.group(2):
                core = m.group(2); cores[m.group(1)] = core
            else:
                core = cores.get(m.group(3))
        m = re.search(r"<pmc-events[^>]*>([\d ]+)</pmc-events>", row)
        if stamp is not None and m and thread and core:
            out.append((stamp, thread, core, [int(v) for v in m.group(1).split()]))
    return out

def totals(path):
    rows = _samples(path)
    by_context = collections.defaultdict(list)
    for stamp, thread, core, counters in rows:
        by_context[(thread, core)].append((stamp, counters))
    cycles = instructions = 0
    for series in by_context.values():
        series.sort()
        for (t0, a), (t1, b) in zip(series, series[1:]):
            gap = (t1 - t0) / 1e9
            if not 0 < gap < 0.01:
                continue
            delta = [y - x for x, y in zip(a, b)]
            if any(v < 0 for v in delta):   # counter reset, or the thread moved cores
                continue
            cycles += delta[CYCLES]
            instructions += delta[INSTRUCTIONS]
    return cycles, instructions, len(rows)

def main():
    out_dir, iters, runs = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    floor = int(os.environ.get("MIN_SAMPLES", "2000"))
    kept = []
    for i in range(1, runs + 1):
        path = f"{out_dir}/r{i}.xml"
        if not os.path.exists(path):
            continue
        cycles, instructions, n = totals(path)
        if n < floor or cycles == 0:
            print(f"  run {i}: {n:>5} samples – DISCARDED, high-frequency sampling did not take")
            continue
        kept.append((instructions / iters, cycles / iters))
        print(f"  run {i}: {n:>5} samples  {instructions / iters:>10,.0f} insn  "
              f"{cycles / iters:>9,.0f} cyc  IPC {instructions / cycles:.2f}")
    if not kept:
        print("no run reached the sample floor – nothing measured"); sys.exit(1)
    ins = [k[0] for k in kept]
    cyc = [k[1] for k in kept]
    spread = 100 * (max(ins) - min(ins)) / min(ins) if len(ins) > 1 else 0.0
    print(f"\n  {len(kept)}/{runs} usable   median {sorted(ins)[len(ins) // 2]:,.0f} insn/iter, "
          f"{sorted(cyc)[len(cyc) // 2]:,.0f} cyc/iter   spread {spread:.2f}%")
    if spread > 1.0:
        print("  spread above 1% – treat a difference smaller than that as unresolved")

main()
