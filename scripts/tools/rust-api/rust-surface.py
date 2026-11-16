#!/usr/bin/env python3
USAGE = """    rust-surface.py [--write]

The crate's public surface from nightly's rustdoc JSON, one line an item: its kind and path, then its signature, a
field's or constant's type, or a variant's fields; trait implementations, fields, variants and methods included.
Held to rust-surface.txt beside this script, so a change to the API shows in the gate before a release has a
baseline for cargo-semver-checks; --write records the surface as it is. Hidden items are documented so re-exports
out of `daecore` resolve, and `daecore` itself is skipped. Exit status is 1 on any difference."""

import difflib
import json
import os
import subprocess
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", ".."))
RECORD = os.path.join(os.path.dirname(os.path.abspath(__file__)), "rust-surface.txt")
# Markers the compiler implements for its own use, which come and go between nightlies and promise a caller nothing.
INTERNAL = {"StructuralPartialEq", "TrivialClone", "Freeze", "UnsafeUnpin", "Forget", "Move"}


def ty(t):
    if t is None:
        return "()"
    kind, v = next(iter(t.items()))
    if kind in ("primitive", "generic"):
        return v
    if kind == "resolved_path":
        return path(v)
    if kind == "borrowed_ref":
        life = f"{v['lifetime']} " if v["lifetime"] else ""
        return f"&{life}{'mut ' if v['is_mutable'] else ''}{ty(v['type'])}"
    if kind == "raw_pointer":
        return f"*{'mut' if v['is_mutable'] else 'const'} {ty(v['type'])}"
    if kind == "slice":
        return f"[{ty(v)}]"
    if kind == "array":
        return f"[{ty(v['type'])}; {v['len']}]"
    if kind == "tuple":
        return f"({', '.join(map(ty, v))})"
    if kind == "impl_trait":
        return f"impl {bounds(v)}"
    if kind == "dyn_trait":
        traits = [path(p["trait"]) for p in v["traits"]]
        return "dyn " + " + ".join(traits + ([v["lifetime"]] if v.get("lifetime") else []))
    if kind == "qualified_path":
        trait = f" as {path(v['trait'])}" if v.get("trait") else ""
        return f"<{ty(v['self_type'])}{trait}>::{v['name']}"
    if kind == "function_pointer":
        return "fn" + signature(v["sig"])
    return json.dumps(t, sort_keys=True)


def path(p):
    a = p.get("args")
    if not a:
        return p["path"]
    if "parenthesized" in a:
        out = a["parenthesized"].get("output")
        return f"{p['path']}({', '.join(map(ty, a['parenthesized']['inputs']))})" + (f" -> {ty(out)}" if out else "")
    parts = []
    for arg in a["angle_bracketed"]["args"]:
        parts.append(ty(arg["type"]) if "type" in arg else arg.get("lifetime") or json.dumps(arg, sort_keys=True))
    for c in a["angle_bracketed"]["constraints"]:
        binding = c["binding"]
        parts.append(f"{c['name']} = {ty(binding['equality']['type'])}" if "equality" in binding
                     else f"{c['name']}: {bounds(binding['constraint'])}")
    return f"{p['path']}<{', '.join(parts)}>"


def bounds(bs):
    out = []
    for b in bs:
        if "trait_bound" in b:
            out.append(("?" if b["trait_bound"]["modifier"] == "maybe" else "") + path(b["trait_bound"]["trait"]))
        elif "use" in b:
            out.append(f"use<{', '.join(a.get('lifetime') or a.get('param', '') for a in b['use'])}>")
        else:
            out.append(b.get("outlives") or json.dumps(b, sort_keys=True))
    return " + ".join(out)


def generics(g):
    params = []
    for p in g["params"]:
        kind = p["kind"]
        if "lifetime" in kind:
            params.append(p["name"])
        elif "type" in kind and not kind["type"]["is_synthetic"]:
            b = bounds(kind["type"]["bounds"])
            params.append(p["name"] + (f": {b}" if b else ""))
        elif "const" in kind:
            params.append(f"const {p['name']}: {ty(kind['const']['type'])}")
    wheres = [json.dumps(w, sort_keys=True) if "bound_predicate" not in w else
              f"{ty(w['bound_predicate']['type'])}: {bounds(w['bound_predicate']['bounds'])}"
              for w in g["where_predicates"]]
    return (f"<{', '.join(params)}>" if params else ""), (f" where {', '.join(wheres)}" if wheres else "")


def signature(sig):
    inputs = ", ".join(f"{n}: {ty(t)}" for n, t in sig["inputs"])
    return f"({inputs})" + (f" -> {ty(sig['output'])}" if sig["output"] is not None else "")


def function(f):
    h = f["header"]
    marks = "".join(m + " " for m, on in (("const", h["is_const"]), ("unsafe", h["is_unsafe"])) if on)
    params, wheres = generics(f["generics"])
    return f"{marks}fn{params}{signature(f['sig'])}{wheres}"


def surface(doc):
    index, out = doc["index"], set()

    def item(i):
        return index.get(str(i))

    def described(it):
        inner = it["inner"]
        kind = next(iter(inner))
        body = inner[kind]
        if kind == "function":
            return function(body)
        if kind == "struct_field":
            return ty(body)
        if kind in ("assoc_const", "constant"):
            return ty(body["type"])
        if kind == "type_alias":
            return ty(body["type"])
        if kind == "assoc_type":
            return bounds(body["bounds"]) or ""
        if kind == "variant":
            shape = body["kind"]
            if shape == "plain":
                return ""
            if "tuple" in shape:
                return "(" + ", ".join(ty(item(f)["inner"]["struct_field"]) if f is not None else "_"
                                       for f in shape["tuple"]) + ")"
            return "{" + ", ".join(f"{item(f)['name']}: {ty(item(f)['inner']['struct_field'])}"
                                   for f in shape["struct"]["fields"]) + "}"
        return ""

    def add(kind, where, it=None):
        text = described(it) if it else ""
        out.add(f"{kind} {where}" + (f" {text}" if text else ""))

    def members(it, where):
        inner = it["inner"]
        kind = next(iter(inner))
        body = inner[kind]
        if kind == "struct":
            shape = body["kind"]
            fields = shape.get("plain", {}).get("fields", []) if isinstance(shape, dict) else []
            if isinstance(shape, dict) and "tuple" in shape:
                fields = [f for f in shape["tuple"] if f is not None]
            for f in fields:
                fi = item(f)
                if fi and fi.get("visibility") == "public":
                    add("field", f"{where}.{fi['name']}", fi)
        if kind == "enum":
            for v in body["variants"]:
                vi = item(v)
                if vi:
                    add("variant", f"{where}::{vi['name']}", vi)
        if kind == "trait":
            for m in body["items"]:
                mi = item(m)
                if mi:
                    add("trait-item", f"{where}::{mi['name']}", mi)
        for imp in body.get("impls", []) if isinstance(body, dict) else []:
            ii = item(imp)
            if not ii:
                continue
            b = ii["inner"]["impl"]
            if b.get("trait") is not None or b.get("is_synthetic") or b.get("blanket_impl") is not None:
                # An auto trait, Send or UnwindSafe, is a promise as much as a written one.
                if b.get("trait") is not None and b.get("blanket_impl") is None and b["trait"]["path"] not in INTERNAL:
                    kind = "auto" if b.get("is_synthetic") else "impl"
                    # An impl for another type that names this one, as `From<Tag> for u32`, says which.
                    own = b["for"].get("resolved_path", {}).get("id") == it["id"]
                    other = "" if own else ty(b["for"])
                    out.add(f"{kind} {where}: {'!' if b.get('is_negative') else ''}{path(b['trait'])}"
                            + (f" for {other}" if other else ""))
                continue
            for m in b["items"]:
                mi = item(m)
                if mi and mi.get("visibility") == "public":
                    k = next(iter(mi["inner"]))
                    add("method" if k == "function" else k, f"{where}::{mi['name']}", mi)

    def walk(i, where, seen):
        it = item(i)
        if it is None or i in seen:
            return
        seen = seen | {i}
        inner = it["inner"]
        kind = next(iter(inner))
        if kind == "module":
            for c in inner["module"]["items"]:
                ci = item(c)
                hidden = where == "daegun" and ci is not None and ci.get("name") == "daecore"
                if ci is None or ci.get("visibility") != "public" or hidden:
                    continue
                if "use" in ci["inner"]:
                    u = ci["inner"]["use"]
                    target = item(u["id"]) if u["id"] is not None else None
                    if u["is_glob"] and target and "module" in target["inner"]:
                        for g in target["inner"]["module"]["items"]:
                            gi = item(g)
                            if gi and gi.get("visibility") == "public" and gi.get("name"):
                                walk(g, f"{where}::{gi['name']}", seen)
                    elif not u["is_glob"] and u["id"] is not None:
                        walk(u["id"], f"{where}::{u['name']}", seen)
                elif ci.get("name"):
                    walk(c, f"{where}::{ci['name']}", seen)
            out.add(f"module {where}")
            return
        if kind in ("impl", "use"):
            return
        add(kind, where, it)
        members(it, where)

    walk(doc["root"], "daegun", frozenset())
    return sorted(out)


def main(argv):
    if argv not in ([], ["--write"]):
        print(USAGE)
        return 0 if argv in (["-h"], ["--help"]) else 1
    meta = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT, check=True,
                          capture_output=True, text=True)
    target = json.loads(meta.stdout)["target_directory"]
    run = subprocess.run(["cargo", "+nightly", "rustdoc", "-q", "--lib", "--", "-Z", "unstable-options",
                          "--output-format", "json", "--document-hidden-items"], cwd=ROOT, capture_output=True,
                         text=True)
    if run.returncode != 0:
        sys.exit("rustdoc's JSON failed (rustup toolchain install nightly):\n" + run.stderr[-2000:])
    with open(os.path.join(target, "doc", "daegun.json"), encoding="utf-8") as f:
        now = surface(json.load(f))
    if argv == ["--write"]:
        with open(RECORD, "w", encoding="utf-8") as f:
            f.write("\n".join(now) + "\n")
        print(f"wrote {len(now)} items")
        return 0
    with open(RECORD, encoding="utf-8") as f:
        recorded = f.read().splitlines()
    if now == recorded:
        print(f"{len(now)} items as recorded, signatures and trait implementations included")
        return 0
    print("issues (rust-surface.py --write records a change meant to be made)")
    for line in difflib.unified_diff(recorded, now, "recorded", "now", lineterm="", n=0):
        if not line.startswith(("---", "+++", "@@")):
            print("  " + line)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
