#!/usr/bin/env python3
USAGE = """    c-signatures.py

Every function the C headers declare against the Rust that defines it: the same number of parameters, each and the
return of the same kind (a scalar of the same width and sign, a pointer of the same constness to the same kind, a
callback), and each named type paired with one Rust type across every call, both ways. Each struct a call or another
struct names is held to its Rust `repr(C)` struct field for field, a pen's and a visitor's callbacks among them.
abi-parity.py holds the names to the library and compiling a C caller holds it to the headers; neither sees a Rust
definition that has drifted from its prototype, which links and then misreads its arguments.

The Rust side is read from the crate expanded by nightly (`-Zunpretty=expanded`), so exports a macro writes are read
as the compiler sees them, and read again by cbindgen from the same expansion: the headers are held to the
prototypes and structs it generates the same way, and its pairing of named types to this script's, so neither
reading of the Rust stands alone. Each pointer's nullability mark is held to how the same expansion treats NULL: one
refused with DAEGUN_NULL is required and one a helper takes NULL for is DAEGUN_NULLABLE, every declaration inside its
header's DAEGUN_HEADER_BEGIN region. Each DAEGUN_READS and DAEGUN_WRITES names a pointer of its constness and a
size_t, and every array a length follows carries one. Exit status is 1 on any difference."""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".."))
INCLUDE = os.path.join(ROOT, "src", "api", "c", "include")

C_SCALARS = {"int8_t": "i8", "uint8_t": "u8", "int16_t": "i16", "uint16_t": "u16", "int32_t": "i32",
             "uint32_t": "u32", "int64_t": "i64", "uint64_t": "u64", "size_t": "usize", "float": "f32",
             "double": "f64", "bool": "bool", "char": "c_char", "void": "void", "daegun_status": "i32",
             "daegun_tag": "u32"}
# Status and GlyphId are i32 and u32 as C sees them: a repr(i32) enum and a transparent u32.
MARK = r"\bDAEGUN_(?:NULLABLE|NONNULL)\b"
MARKS = {"DAEGUN_NULLABLE": "nullable", "DAEGUN_NONNULL": "nonnull"}
RUST_SCALARS = {"i8": "i8", "u8": "u8", "i16": "i16", "u16": "u16", "i32": "i32", "u32": "u32", "i64": "i64",
                "u64": "u64", "usize": "usize", "f32": "f32", "f64": "f64", "bool": "bool", "c_char": "c_char",
                "c_void": "void", "Status": "i32", "GlyphId": "u32"}


def headers():
    umbrella = open(os.path.join(INCLUDE, "daegun.h"), encoding="utf-8").read()
    names = re.findall(r'^#include <(daegun/\w+\.h)>', umbrella, re.M)
    return [open(os.path.join(INCLUDE, n), encoding="utf-8").read() for n in names]


def split(params, opens="(", closes=")"):
    out, depth, cur = [], 0, ""
    for ch in params:
        depth += (ch in opens) - (ch in closes)
        if ch == "," and depth == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += ch
    out.append(cur.strip())
    return [p for p in out if p and p != "void"]


def c_callback(decl, callbacks):
    # A callback written out, `RET (*name)(PARAMS)`, as its return and parameters.
    decl = " ".join(re.sub(MARK, " ", decl).split())
    hit = re.fullmatch(r"(.*?)\(\s*\*\s*\w*\s*\)\s*\((.*)\)", decl.strip())
    return ("fn", (c_kind(hit.group(1) + " x", callbacks), tuple(c_kind(p + " x", callbacks)
                                                               for p in split(hit.group(2)))))


def c_kind(decl, callbacks):
    # One parameter or return type, its name last: an array parameter is the pointer C passes.
    if re.search(r"\(\s*\*", decl):
        return c_callback(decl, callbacks)
    decl = re.sub(r"(\w+)\s*\[\s*\d*\s*\]\s*$", r"* \1", re.sub(MARK, " ", decl)).replace("*", " * ")
    words = decl.split()
    base = next(w for w in words if w not in ("const", "*"))
    words = words[words.index(base) + 1:]
    stars = words.count("*")
    kind = callbacks.get(base) or C_SCALARS.get(base) or ("named", base)
    # Only the first pointer can be const here, as `const T *` or `const T **` is all the headers write.
    for i in range(stars):
        const = i == 0 and "const" in decl.split()[: decl.split().index(base)]
        kind = ("ptr", "const" if const else "mut", kind)
    return kind


def c_field(decl, callbacks):
    # A struct's field, an array kept as one where a parameter's is the pointer C passes.
    arr = re.fullmatch(r"(.*?\w)\s*\[\s*(\d+)\s*\]", decl.strip())
    return ("arr", int(arr.group(2)), c_kind(arr.group(1), callbacks)) if arr else c_kind(decl, callbacks)


def flat(fields):
    # Fields as their layout lays them out: Rust's `[f64; 2]` for C's two doubles, an array as its elements.
    return [k for f in fields for k in (flat([f[2]]) * f[1] if isinstance(f, tuple) and f[0] == "arr" else [f])]


def name_of(decl):
    hit = re.search(r"\(\s*\*[^)]*?(\w+)\s*\)", decl)
    return hit.group(1) if hit else re.findall(r"\w+", re.sub(r"\[[^\]]*\]\s*$", "", decl))[-1]


def c_marks(decl, callbacks, field=False):
    # One declaration's pointer levels, outermost first, as its marks say, and a callback's parameters' by name. A
    # lone pointer unmarked is nonnull, as assume_nonnull makes it; clang leaves a `T **`'s or an array's unspecified.
    decl = " ".join(decl.split())
    hit = re.fullmatch(r"(.*?)\(\s*\*\s*(DAEGUN_\w+)?\s*\w*\s*\)\s*\((.*)\)", decl)
    if hit:
        return [MARKS.get(hit.group(2), "nonnull")], {name_of(p): c_marks(p, callbacks)[0] for p in split(hit.group(3))}
    arr = None if field else re.search(r"\[\s*(DAEGUN_\w+)?\s*\d*\s*\]\s*$", decl)
    words = re.sub(r"\[[^\]]*\]\s*$", "", decl).replace("*", " * ").split()
    levels = [MARKS.get(after, "unmarked") for w, after in zip(words, words[1:] + [""]) if w == "*"]
    base = next(w for w in words if w not in ("const", "*"))
    if isinstance(callbacks.get(base), tuple):
        levels.insert(0, MARKS.get(words[words.index(base) + 1], "unmarked"))
    if arr:
        levels.append(MARKS.get(arr.group(1), "unmarked"))
    if levels == ["unmarked"] and not arr:
        levels = ["nonnull"]
    return levels[::-1], {}


def c_declarations(texts):
    found, callbacks, structs, marks = {}, {}, {}, {}
    for header in texts:
        # A callback's calling convention is C's, as Rust's `extern "C"` is, so it goes before anything reads a type.
        text = re.sub(r"/\*.*?\*/|//[^\n]*|\bDAEGUN_CALL(?:BACK)?\b"
                      r"|\bDAEGUN_(?:COUNTED_BY|LENT|IN|OUT)\((?:[^()]|\([^()]*\))*\)", " ", header, flags=re.S)
        # cbindgen's `struct Font *` as the headers' `daegun_font *`, and its typedefs of a scalar as that scalar.
        text = re.sub(r"\b(?:struct|enum)\s+(?=\w+\b\s*[*\w])", "", text)
        callbacks.update((m.group(2), C_SCALARS[m.group(1)])
                         for m in re.finditer(r"\btypedef\s+(\w+)\s+(\w+)\s*;", text) if m.group(1) in C_SCALARS)
        callbacks.update((m.group(1), c_callback(m.group(0)[len("typedef"):-1], callbacks))
                         for m in re.finditer(r"typedef[^;{]*\(\s*\*\s*(\w+)\s*\)[^;]*;", text))
        # cbindgen spells an `Option` of a callback alias as `Option_` and the alias.
        callbacks.update((f"Option_{name}", kind) for name, kind in list(callbacks.items()))
        for m in re.finditer(r"typedef[^;{]*\(\s*\*\s*(\w+)\s*\)[^;]*;", text):
            for param, levels in c_marks(m.group(0)[len("typedef"):-1], callbacks)[1].items():
                marks[("callback", m.group(1), param)] = levels
        for m in re.finditer(r"typedef struct\s*(?:\w+\s*)?\{([^{}]*)\}\s*(\w+)\s*;", text):
            fields = [f for f in m.group(1).split(";") if f.strip()]
            structs[m.group(2)] = [c_field(f, callbacks) for f in fields]
            for f in fields:
                levels, params = c_marks(f, callbacks, field=True)
                name = name_of(f)
                if levels:
                    marks[("field", m.group(2), name)] = levels
                for param, inner in params.items():
                    marks[("callback", f"{m.group(2)}.{name}", param)] = inner
        text = re.sub(r"^[ \t]*#[ \t]*ifdef[ \t]+__cplusplus.*?^[ \t]*#[ \t]*endif.*?$", " ", text, flags=re.S | re.M)
        text = re.sub(r"^[ \t]*#(?:.*\\\n)*.*$", " ", text, flags=re.M)
        text = re.sub(r"\bDAEGUN_LAYOUT_ASSERT\(.*?\);", " ", text, flags=re.S)
        text = re.sub(r"\bDAEGUN_(?:MALLOC|ALLOC_SIZE|READS|WRITES|COUNTED_BY|LENT|IN|OUT(?:_OR_NULL)?)"
                      r"\((?:[^()]|\([^()]*\))*\)"
                      r"|\bDAEGUN_(?:NODISCARD|API|CONST|COLD|(?:HEADER|NONNULL)_(?:BEGIN|END))\b", " ", text)
        depth, start = 0, 0
        for i, ch in enumerate(text):
            if ch in "{}":
                depth += 1 if ch == "{" else -1
            elif ch == ";" and depth == 0:
                decl, start = " ".join(text[start:i].split()), i + 1
                hit = re.fullmatch(r"(.*?)\b(daegun_[a-z_0-9]+)\s*\((.*)\)", decl)
                if hit and not decl.startswith("typedef"):
                    ret = c_kind(hit.group(1) + " x", callbacks)
                    params = [c_kind(p, callbacks) for p in split(hit.group(3))]
                    found[hit.group(2)] = (ret, params)
                    marks[("return", hit.group(2))] = c_marks(hit.group(1) + " x", callbacks)[0]
                    for at, p in enumerate(split(hit.group(3))):
                        levels, inner = c_marks(p, callbacks)
                        name = name_of(p)
                        marks[("param", hit.group(2), at, name)] = levels
                        for param, more in inner.items():
                            marks[("callback", f"{hit.group(2)}.{name}", param)] = more
    return found, structs, marks


def rust_callback(ty, aliases):
    # `unsafe extern "C" fn(PARAMS) -> RET` as its return and parameters.
    hit = re.fullmatch(r"(?:unsafe\s+)?extern \"C\" fn\((.*)\)\s*(?:->\s*(.+))?", ty.strip(), re.S)
    params = tuple(rust_kind(re.sub(r"^\s*\w+\s*:(?!:)", "", p), aliases) for p in split(hit.group(1), "(<[", ")>]"))
    return ("fn", (rust_kind(hit.group(2), aliases) if hit.group(2) else "void", params))


def rust_kind(ty, aliases):
    ty = " ".join(ty.split())
    if ty.startswith("Option<") and ty.endswith(">"):
        inner = ty[7:-1].strip()
        if inner in aliases:
            return aliases[inner]
        if "fn(" in inner:
            return rust_callback(inner, aliases)
    for prefix, const in (("*const ", "const"), ("*mut ", "mut")):
        if ty.startswith(prefix):
            return ("ptr", const, rust_kind(ty[len(prefix):], aliases))
    last = ty.split("::")[-1]
    return RUST_SCALARS.get(last) or ("named", last)


def cbindgen(source):
    # cbindgen's header for the expanded crate, read as one file, so it sees what the compiler does. It leaves an
    # `Option` of a callback alias opaque, so each alias is asked for by name.
    if not shutil.which("cbindgen"):
        sys.exit("no cbindgen (cargo install --locked cbindgen)")
    aliases = re.findall(r"\btype (\w+)\s*=\s*(?:unsafe\s+)?extern \"C\" fn\b", source)
    # Nightly from 2026-11 spells `offset_of!` as `k#offset_of`, which cbindgen's syn can't parse, so it's handed the
    # old spelling.
    source = source.replace("k#offset_of(", "builtin # offset_of(")
    config = f'language = "C"\nusize_is_size_t = true\n[export]\ninclude = {json.dumps(aliases)}\n'
    with tempfile.TemporaryDirectory() as tmp:
        for name, text in (("expanded.rs", source), ("cbindgen.toml", config)):
            open(os.path.join(tmp, name), "w", encoding="utf-8").write(text)
        run = subprocess.run(["cbindgen", "--config", os.path.join(tmp, "cbindgen.toml"), "-o",
                              os.path.join(tmp, "out.h"), os.path.join(tmp, "expanded.rs")], capture_output=True,
                             text=True)
        if run.returncode != 0:
            sys.exit("cbindgen failed:\n" + run.stderr[-2000:])
        return open(os.path.join(tmp, "out.h"), encoding="utf-8").read()


def expanded():
    # Where cargo builds, which CARGO_TARGET_DIR or a config can move; nightly's artifacts kept apart there.
    meta = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT, capture_output=True,
                          text=True, check=True).stdout
    env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(json.loads(meta)["target_directory"], "expanded"))
    run = subprocess.run(["cargo", "+nightly", "rustc", "-q", "--features", "capi", "--lib", "--",
                          "-Zunpretty=expanded"], cwd=ROOT, env=env, capture_output=True, text=True)
    if run.returncode != 0:
        sys.exit("the crate does not expand on nightly:\n" + run.stderr[-2000:])
    return run.stdout


def rust_field(ty, aliases):
    arr = re.fullmatch(r"\[(.*);\s*(\d+)\s*\]", " ".join(ty.split()))
    return ("arr", int(arr.group(2)), rust_kind(arr.group(1), aliases)) if arr else rust_kind(ty, aliases)


def rust_structs(source, aliases):
    # Each `repr(C)` struct with named fields, as its fields' kinds in order.
    found = {}
    for m in re.finditer(r"\bstruct (\w+)\s*\{", source):
        attrs = source[max(source.rfind("}", 0, m.start()), source.rfind(";", 0, m.start())) + 1:m.start()]
        if not re.search(r"#\[repr\(C\)\]", attrs):
            continue
        i, depth = m.end(), 1
        while depth:
            depth += (source[i] == "{") - (source[i] == "}")
            i += 1
        body = re.sub(r"#\[doc\s*=\s*\"(?:[^\"\\]|\\.)*\"\]|//[^\n]*", " ", source[m.end():i - 1])
        fields = split(re.sub(r"#\[[^\]]*\]", " ", body), "(<[{", ")>]}")
        found[m.group(1)] = [rust_field(f.split(":", 1)[1], aliases) for f in fields]
    return found


def rust_definitions(source):
    # Callback aliases, as `type SegmentFn = unsafe extern "C" fn(...)`, read as the signatures they name.
    aliases = {}
    for m in re.finditer(r"\btype (\w+)\s*=\s*((?:unsafe\s+)?extern \"C\" fn\b[^;]*);", source):
        aliases[m.group(1)] = rust_callback(m.group(2), aliases)
    found = {}
    for m in re.finditer(r"extern \"C\" fn (daegun_[a-z0-9_]+)\s*\(", source):
        i, depth = m.end(), 1
        while depth:
            depth += (source[i] == "(") - (source[i] == ")")
            i += 1
        params = [p.split(":", 1)[1] for p in split(source[m.start(0) + len(m.group(0)):i - 1], "(<[", ")>]")]
        ret = re.match(r"\s*->\s*(.+?)\s*(?:where\b.*?)?\{", source[i:i + 300], re.S)
        found[m.group(1)] = (rust_kind(ret.group(1), aliases) if ret else "void",
                             [rust_kind(p, aliases) for p in params])
    return found, rust_structs(source, aliases)


def closing(text, i, opens="{", closes="}"):
    # Past the bracket that closes the one opened just before `i`.
    depth = 1
    while depth:
        depth += (text[i] == opens) - (text[i] == closes)
        i += 1
    return i


NULL_RETURN = (r"return\s+(?:Status::Null|Err\(\s*Status::Null\s*\)|(?:::)?core::ptr::null(?:_mut)?\(\)"
               r"|crate::ffi::handle::Status::Null)")
# The translation's own helpers: a NULL either refused where their caller turns None into DAEGUN_NULL, or taken.
EITHER = ("borrow", "borrow_mut", "cleared", "array_of", "target")
TAKEN = ("slice_of", "slice_mut_of", "text_of", "release", "fill", "options_of", "at")


class Nulls:
    # Which pointers each export refuses as NULL, read from the C translation as rule 2 has it: one answered with
    # DAEGUN_NULL or a NULL return is required, one a helper takes NULL for or that is only tested is nullable.

    def __init__(self, source):
        start = source.index("\nmod ffi {")
        ffi = source[start:closing(source, source.index("{", start) + 1)]
        self.ffi = re.sub(r"#\[doc\s*=\s*\"(?:[^\"\\]|\\.)*\"\]|//[^\n]*", " ", ffi)
        self.aliases = set(re.findall(r"\btype (\w+)\s*=\s*(?:unsafe\s+)?extern \"C\" fn\b", self.ffi))
        self.fns, self.memo = {}, {}
        for m in re.finditer(r"\bfn (\w+)\s*(?:<(?:[^<>]|<[^<>]*>)*>)?\s*\(", self.ffi):
            close = closing(self.ffi, m.end(), "(", ")")
            params = [(p.split(":", 1)[0].split()[-1], " ".join(p.split(":", 1)[1].split()))
                      for p in split(self.ffi[m.end():close - 1], "(<[", ")>]") if ":" in p]
            brace, semi = self.ffi.find("{", close), self.ffi.find(";", close)
            if brace >= 0 and not 0 <= semi < brace:
                body = self.ffi[brace + 1:closing(self.ffi, brace + 1) - 1]
                self.fns.setdefault(m.group(1), []).append((params, body))

    def pointer(self, ty):
        return ty.startswith("*") or ty.startswith("Option<") and ("fn" in ty or ty[7:-1].strip() in self.aliases)

    def param(self, fn, index):
        # Required where any definition refuses it, nullable where every one takes it, else unknown.
        if (fn, index) in self.memo:
            return self.memo[(fn, index)]
        if fn in TAKEN:
            return "nullable"
        self.memo[(fn, index)] = "?"
        verdicts = {self.verdict(params[index][0], body, params[index][1])
                    for params, body in self.fns.get(fn, []) if index < len(params) and self.pointer(params[index][1])}
        out = "required" if "required" in verdicts else "nullable" if verdicts == {"nullable"} else "?"
        self.memo[(fn, index)] = out
        return out

    def verdict(self, name, body, ty):
        n = re.escape(name)
        # Past a binding of the same name, the name is that binding's.
        rebound = re.search(r"\blet\s+(?:mut\s+)?(?:Some\(\s*)?\(?[^=;]*\b" + n + r"\b[^=;]*=", body)
        if rebound:
            body = body[:body.find(";", rebound.end()) + 1]
        found = set()
        for m in re.finditer(r"\bif\s+([^{]*?)\{", body):
            inner = body[m.end():closing(body, m.end())]
            for term in (t.strip() for t in m.group(1).split("||")):
                if re.fullmatch(n + r"\.is_null\(\)", term):
                    found.add("required" if re.match(r"\s*" + NULL_RETURN, inner) else "nullable")
                elif re.fullmatch(r"!\s*" + n + r"\.is_null\(\)", term):
                    found.add("nullable")
        if re.search(r"match\s*\(\s*\w+\s*,\s*" + n + r"\.is_null\(\)\s*\)", body):
            found.add("nullable")
        if re.search(r"let\s+Some\(\w+\)\s*=\s*" + n + r"\s+else\s*\{\s*" + NULL_RETURN, body):
            found.add("required")
        if re.search(r"\b" + n + r"\s*\.\s*map\(|if\s+let\s+Some\(\w+\)\s*=\s*" + n + r"\b", body):
            found.add("nullable")
        for m in re.finditer(r"(?:\.|\b)(\w+)\s*(?:::<(?:[^<>]|<[^<>]*>)*>)?\s*\(", body):
            close = closing(body, m.end(), "(", ")")
            args = split(body[m.end():close - 1], "(<[{", ")>]}")
            hit = [i for i, a in enumerate(args)
                   if re.fullmatch(r"\*?\s*" + n + r"(?:\.cast(?:_const|_mut)?(?:::<[^>]*>)?\(\))?", a)]
            callee = m.group(1)
            if not hit:
                continue
            if callee in EITHER:
                before = body[body.rfind(";", 0, m.start()) + 1:m.start()]
                after = body[close:body.find(";", close) + 1]
                bound = re.search(r"\blet\s*\(?\s*Some\(", before) and re.search(r"else\s*\{\s*" + NULL_RETURN, after)
                refused = re.match(r"[)}\s]*\.ok_or\(\s*Status::Null", after)
                found.add("required" if bound or refused else "nullable")
            elif callee in self.fns or callee in TAKEN:
                found.update(self.param(callee, i) for i in hit)
            elif callee not in ("Some", "Ok", "Err", "drop", "unsafe"):
                found.add("passed")
        if "required" in found:
            return "required"
        # A user pointer handed on to the caller's own callback is the caller's to make NULL.
        if "passed" in found and "c_void" not in ty and not re.search(n + r"\.is_null\(\)", body):
            return "?"
        return "nullable" if found and found <= {"nullable", "passed"} else "?"

    def returns(self, fn):
        # A view answers NULL for a NULL handle, and so does any return that writes a NULL; an allocator never does.
        body = self.fns[fn][0][1]
        return "nullable" if re.search(r"\bnull(?:_mut)?\(\)|\bview\s*\(", body) else "nonnull"


def regions(texts):
    # Every declaration inside its header's assume_nonnull region, where an unmarked pointer means nonnull.
    problems = []
    for header in texts:
        text = re.sub(r"/\*.*?\*/|//[^\n]*", " ", header, flags=re.S)
        text = re.sub(r"^[ \t]*#(?:.*\\\n)*.*$", " ", text, flags=re.M)
        begin, end = text.find("DAEGUN_HEADER_BEGIN"), text.rfind("DAEGUN_HEADER_END")
        if begin < 0 or end < begin or ";" in text[:begin] + text[end:]:
            guard = re.search(r"#ifndef (\w+)", header).group(1)
            problems.append(f"  REGION  {guard}'s header declares outside DAEGUN_HEADER_BEGIN and _END")
    return problems


def nullability(marks, nulls):
    # Each mark against how its call treats NULL: parameters as Nulls reads them, a `T **`'s inner level as rule 1 sets
    # it, every struct field, the caller's zeroed storage until a call fills it, and callbacks' user pointers and stops.
    problems = []
    for site, levels in sorted(marks.items(), key=str):
        if not levels:
            continue
        if site[0] == "param":
            _, fn, at, name = site
            if fn not in nulls.fns:
                continue
            verdict = nulls.param(fn, at)
            want = [{"required": "nonnull"}.get(verdict, verdict)] + ["nullable"] * (len(levels) - 1)
        elif site[0] == "return":
            if site[1] not in nulls.fns:
                continue
            want, name = [nulls.returns(site[1])], "its return"
        elif site[0] == "field":
            want, name = ["nullable"] * len(levels), site[2]
        else:
            want, name = ["nullable" if site[2] in ("user", "stops") else "nonnull"] * len(levels), site[2]
        if levels != want:
            problems.append(f"  NULLABILITY  {site[1]}: {name} is {' '.join(levels)} in C and {' '.join(want)} as "
                            "its call treats NULL")
    return problems


def agree(c, rust, pairs, pointee=False):
    # Whether a C kind and a Rust kind are the same at the ABI, a named type's pairing noted as it's met.
    if isinstance(c, tuple) and isinstance(rust, tuple) and c[0] == rust[0] == "ptr":
        return c[1] == rust[1] and agree(c[2], rust[2], pairs, True)
    if isinstance(c, tuple) and isinstance(rust, tuple) and c[0] == rust[0] == "arr":
        return c[1] == rust[1] and agree(c[2], rust[2], pairs, pointee)
    if isinstance(c, tuple) and isinstance(rust, tuple) and c[0] == rust[0] == "fn":
        (c_ret, c_params), (r_ret, r_params) = c[1], rust[1]
        return len(c_params) == len(r_params) and all(
            agree(a, b, pairs) for a, b in [(c_ret, r_ret)] + list(zip(c_params, r_params)))
    if isinstance(c, tuple) and isinstance(rust, tuple) and c[0] == rust[0] == "named":
        pairs.setdefault(c[1], set()).add(rust[1])
        return True
    # A list of yes or no is bytes to C, which a Rust bool, always 0 or 1, can be read as; never the other way. A C
    # bool Rust reads as a byte is the other side of that, as a caller may write any byte there.
    return c == rust or (pointee and c == "u8" and rust == "bool") or (c == "bool" and rust == "u8")


def compare(c, c_structs, other, other_structs, side):
    # Each call, and each struct it names and its fields name in turn, against `side`'s reading of the Rust.
    problems, pairs = [], {}
    for name in sorted(set(c) | set(other)):
        if name not in other:
            problems.append(f"  NO RUST  {name} is declared and {side} finds nothing defining it")
            continue
        if name not in c:
            problems.append(f"  NO C  {name} is defined in {side} and no header declares it")
            continue
        (c_ret, c_params), (r_ret, r_params) = c[name], other[name]
        if len(c_params) != len(r_params):
            problems.append(f"  ARITY  {name} takes {len(c_params)} in C and {len(r_params)} in {side}")
            continue
        for at, (a, b) in enumerate([(c_ret, r_ret)] + list(zip(c_params, r_params))):
            if not agree(a, b, pairs):
                where = "its return" if at == 0 else f"parameter {at}"
                problems.append(f"  KIND  {name}: {where} is {a} in C and {b} in {side}")
    checked = set()
    while todo := [(n, r) for n, rs in sorted(pairs.items()) for r in sorted(rs) if (n, r) not in checked]:
        for c_name, r_name in todo:
            checked.add((c_name, r_name))
            if c_name not in c_structs:
                continue
            if r_name not in other_structs:
                problems.append(f"  NO STRUCT  {c_name} has fields and {r_name} is no repr(C) struct in {side}")
                continue
            fields, r_fields = flat(c_structs[c_name]), flat(other_structs[r_name])
            if len(fields) != len(r_fields):
                problems.append(f"  FIELDS  {c_name} has {len(fields)} fields in C and {len(r_fields)} in {side}")
                continue
            for at, (a, b) in enumerate(zip(fields, r_fields), 1):
                if not agree(a, b, pairs):
                    problems.append(f"  FIELD  {c_name}: field {at} is {a} in C and {b} in {side}")
    unpaired = sorted(set(c_structs) - {n for n, _ in checked})
    if unpaired:
        problems.append(f"  UNCHECKED  no call or struct pairs {', '.join(unpaired)} with a type in {side}")
    # A named type means one thing: two Rust types behind one C name, or one behind two, is a cast waiting to misread.
    rust_of = {}
    for c_name, rust_names in sorted(pairs.items()):
        if len(rust_names) > 1:
            problems.append(f"  PAIRING  {c_name} is {', '.join(sorted(rust_names))} in {side}")
        for r in rust_names:
            rust_of.setdefault(r, set()).add(c_name)
    for r, c_names in sorted(rust_of.items()):
        if len(c_names) > 1:
            problems.append(f"  PAIRING  {side}'s {r} stands for {', '.join(sorted(c_names))}")
    return problems, pairs, sum(1 for n, _ in checked if n in c_structs)


# Arrays the access attributes leave out: the buffer passes to daegun, or only its address is used.
UNACCESSED = {"daegun_font_buffer_free", "daegun_font_open_owned", "daegun_bytes_window"}
# Buffers a call takes NULL for whatever the length, each with its length's index: GCC would warn of NULL there as it
# does with a length elsewhere, so each carries DAEGUN_OUT_OR_NULL alone.
OPTIONAL = {("daegun_font_glyph_ids", 5): 6, ("daegun_font_coverage", 2): 4, ("daegun_font_coverage", 3): 4,
            ("daegun_font_ligature_carets", 5): 7, ("daegun_font_ligature_carets", 6): 7,
            ("daegun_font_palette_entry_labels", 2): 4, ("daegun_font_palette_entry_labels", 3): 4}


def accesses(texts):
    problems, n = [], 0
    for header in texts:
        text = re.sub(r"/\*.*?\*/|//[^\n]*", " ", header, flags=re.S)
        text = re.sub(r"^[ \t]*#[ \t]*ifdef[ \t]+__cplusplus.*?^[ \t]*#[ \t]*endif.*?$", " ", text, flags=re.S | re.M)
        text = re.sub(r"^[ \t]*#(?:.*\\\n)*.*$", " ", text, flags=re.M)
        while (bare := re.sub(r"\{[^{}]*\}", " ", text)) != text:
            text = bare
        for decl in text.split(";"):
            hit = re.search(r"\b(daegun_[a-z_0-9]+)\s*\((.*)\)\s*$", decl, re.S)
            if not hit or decl.strip().startswith("typedef"):
                continue
            name, params = hit.group(1), split(hit.group(2))
            marks = [(k, int(a), int(b)) for k, a, b in
                     re.findall(r"\bDAEGUN_(READS|WRITES)\((\d+), (\d+)\)", decl[:hit.start()])]
            # Each parameter's own mark, for MSVC and clang, taken off before its type is read.
            onto = [re.match(r"\s*DAEGUN_(IN|OUT|OUT_OR_NULL)\((\w+)\)", p) for p in params]
            params = [p[m.end():] if m else p for p, m in zip(params, onto)]
            sizes = [(("IN" if kind == "READS" else "OUT"), at, size) for kind, at, size in marks]
            sizes += [("OUT_OR_NULL", at, size) for (of, at), size in OPTIONAL.items() if of == name]
            want = {at: (kind, re.findall(r"\w+", params[size - 1])[-1])
                    for kind, at, size in sizes if 1 <= at <= len(params) and 1 <= size <= len(params)}
            for at, m in enumerate(onto, 1):
                got = m and (m.group(1), m.group(2))
                if got != want.get(at):
                    say = f"DAEGUN_{got[0]}({got[1]})" if got else "no bounds mark"
                    problems.append(f"  ACCESS  {name}: parameter {at} has {say} where its access wants "
                                    + (f"DAEGUN_{want[at][0]}({want[at][1]})" if at in want else "none"))
            for kind, at, size in marks:
                if not (1 <= at <= len(params) and 1 <= size <= len(params)):
                    problems.append(f"  ACCESS  {name}: DAEGUN_{kind}({at}, {size}) names a parameter it lacks")
                    continue
                if (name, at) in OPTIONAL:
                    problems.append(f"  ACCESS  {name}: DAEGUN_{kind}({at}, {size}) on a buffer NULL may stand for")
                pointer, length = " ".join(params[at - 1].split()), " ".join(params[size - 1].split())
                if "*" not in pointer or "(" in pointer or pointer.startswith("const ") != (kind == "READS"):
                    want = "a const pointer" if kind == "READS" else "a pointer to what it writes"
                    problems.append(f"  ACCESS  {name}: DAEGUN_{kind}'s {at} is `{pointer}`, not {want}")
                if not re.fullmatch(r"size_t \w+", length):
                    problems.append(f"  ACCESS  {name}: DAEGUN_{kind}'s {size} is `{length}`, not a size_t")
            sized = {size for _, _, size in marks}
            for i in range(1, len(params)):
                length, before = " ".join(params[i].split()), params[i - 1]
                if (re.fullmatch(r"size_t (?:len|count|\w+_len)", length) and "*" in before and "(" not in before
                        and i + 1 not in sized and name not in UNACCESSED and (name, i) not in OPTIONAL):
                    problems.append(f"  ACCESS  {name}: `{length}` sizes an array no DAEGUN_READS or WRITES names")
            n += len(marks)
    return problems, n


# A view a call returns with its count beside it: clang's bounds safety takes the pointer as one item without the
# mark, so its second doesn't build or traps.
def lent(texts):
    problems, n = [], 0
    for text in texts:
        text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
        calls = r"DAEGUN_API ([^;{}()]*(?:\([^()]*\))?[^;{}()]*)\b(daegun_\w+)\(([^;]*?)\);"
        for ret, name, args in re.findall(calls, text):
            if "size_t *out_count" in args and "*" in ret:
                n += 1
                if "DAEGUN_LENT(*out_count)" not in ret:
                    problems.append(f"  LENT  {name} returns a view without DAEGUN_LENT(*out_count)")
    return problems, n


def main(argv):
    if argv:
        print(USAGE)
        return 0 if argv in (["-h"], ["--help"]) else 1
    texts = headers()
    c, c_structs, marks = c_declarations(texts)
    source = expanded()
    rust, rust_structs_ = rust_definitions(source)
    problems, pairs, structs = compare(c, c_structs, rust, rust_structs_, "the expanded source")
    problems += nullability(marks, Nulls(source)) + regions(texts)
    access_problems, arrays = accesses(texts)
    problems += access_problems
    lent_problems, views = lent(texts)
    problems += lent_problems
    generated, generated_structs, _ = c_declarations([cbindgen(source)])
    more, generated_pairs, _ = compare(c, c_structs, generated, generated_structs, "cbindgen")
    problems += more
    for name in sorted(set(pairs) | set(generated_pairs)):
        if pairs.get(name) != generated_pairs.get(name):
            problems.append(f"  READINGS  {name} is {sorted(pairs.get(name, []))} in the expanded source and "
                            f"{sorted(generated_pairs.get(name, []))} to cbindgen")
    if problems:
        print("issues")
        print("\n".join(problems))
        return 1
    print(f"{len(c)} signatures agree, {structs} structs field for field, {len(pairs)} named types each one Rust type,"
          f" cbindgen reads the Rust the same, and {sum(map(len, marks.values()))} pointers are marked as their calls"
          f" treat NULL; {arrays} arrays and {views} returned views carry their lengths")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
