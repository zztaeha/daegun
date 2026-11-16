#!/usr/bin/env python3
# The fuzzing campaign: builds, seeds and dictionaries, the fuzzers started and stopped, a live readout, a watch for
# new findings, triage, coverage and replay. Its data lives beside the engine in fuzz-campaign/, never in a temporary
# folder. Usage: campaign.py build | seeds | merge | start [names] | stop [names] | live | watch | status | triage
#        | fixed <ids> | coverage [names] | replay
import hashlib
import json
import os
import random
import re
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

E = Path(__file__).resolve().parents[3]
FUZZ = E / "scripts/tools/fuzz"
C = Path(os.environ.get("DAEGUN_CAMPAIGN", E.parent / "fuzz-campaign"))
HOST = re.search(r"host: (\S+)", subprocess.run(["rustc", "-vV"], capture_output=True, text=True).stdout)[1]
OUT = E / "scripts/.out/target"
LIBFUZZER = OUT / HOST / "release"
ASAN = C / "build/asan" / HOST / "release"
AFL = OUT / "release"
CMPLOG = C / "build/cmplog/release"
ABI = C / "build/c/abi"
ABI_PLAIN = C / "build/c/abi-plain"
RUN = C / "build/run/release/run"
# What one input may take in a plain build before it's a finding: the work budgets' "within seconds" a call, times
# the heavy calls a target makes of an input, with room for a busy machine. The budget holes the sweep found took
# 41 s to hours. The timeouts beside them are the instrumented builds', which trace every comparison.
FONT_MAX = 512 << 10
# The largest input AFL++ and the small instances are fed: a big font takes AFL++ a second a run, and calibrating it
# up to forty runs.
SMALL_MAX = 32 << 10
CHOICES = 32

# Each target: seconds an input may take instrumented, the longest input, its seeds, and seconds plain. `outline`
# resolves each of two glyphs' outlines twice, plain and chosen, and a hostile stroke took 1.9 s a resolve within its
# 2^25-step budget: 40 s allows eight such, with room.
TARGETS = {
    "load": (10, FONT_MAX + 4096, "fonts", 3),
    "outline": (60, FONT_MAX + 4096, "fonts", 40),
    "raster": (60, FONT_MAX + 4096, "fonts", 10),
    "shape": (30, FONT_MAX + 4096, "fonts", 5),
    "derive": (60, FONT_MAX + 4096, "fonts", 20),
    "text": (5, 4096, "text", 1),
    "everything": (120, FONT_MAX + 4096, "fonts", 60),
    "pool": (60, FONT_MAX + 4096, "fonts", 20),
    "stack": (120, FONT_MAX + 4096, "fonts", 60),
    "c_abi": (60, 9 * 512 + (96 << 10), "c", 20),
    "shape_small": (30, SMALL_MAX, "fonts", 5),
    "raster_small": (60, SMALL_MAX, "fonts", 10),
}
# Targets started from another's corpus rather than the seeds: that target, and the largest input taken.
FED = {"stack": ("everything", FONT_MAX + 4096), "shape_small": ("shape", SMALL_MAX),
       "raster_small": ("raster", SMALL_MAX)}
AFL_TARGETS = ["load", "outline", "raster", "shape", "derive", "text", "everything"]
STATUS = re.compile(r"^#(\d+): cov: (\d+) ft: (\d+) corp: (\d+) exec/s: (\d+) oom/timeout/crash: (\d+)/(\d+)/(\d+) "
                    r"time: (\d+)s")
TEXTS = ["", "A", "fi ffi ffl", "Hello, world! 123", "سلام (a) سلام", "हिन्दी किताब", "日本語のテキスト", "ภาษาไทย",
         "שלום עולם", "한국어", "ខ្មែរ", "မြန်မာ", "বাংলা", "👨‍👩‍👧 ❤️ 🏳️‍🌈", "á̧ë",
         "⁦abc⁩ ‮def‬", "الـله", "x‍y‌z", "\t\n\r  ",
         "((([[[{{{", "1/2 3/4", "ﷺ ﷽", "𝕏𝔸𝕄", "﻿�\u0000",
         # Paths only these reach: Syriac's abbreviation mark, which stch stretches; a meteg after a wide and a narrow
         # vowel; old Hangul jamo and tone marks; and reph.
         "ܐܠܗܐ \u070fܡܪܝ", "\u05dc\u05b7\u05b4\u05bd\u05dd \u05d1\u05b8\u05b0\u0591",
         "\u1100\u1161\u11a8 \uac00\u302e \u302f\u1112\u1175", "र्क्ष र्ति র্ক"]


def run(cmd, cwd=None, env=None, check=True, **kw):
    print("$", " ".join(map(str, cmd)), flush=True)
    done = subprocess.run(cmd, cwd=cwd, env={**os.environ, **(env or {})}, **kw)
    if check and done.returncode:
        sys.exit(f"failed: {cmd[0]} exit {done.returncode}")
    return done


def clang():
    llvm = re.search(r"LLVM version: (\d+)", subprocess.run(["rustc", "+nightly", "-vV"], capture_output=True,
                                                             text=True).stdout)[1]
    for c in [f"/opt/homebrew/opt/llvm@{llvm}/bin/clang", "/opt/homebrew/opt/llvm/bin/clang"]:
        if Path(c).exists() and f"clang version {llvm}." in subprocess.run([c, "--version"], capture_output=True,
                                                                           text=True).stdout:
            return Path(c)
    sys.exit(f"no clang on nightly's LLVM {llvm} (brew install llvm@{llvm})")


def build():
    run(["cargo", "+nightly", "fuzz", "build", "--fuzz-dir", ".", "-s", "none"], cwd=FUZZ / "libfuzzer")
    # The same targets under ASan, which prints a stack for a timeout or a memory limit: triage's build.
    run(["cargo", "+nightly", "fuzz", "build", "--fuzz-dir", ".", "-s", "address"], cwd=FUZZ / "libfuzzer",
        env={"CARGO_TARGET_DIR": str(C / "build/asan-target")})
    ASAN.parent.mkdir(parents=True, exist_ok=True)
    shutil.rmtree(ASAN, ignore_errors=True)
    shutil.copytree(C / "build/asan-target" / HOST / "release", ASAN,
                    ignore=shutil.ignore_patterns("build", "deps", "incremental", "examples", "*.d", "*.rlib"))
    run(["cargo", "afl", "build", "--release"], cwd=FUZZ / "afl")
    run(["cargo", "afl", "build", "--release"], cwd=FUZZ / "afl",
        env={"AFL_LLVM_CMPLOG": "1", "CARGO_TARGET_DIR": str(C / "build/cmplog")})
    run(["cargo", "build", "--release"], cwd=FUZZ)
    run(["cargo", "build", "--release", "--features", "threading", "--bin", "run"], cwd=FUZZ,
        env={"CARGO_TARGET_DIR": str(C / "build/run")})
    # The C side as `c/run.sh` builds it, with UBSan over abi.c too.
    cov = ("-Cpasses=sancov-module -Cllvm-args=-sanitizer-coverage-level=4 -Cllvm-args=-sanitizer-coverage-pc-table "
           "-Cllvm-args=-sanitizer-coverage-inline-8bit-counters -Cllvm-args=-sanitizer-coverage-trace-compares")
    run(["cargo", "+nightly", "rustc", "--release", "--features", "capi", "--crate-type", "staticlib",
         "--target", HOST],
        cwd=E, env={"RUSTFLAGS": f"-Zsanitizer=address {cov} -Cdebug-assertions -Coverflow-checks",
                    "CARGO_TARGET_DIR": str(C / "build/c")})
    run([clang(), "-std=c11", "-g", "-O1", "-fsanitize=fuzzer,address,undefined", "-fno-sanitize-recover=undefined",
         "-I", "src/api/c/include", "scripts/tools/fuzz/c/abi.c", C / "build/c" / HOST / "release/libdaegun.a",
         "-o", ABI], cwd=E)
    # And plain, the library as a caller links it, to time what the instrumented one found slow.
    run(["cargo", "rustc", "--release", "--features", "capi", "--crate-type", "staticlib"], cwd=E,
        env={"CARGO_TARGET_DIR": str(C / "build/c-plain")})
    run([clang(), "-std=c11", "-g", "-O2", "-fsanitize=fuzzer", "-I", "src/api/c/include",
         "scripts/tools/fuzz/c/abi.c", C / "build/c-plain/release/libdaegun.a", "-o", ABI_PLAIN], cwd=E)


def fonts():
    roots = [E / "assets/test-fonts", E / "tasks/tools/gate/trt/fonts", Path("/System/Library/Fonts"),
             Path("/Library/Fonts")]
    found = sorted(p for r in roots for p in r.rglob("*") if p.suffix.lower() in {".ttf", ".otf", ".ttc", ".otc"})
    return [p for p in found if p.is_file()]


def framed(choices, text, font):
    data = text.encode()[:4096]
    return bytes(choices) + len(data).to_bytes(2, "big") + data + font


def seeds():
    for kind in ["fonts", "text", "c"]:
        shutil.rmtree(C / "seeds" / kind, ignore_errors=True)
        (C / "seeds" / kind).mkdir(parents=True)
    small = C / "seeds/small"
    shutil.rmtree(small, ignore_errors=True)
    small.mkdir(parents=True)
    every = fonts()
    big = [p for p in every if p.stat().st_size > 256 << 10]
    run([OUT / "release/shrink", small], input="\n".join(map(str, big)), text=True)
    picked = [p for p in every if p.stat().st_size <= 256 << 10] + sorted(small.iterdir())
    # Each font also with the text in its own script: what a script's shaper does is out of reach in any other.
    (C / "seeds/texts").write_bytes(b"".join(len(t).to_bytes(4, "big") + t for t in (t.encode() for t in TEXTS)))
    found = run([OUT / "release/best-text", C / "seeds/texts"], input="\n".join(map(str, picked)), text=True,
                capture_output=True).stdout
    own = {Path(p): int(i) for p, i in (line.rsplit("\t", 1) for line in found.splitlines())}
    n = 0
    for k, path in enumerate(picked):
        font = path.read_bytes()
        if len(font) > FONT_MAX:
            continue
        r = random.Random(k)
        texts = [TEXTS[k % len(TEXTS)], TEXTS[(k + 7) % len(TEXTS)]]
        if own.get(path, -1) >= 0:
            texts.append(TEXTS[own[path]])
        for v, text in enumerate(texts):
            choices = [0] * CHOICES if v == 0 else [r.randrange(256) for _ in range(CHOICES)]
            (C / "seeds/fonts" / f"{k}-{v}").write_bytes(framed(choices, text, font))
            n += 1
    for k, text in enumerate(TEXTS):
        (C / "seeds/text" / f"t{k}").write_bytes(framed([k] * CHOICES, text, b""))
    # The C fuzzer's layout, as `c/run.sh` seeds it: a variation store's offset where a font has one.
    small_c = [p for p in every if p.stat().st_size <= 96 << 10 and p.suffix.lower() != ".ttc"]
    for k, path in enumerate(small_c):
        font = path.read_bytes()
        for v in range(2):
            r = random.Random(k * 2 + v)
            (C / "seeds/c" / f"{k}-{v}").write_bytes(bytes(r.randrange(256) for _ in range(9 * 512)) + font)
    print(f"{n} font seeds from {len(picked)} fonts, {len(TEXTS)} texts, {2 * len(small_c)} C seeds")
    dictionary()


def dictionary():
    tags = set()
    for path in (E / "src").rglob("*.rs"):
        tags |= set(re.findall(r'b"([A-Za-z0-9 /_]{4})"', path.read_text(encoding="utf-8", errors="replace")))
    magics = [b"\x00\x01\x00\x00", b"OTTO", b"true", b"ttcf", b"typ1", b"\x00\x02\x00\x00", b"\xff\xff",
              b"\xff\xff\xff\xff", b"\x00\x00\x00\x00", b"\x7f\xff", b"\x80\x00", b"\x00\x04", b"\x00\x0c",
              b"\x00\x0e", b"\x00\x0d", b"\x00\x06", b"\x00\x02", b"\x00\x03"]
    words = sorted(t.encode() for t in tags) + magics
    lines = ['w%d="%s"' % (i, "".join(f"\\x{b:02x}" if b < 32 or b > 126 or b in b'"\\' else chr(b) for b in w))
             for i, w in enumerate(words)]
    (C / "fonts.dict").write_text("\n".join(lines) + "\n")
    print(f"{len(words)} dictionary words")


# Resident megabytes a fuzzer may reach. The harness counts the Rust targets' heap exactly (`memory.rs`), so theirs
# only backs that, with room for what the allocator holds freed; the C target's is ASan's own measure.
def rss_mb(name):
    return 4096 if name == "c_abi" else 12288


# Instances of a target on its inputs up to 32 KB alone, which run several times as many a second.
SMALL = {"shape_small": "shape", "raster_small": "raster"}


def base(name):
    return SMALL.get(name, name)


def binary(name, asan=False):
    if name == "c_abi":
        return ABI
    return (ASAN if asan else LIBFUZZER) / base(name)


def merge():
    jobs = []
    for name, (timeout, max_len, kind, _) in TARGETS.items():
        if name in FED:
            continue
        corpus = C / "corpus" / name
        corpus.mkdir(parents=True, exist_ok=True)
        fresh = C / "corpus" / f"{name}.merging"
        shutil.rmtree(fresh, ignore_errors=True)
        fresh.mkdir()
        cmd = [binary(name), "-merge=1", f"-timeout={timeout}", f"-max_len={max_len}", f"-rss_limit_mb={rss_mb(name)}",
               fresh, corpus, C / "seeds" / kind]
        log = open(C / "logs" / f"{name}.merge.log", "w")
        jobs.append((name, corpus, fresh, subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT, cwd=C)))
    for name, corpus, fresh, job in jobs:
        job.wait()
        if any(fresh.iterdir()):
            shutil.rmtree(corpus)
            fresh.rename(corpus)
        print(name, sum(1 for _ in corpus.iterdir()), "inputs, exit", job.returncode)


def pid_of(name):
    try:
        pid = int((C / "run" / f"{name}.pid").read_text())
        os.killpg(pid, 0)
        return pid
    except (OSError, ValueError):
        return None


def start(names):
    for d in ["run", "logs", "artifacts", "afl"]:
        (C / d).mkdir(parents=True, exist_ok=True)
    for name in names or list(TARGETS) + ["afl"]:
        if pid_of(name):
            print(name, "already running")
            continue
        log = open(C / "logs" / f"{name}.log", "ab")
        if name == "afl":
            target = os.environ.get("AFL_TARGET", "load")
            timeout, _, _, _ = TARGETS[target]
            out, small = C / "afl" / target, feed(C / "corpus" / target, C / "afl-in" / target, SMALL_MAX)
            cmd = ["cargo", "afl", "fuzz", "-i", small, "-o", out, "-M", "main", "-c", CMPLOG / target, "-x",
                   C / "fonts.dict", "-F", small, "-G", SMALL_MAX, "-t", f"{timeout * 1000}+", "-m", "none", "--",
                   AFL / target]
            env = {"AFL_SKIP_CPUFREQ": "1", "AFL_NO_UI": "1", "AFL_IMPORT_FIRST": "1", "AFL_AUTORESUME": "1"}
        else:
            timeout, max_len, kind, _ = TARGETS[name]
            (C / "artifacts" / name).mkdir(exist_ok=True)
            corpus = C / "corpus" / name
            corpus.mkdir(parents=True, exist_ok=True)
            if name in FED and not any(corpus.iterdir()):
                feed(C / "corpus" / FED[name][0], corpus, FED[name][1])
            # A merged corpus holds what its seeds reach, so a restart doesn't run them all again.
            seeded = [] if any(corpus.iterdir()) else [C / "seeds" / kind]
            cmd = [binary(name), "-fork=1", "-ignore_crashes=1", "-ignore_timeouts=1", "-ignore_ooms=1",
                   "-use_value_profile=1", f"-timeout={timeout}", f"-rss_limit_mb={rss_mb(name)}",
                   "-malloc_limit_mb=2048",
                   f"-max_len={max_len}", f"-dict={C / 'fonts.dict'}", f"-artifact_prefix={C / 'artifacts' / name}/",
                   corpus, *seeded]
            env = {"ASAN_OPTIONS": "detect_leaks=0:allocator_may_return_null=1",
                   "UBSAN_OPTIONS": "print_stacktrace=1:halt_on_error=1"}
        # AFL++ hands every input across to the target and waits: niced, each handoff waits behind the other
        # fuzzers, 17 times slower here, where libFuzzer runs in one process.
        nice = [] if name == "afl" else ["nice", "-n", "10"]
        p = subprocess.Popen([*nice, *map(str, cmd)], stdout=log, stderr=subprocess.STDOUT,
                             start_new_session=True, env={**os.environ, **env}, cwd=C)
        (C / "run" / f"{name}.pid").write_text(str(p.pid))
        print("started", name, p.pid)


def stop(names):
    for name in names or list(TARGETS) + ["afl"]:
        pid = pid_of(name)
        if not pid:
            continue
        os.killpg(pid, signal.SIGTERM)
        for _ in range(50):
            if not pid_of(name):
                break
            time.sleep(0.1)
        else:
            os.killpg(pid, signal.SIGKILL)
        print("stopped", name)


# What each fuzzer reports now, read from the tail of its log or AFL++'s stats.
def snapshot():
    rows = {}
    rss = {}
    for line in subprocess.run(["ps", "-A", "-o", "pgid=,rss="], capture_output=True, text=True).stdout.splitlines():
        g, r = line.split()
        rss[int(g)] = rss.get(int(g), 0) + int(r)
    for name in list(TARGETS) + ["afl"]:
        pid = pid_of(name)
        row = {"alive": bool(pid), "rss": rss.get(pid, 0) // 1024 if pid else 0}
        if name == "afl":
            target = os.environ.get("AFL_TARGET", "load")
            stats = C / "afl" / target / "main/fuzzer_stats"
            if stats.exists():
                s = dict(l.split(":", 1) for l in stats.read_text().splitlines() if ":" in l)
                s = {k.strip(): v.strip() for k, v in s.items()}
                row.update(execs=int(s.get("execs_done", 0)), rate=float(s.get("execs_per_sec", 0)),
                           cov=s.get("bitmap_cvg", "?"), ft="", corp=int(s.get("corpus_count", 0)),
                           crash=int(s.get("saved_crashes", 0)), timeout=int(s.get("saved_hangs", 0)), oom=0,
                           since=int(s.get("time_wo_finds", 0)), name=f"afl:{target}")
        else:
            log = C / "logs" / f"{name}.log"
            if log.exists():
                with open(log, "rb") as f:
                    f.seek(max(0, log.stat().st_size - 65536))
                    tail = f.read().decode(errors="replace").splitlines()
                # The log keeps every run of the target: only the latest's lines, from where libFuzzer starts.
                starts = [i for i, line in enumerate(tail) if line.startswith("INFO: Seed:")]
                hits = [m for m in map(STATUS.match, tail[starts[-1] if starts else 0:]) if m]
                if hits:
                    m = hits[-1]
                    row.update(execs=int(m[1]), cov=int(m[2]), ft=int(m[3]), corp=int(m[4]), rate=int(m[5]),
                               failed=int(m[6]) + int(m[7]) + int(m[8]), time=int(m[9]))
        # Findings by the inputs saved. The status line's split can't be read here: macOS's fork parent takes a job's
        # wait status for its exit code (`FuzzerUtilDarwin.cpp`), so every failed job counts as a crash.
        artifacts = C / "artifacts" / name
        if artifacts.exists():
            pid = C / "run" / f"{name}.pid"
            began = pid.stat().st_mtime if pid.exists() else 0
            found = [(p.name.split("-")[0], p.stat().st_mtime) for p in artifacts.iterdir()]
            for kind in ["crash", "timeout", "oom"]:
                row[kind] = sum(1 for k, _ in found if k == kind)
            # Jobs this run that failed and saved nothing, as a kill from outside leaves; a job failing on an input
            # already saved counts too.
            saved = sum(1 for k, at in found if k in ("crash", "timeout", "oom") and at >= began)
            row["lost"] = max(0, row.get("failed", 0) - saved)
        rows[name] = row
    return rows


def history():
    hist = C / "status.jsonl"
    return [json.loads(l) for l in hist.read_text().splitlines()] if hist.exists() else []


# Minutes since each target's coverage last grew, from the readings `watch` keeps.
def stale(rows, hist):
    out = {}
    for name, row in rows.items():
        if "ft" not in row or name == "afl":
            out[name] = row.get("since", 0) // 60 if name == "afl" else None
            continue
        # Only this run's readings: a restarted target finds its coverage again from the corpus.
        pid = C / "run" / f"{name}.pid"
        began = pid.stat().st_mtime if pid.exists() else 0
        grown = time.time()
        for h in reversed(hist):
            old = h["rows"].get(name, {})
            if h["at"] < began or old.get("ft", row["ft"]) < row["ft"] or old.get("cov", row["cov"]) < row["cov"]:
                break
            grown = h["at"]
        out[name] = int((time.time() - grown) // 60)
    return out


def index():
    path = C / "triage/index.json"
    return json.loads(path.read_text()) if path.exists() else {}


def table(rows):
    hist = history()
    since = stale(rows, hist)
    lines = [f"{'target':<16}{'up':>3} {'execs':>10} {'exec/s':>7} {'cov':>7} {'ft':>8} {'corpus':>7} {'flat':>5}"
             f" {'crash':>6} {'hang':>5} {'oom':>4} {'MB':>6}"]
    for name, r in rows.items():
        flat = "-" if since.get(name) is None else f"{since[name]}m"
        lines.append(f"{r.get('name', name):<16}{'✓' if r['alive'] else '✗':>3} {r.get('execs', 0):>10,} "
                     f"{r.get('rate', 0):>7.0f} {str(r.get('cov', '')):>7} {str(r.get('ft', '')):>8} "
                     f"{r.get('corp', 0):>7,} {flat:>5} {r.get('crash', 0):>6} {r.get('timeout', 0):>5} "
                     f"{r.get('oom', 0):>4} {r['rss']:>6,}")
    return lines


def live():
    while True:
        rows = snapshot()
        idx = index()
        out = ["\x1b[2J\x1b[H\x1b[1mdaegun fuzzing\x1b[0m  " + time.strftime("%H:%M:%S") + f"  {C}", ""]
        out += table(rows)
        total = sum(r.get("execs", 0) for r in rows.values())
        out += ["", f"{total:,} inputs run; {len(idx)} unique findings, "
                    f"{sum(1 for v in idx.values() if v['status'] == 'open')} open", ""]
        recent = sorted((p for p in (C / "artifacts").rglob("*") if p.is_file()), key=lambda p: p.stat().st_mtime)[-8:]
        out += ["latest artifacts:"] + [f"  {time.strftime('%H:%M', time.localtime(p.stat().st_mtime))} "
                                        f"{p.parent.name:<11} {p.name[:60]}" for p in reversed(recent)]
        out += ["", "findings:"] + [f"  [{v['status']}] {v['target']:<10} {v['kind']:<8} x{v['count']:<4} {sig[:80]}"
                                    for sig, v in sorted(idx.items(), key=lambda kv: -kv[1]["last"])[:10]]
        print("\n".join(out), flush=True)
        time.sleep(2)


# The inputs in `source` up to `limit` bytes copied into `into`, where they aren't yet; `into`.
def feed(source, into, limit):
    into.mkdir(parents=True, exist_ok=True)
    for p in source.iterdir():
        if p.stat().st_size <= limit and not (into / p.name).exists():
            shutil.copy(p, into / p.name)
    return into


# Inputs handed across hourly: libFuzzer's new small ones to AFL++ and the small instances, theirs to the target
# they came from; libFuzzer reads a corpus only as it starts, so its own get them at their next start.
def sync():
    for name, (source, limit) in FED.items():
        if name in SMALL:
            feed(C / "corpus" / source, C / "corpus" / name, limit)
            feed(C / "corpus" / name, C / "corpus" / source, FONT_MAX + 4096)
    for queue in (C / "afl").glob("*/main/queue"):
        name = queue.parents[1].name
        feed(C / "corpus" / name, C / "afl-in" / name, SMALL_MAX)
        added = 0
        for p in queue.glob("id*"):
            to = C / "corpus" / name / hashlib.sha1(p.read_bytes()).hexdigest()
            if not to.exists():
                shutil.copy(p, to)
                added += 1
        if added and pid_of(name):
            stop([name])
            start([name])
        if added:
            print(f"[sync] {name} +{added} from afl", flush=True)


# A status line every 10 minutes, a line the moment a crash, timeout or memory hit lands or a fuzzer stops, and the
# engines' inputs handed across hourly, for Monitor.
def watch():
    seen_file = C / "watch-seen.txt"
    seen = set(seen_file.read_text().splitlines()) if seen_file.exists() else {
        str(p) for p in (C / "artifacts").rglob("*") if p.is_file()}
    down = set()
    # Kept in a file, as Monitor restarts the watch every half hour.
    synced = C / "run/afl-synced"
    synced.touch(exist_ok=True)
    last, slow = 0.0, 0
    while True:
        files = {str(p) for p in (C / "artifacts").rglob("*") if p.is_file()}
        for afl_dir in (C / "afl").glob("*/main"):
            files |= {str(p) for d in ["crashes", "hangs"] for p in (afl_dir / d).glob("id*")}
        for new in map(Path, sorted(files - seen)):
            # Slow units, and AFL++'s hangs at its own two seconds, are nearly all the tracing build's slowness:
            # counted, to triage with the status.
            if new.name.startswith("slow-unit") or new.parent.name == "hangs":
                slow += 1
            else:
                print(f"[finding] {new.parent.name} {new.name}", flush=True)
        if files - seen:
            seen |= files
            seen_file.write_text("\n".join(sorted(seen)))
        for name in list(TARGETS) + ["afl"]:
            alive = (C / "run" / f"{name}.pid").exists() and pid_of(name)
            if (C / "run" / f"{name}.pid").exists() and not alive and name not in down:
                print(f"[down] {name}", flush=True)
                down.add(name)
            elif alive:
                down.discard(name)
        if time.time() - last >= 600:
            last = time.time()
            rows = snapshot()
            with open(C / "status.jsonl", "a") as f:
                f.write(json.dumps({"at": last, "rows": rows}) + "\n")
            since = stale(rows, history())
            parts = [f"{n}:{r.get('cov', '?')}/{r.get('crash', 0)}c{r.get('timeout', 0)}h{r.get('oom', 0)}o"
                     f"/{since.get(n)}m" for n, r in rows.items() if r["alive"] or "execs" in r]
            idx = index()
            lost = " ".join(f"{n}:{r['lost']}" for n, r in rows.items() if r.get("lost"))
            print(f"[status] {time.strftime('%H:%M')} " + " ".join(parts) +
                  f" | open {sum(1 for v in idx.values() if v['status'] == 'open')} | slow +{slow}" +
                  (f" | lost {lost}" if lost else ""), flush=True)
            slow = 0
        if time.time() - synced.stat().st_mtime >= 3600:
            synced.touch()
            sync()
        time.sleep(5)


def status():
    print("\n".join(table(snapshot())))


# A finding's signature: a panic's place and message, or the first frames in daegun of a stack or a sample.
def signature(kind, report):
    panic = re.search(r"panicked at (\S+?):(\d+):\d+:\n(.*)", report)
    if panic:
        message = re.sub(r"\d+", "N", panic[3])[:100]
        return f"panic {panic[1].split('/src/')[-1]}:{panic[2]} {message}"
    frames = re.findall(r"#\d+ 0x[0-9a-f]+ in (\S+)", report) + re.findall(r"^\s+(\S*daegun\S*)\s+\(in ", report, re.M)
    ours = [re.sub(r"::h[0-9a-f]{16}$", "", f) for f in frames if own(f) and "fuzz" not in f][:3]
    return f"{kind} " + (" < ".join(ours) if ours else (re.search(r"ERROR: (.*)", report) or ["", "?"])[1][:100])


# Whether a frame is daegun's own function rather than a library one daegun's types fill in: the first crate a
# mangled name names, or the path a demangled one starts with.
def own(frame):
    first = re.search(r"C(?:s[0-9A-Za-z]+_)?\d+([a-z_][a-z0-9_]*)", frame) if frame.startswith("_R") else None
    return (first[1] == "daegun") if first else frame.startswith("daegun::")


# The heaviest daegun frames of a running process, sampled for three seconds.
def sampled(pid):
    out = C / "triage/work/sample.txt"
    subprocess.run(["sample", str(pid), "3", "-file", out], capture_output=True)
    text = out.read_text(errors="replace") if out.exists() else ""
    tops = text.split("Sort by top of stack")[-1].splitlines()
    return "\n".join(l for l in tops if own(l.strip().split()[0] if l.strip() else "") and "fuzz" not in l)[:4000]


# An input through the plain build: its exit, seconds, peak bytes and output, or a sample where it ran past `limit`.
def plain(name, case, limit):
    cmd = [ABI_PLAIN, "-rss_limit_mb=0", case] if name == "c_abi" else [RUN, base(name), case]
    env = {**os.environ, "RUST_BACKTRACE": "1"}
    start = time.time()
    p = subprocess.Popen(["/usr/bin/time", "-l", *map(str, cmd)], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         env=env, cwd=C / "triage/work", start_new_session=True)
    try:
        out, err = p.communicate(timeout=limit)
    except subprocess.TimeoutExpired:
        child = subprocess.run(["pgrep", "-P", str(p.pid)], capture_output=True, text=True).stdout.split()
        frames = sampled(int(child[0])) if child else ""
        os.killpg(p.pid, signal.SIGKILL)
        p.communicate()
        return None, time.time() - start, 0, frames
    text = out.decode(errors="replace") + err.decode(errors="replace")
    peak = int((re.search(r"(\d+)\s+maximum resident set size", text) or [0, 0])[1])
    return p.returncode, time.time() - start, peak, text


def triage(paths):
    symbolizer = clang().parent / "llvm-symbolizer"
    idx = index()
    done = {f for v in idx.values() for f in v["files"]}
    found = [p for p in (C / "artifacts").rglob("*") if p.is_file()]
    found += [p for d in ["crashes", "hangs"] for p in (C / "afl").glob(f"*/main/{d}/id*")]
    todo = [Path(p) for p in paths] or sorted(p for p in found if str(p) not in done)
    (C / "triage/work").mkdir(parents=True, exist_ok=True)
    for path in todo:
        afl = path.parent.name in ("crashes", "hangs")
        name = path.parents[2].name if afl else path.parent.name
        kind = {"crashes": "crash", "hangs": "timeout"}[path.parent.name] if afl else path.name.split("-")[0]
        timeout, max_len, _, limit = TARGETS[name]
        case = path
        if kind == "crash" and not afl:
            smallest = C / "triage/work" / f"{path.name}.min"
            # libFuzzer shrinks in rounds without end while each finds a smaller crash: two minutes in all.
            p = subprocess.Popen([binary(name, asan=True), "-minimize_crash=1", "-max_total_time=30",
                                  f"-timeout={timeout}", f"-exact_artifact_path={smallest}", path],
                                 stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, cwd=C / "triage/work",
                                 env={**os.environ, "ASAN_OPTIONS": "detect_leaks=0"}, start_new_session=True)
            try:
                p.wait(timeout=120)
            except subprocess.TimeoutExpired:
                os.killpg(p.pid, signal.SIGKILL)
                p.wait()
            case = smallest if smallest.exists() and smallest.stat().st_size else path
        status, seconds, peak, report = plain(name, case, limit * 6)
        if kind == "crash" and status == 0 and case != path:
            # A shrinking cut short can leave an input that no longer crashes: the one found, then.
            case = path
            status, seconds, peak, report = plain(name, case, limit * 6)
        if kind == "crash":
            if status in (0, None) or "panicked at" not in report:
                # Not in the plain build, or no panic there to place it, as an abort or a stack overflow leaves: the
                # instrumented build's own report, with a stack.
                env = {**os.environ, "ASAN_OPTIONS": "detect_leaks=0:symbolize=1",
                       "ASAN_SYMBOLIZER_PATH": str(symbolizer)}
                done_run = subprocess.run([binary(name, asan=True), f"-timeout={timeout}",
                                           f"-rss_limit_mb={rss_mb(name)}", case], capture_output=True, text=True,
                                          errors="replace", env=env, cwd=C / "triage/work")
                traced = done_run.stdout + done_run.stderr
                if "ERROR" in traced or "panicked" in traced:
                    report = traced
                elif status in (0, None):
                    kind = "flaky"
            verdict = "open"
        elif status is None or seconds > limit:
            kind, verdict = "hang" if status is None else "slow", "open"
            if status is not None:
                _, _, _, report = plain(name, case, max(2, int(seconds / 2)))
        elif peak > 4 << 30:
            kind, verdict = "memory", "open"
        else:
            # Fast enough uninstrumented: the tracing build's own slowness, kept for the record.
            kind, verdict = "slow-instrumented", "instrumentation"
            report = f"plain build: {seconds:.2f} s, {peak >> 20} MB\n" + report[-2000:]
        # A flaky crash has no place to sign it by, so it is told apart by its target, as the slow ones are.
        sig = signature(kind, report) if verdict == "open" and kind != "flaky" else f"{kind} {name}"
        entry = idx.setdefault(sig, {"id": hashlib.sha1(sig.encode()).hexdigest()[:10], "target": name, "kind": kind,
                                     "count": 0, "first": time.time(), "files": [], "status": verdict})
        if entry["status"] == "fixed":
            # Found again on a build with its fix.
            entry["status"] = "open"
        entry["count"] += 1
        entry["last"] = time.time()
        entry["files"].append(str(path))
        keep = C / "triage" / entry["id"]
        keep.mkdir(parents=True, exist_ok=True)
        if not (keep / "input").exists() or case.stat().st_size < (keep / "input").stat().st_size:
            shutil.copy(case, keep / "input")
            (keep / "report.txt").write_text(f"{sig}\n{path}\nplain: exit {status}, {seconds:.2f} s, "
                                             f"{peak >> 20} MB\n\n{report[-20000:]}")
        print(f"{entry['id']} {name:<10} {kind:<18} {verdict:<15} {seconds:6.2f}s {sig[:90]}")
        (C / "triage/index.json").write_text(json.dumps(idx, indent=1))


# Findings whose fix every running build now holds, by id.
def fixed(ids):
    idx = index()
    for v in idx.values():
        if v["id"] in ids:
            v["status"] = "fixed"
            print(v["id"], v["target"], "fixed")
    (C / "triage/index.json").write_text(json.dumps(idx, indent=1))


# What the corpora reach of the engine, every Rust target's at once: per folder, least first, and each function no
# input ran, where a seed or a call is missing. c_abi's own run proves each line of abi.c reached.
def coverage(names):
    tools = clang().parent
    names = names or [n for n in TARGETS if n != "c_abi" and n == base(n)]
    # A target fed from another holds nothing until it first runs.
    names = [n for n in names if any((C / "corpus" / n).glob("*"))]
    for name in names:
        # cargo-fuzz runs the corpus, then merges with the toolchain's llvm-tools, which this one doesn't have:
        # the raw profiles are merged here with the same LLVM's, then moved beside the campaign's other data.
        run(["cargo", "+nightly", "fuzz", "coverage", "--fuzz-dir", ".", "--target-dir", C / "build/coverage", name,
             C / "corpus" / name], cwd=FUZZ / "libfuzzer", check=False)
        made, kept = FUZZ / "libfuzzer/coverage" / name, C / "coverage" / name
        shutil.rmtree(kept, ignore_errors=True)
        kept.parent.mkdir(exist_ok=True)
        shutil.move(made, kept)
        run([tools / "llvm-profdata", "merge", "-sparse", *sorted((kept / "raw").glob("*.profraw")), "-o",
             kept / "coverage.profdata"])
    shutil.rmtree(FUZZ / "libfuzzer/coverage", ignore_errors=True)
    report(names)


# The union of the targets' profiles `coverage` collected, as `coverage/report.txt`.
def report(names):
    tools = clang().parent
    profiles = [C / "coverage" / name / "coverage.profdata" for name in names]
    objects = [C / "build/coverage" / HOST / "release" / name for name in names]
    merged = C / "coverage/all.profdata"
    run([tools / "llvm-profdata", "merge", "-sparse", *profiles, "-o", merged])
    args = [objects[0], *[a for o in objects[1:] for a in ("-object", o)], f"-instr-profile={merged}",
            "--ignore-filename-regex", r"(\.cargo|rustc|scripts/tools)"]
    summary = run([tools / "llvm-cov", "report", *args], capture_output=True, text=True).stdout
    folders = {}
    for line in summary.splitlines():
        cols = line.split()
        if len(cols) > 10 and "/src/" in cols[0]:
            folder = "/".join(cols[0].split("/src/", 1)[1].split("/")[:4])
            lines, missed = int(cols[7]), int(cols[8])
            a, b = folders.get(folder, (0, 0))
            folders[folder] = (a + lines, b + missed)
    out = [f"{100 * (l - m) / max(l, 1):5.1f}%  {l - m:>7}/{l:<7} {f}" for f, (l, m) in
           sorted(folders.items(), key=lambda kv: (kv[1][0] - kv[1][1]) / max(kv[1][0], 1))]
    lcov = run([tools / "llvm-cov", "export", "-format=lcov", *args], capture_output=True, text=True).stdout
    unrun, source = [], ""
    for line in lcov.splitlines():
        if line.startswith("SF:"):
            source = line[3:].split("/src/", 1)[-1]
        elif line.startswith("FNDA:0,"):
            unrun.append((source, line[7:]))
    demangled = run([tools / "llvm-cxxfilt"], input="\n".join(n for _, n in unrun), capture_output=True,
                    text=True).stdout.splitlines()
    listed = sorted({f"{f}: {re.sub(r'::h[0-9a-f]{16}$', '', n)}" for (f, _), n in zip(unrun, demangled)})
    (C / "coverage/report.txt").write_text(summary + "\n\nBy folder, least reached first:\n" + "\n".join(out) +
                                          f"\n\nFunctions no input ran ({len(listed)}):\n" + "\n".join(listed))
    print("\n".join(out), f"\n{len(listed)} functions no input ran; all in {C / 'coverage/report.txt'}")


# Every target over its corpus and its findings once, which a fix must leave clean.
def replay():
    failed = []
    for name, (timeout, max_len, kind, _) in TARGETS.items():
        inputs = [C / "corpus" / name] + [C / "triage" / v["id"] / "input" for v in index().values()
                                          if v["target"] == name]
        done = subprocess.run([binary(name), "-runs=0", f"-timeout={timeout}", f"-max_len={max_len}",
                               f"-rss_limit_mb={rss_mb(name)}", *[p for p in inputs if p.exists()]],
                              capture_output=True, text=True, errors="replace")
        ok = done.returncode == 0
        print(f"{name:<11} {'clean' if ok else 'FAILED'}")
        if not ok:
            failed.append(name)
            print("\n".join(done.stderr.splitlines()[-15:]))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    command, rest = (sys.argv[1], sys.argv[2:]) if len(sys.argv) > 1 else ("status", [])
    {"build": lambda: build(), "seeds": lambda: seeds(), "merge": lambda: merge(), "start": lambda: start(rest),
     "stop": lambda: stop(rest), "live": lambda: live(), "watch": lambda: watch(), "status": lambda: status(),
     "triage": lambda: triage(rest), "fixed": lambda: fixed(rest), "coverage": lambda: coverage(rest),
     "replay": lambda: replay()}[command]()
