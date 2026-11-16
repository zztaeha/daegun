"""Every status a C call can answer, traced from src/api/c/translation and held to c-statuses.txt, and each failure
given with its reason.

A changed status set fails here until the record is updated, which is when the call's words in its header get
reviewed. From a major's release on, c-statuses-released.txt keeps the record as released, and a call answering a
status it lacks there fails, as base.h promises a minor gives no call a new one.

Usage: c-statuses.py [--write]   (--write rewrites the record from the trace)
"""

import os
import re
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".."))
INCLUDE = os.path.join(ROOT, "src", "api", "c", "include")
TRANSLATION = os.path.join(ROOT, "src", "api", "c", "translation")
RECORD = os.path.join(ROOT, "scripts", "tools", "c-api", "c-statuses.txt")
RELEASED = os.path.join(ROOT, "scripts", "tools", "c-api", "c-statuses-released.txt")

CONST = re.compile(r"^#define (DAEGUN_[A-Z0-9_]+)\s+\S", re.M)
# The headers' own machinery, which no caller uses as a value: linkage, calling convention, attributes, nullability,
# layout checks and include guards.
MACHINERY = re.compile(
    r"DAEGUN_(?:API|CALL(?:BACK)?|NODISCARD|COLD|NULLABLE|NONNULL|(?:HEADER|NONNULL)_(?:BEGIN|END)|LAYOUT_\w+"
    r"|(?:\w+_)?H)$")
# Every status but OK, NULL and ABSENT is a failure, which must leave its reason in daegun_last_error.
FAILURE = r"Status::(?:Malformed|InvalidArgument|Unsupported|Missing|LimitExceeded|NoOutline|Overflow)\b"


def block(s, i):
    depth = 1
    while depth:
        depth += (s[i] == "{") - (s[i] == "}")
        i += 1
    return i


def headers():
    """Every header daegun.h includes, in its order."""
    umbrella = open(os.path.join(INCLUDE, "daegun.h"), encoding="utf-8").read()
    names = re.findall(r'^#include <(daegun/\w+\.h)>', umbrella, re.M)
    return [open(os.path.join(INCLUDE, n), encoding="utf-8").read() for n in names]


def declared():
    """Each function the headers declare, with whether it returns a status, and each constant they define."""
    functions, constants = {}, set()
    for header in headers():
        constants.update(c for c in CONST.findall(header) if not MACHINERY.match(c))
        text = re.sub(r"/\*.*?\*/", " ", header, flags=re.S)
        text = re.sub(r"^[ \t]*#[ \t]*ifdef[ \t]+__cplusplus.*?^[ \t]*#[ \t]*endif.*?$", " ", text, flags=re.S | re.M)
        text = re.sub(r"^[ \t]*#(?:.*\\\n)*.*$", " ", text, flags=re.M)
        text = re.sub(r"\bDAEGUN_LAYOUT_ASSERT\(.*?\);", " ", text, flags=re.S)
        text = re.sub(r"\bDAEGUN_(?:MALLOC|ALLOC_SIZE|READS|WRITES|COUNTED_BY|LENT|IN|OUT(?:_OR_NULL)?)"
                      r"\((?:[^()]|\([^()]*\))*\)"
                      r"|\bDAEGUN_(?:NODISCARD|API|CALL(?:BACK)?|CONST|COLD|(?:HEADER|NONNULL)_(?:BEGIN|END))\b",
                      " ", text)
        depth, start = 0, 0
        for i, ch in enumerate(text):
            if ch in "{}":
                depth += 1 if ch == "{" else -1
            elif ch == ";" and depth == 0:
                decl, start = " ".join(text[start:i].split()), i + 1
                hit = re.match(r"(.*?)\b(daegun_[a-z_0-9]+)\s*\(", decl)
                if hit and not decl.startswith("typedef"):
                    functions[hit.group(2)] = hit.group(1).strip() == "daegun_status"
    return functions, constants


def traced():
    """Each export's statuses, followed through the C layer's functions, methods and export macros to a fixpoint,
    test code left out: N null, A absent, I an invalid argument, M malformed bytes, O an options struct's size, F a
    failure of the Rust call, its kind's status; r a failure with no reason."""
    src = ""
    for f in sorted(os.listdir(TRANSLATION)):
        if f.endswith(".rs"):
            s = open(os.path.join(TRANSLATION, f), encoding="utf-8").read()
            # A name in a comment is no call; the layer holds no `//` inside a string.
            s = re.sub(r"//[^\n]*", "", s)
            while m := re.search(r"#\[cfg\(test\)\]\s*(?:pub(?:\([^)]*\))?\s+)?mod \w+ \{", s):
                s = s[: m.start()] + s[block(s, m.end()) :]
            src += s

    helper = r"(?:(?<![\w.:])|(?<![A-Z]\w)(?<![A-Z])::)"
    kinds_of = {"N": r"Status::Null", "A": r"Status::Absent", "I": helper + r"invalid\(", "r": FAILURE}
    # The helpers that set a reason, each taken as what it answers rather than read for its bare statuses.
    fixed = {("f", "", "failed"): {"F"}, ("f", "", "invalid"): {"I"}, ("f", "", "past_end"): {"I"},
             ("f", "", "malformed"): {"M"}, ("f", "", "sized"): {"O"},
             ("f", "", "too_small"): {"I"}}

    impls = []
    for m in re.finditer(r"\nimpl\b([^{;]*)\{", src):
        head = re.sub(r"<[^<>]*(?:<[^<>]*>[^<>]*)*>", "", m.group(1)).strip()
        impls.append((m.start(), block(src, m.end()), re.split(r"\s+for\s+", head)[-1].split("::")[-1].strip()))

    def owner(i):
        return next((ty for a, b, ty in impls if a <= i < b), None)

    bodies, locals = {}, {}
    fn = r"\n\s*(?:pub(?:\(crate\))? )?(?:const )?(?:unsafe )?(?:extern \"C\" )?fn ([a-z_0-9]+)([^{;]*)\{"
    for m in re.finditer(fn, src):
        ty = owner(m.start())
        key = ("m", ty, m.group(1)) if ty else ("f", "", m.group(1))
        body = src[m.end() : block(src, m.end())]
        bodies.setdefault(key, []).append(body)
        # A parameter, `let` binding or closure parameter of the same name as a C-layer function is a value, not it.
        bound = re.findall(r"(\w+)\s*:", m.group(2)) + re.findall(r"\|([^|]*)\|", body)
        bound += re.findall(r"\blet\s+(?:mut\s+)?(\(?[^=:]*)", body)
        locals.setdefault(key, set()).update(re.findall(r"\b[a-z_][a-z_0-9]*\b", " ".join(bound)))
    for m in re.finditer(r"macro_rules! (\w+) \{", src):
        mac = src[m.end() : block(src, m.end())]
        for inv in re.finditer(r"\n" + m.group(1) + r"!\((daegun_\w+)", src):
            bodies.setdefault(("f", "", inv.group(1)), []).append(mac)
            locals.setdefault(("f", "", inv.group(1)), set())
    bodies = {k: "\n".join(b) for k, b in bodies.items()}

    def called(body, key, bound):
        kind, ty, name = key
        # Called, or passed as a value: `.map_err(failed)`, `.map(Text::new)`.
        pattern = r"(\.|(?<![\w.])(?:[A-Za-z_]\w*::)*)" + name + r"(?:\s*(?:::<[^>]*>)?(\()|(?=\s*[),]))"
        for m in re.finditer(pattern, body):
            path = m.group(1)
            if not m.group(2) and not path and name in bound:
                continue
            last = path.split("::")[-2] if path.endswith("::") else ""
            if kind == "f" and path != "." and not last[:1].isupper():
                return True
            if kind == "m" and (path == "." or last in (ty, "Self")):
                return True
        return False

    calls = {k: {c for c in bodies if c != k and called(b, c, locals[k])} for k, b in bodies.items()}
    kinds = {k: fixed.get(k) or {kk for kk, p in kinds_of.items() if re.search(p, b)} for k, b in bodies.items()}
    changed = True
    while changed:
        changed = False
        for k in bodies:
            if k not in fixed:
                new = set().union(kinds[k], *(kinds[c] for c in calls[k]))
                if new != kinds[k]:
                    kinds[k], changed = new, True
    return {k[2]: "".join(c for c in "NAIMOFr" if c in v) for k, v in kinds.items() if k[0] == "f"}


def read(path):
    if not os.path.exists(path):
        return {}
    return dict(line.split() for line in open(path, encoding="utf-8") if line.strip() and not line.startswith("#"))


def main(argv):
    functions, _ = declared()
    trace = traced()
    sets = {}
    for name, returns_status in sorted(functions.items()):
        sets[name] = (trace.get(name) or "-") if returns_status else "-"

    lines = ["# Each C export and the statuses it can answer: N null, A absent, I invalid argument, M malformed,",
             "# O an options struct's size, F the Rust call's failure; each failure with its reason, and - for a",
             "# call that returns no status. Written by c-statuses.py --write from src/api/c/translation."]
    lines += [f"{n} {s}" for n, s in sorted(sets.items())]
    if "--write" in argv:
        open(RECORD, "w", encoding="utf-8").write("\n".join(lines) + "\n")
        print(f"wrote {len(sets)} statuses")
        return 0

    problems = []
    record, released = read(RECORD), read(RELEASED)
    for name, s in sorted(released.items()):
        new = sorted(set(sets.get(name, "")) - set(s) - {"-"})
        if new:
            problems.append(f"  NEW STATUS  {name} can answer {''.join(new)}, which it couldn't as released ({s})")
    for name in sorted(set(sets) | set(record)):
        if sets.get(name) != record.get(name):
            problems.append(f"  STATUSES  {name} traces {sets.get(name, 'nothing')}, the record says "
                            f"{record.get(name, 'nothing')}: review its words in its header, then --write")
        if "r" in sets.get(name, ""):
            problems.append(f"  NO REASON  {name} can answer a failure without giving its reason")
        if functions.get(name) and name not in trace:
            problems.append(f"  UNTRACED  {name} returns a status the trace can't follow")

    if problems:
        print("issues")
        print("\n".join(problems))
        return 1
    print(f"{sum(1 for s in sets.values() if s != '-')} status sets held to the record, each failure with its reason")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
