#!/usr/bin/env python3
USAGE = """    abi-parity.py                    check the newest daegun library cargo built for each platform
    abi-parity.py <lib> [<lib>...]   check the libraries named instead
    abi-parity.py --record           record the header as the released ABI, once its version is final

What the C headers promise against what the library actually exports, in both directions: a declaration
with no symbol behind it is a link error waiting for the first caller, and an exported symbol with
no declaration is unreachable from C. And every integer constant in the header against the Rust
value it copies, which no linker compares. And the header against the last released ABI
(abi-released.txt): a declaration changed or removed since needs a new major, one added a new minor,
or a caller following the header's version rule would take a library its structs don't fit. Exit
status is 1 on any difference, and for a library that is missing or that nm cannot read.

Names only – a library carries no C types. Signature agreement is what compiling roundtrip.c
against the headers proves; this cannot.

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
ROOT = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
NAME = re.compile(r"\b(daegun_[a-z_0-9]+)\s*\(")


def headers():
    # Every header daegun.h includes, in its order, as its path from include/ and its text.
    umbrella = read("src", "api", "c", "include", "daegun.h")
    names = re.findall(r'^#include <(daegun/\w+\.h)>', umbrella, re.M)
    return [(name, read("src", "api", "c", "include", name)) for name in names]


def blank(m):
    # A stretch taken out, its newlines kept so what follows keeps its line.
    return re.sub(r"[^\n]", " ", m.group(0))


def statements(text):
    # Each file-level declaration, typedef and struct, spaces collapsed, with its line. Comments, C++ linkage blocks,
    # directives, layout asserts and the marks that change no binary are left out.
    text = re.sub(r"/\*.*?\*/", blank, text, flags=re.S)
    text = re.sub(rf"\bDAEGUN_(?:{'|'.join(NEUTRAL)})\b", blank, text)
    text = re.sub(r"\bDAEGUN_(?:READS|WRITES|COUNTED_BY|LENT|IN|OUT(?:_OR_NULL)?)\((?:[^()]|\([^()]*\))*\)",
                  blank, text)
    text = re.sub(r"^[ \t]*#[ \t]*ifdef[ \t]+__cplusplus.*?^[ \t]*#[ \t]*endif.*?$", blank, text, flags=re.S | re.M)
    text = re.sub(r"^[ \t]*#(?:.*\\\n)*.*$", blank, text, flags=re.M)
    text = re.sub(r"\bDAEGUN_LAYOUT_ASSERT\(.*?\);", blank, text, flags=re.S)
    depth, start = 0, 0
    for i, ch in enumerate(text):
        if ch in "{}":
            depth += 1 if ch == "{" else -1
        elif ch == ";" and depth == 0:
            chunk = text[start:i]
            at = start + len(chunk) - len(chunk.lstrip())
            # Spacing as a mark taken out leaves none: `T *x`, never `T * x`, and nothing before `;`, `,` or `)`.
            decl = re.sub(r"\*\s+", "*", " ".join(chunk.split()))
            decl = re.sub(r"\s+([;,)\]])", r"\1", re.sub(r"([(\[])\s+", r"\1", decl))
            yield decl, text.count("\n", 0, at) + 1
            start = i + 1


def declared(files):
    # Each function a header declares, with where. One under a conditional past its include guard would be
    # declared for some callers and not others, so the count would depend on who compiles it.
    found = {}
    for path, text in files:
        depth, depths = 0, []
        for line in text.split("\n"):
            depths.append(depth)
            if re.match(r"\s*#\s*if", line):
                depth += 1
            elif re.match(r"\s*#\s*endif", line):
                depth -= 1
        for decl, line in statements(text):
            hit = NAME.search(decl)
            if decl.startswith("typedef") or not hit:
                continue
            if depths[line - 1] != 1:
                sys.exit(f"{path}:{line} declares {hit.group(1)} under a conditional, so not every caller sees it")
            found[hit.group(1)] = f"{path}:{line}"
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
    for name in sorted(os.listdir(os.path.join(ROOT, "src", "api", "c", "translation"))):
        if name.endswith(".rs"):
            text = read("src", "api", "c", "translation", name)
            pattern = r"pub(?:\(crate\))? const ([A-Z0-9_]+): [iu]\d+ = (-?\d+);"
            for c, v in re.findall(pattern, text):
                found[c] = int(v)
    found.update(enum_values(read("src", "api", "c", "translation", "handle.rs"), "Status"))
    categories = read(*CORE, "daeshaper", "generated", "unicode_tables.rs")
    found.update(enum_values(categories, "GeneralCategory", "GC_"))
    found.update(enum_values(categories, "VerticalOrientation", "VO_"))
    found.update(enum_values(read(*CORE, "daeshaper", "unicode", "scripts.rs"), "ScriptDirection", "SCRIPT_DIRECTION_"))
    found.update(enum_values(read(*CORE, "daeshaper", "script", "mod.rs"), "Shaper", "SHAPER_"))
    found.update(enum_values(read("src", "api", "rust", "outline.rs"), "PointKind", "POINT_"))
    found.update(enum_values(read(*CORE, "daetype", "outline", "path.rs"), "Verb", "VERB_"))
    found.update(enum_values(read(*CORE, "daekit", "draw", "mod.rs"), "Blend", "COMPOSITE_"))
    aat = read(*CORE, "daetype", "format", "aat.rs")
    for mod, body in re.findall(r"pub mod (\w+) \{(.*?)\}", aat, re.S):
        for c, v in re.findall(r"pub const (\w+): u16 = (\d+);", body):
            found["AAT_" + mod.upper() + "_" + c] = int(v)
    metrics = read(*CORE, "daekit", "cache", "metrics.rs")
    for c, v in re.findall(r"pub const (\w+): PdfFlags = PdfFlags\((\w+)\);", metrics):
        found["PDF_" + c] = int(v, 0)
    # Private to fvar's reader, as `Axis` gives it as `hidden`; C hands out each axis's flags as stored.
    hidden = re.search(r"const AXIS_HIDDEN: u16 = (\w+);", read(*CORE, "daetype", "decoder", "fvar.rs"))
    found["AXIS_HIDDEN"] = int(hidden.group(1), 0)
    limits = {}
    for parts in (("daetype", "outline", "quadratic.rs"), ("daetype", "outline", "simplify.rs"),
                  ("daetype", "outline", "flatten.rs"), ("daetype", "hinting", "state.rs"),
                  ("daekit", "draw", "subpixel.rs"), ("daekit", "draw", "bitmap.rs"), ("daekit", "cache", "metrics.rs"),
                  ("daetype", "decoder", "fvar.rs"), ("daetype", "glyph_names", "mod.rs")):
        limits.update(re.findall(r"pub const (\w+): \w+ = ([^;]+);", read(*CORE, *parts)))
    values = {}
    for c, expr in limits.items():
        expr = re.sub(r"\bas \w+", "", expr)
        for known, v in values.items():
            expr = re.sub(r"\b" + known + r"\b", str(v), expr)
        expr = re.sub(r"(?<=[0-9a-fA-F])_(?=[0-9a-fA-F])", "", expr)
        if re.fullmatch(r"[0-9x a-fA-F+*()<-]+", expr):
            values[c] = int(eval(expr))
    # C names the subpixel limits for their area, which Rust's `raster` module names instead.
    for real, alias in (("MAX_TAPS", "MAX_SUBPIXEL_TAPS"), ("MAX_WEIGHTS", "MAX_SUBPIXEL_WEIGHTS")):
        values[alias] = values[real]
    # The point size trak is read at with none given, a whole number of points held as an f64.
    size = re.search(r"pub const DEFAULT_POINT_SIZE: f64 = (\d+)\.0;", read(*CORE, "daetype", "trak.rs"))
    values["DEFAULT_POINT_SIZE"] = int(size.group(1))
    found.update(values)
    return found


# The headers' own machinery rather than values Rust holds: linkage, attributes, nullability and the layout checks.
NULLABILITY = {"NULLABLE", "NONNULL", "NONNULL_BEGIN", "NONNULL_END", "HEADER_BEGIN", "HEADER_END"}
# Marks a binary never sees: a caller built with or without one runs the same, where a promise a compiler acts on, as
# CONST, MALLOC and ALLOC_SIZE are, stays. CALLBACK is __cdecl, Rust's "C" on every target released.
NEUTRAL = sorted(NULLABILITY | {"API", "NODISCARD", "COLD", "CALL", "CALLBACK"})
MACHINERY = {"API", "NODISCARD", "CONST", "COLD", "CALL", "CALLBACK", "LAYOUT_HEADER", "LAYOUT_64"} | NULLABILITY


def constants(files):
    # The headers' integer constants are copies C compiles against: one that drifts from its Rust
    # source misreads every call that uses it, and no linker notices.
    rust = rust_constants()
    problems, n = [], 0
    # Every define with a value is read. One that is not a plain integer cannot be compared, so it fails
    # rather than slipping by; the ABI version is the exception, which version() reads.
    pattern = r"^[ \t]*#[ \t]*define[ \t]+DAEGUN_([A-Z0-9_]+)[ \t]+(\S+)"
    for path, header in files:
        for m in re.finditer(pattern, header, re.M):
            name, value = m.groups()
            at = f"{path}:{header.count(chr(10), 0, m.start()) + 1}"
            if name == "ABI_VERSION" or name in MACHINERY:
                continue
            n += 1
            value = value.removesuffix("u")
            if not re.fullmatch(r"-?(?:0x[0-9a-fA-F]+|\d+)", value):
                problems.append(f"        UNREAD  DAEGUN_{name} is {value} in {at}, not a plain integer")
            elif name not in rust:
                problems.append(f"        NO RUST SOURCE  DAEGUN_{name}  ({at})")
            elif rust[name] != int(value, 0):
                problems.append(f"        DIFFERS  DAEGUN_{name} is {value} in {at}, {rust[name]} in Rust")
    verdict = " – all equal to their Rust source" if not problems else ""
    print(f"constants  headers define {n}{verdict}")
    for p in problems:
        print(p)
    print()
    return len(problems)


RELEASED = os.path.join(HERE, "abi-released.txt")
# The code sets a call answers, frozen for a major as base.h says: a new one is a value an old program's switch never
# met, so it counts as a break. DAEGUN_SHAPER_* and DAEGUN_VO_* say a newer library may answer more; the rest go in.
ANSWERED = ("BITMAP", "COMPOSITE", "EXTEND", "FILL", "GC", "GLYPH_CLASS", "GRADIENT", "OP", "PAINT", "POINT",
            "SCRIPT_DIRECTION", "STAT", "STYLE", "VERB")


def version(files):
    # The headers' DAEGUN_ABI_VERSION as (major, minor, patch), held equal to the library's own.
    header = "\n".join(text for _, text in files)
    parts = re.search(r"#define DAEGUN_ABI_VERSION 0x([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})u$", header, re.M)
    rust = re.search(r"const ABI_VERSION: u32 = abi\((\d+), (\d+), (\d+)\);",
                     read("src", "api", "c", "translation", "mod.rs"))
    if not parts or not rust:
        sys.exit("DAEGUN_ABI_VERSION or mod.rs's ABI_VERSION is not in the form this reads")
    ours, theirs = tuple(int(part, 16) for part in parts.groups()), tuple(map(int, rust.groups()))
    if ours != theirs:
        sys.exit(f"DAEGUN_ABI_VERSION is {dotted(ours)} in daegun/base.h, {dotted(theirs)} in mod.rs")
    return ours


def dotted(v):
    return ".".join(map(str, v))


def surface(files):
    # What a compiled caller depends on, keyed by the name each declares. Comments, spacing and the
    # asserts that check the layouts are left out: a change to them can't break a binary.
    found = {}
    for path, header in files:
        text = re.sub(r"/\*.*?\*/", " ", header, flags=re.S)
        for name, value in re.findall(r"^[ \t]*#[ \t]*define[ \t]+(DAEGUN_\w+)[ \t]+(.*?)[ \t]*$", text, re.M):
            if name != "DAEGUN_ABI_VERSION" and not name.startswith("DAEGUN_LAYOUT_") and name[7:] not in NEUTRAL:
                found[name] = value
        for decl, line in statements(header):
            pointer = re.search(r"\(\s*\*\s*(\w+)\s*\)", decl)
            if decl.startswith("typedef"):
                name = pointer.group(1) if pointer and "{" not in decl else re.findall(r"\w+", decl)[-1]
            elif NAME.search(decl):
                name = NAME.search(decl).group(1)
            else:
                sys.exit(f"{path}:{line} declares something abi-parity.py can't name: {decl[:80]}")
            found[name] = decl
    return found


def released(files):
    # The headers against the last release's ABI, which a caller built then still holds.
    now, current = version(files), surface(files)
    if not os.path.isfile(RELEASED):
        print("released   none recorded (abi-parity.py --record)\n")
        return 1
    lines = open(RELEASED, encoding="utf-8").read().splitlines()
    then = tuple(map(int, lines[0].split()[1].split(".")))
    old = dict(line.split("\t", 1) for line in lines[1:] if line)
    added = sorted(k for k in current if k not in old)
    grown = [k for k in added if k.startswith(tuple(f"DAEGUN_{s}_" for s in ANSWERED))]
    broken = sorted({k for k in old if current.get(k) != old[k]} | set(grown))
    need = (then[0] + 1, 0, 0) if broken else (then[0], then[1] + 1, 0) if added else then
    print(f"released   {dotted(then)} against the header's {dotted(now)}: {len(broken)} changed or removed, "
          f"{len(added)} added", end="")
    if now >= need:
        print(" – the version moves as they need\n")
        return 0
    print(f"\n        DAEGUN_ABI_VERSION must be at least {dotted(need)}, here and in mod.rs")
    for name in (broken or added)[:12]:
        how = "CHANGED" if name in old and name in current else "REMOVED" if name in old else "ADDED"
        how = "GROWN (an answered set)" if name in grown else how
        print(f"        {how}  {name}")
    print()
    return 1


def record(files):
    now, current = version(files), surface(files)
    with open(RELEASED, "w", encoding="utf-8") as out:
        out.write(f"version {dotted(now)}\n")
        out.writelines(f"{k}\t{v}\n" for k, v in sorted(current.items()))
    print(f"recorded {len(current)} declarations as the released ABI {dotted(now)}")
    return 0


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


# A shared library, or a DLL's import library, binds a program to every name it exports, so one that isn't daegun's
# is a clash waiting for another library that carries the same.
SHARED = (".dylib", ".so", ".dll.lib")
# What every import library holds beside the names it imports.
IMPORT_MACHINERY = re.compile(r"__IMPORT_DESCRIPTOR_[\w-]+|__NULL_IMPORT_DESCRIPTOR|\x7f[\w-]+_NULL_THUNK_DATA")


def exported(path):
    # llvm-nm reads Mach-O, ELF and COFF alike, so a Windows .lib cross-built on a Mac is still
    # readable. The system nm is the fallback for everything but COFF.
    tool = llvm_nm()
    shared = path.endswith(SHARED)
    if tool:
        table = ["--extern-only"] + (["--dynamic"] if path.endswith(".so") else []) if shared else []
        argv = [tool, "--defined-only", *table, path]
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
    names, strays = set(), set()
    for line in run.stdout.splitlines():
        parts = line.split()
        if len(parts) < 2 or parts[-2] in ("U", "u", "w"):
            continue
        hit = re.fullmatch(r"(?:__imp_)?_?(daegun_[a-z_0-9]+)", parts[-1])
        if hit:
            names.add(hit.group(1))
        elif shared and not IMPORT_MACHINERY.fullmatch(parts[-1]):
            strays.add(parts[-1])
    return names, strays


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


def neutral_marks_change_nothing():
    # A declaration reads the same with the marks a binary never sees, and differs with a promise a caller acts on.
    plain = "daegun_status\ndaegun_f(const uint8_t *data, size_t n);\n"
    marked = ("DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL\n"
              "daegun_f(const uint8_t *DAEGUN_NULLABLE data, size_t n);\n")
    if not surface([("a.h", plain)]) == surface([("a.h", marked)]) != surface([("a.h", "DAEGUN_CONST " + plain)]):
        sys.exit("abi-parity.py reads a binary-neutral mark as a change, or a promise as none")


def main(argv):
    if "-h" in argv or "--help" in argv:
        print(USAGE)
        return 0

    neutral_marks_change_nothing()
    files = headers()
    if argv == ["--record"]:
        return record(files)
    constant_differences = constants(files) + released(files)
    libs = argv or libraries()
    # No library read is no parity shown, so it fails rather than passing on the constants alone.
    if not libs:
        print("nothing built (run: cargo rustc --features capi --crate-type staticlib)")
        return 1

    header = declared(files)
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
            read = exported(lib)
        except Unreadable as e:
            print(f"{platform:7} {shown(lib)}\n        unreadable: {e}\n")
            missing += 1
            continue
        symbols, strays = read if read is not None else (None, set())
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
        print(f"        headers declare {len(header)}, library exports {len(symbols)}", end="")
        if not unbuilt and not undeclared and not strays:
            print(" – matched" + (", and nothing else" if lib.endswith(SHARED) else ""))
        else:
            print()
            differences += len(unbuilt) + len(undeclared) + len(strays)
        for name in unbuilt:
            print(f"        DECLARED, NEVER EXPORTED  {name}  ({header[name]})")
        for name in undeclared:
            print(f"        EXPORTED, NEVER DECLARED  {name}")
        for name in sorted(strays):
            print(f"        EXPORTED, NOT DAEGUN'S    {name}")
        print()

    if not checked:
        print("nothing checked")
        return 1
    print(f"{checked} librar{'y' if checked == 1 else 'ies'} checked, "
          f"{differences or 'no'} difference{'' if differences == 1 else 's'}.")
    return 1 if differences or missing else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
