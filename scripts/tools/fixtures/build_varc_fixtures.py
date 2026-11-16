# The VARC fixtures, into the directory given: 가 and 각 from jamo, a CFF2 short of maxp's glyphs, and crafted fonts for
# what a well-made font never reaches. fontTools' VARC reader predates the revised store, so VARC goes in as bytes.
# Usage: python3 -I scripts/tools/fixtures/build_varc_fixtures.py assets/test-fonts/varc
import math
import os
import struct
import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib.tables.TupleVariation import TupleVariation

AXES = [('wght', 100, 400, 900), ('ZZZZ', -1, 0, 1)]


def u8(v): return struct.pack('>B', v)
def u16(v): return struct.pack('>H', v)
def i16(v): return struct.pack('>h', v)
def u24(v): return struct.pack('>I', v)[1:]
def u32(v): return struct.pack('>I', v)


def var32(v):
    if v < 0x80:
        return u8(v)
    if v < 0x4000:
        return u16(0x8000 | v)
    if v < 0x200000:
        return u24(0xC00000 | v)
    if v < 0x10000000:
        return u32(0xE0000000 | v)
    return u8(0xF0) + u32(v)


def tuples(values, width=None):
    # TupleValues, one run per call unless the values need a wider one: width 0 zeros, 1, 2 or 4 bytes.
    out = b''
    for i in range(0, len(values), 64):
        run = values[i:i + 64]
        w = width if width is not None else (
            0 if all(v == 0 for v in run) else 1 if all(-128 <= v < 128 for v in run) else
            2 if all(-32768 <= v < 32768 for v in run) else 4)
        control = {0: 0x80, 1: 0x00, 2: 0x40, 4: 0xC0}[w] | (len(run) - 1)
        fmt = {1: '>b', 2: '>h', 4: '>i'}.get(w)
        out += u8(control) + (b''.join(struct.pack(fmt, v) for v in run) if fmt else b'')
    return out


def index(items):
    data = b''.join(items)
    offsets = [1]
    for it in items:
        offsets.append(offsets[-1] + len(it))
    size = 1 if offsets[-1] < 0x100 else 2 if offsets[-1] < 0x10000 else 3 if offsets[-1] < 0x1000000 else 4
    return u32(len(items)) + u8(size) + b''.join(v.to_bytes(size, 'big') for v in offsets) + data


FLAG = dict(RESET=1, AXES=2, AXES_VAR=4, TRANSFORM_VAR=8, TX=16, TY=32, ROT=64, COND=128, SX=256, SY=512, CX=1024,
            CY=2048, GID24=4096, SKX=8192, SKY=16384)
ORDER = ['TX', 'TY', 'ROT', 'SX', 'SY', 'SKX', 'SKY', 'CX', 'CY']


def component(gid, cond=None, axes=None, axes_var=None, transform_var=None, reserved=(), gid24=False, **fields):
    # `axes` is (axis indices entry, values, run width or None); `fields` the transform's raw 16-bit values.
    flags = sum(FLAG[k] for k in fields)
    flags |= (FLAG['COND'] if cond is not None else 0) | (FLAG['AXES'] if axes else 0)
    flags |= FLAG['AXES_VAR'] if axes_var is not None else 0
    flags |= FLAG['TRANSFORM_VAR'] if transform_var is not None else 0
    flags |= FLAG['GID24'] if gid24 else 0
    flags |= fields.pop('RESET', 0) and FLAG['RESET']
    for bit in reserved:
        flags |= 1 << bit
    out = var32(flags) + (u24(gid) if gid24 else u16(gid))
    if cond is not None:
        out += var32(cond)
    if axes:
        out += var32(axes[0]) + tuples(axes[1], axes[2])
    if axes_var is not None:
        out += var32(axes_var)
    if transform_var is not None:
        out += var32(transform_var)
    for k in ORDER:
        if k in fields:
            out += i16(fields[k])
    for bit in reserved:
        out += var32(bit * 1000)
    return out


def store(regions, sets):
    # Regions as lists of (axis, start, peak, end) in 2.14; sets as (region indices, rows of deltas per region).
    reg = []
    for r in regions:
        head = u16(len(r))
        body = b''.join(u16(a) + i16(s) + i16(p) + i16(e) for a, s, p, e in r)
        head += b''.join(u32(2 + 4 * len(r) + 8 * i) for i in range(len(r)))
        reg.append(head + body)
    lst = u32(len(reg))
    at = 4 + 4 * len(reg)
    for r in reg:
        lst += u32(at)
        at += len(r)
    lst += b''.join(reg)
    datas = []
    for indices, rows in sets:
        head = u8(1) + u16(len(indices)) + b''.join(u16(i) for i in indices)
        head += u32(len(head) + 4)
        datas.append(head + index([tuples([v for region in row for v in region]) for row in rows]))
    head_len = 2 + 4 + 2 + 4 * len(datas)
    out = u16(1) + u32(head_len) + u16(len(datas))
    at = head_len + len(lst)
    for d in datas:
        out += u32(at)
        at += len(d)
    return out + lst + b''.join(datas)


def condition_list(conds):
    out = u32(len(conds))
    at = 4 + 4 * len(conds)
    for c in conds:
        out += u32(at if c is not None else 0)
        at += len(c or b'')
    return out + b''.join(c or b'' for c in conds)


def cond_range(axis, lo, hi): return u16(1) + u16(axis) + i16(lo) + i16(hi)
def cond_value(default, var): return u16(2) + i16(default) + u32(var)


def cond_group(fmt, children):
    head = u16(fmt) + u8(len(children))
    at = len(head) + 3 * len(children)
    out, body = head, b''
    for c in children:
        out += u24(at + len(body) if c is not None else 0)
        body += c or b''
    return out + body


def cond_negate(child): return u16(5) + u24(5) + child


def varc(coverage_gids, records, conditions=None, axis_indices=None, var_store=None, coverage=None):
    cov = coverage or (u16(1) + u16(len(coverage_gids)) + b''.join(u16(g) for g in coverage_gids))
    parts = [cov, var_store or b'', conditions or b'', axis_indices or b'', index(records)]
    out, at = b'', 24
    for p in parts:
        out += u32(at if p else 0)
        at += len(p)
    return u32(0x10000) + out + b''.join(parts)


def glyph(points, offs=()):
    pen = TTGlyphPen(None)
    pen.moveTo(points[0])
    for k, p in enumerate(points[1:], 1):
        if k in offs:
            continue
        if k - 1 in offs:
            pen.qCurveTo(points[k - 1], p)
        else:
            pen.lineTo(p)
    pen.closePath()
    return pen.glyph()


def base(path, extra_varc_glyphs):
    names = ['.notdef', 'sq', 'qd', 'big', 'tri'] + [f'v{i}' for i in range(extra_varc_glyphs)]
    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder(names)
    fb.setupCharacterMap({})
    big = [(round(400 + 300 * math.cos(k / 180)), round(400 + 300 * math.sin(k / 180))) for k in range(1000)]
    glyphs = {
        '.notdef': TTGlyphPen(None).glyph(),
        'sq': glyph([(0, 0), (100, 0), (100, 100), (0, 100)]),
        'qd': glyph([(0, 0), (60, 120), (140, 130), (200, 0)], offs=(1,)),
        'big': glyph(big),
        'tri': glyph([(10, 10), (90, 10), (50, 80)]),
    }
    for i in range(extra_varc_glyphs):
        glyphs[f'v{i}'] = glyph([(-5, -5), (-25, -5), (-15, -30)])
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({n: (500, 0) for n in names})
    fb.setupHorizontalHeader()
    fb.setupNameTable({'familyName': 'VarcHostile', 'styleName': 'Regular'})
    fb.setupOS2()
    fb.setupPost()
    fb.setupFvar(axes=[(t, lo, d, hi, t) for t, lo, d, hi in AXES], instances=[])
    phantoms = [(0, 0)] * 4
    fb.setupGvar({
        'sq': [TupleVariation({'wght': (0, 1, 1)}, [(0, 0), (60, 0), (60, 20), (0, 20)] + phantoms),
               TupleVariation({'ZZZZ': (-1, -1, 0)}, [(7, 3), (0, 9), (-11, 0), (0, -13)] + phantoms)],
        'qd': [TupleVariation({'ZZZZ': (0, 1, 1)}, [(0, 0), (33, 17), (21, -9), (0, 5)] + phantoms)],
    })
    fb.save(path)


def read_table(path, tag):
    d = open(path, 'rb').read()
    for i in range(struct.unpack('>H', d[4:6])[0]):
        t, _, off, ln = struct.unpack('>4sIII', d[12 + 16 * i:28 + 16 * i])
        if t == tag.encode():
            return d[off:off + ln]
    raise KeyError(tag)


def splice(path, tag, data):
    d = open(path, 'rb').read()
    n = struct.unpack('>H', d[4:6])[0]
    tabs = {}
    for i in range(n):
        t, _, off, ln = struct.unpack('>4sIII', d[12 + 16 * i:28 + 16 * i])
        tabs[t] = d[off:off + ln]
    tabs[tag.encode()] = data
    tags = sorted(tabs)
    out = d[:4] + u16(len(tags)) + d[6:12]
    at = 12 + 16 * len(tags)
    body = b''
    for t in tags:
        out += t + u32(0) + u32(at + len(body)) + u32(len(tabs[t]))
        body += tabs[t] + b'\0' * (-len(tabs[t]) % 4)
    open(path, 'wb').write(out + body)


def contours(*cs):
    pen = TTGlyphPen(None)
    for c in cs:
        pen.moveTo(c[0])
        for p in c[1:]:
            pen.lineTo(p)
        pen.closePath()
    return pen.glyph()


def hangul(out):
    # 가 and 각 from the jamo ㄱ and ㅏ, glyf leaves whose strokes widen 40 on wght. 각's ㅏ and final ㄱ move on wght
    # too, 80 across and 40 up, so a location varies both the leaves and the transforms.
    path = os.path.join(out, 'varc-hangul.ttf')
    names = ['.notdef', 'uniAC00', 'uniAC01', 'giyeok', 'a']
    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder(names)
    fb.setupCharacterMap({0xAC00: 'uniAC00', 0xAC01: 'uniAC01'})
    fb.setupGlyf({'.notdef': contours(), 'uniAC00': contours(), 'uniAC01': contours(),
                  'giyeok': contours([(0, 600), (400, 600), (400, 0), (300, 0), (300, 500), (0, 500)]),
                  'a': contours([(0, 0), (0, 800), (100, 800), (100, 0)],
                                [(100, 320), (100, 480), (250, 480), (250, 320)])})
    fb.setupHorizontalMetrics({n: (1000, 0) for n in names})
    fb.setupHorizontalHeader()
    fb.setupNameTable({'familyName': 'VarcHangul', 'styleName': 'Regular'})
    fb.setupOS2()
    fb.setupPost()
    fb.setupFvar(axes=[('wght', 400, 400, 900, 'wght')], instances=[])
    phantoms = [(0, 0)] * 4
    fb.setupGvar({
        'giyeok': [TupleVariation({'wght': (0, 1, 1)}, [(0, 0), (40, 0), (40, 0), (0, 0), (0, 0), (0, 0)] + phantoms)],
        'a': [TupleVariation({'wght': (0, 1, 1)}, [(0, 0), (0, 0)] + [(40, 0)] * 6 + phantoms)],
    })
    fb.save(path)
    giyeok, a = 3, 4
    var_store = store([[(0, 0, 16384, 16384)]], [([0], [[[80, 0, 0, 0]], [[0, 40, 0, 0]]])])
    records = [component(giyeok, TX=50, TY=100) + component(a, TX=550),
               component(giyeok, TX=50, TY=450, SX=1024, SY=512)
               + component(a, transform_var=0, TX=550, TY=100, SX=1024, SY=896)
               + component(giyeok, transform_var=1, TX=100, TY=0, SX=1280, SY=256)]
    splice(path, 'VARC', varc([1, 2], records, var_store=var_store))
    return path


def short_cff2(out):
    # maxp counts 4 glyphs and CharStrings hold 3, the last empty: past them, glyph 3 is VARC's, the triangle of glyph 1
    # scaled by 2 and moved 400 across.
    from fontTools.misc.psCharStrings import T2CharString
    path = os.path.join(out, 'varc-short-cff2.otf')
    names = ['.notdef', 'tri', 'empty']
    fb = FontBuilder(1000, isTTF=False)
    fb.setupGlyphOrder(names)
    fb.setupCharacterMap({})
    fb.setupNameTable({'familyName': 'VarcShortCff2', 'styleName': 'Regular'})
    tri = [100, 'hmoveto', 100, 0, -100, 100, 'rlineto']
    fb.setupCFF2({'.notdef': T2CharString(program=[]), 'tri': T2CharString(program=tri),
                  'empty': T2CharString(program=[])})
    fb.setupHorizontalMetrics({n: (500, 0) for n in names})
    fb.setupHorizontalHeader()
    fb.setupOS2()
    fb.setupPost(keepGlyphNames=False)
    fb.save(path)
    hhea = read_table(path, 'hhea')
    splice(path, 'maxp', u32(0x5000) + u16(4))
    splice(path, 'hhea', hhea[:34] + u16(4))
    splice(path, 'hmtx', (u16(500) + i16(0)) * 4)
    splice(path, 'VARC', varc([3], [component(1, TX=400, SX=2048)]))
    return path


SQ, QD, BIG, TRI, V = 1, 2, 3, 4, 5


def hostile(out):
    # v0..: VARC glyphs from gid 5, each its own case; their glyf a small triangle left of the origin, which shows
    # where a glyph falls back to its own outline.
    n = 40
    path = os.path.join(out, 'varc-hostile.ttf')
    base(path, n)
    g = lambda i: V + i
    # Store: regions on wght and ZZZZ; set 0 rows for transforms and axis values, set 1 for conditions.
    regions = [[(0, 0, 16384, 16384)], [(1, -16384, -16384, 0)], [(0, 0, 8192, 16384), (1, 0, 16384, 16384)]]
    full_rows = [[[30, -20, 400, 300, -200, 500, -700, 40, -30], [11, 7, -900, 0, 100, -300, 200, -5, 9],
                  [-3, 5, 50, 70, 0, 0, 0, 0, 0]],
                 [[-60, 90, -1500, 2000, 0, -100, 4000, 0, 0], [0, 0, 0, 0, 0, 0, 0, 0, 0],
                  [100000, -100000, 7, 7, 7, 7, 7, 7, 7]],
                 [[-300, 50], [120, -7], [0, 1]]]
    sets = [([0, 1, 2], full_rows),
            ([0, 1], [[[1], [-1]], [[0], [3]], [[-2], [0]]]),
            ([0, 2], [[[4000, -2000], [-3000, 1000]], [[1000, 2000], [0, 0]]]),
            ([0], [[[1]]])]
    var_store = store(regions, sets)
    deep = cond_range(0, 0, 16384)
    for _ in range(18):
        deep = cond_negate(deep)
    conds = [cond_range(0, 4096, 16384),                                   # 0: wght past a quarter
             cond_value(0, 0x00010000),                                    # 1: value on set 1 row 0
             cond_group(3, [cond_range(0, 0, 16384), cond_negate(cond_range(1, -16384, -8192))]),  # 2: and
             cond_group(4, [None, cond_range(1, 8192, 16384)]),            # 3: or with a null child
             deep,                                                         # 4: nested 19 deep
             u16(9) + u16(0),                                              # 5: unknown format
             None,                                                         # 6: null offset
             cond_value(-1, 0x00010001),                                   # 7: value negative default
             cond_negate(None or u16(4) + u8(0)),                          # 8: negate of an empty or
             cond_value(1, 0xFFFFFFFF)]                                    # 9: no variation
    axis_lists = index([tuples([0]), tuples([1]), tuples([0, 1]), tuples([1, 0], 4), tuples([0, 1], 2),
                        tuples([7]), tuples([0, 1, 1])])
    rec = {}
    full = dict(TX=120, TY=-40, ROT=1024, SX=1200, SY=900, SKX=300, SKY=-200, CX=50, CY=60)
    rec[g(0)] = [component(SQ, **full), component(QD, TX=300, ROT=-2048, SKX=4000, CY=10)]
    rec[g(1)] = [component(SQ, transform_var=0x00000000, **full),
                 component(QD, transform_var=0x00000001, **full),
                 component(SQ, transform_var=0x00000002, SX=-700, TX=600)]   # scale x alone: y mirrors it
    rec[g(2)] = [component(g(2), TX=200), component(SQ, TY=300)]              # itself
    rec[g(3)] = [component(g(4), TX=100, ROT=512), component(TRI)]
    rec[g(4)] = [component(g(3), TY=150, SX=1500), component(SQ)]           # a cycle of two
    for i in range(5, 25):                                                   # a chain 20 deep
        rec[g(i)] = [component(g(i + 1) if i < 24 else SQ, TX=13, ROT=37)]
    rec[g(25)] = [component(SQ, TX=10), component(QD, TX=500)[:-1]]          # cut short
    rec[g(26)] = [component(BIG)] * 66                                      # 66,000 points
    rec[g(27)] = [component(g(26), TX=300, ROT=900), component(SQ)]         # those inside another
    rec[g(28)] = [component(SQ, cond=c, TX=120 * c) for c in range(10)] + [component(QD, cond=77)]
    rec[g(29)] = [component(SQ, axes=(0, [8192], None)), component(QD, axes=(1, [-16384], None), TX=300),
                  component(SQ, axes=(2, [16384, 4000], 4), TY=300, RESET=1),
                  component(QD, axes=(3, [-3000, 16000], None), axes_var=0x00020000, TX=500),
                  component(SQ, axes=(4, [1, -1], 2), axes_var=0x00020001, TX=700),
                  component(SQ, axes=(5, [16384], None), TY=-300),
                  component(QD, axes=(6, [100, -100, 50], 1), TX=900),
                  component(SQ, RESET=1, TX=-300)]
    rec[g(30)] = [component(SQ, reserved=(15, 20, 31), TX=40), component(QD, gid24=True, TY=-80)]
    rec[g(31)] = [component(9999), component(0x12345, gid24=True), component(SQ, TX=-90)]
    rec[g(32)] = [component(SQ, cond=3, axes=(0, [12000], None), axes_var=0x00030000)]
    rec[g(33)] = [component(g(27), TX=-500, SKY=1000), component(g(2), TX=900)]
    rec[g(34)] = [component(SQ, ROT=4096 * 7 + 13, SKX=2047, SKY=-2049, TX=-7, CX=-33)]
    rec[g(35)] = []                                                         # empty record
    rec[g(36)] = [component(g(37), SX=-1024, TX=900), component(TRI, TX=-200)]  # child's record unreadable
    covered = sorted(rec)
    records = [b''.join(rec[k]) for k in covered]
    # Coverage format 2 lists each covered glyph; g(37) maps to an index past the records.
    ranges = [(k, k, i) for i, k in enumerate(covered)] + [(g(37), g(37), 999)]
    ranges.sort()
    coverage = u16(2) + u16(len(ranges)) + b''.join(u16(a) + u16(b) + u16(i) for a, b, i in ranges)
    table = varc(covered, records, condition_list(conds), axis_lists, var_store, coverage)
    splice(path, 'VARC', table)
    return path


def cff2_store(out):
    # A CFF2 with a store and no fvar: the square's width blends on the store's one axis, which only the VARC glyph's
    # component sets, so a subset that resolves blends has to draw it.
    from fontTools.misc.psCharStrings import T2CharString
    path = os.path.join(out, 'varc-cff2-store.otf')
    names = ['.notdef', 'sq', 'vg']
    fb = FontBuilder(1000, isTTF=False)
    fb.setupGlyphOrder(names)
    fb.setupCharacterMap({})
    fb.setupNameTable({'familyName': 'VarcCff2Store', 'styleName': 'Regular'})
    fb.setupFvar(axes=[('wght', 100, 400, 900, 'wght')], instances=[])
    square = [0, 0, 'rmoveto', 100, 50, 1, 'blend', 0, 'rlineto', 0, 100, 'rlineto', -100, -50, 1, 'blend', 0,
              'rlineto']
    fb.setupCFF2({'.notdef': T2CharString(program=[]), 'sq': T2CharString(program=square),
                  'vg': T2CharString(program=[])}, regions=[{'wght': (0, 1, 1)}])
    fb.setupHorizontalMetrics({n: (500, 0) for n in names})
    fb.setupHorizontalHeader()
    fb.setupOS2()
    fb.setupPost()
    del fb.font['fvar']
    fb.save(path)
    record = component(1, axes=(0, [16384], None), TX=200)
    splice(path, 'VARC', varc([2], [record], axis_indices=index([tuples([0])])))
    return path


def composite_leaf(out):
    # A VARC glyph whose component is a glyf composite of two squares at wght 1.0: the composite's deltas move its
    # second square 40 right and the square's widen each, so every glyph a leaf loads draws with its own deltas.
    path = os.path.join(out, 'varc-composite-leaf.ttf')
    names = ['.notdef', 'sq', 'pair', 'vg']
    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder(names)
    fb.setupCharacterMap({})
    sq = glyph([(0, 0), (100, 0), (100, 100), (0, 100)])
    pair = TTGlyphPen({'sq': sq})
    pair.addComponent('sq', (1, 0, 0, 1, 0, 0))
    pair.addComponent('sq', (1, 0, 0, 1, 150, 0))
    fb.setupGlyf({'.notdef': TTGlyphPen(None).glyph(), 'sq': sq, 'pair': pair.glyph(), 'vg': TTGlyphPen(None).glyph()})
    fb.setupHorizontalMetrics({n: (500, 0) for n in names})
    fb.setupHorizontalHeader()
    fb.setupNameTable({'familyName': 'VarcCompositeLeaf', 'styleName': 'Regular'})
    fb.setupOS2()
    fb.setupPost()
    fb.setupFvar(axes=[('wght', 100, 400, 900, 'wght')], instances=[])
    phantoms = [(0, 0)] * 4
    fb.setupGvar({
        'sq': [TupleVariation({'wght': (0, 1, 1)}, [(0, 0), (60, 0), (60, 20), (0, 20)] + phantoms)],
        'pair': [TupleVariation({'wght': (0, 1, 1)}, [(0, 0), (40, 0)] + phantoms)],
    })
    fb.save(path)
    record = component(2, axes=(0, [16384], None))
    splice(path, 'VARC', varc([3], [record], axis_indices=index([tuples([0])])))
    return path


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    print(hangul(out))
    print(short_cff2(out))
    print(hostile(out))
    print(cff2_store(out))
    print(composite_leaf(out))


if __name__ == '__main__':
    main()
