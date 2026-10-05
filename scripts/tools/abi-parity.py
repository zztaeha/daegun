#!/usr/bin/env python3
USAGE = """    abi-parity.py                    check the newest daegun library cargo built for each platform
    abi-parity.py <lib> [<lib>...]   check the libraries named instead

What daegun.h promises against what the library actually exports, in both directions: a declaration
with no symbol behind it is a link error waiting for the first caller, and an exported symbol with
no declaration is unreachable from C. And every integer constant in the header against the Rust
value it copies, which no linker compares. Exit status is 1 on any difference, and for a library
that is missing or that nm cannot read.

Names only – a library carries no C types. Signature agreement is what compiling roundtrip.c
against the header proves; this cannot.

c-parity.sh asks a different question – whether the Rust API is reachable from C at all – and
answers it from names in the source. This one answers from the linker's own table.

Build what you want checked first:
    cargo rustc --features capi --crate-type staticlib
    cargo rustc --target aarch64-pc-windows-msvc --features capi --crate-type staticlib"""


import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
HEADER = os.path.join(ROOT, "src", "c-wrapper", "daegun.h")

DECL = re.compile(r"^(?:const\s+)?[A-Za-z_][A-Za-z0-9_ *]*\b(daegun_[a-z_0-9]+)\s*\(")
NAME = re.compile(r"\b(daegun_[a-z_0-9]+)\s*\(")


def declared(lines):
    # The header's conditionals are its include guard, the C++ linkage block and the C11 asserts. Any
    # other could gate declarations by platform, so the count would depend on who compiles it.
    known = {"ifndef DAEGUN_H", "ifdef __cplusplus",
             "if defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L"}
    for n, line in enumerate(lines):
        hit = re.match(r"\s*#\s*((?:if|ifdef|ifndef|elif)\b.*?)\s*$", line)
        if hit and re.sub(r"\s+", " ", hit.group(1)) not in known:
            sys.exit(f"daegun.h:{n + 1} has a conditional declared() does not handle: {hit.group(1)}")
    found = {}
    for i, line in enumerate(lines):
        if line.lstrip().startswith(("#", "*", "/")) or not DECL.match(line):
            continue
        # A declaration may wrap over several lines; join until the semicolon closes it.
        joined, j = line, i
        while ";" not in joined and j + 1 < len(lines):
            j += 1
            joined += " " + lines[j].strip()
        hit = NAME.search(joined)
        if hit:
            found[hit.group(1)] = i + 1
    return found


CORE = ("src", "daecore", "src")


def read(*parts):
    return open(os.path.join(ROOT, *parts), encoding="utf-8").read()


def snake(camel):
    return re.sub(r"(?<!^)(?=[A-Z])", "_", camel).upper()


def enum_values(text, name, prefix=""):
    body = re.search(r"pub enum " + name + r" \{(.*?)\n\}", text, re.S).group(1)
    values, at = {}, 0
    for variant, explicit in re.findall(r"^\s*(\w+)(?:\s*=\s*(-?\d+))?,", body, re.M):
        at = int(explicit) if explicit else at
        values[prefix + snake(variant)] = at
        at += 1
    return values


def rust_constants():
    # Every source a header constant can take its value from: the wrapper's own constants, the
    # enums cast with `as`, the MATH match, AAT's codes and daecore's limits under their API names.
    found = {}
    for name in sorted(os.listdir(os.path.join(ROOT, "src", "c-wrapper"))):
        if name.endswith(".rs"):
            text = read("src", "c-wrapper", name)
            pattern = r"pub(?:\(crate\))? const ([A-Z0-9_]+): [iu]\d+ = (-?\d+);"
            for c, v in re.findall(pattern, text):
                found[c] = int(v)
    found.update(enum_values(read("src", "c-wrapper", "handle.rs"), "Status"))
    categories = read(*CORE, "daeshaper", "unicode", "mod.rs")
    found.update(enum_values(categories, "GeneralCategory", "GC_"))
    found.update(enum_values(read(*CORE, "daetype", "outline", "path.rs"), "Verb", "VERB_"))
    math = re.search(r"fn daegun_font_math_constant\(.*?\n\}", read("src", "c-wrapper", "tables.rs"), re.S)
    for i, field in re.findall(r"(\d+) => (\w+),", math.group(0)):
        found["MATH_" + field.upper()] = int(i)
    aat = read(*CORE, "daetype", "format", "aat.rs")
    for mod, body in re.findall(r"pub mod (\w+) \{(.*?)\}", aat, re.S):
        for c, v in re.findall(r"pub const (\w+): u16 = (\d+);", body):
            found["AAT_" + mod.upper() + "_" + c] = int(v)
    limits = {}
    for parts in (("daetype", "outline", "quadratic.rs"), ("daetype", "outline", "simplify.rs"),
                  ("daetype", "outline", "flatten.rs"), ("daetype", "hinting", "state.rs"),
                  ("daemachine", "subpixel", "mod.rs")):
        limits.update(re.findall(r"pub const (\w+): \w+ = ([^;]+);", read(*CORE, *parts)))
    values = {}
    for c, expr in limits.items():
        expr = re.sub(r"\bas \w+", "", expr)
        for known, v in values.items():
            expr = re.sub(r"\b" + known + r"\b", str(v), expr)
        expr = re.sub(r"(?<=[0-9a-fA-F])_(?=[0-9a-fA-F])", "", expr)
        if re.fullmatch(r"[0-9x a-fA-F+*()-]+", expr):
            values[c] = int(eval(expr))
    for real, alias in re.findall(r"(\w+) as (MAX_\w+)", read("src", "daegun", "api", "mod.rs")):
        values[alias] = values[real]
    found.update(values)
    return found


def constants():
    # The header's integer constants are copies C compiles against: one that drifts from its Rust
    # source misreads every call that uses it, and no linker notices.
    header = read("src", "c-wrapper", "daegun.h")
    rust = rust_constants()
    problems, n = [], 0
    # Every define with a value is read. One that is not a plain integer cannot be compared, so it fails
    # rather than slipping by; the ABI version is the exception, checked against the library at run time.
    pattern = r"^[ \t]*#[ \t]*define[ \t]+DAEGUN_([A-Z0-9_]+)[ \t]+(\S+)"
    for m in re.finditer(pattern, header, re.M):
        name, value = m.groups()
        line = header.count("\n", 0, m.start()) + 1
        if name == "ABI_VERSION":
            continue
        n += 1
        if not re.fullmatch(r"-?(?:0x[0-9a-fA-F]+|\d+)", value):
            problems.append(f"        UNREAD  DAEGUN_{name} is {value} in daegun.h:{line}, not a plain integer")
        elif name not in rust:
            problems.append(f"        NO RUST SOURCE  DAEGUN_{name}  (daegun.h:{line})")
        elif rust[name] != int(value, 0):
            problems.append(f"        DIFFERS  DAEGUN_{name} is {value} in daegun.h:{line}, "
                            f"{rust[name]} in Rust")
    verdict = " – all equal to their Rust source" if not problems else ""
    print(f"constants  header defines {n}{verdict}")
    for p in problems:
        print(p)
    print()
    return len(problems)


def llvm_nm():
    try:
        sysroot = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True,
                                 check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None
    rustlib = os.path.join(sysroot, "lib", "rustlib")
    if not os.path.isdir(rustlib):
        return None
    for host in sorted(os.listdir(rustlib)):
        found = os.path.join(rustlib, host, "bin", "llvm-nm")
        if os.path.isfile(found):
            return found
    return None


class Unreadable(Exception):
    pass


def exported(path):
    # llvm-nm reads Mach-O, ELF and COFF alike, so a Windows .lib cross-built on a Mac is still
    # readable. The system nm is the fallback for everything but COFF.
    tool = llvm_nm()
    if tool:
        argv = [tool, "--defined-only", path]
    elif path.endswith(".lib"):
        return None
    elif sys.platform == "darwin":
        argv = ["nm", "-gU", path]
    elif path.endswith(".so"):
        argv = ["nm", "-D", "--defined-only", path]
    else:
        # -D reads only the dynamic table, which a static archive does not have.
        argv = ["nm", "--defined-only", path]

    run = subprocess.run(argv, capture_output=True, text=True)
    if run.returncode != 0:
        lines = run.stderr.strip().splitlines()
        raise Unreadable(lines[0] if lines else f"{argv[0]} exited {run.returncode}")
    names = set()
    for line in run.stdout.splitlines():
        parts = line.split()
        if len(parts) < 2 or parts[-2] in ("U", "u", "w"):
            continue
        hit = re.fullmatch(r"_?(daegun_[a-z_0-9]+)", parts[-1])
        if hit:
            names.add(hit.group(1))
    return names


def shown(path):
    rel = os.path.relpath(path, ROOT)
    return path if rel.startswith("..") else rel


def platform_of(path):
    for part in path.split(os.sep):
        if "-windows-" in part:
            return "win32"
        if "-apple-" in part:
            return "apple"
        if "-linux-" in part:
            return "linux"
    if path.endswith(".lib"):
        return "win32"
    return {"darwin": "apple", "win32": "win32"}.get(sys.platform, "linux")


# Where cargo builds, which CARGO_TARGET_DIR or a config can move away from ./target.
def target_dir():
    try:
        out = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT,
                             capture_output=True, text=True, check=True).stdout
        return json.loads(out)["target_directory"]
    except (OSError, subprocess.CalledProcessError, ValueError, KeyError):
        return os.path.join(ROOT, "target")


def libraries():
    target = target_dir()
    names = ("libdaegun.a", "libdaegun.dylib", "libdaegun.so", "daegun.lib")
    found = []
    for profile_dir, _, files in os.walk(target):
        if os.path.basename(profile_dir) not in ("debug", "release"):
            continue
        for name in names:
            if name in files:
                found.append(os.path.join(profile_dir, name))
    # Newest first, so a stale debug build never gets checked in place of the release one beside it.
    return sorted(found, key=os.path.getmtime, reverse=True)


def main(argv):
    if "-h" in argv or "--help" in argv:
        print(USAGE)
        return 0

    constant_differences = constants()
    libs = argv or libraries()
    # No library read is no parity shown, so it fails rather than passing on the constants alone.
    if not libs:
        print("nothing built (run: cargo rustc --features capi --crate-type staticlib)")
        return 1

    header = declared(open(HEADER, encoding="utf-8").read().split("\n"))
    differences, checked, missing, seen = constant_differences, 0, 0, set()

    # The newest library a platform has stands for it, but every library named is checked.
    for lib in libs:
        platform = platform_of(lib)
        if not argv and platform in seen:
            continue
        if not os.path.isfile(lib):
            print(f"{platform:7} {lib}\n        skipped: no such file\n")
            missing += 1
            continue
        try:
            symbols = exported(lib)
        except Unreadable as e:
            print(f"{platform:7} {shown(lib)}\n        unreadable: {e}\n")
            missing += 1
            continue
        if symbols is None:
            print(f"{platform:7} {shown(lib)}\n"
                  f"        skipped: no llvm-nm to read a COFF archive with\n")
            continue
        if not symbols:
            print(f"{platform:7} {shown(lib)}\n"
                  f"        skipped: no daegun symbols, so it was built without --features capi\n")
            continue

        seen.add(platform)
        checked += 1
        unbuilt = sorted(n for n in header if n not in symbols)
        undeclared = sorted(n for n in symbols if n not in header)

        print(f"{platform:7} {shown(lib)}")
        print(f"        header declares {len(header)}, library exports {len(symbols)}", end="")
        if not unbuilt and not undeclared:
            print(" – matched")
        else:
            print()
            differences += len(unbuilt) + len(undeclared)
        for name in unbuilt:
            print(f"        DECLARED, NEVER EXPORTED  {name}  (daegun.h:{header[name]})")
        for name in undeclared:
            print(f"        EXPORTED, NEVER DECLARED  {name}")
        print()

    if not checked:
        print("nothing checked")
        return 1
    print(f"{checked} librar{'y' if checked == 1 else 'ies'} checked, "
          f"{differences or 'no'} difference{'' if differences == 1 else 's'}.")
    return 1 if differences or missing else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
