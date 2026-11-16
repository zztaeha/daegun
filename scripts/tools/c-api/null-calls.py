#!/usr/bin/env python3
USAGE = """    null-calls.py > null_calls.c

Rule 2 for every export the C headers declare, written out as a C program: each call that takes a pointer is
made with NULL for every pointer and 1 for every number, so each buffer is NULL with a count. One answering a
status must answer DAEGUN_NULL, one returning a pointer must return NULL, and a free must take NULL; the
program prints each that doesn't and exits 1."""

import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
INCLUDE = os.path.join(HERE, "..", "..", "..", "src", "api", "c", "include")


def headers():
    # Every header daegun.h includes, in its order.
    umbrella = open(os.path.join(INCLUDE, "daegun.h"), encoding="utf-8").read()
    names = re.findall(r'^#include <(daegun/\w+\.h)>', umbrella, re.M)
    return "\n".join(open(os.path.join(INCLUDE, n), encoding="utf-8").read() for n in names)


def declarations(header):
    # A callback's calling convention mark sits inside its typedef's parentheses, where it would hide the name.
    text = re.sub(r"/\*.*?\*/|\bDAEGUN_CALL(?:BACK)?\b", " ", header, flags=re.S)
    # The C++ linkage block opens a brace C never sees.
    text = re.sub(r"^[ \t]*#[ \t]*ifdef[ \t]+__cplusplus.*?^[ \t]*#[ \t]*endif.*?$", " ", text, flags=re.S | re.M)
    # A directive runs on past each line it ends with a backslash.
    text = re.sub(r"^[ \t]*#(?:.*\\\n)*.*$", " ", text, flags=re.M)
    text = re.sub(r"\bDAEGUN_LAYOUT_ASSERT\(.*?\);", " ", text, flags=re.S)
    callbacks = set(re.findall(r"typedef[^;{]*\(\s*\*\s*(\w+)\s*\)", text))
    depth, start = 0, 0
    for i, ch in enumerate(text):
        if ch in "{}":
            depth += 1 if ch == "{" else -1
        elif ch == ";" and depth == 0:
            decl, start = " ".join(text[start:i].split()), i + 1
            # The attributes before a return type, those taking arguments with their parentheses,
            # are no part of it, and nor is the nullability region opening before a header's first.
            attributes = (r"\bDAEGUN_(?:MALLOC|ALLOC_SIZE|READS|WRITES|COUNTED_BY|LENT|IN|OUT(?:_OR_NULL)?)"
                          r"\((?:[^()]|\([^()]*\))*\)"
                          r"|\bDAEGUN_(?:NODISCARD|API|CONST|COLD|(?:HEADER|NONNULL)_(?:BEGIN|END))\b")
            decl = re.sub(attributes, "", decl).strip()
            hit = re.fullmatch(r"(.*?)\b(daegun_[a-z_0-9]+)\s*\((.*)\)", decl)
            if hit and not decl.startswith("typedef"):
                yield hit.group(1).strip(), hit.group(2), split(hit.group(3)), callbacks


def split(params):
    out, depth, cur = [], 0, ""
    for ch in params:
        depth += {"(": 1, ")": -1}.get(ch, 0)
        if ch == "," and depth == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += ch
    out.append(cur.strip())
    return [] if out == ["void"] else out


def main():
    if len(sys.argv) > 1:
        sys.exit(USAGE)
    calls = []
    for ret, name, params, callbacks in declarations(headers()):
        pointer = [("*" in p or "[" in p or p.split()[0] in callbacks) for p in params]
        if not any(pointer):
            continue
        args = ", ".join("0" if p else "1" for p in pointer)
        if ret == "daegun_status":
            calls.append(f'    if ({name}({args}) != DAEGUN_NULL) {{ puts("  {name} did not answer DAEGUN_NULL"); '
                         f"failed++; }}")
        elif "*" in ret:
            calls.append(f'    if ({name}({args}) != NULL) {{ puts("  {name} did not return NULL"); failed++; }}')
        elif ret == "void":
            calls.append(f"    {name}({args});")
        else:
            # A return read wrongly would drop its call without a word, as a macro taken for its type once did.
            sys.exit(f"{name} returns {ret}, which rule 2 says nothing of")
    # Every NULL here is deliberate, and clang warns of each a header marks required.
    print('#include "daegun.h"\n\n#include <stdio.h>\n')
    print('#ifdef __clang__\n#pragma clang diagnostic ignored "-Wnonnull"\n#endif\n')
    print("int main(void)\n{\n    int failed = 0;")
    print("\n".join(calls))
    print(f'    printf("  %d of {len(calls)} calls broke rule 2\\n", failed);\n    return failed != 0;\n}}')


main()
