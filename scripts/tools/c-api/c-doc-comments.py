"""Each header comment that documents a declaration is a doc comment, which clang attaches to it for Xcode's Quick
Help, SourceKit and Swift's generated interface: one directly above a function, type or field opens /**, a field's
trailing one /**<. Sections (a blank line after), preprocessor machinery, macros and the layout checks stay plain.

Usage: c-doc-comments.py [--write]   (--write makes each such comment a doc comment)
"""

import os
import re
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".."))
INCLUDE = os.path.join(ROOT, "src", "api", "c", "include", "daegun")

PLAIN_NEXT = re.compile(r"\s*(#|DAEGUN_LAYOUT_ASSERT|DAEGUN_(?:HEADER|NONNULL)_|extern\s+\"C\"|/[*/]|}\s*$)")
FIELD = re.compile(r"^(\s+[^/#\s][^;]*;\s*)/\*(?![*!])(.*)$")


def plain(lines):
    """Each plain comment that documents a declaration: its line index, and the line as a doc comment."""
    found, depth, i = [], 0, 0
    while i < len(lines):
        line = lines[i]
        if re.match(r"^\s*/\*(?![*!])", line):
            end = i
            while "*/" not in lines[end]:
                end += 1
            after = lines[end + 1] if end + 1 < len(lines) else ""
            if after.strip() and not PLAIN_NEXT.match(after):
                found.append((i, line.replace("/*", "/**", 1)))
            i = end + 1
            continue
        field = FIELD.match(line)
        if field and depth > 0:
            found.append((i, field.group(1) + "/**<" + field.group(2)))
        code = re.sub(r"/\*.*?\*/", "", line)
        # extern "C"'s braces, the file's own, open and close no struct.
        closing = i > 0 and lines[i - 1].strip() == "#ifdef __cplusplus" and code.strip() == "}"
        if not re.search(r'extern\s+"C"', code) and not closing:
            depth += code.count("{") - code.count("}")
        i += 1
    return found


def main(argv):
    problems, total = [], 0
    for name in sorted(os.listdir(INCLUDE)):
        path = os.path.join(INCLUDE, name)
        lines = open(path, encoding="utf-8").read().split("\n")
        found = plain(lines)
        if "--write" in argv:
            for i, doc in found:
                lines[i] = doc
            open(path, "w", encoding="utf-8").write("\n".join(lines))
        else:
            problems += [f"  PLAIN  {name}:{i + 1} documents a declaration: make it /** or /**<" for i, _ in found]
        total += sum(1 for line in lines if "/**" in line)
    if problems:
        print("issues")
        print("\n".join(problems))
        return 1
    print(f"{total} doc comments, none documenting a declaration left plain")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
