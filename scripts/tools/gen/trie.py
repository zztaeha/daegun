from rows import wrapped

SEARCH_BITS = range(3, 10)
MAX_CP = 0x110000

def _dedup(values, size):
    seen, indices, flat = {}, [], []
    for b in range(0, len(values), size):
        key = tuple(values[b:b + size])
        i = seen.get(key)
        if i is None:
            i = len(seen)
            seen[key] = i
            flat.extend(key)
        indices.append(i)
    return indices, flat

def _bytes(top, mid, leaf):
    return sum(len(a) * (1 if max(a, default=0) < 256 else 2) for a in (top, mid, leaf))

def build(columns, defaults):
    cols = []
    for ranges, default in zip(columns, defaults):
        col = [default] * MAX_CP
        for entry in ranges:
            lo, hi, value = entry[0], entry[1], entry[2]
            for cp in range(lo, min(hi + 1, MAX_CP)):
                col[cp] = value
        cols.append(col)

    records, ids = {}, [0] * MAX_CP
    for cp in range(MAX_CP):
        rec = tuple(c[cp] for c in cols)
        i = records.get(rec)
        if i is None:
            i = len(records)
            records[rec] = i
        ids[cp] = i
    assert len(records) <= 0xFFFF, "records overflowed a u16 index: {}".format(len(records))

    best = None
    for leaf_bits in SEARCH_BITS:
        mid_ids, leaf = _dedup(ids, 1 << leaf_bits)
        for mid_bits in SEARCH_BITS:
            if leaf_bits + mid_bits > 16:
                continue
            top, mid = _dedup(mid_ids, 1 << mid_bits)
            if max(len(leaf) >> leaf_bits, len(mid) >> mid_bits) > 0xFFFF:
                continue
            size = _bytes(top, mid, leaf)
            if best is None or size < best[0]:
                best = (size, leaf_bits, mid_bits, top, mid, leaf)
    assert best is not None, "no block shape fit a u16 index"
    _, leaf_bits, mid_bits, out_top, out_mid, out_leaf = best
    out_records = sorted(records, key=records.get)

    leaf_mask, mid_mask = (1 << leaf_bits) - 1, (1 << mid_bits) - 1
    for cp in range(MAX_CP):
        mid = out_top[cp >> (leaf_bits + mid_bits)]
        leaf = out_mid[(mid << mid_bits) | ((cp >> leaf_bits) & mid_mask)]
        got = out_records[out_leaf[(leaf << leaf_bits) | (cp & leaf_mask)]]
        want = tuple(c[cp] for c in cols)
        assert got == want, "trie disagrees at U+{:04X}: {} vs {}".format(cp, got, want)

    return out_records, out_top, out_mid, out_leaf, leaf_bits, mid_bits

def width_for(values):
    return "u8" if max(values, default=0) < 256 else "u16"

def emit_stage(f, name, values, comment, per_line=16):
    f.write("    // {}\n    {}: &[\n".format(comment, name))
    for i in range(0, len(values), per_line):
        f.write("        " + " ".join("{},".format(v) for v in values[i:i + per_line]) + "\n")
    f.write("    ],\n")

# Writes `record`, a struct of `fields`, and `name`, a trie of one for every code point. Each value is written as it
# is, so a column may hold Rust expressions.
def emit(f, name, record, fields, columns, defaults, doc):
    records, top, mid, leaf, leaf_bits, mid_bits = build(columns, defaults)

    f.write(doc)
    f.write("#[derive(Clone, Copy)]\npub(crate) struct {} {{\n".format(record))
    for field, ty in fields:
        f.write("    pub(crate) {}: {},\n".format(field, ty))
    f.write("}\n\n")

    f.write("pub(crate) static {}: Trie<{}, {}, {}, {}> = Trie {{\n".format(
        name, record, width_for(top), width_for(mid), width_for(leaf)))
    f.write("    leaf_bits: {},\n    mid_bits: {},\n    records: &[\n".format(leaf_bits, mid_bits))
    for rec in records:
        parts = ["{}: {},".format(n, v) for (n, _), v in zip(fields, rec)]
        one = "        {} {{ {} }},".format(record, " ".join(parts)[:-1])
        if len(one) <= 120:
            f.write(one + "\n")
        else:
            f.write("        {} {{\n{}\n        }},\n".format(record, "\n".join(wrapped(" " * 12, parts))))
    f.write("    ],\n")
    emit_stage(f, "top", top, "`cp >> {}` to a mid block.".format(leaf_bits + mid_bits))
    emit_stage(f, "mid", mid, "A mid block plus `(cp >> {}) & {}` to a leaf block.".format(
        leaf_bits, (1 << mid_bits) - 1))
    emit_stage(f, "leaf", leaf, "A leaf block plus `cp & {}` to a record.".format((1 << leaf_bits) - 1))
    f.write("};\n\n")

    return len(records), len(mid) >> mid_bits, len(leaf) >> leaf_bits
