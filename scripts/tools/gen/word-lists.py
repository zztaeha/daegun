#!/usr/bin/env python3
# Builds the word lists line breaking divides Thai, Lao, Khmer and Myanmar text with, from ICU's dictionaries
# (icu4c/source/data/brkitr/dictionaries at release-78.3). Each file, little-endian: the base code point, the bucket
# count, each bucket's offset, then the words, a byte a character (code point less base, U+200C and U+200D as 0xFE and
# 0xFF), sorted and front coded as shared length, rest length, rest, each bucket's first word whole.

import os
import struct
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "..", "..", "data", "words")
UNICODE_LICENSE = os.path.join(HERE, "..", "..", "data", "unicode", "UnicodeLicense.txt")
OUT = os.path.join(HERE, "..", "..", "..", "src", "daecore", "src", "daeshaper", "generated")

BUCKET = 16

# (name in the code, source file, the base of its byte alphabet), as ICU's data build gives gendict.
LISTS = [
    ("THAI", "thaidict.txt", 0x0E00),
    ("LAO", "laodict.txt", 0x0E80),
    ("KHMER", "khmerdict.txt", 0x1780),
    ("MYANMAR", "burmesedict.txt", 0x1000),
]


def read(path):
    # gendict's reading: a '#' starts a comment, trailing space goes, and a word ends at the first space.
    header, words, in_header = [], [], True
    with open(path, encoding="utf-8-sig") as f:
        for line in f:
            line = line.rstrip("\r\n")
            if in_header and line.lstrip().startswith("#"):
                header.append(line.lstrip()[1:].strip())
                continue
            in_header = False
            line = line.split("#", 1)[0].rstrip()
            if line:
                if line[0].isspace():
                    sys.exit("{}: a line starts with a space: {!r}".format(path, line))
                words.append(line.split()[0])
    return header, words


def encode(word, base):
    out = bytearray()
    for c in word:
        cp = ord(c)
        if cp == 0x200D:
            out.append(0xFF)
        elif cp == 0x200C:
            out.append(0xFE)
        elif 0 <= cp - base <= 0xFD:
            out.append(cp - base)
        else:
            sys.exit("U+{:04X} in {!r} is outside the alphabet at U+{:04X}".format(cp, word, base))
    return bytes(out)


def build(words, base):
    keys = sorted(encode(w, base) for w in words)
    if any(a == b for a, b in zip(keys, keys[1:])):
        sys.exit("a word is listed twice")
    if any(len(k) > 255 for k in keys):
        sys.exit("a word is longer than 255 characters")
    body, offsets, prev = bytearray(), [], b""
    for i, key in enumerate(keys):
        if i % BUCKET == 0:
            offsets.append(len(body))
            prev = b""
        shared = 0
        while shared < min(len(key), len(prev)) and key[shared] == prev[shared]:
            shared += 1
        body += bytes([shared, len(key) - shared]) + key[shared:]
        prev = key
    head = struct.pack("<II", base, len(offsets)) + b"".join(struct.pack("<I", o) for o in offsets)
    return head + bytes(body), keys


def decode(data):
    base, count = struct.unpack_from("<II", data, 0)
    body = data[8 + 4 * count:]
    keys, at, prev = [], 0, b""
    while at < len(body):
        shared, rest = body[at], body[at + 1]
        prev = prev[:shared] + body[at + 2:at + 2 + rest]
        keys.append(prev)
        at += 2 + rest
    return base, keys


def comment(lines):
    return "".join("//{}\n".format(" " + l if l else "") for l in lines)


def main():
    with open(UNICODE_LICENSE, encoding="utf-8") as f:
        unicode_license = f.read().strip().splitlines()
    os.makedirs(os.path.join(OUT, "word_lists"), exist_ok=True)
    notices, statics, total = [], [], 0
    for name, source, base in LISTS:
        header, words = read(os.path.join(DATA, source))
        data, keys = build(words, base)
        if decode(data) != (base, keys):
            sys.exit("{} does not decode to its words".format(source))
        with open(os.path.join(OUT, "word_lists", name.lower() + ".bin"), "wb") as f:
            f.write(data)
        notices += ["", "{} ({:,} words):".format(name.capitalize(), len(keys)), ""] + header
        statics.append('pub(crate) static {}: &[u8] = include_bytes!("word_lists/{}.bin");\n'.format(
            name, name.lower()))
        total += len(data)
        print("  {:8} {:6} words, {:7} bytes".format(name.lower(), len(keys), len(data)))
    print("  total    {:7} bytes".format(total))

    # The notices in CREDITS.txt too, between this script's marks, for a C library built with the lists.
    credits = os.path.join(HERE, "..", "..", "..", "CREDITS.txt")
    begin, end = "[word-lists.py writes from here]\n", "[word-lists.py writes to here]\n"
    with open(credits, encoding="utf-8") as f:
        head, rest = f.read().split(begin)
    _, tail = rest.split(end)
    with open(credits, "w", encoding="utf-8") as f:
        f.write(head + begin + "\n".join(notices[1:]) + "\n" + end + tail)

    with open(os.path.join(OUT, "word_lists.rs"), "w", encoding="utf-8") as f:
        f.write("// GENERATED FILE – do not edit by hand.\n")
        f.write("// Regenerate with: python3 scripts/tools/gen/word-lists.py\n")
        f.write("// Source: the Thai, Lao, Khmer and Myanmar word lists in scripts/data/words\n")
        f.write("//\n")
        f.write("// The notices below are required by the licenses the lists are used under: each list's header,\n")
        f.write("// then the Unicode license from scripts/data/unicode/UnicodeLicense.txt. They cover the word lists\n")
        f.write("// and the data built from them, not daegun's own code, which is MIT (see LICENSE).\n")
        f.write(comment(notices))
        f.write("//\n")
        f.write(comment(unicode_license))
        f.write("\n")
        f.write("// Each list as word-lists.py lays it out: its alphabet's base, its buckets' offsets, "
                "then its words\n")
        f.write("// sorted and front coded, sixteen to a bucket, each bucket's first written whole.\n")
        for s in statics:
            f.write(s)


if __name__ == "__main__":
    main()
