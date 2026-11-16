#!/usr/bin/env python3
# Every public method and function of the Rust API, as rust-surface.txt records it, named in the Rust fuzzers'
# harness, and every C call in the C one, so a new call can't go unfuzzed.
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[3]
# Its bytes must live forever, so a fuzzer would leak each input; from_vec shares its reader, and C's twin runs.
NOT_FUZZED = {"daegun::Font::from_static"}


# (path, pattern) for each public method and function: a method named after a dot or its type, a function called
# or named through its module.
def rust_calls():
    calls = []
    for line in (ROOT / "scripts/tools/rust-api/rust-surface.txt").read_text(encoding="utf-8").splitlines():
        kind, path = line.split(" ", 2)[:2]
        if kind not in ("method", "function") or path in NOT_FUZZED:
            continue
        *_, owner, name = path.split("::")
        pattern = rf"(\.{name}\b|\b{owner}::{name}\b)" if kind == "method" else rf"((?<![.\w]){name}\s*\(|::{name}\b)"
        calls.append((path, pattern))
    return calls


def c_calls():
    calls = set()
    for path in (ROOT / "src/api/c/include/daegun").glob("*.h"):
        text = re.sub(r"/\*.*?\*/", "", path.read_text(encoding="utf-8"), flags=re.S)
        calls |= set(re.findall(r"\bDAEGUN_CALL\s+(daegun_\w+)\s*\(", text))
    return calls


rust = rust_calls()
c = c_calls()
if len(rust) < 200 or not c:
    sys.exit(f"read {len(rust)} Rust and {len(c)} C calls: the parsers have gone stale")
fuzz = "\n".join(p.read_text(encoding="utf-8") for p in sorted((ROOT / "scripts/tools/fuzz/harness/src").glob("*.rs")))
abi = (ROOT / "scripts/tools/fuzz/c/abi.c").read_text(encoding="utf-8")
missing = [path for path, pattern in rust if not re.search(pattern, fuzz)]
missing += [call for call in sorted(c) if not re.search(rf"\b{call}\b", abi)]
if missing:
    sys.exit("no fuzzer reaches: " + ", ".join(missing))
print(f"{len(rust)} Rust and {len(c)} C calls")
