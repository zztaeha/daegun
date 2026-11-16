#!/usr/bin/env python3
# Writes test-fixtures/fuzz-reach.ttf: every glyph a MATH construction with an assembly both ways, BASE and JSTF for
# every script it lists, and a named instance with a PostScript name, so a fuzzer reaches each reader whatever it picks.
import pathlib

from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.fontBuilder import FontBuilder
from fontTools.otlLib.builder import buildMathTable
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import newTable
from fontTools.ttLib.tables import otTables as ot

OUT = pathlib.Path(__file__).resolve().parents[3] / "assets/test-fonts/test-fixtures/fuzz-reach.ttf"
GLYPHS = [".notdef", "A", "B", "C"]
SCRIPTS = ["DFLT", "arab", "latn"]
FEA = """
languagesystem DFLT dflt;
languagesystem arab dflt;
languagesystem latn dflt;
lookup widen { sub A by B; } widen;
lookup narrow { sub B by C; } narrow;
feature salt { lookup widen; } salt;
feature ss01 { lookup narrow; } ss01;
feature kern { pos A B -50; } kern;
table BASE {
  HorizAxis.BaseTagList ideo romn;
  HorizAxis.BaseScriptList DFLT romn -120 0, arab romn -120 0, latn romn -120 0;
  VertAxis.BaseTagList ideo romn;
  VertAxis.BaseScriptList DFLT ideo 0 120, arab ideo 0 120, latn ideo 0 120;
} BASE;
"""


def box(x0, y0, x1, y1):
    pen = TTGlyphPen(None)
    pen.moveTo((x0, y0))
    pen.lineTo((x1, y0))
    pen.lineTo((x1, y1))
    pen.lineTo((x0, y1))
    pen.closePath()
    return pen.glyph()


def lookups(kind, indexes):
    mods = kind()
    mods.LookupCount = len(indexes)
    setattr(mods, "GSUBLookupIndex" if kind is ot.JstfGSUBModList else "GPOSLookupIndex", indexes)
    return mods


def priority(gsub, gpos):
    level = ot.JstfPriority()
    for phase in ("Shrinkage", "Extension"):
        setattr(level, f"{phase}EnableGSUB", lookups(ot.JstfGSUBModList, gsub))
        setattr(level, f"{phase}DisableGSUB", lookups(ot.JstfGSUBModList, gsub))
        setattr(level, f"{phase}EnableGPOS", lookups(ot.JstfGPOSModList, gpos))
        setattr(level, f"{phase}DisableGPOS", lookups(ot.JstfGPOSModList, gpos))
        setattr(level, f"{phase}JstfMax", None)
    return level


def jstf():
    records = []
    for tag in SCRIPTS:
        system = ot.JstfLangSys()
        system.JstfPriorityCount = 2
        system.JstfPriority = [priority([0], [0]), priority([1], [])]
        script = ot.JstfScript()
        script.ExtenderGlyph = None
        script.DefJstfLangSys = system
        script.JstfLangSysCount = 0
        script.JstfLangSysRecord = []
        record = ot.JstfScriptRecord()
        record.JstfScriptTag = tag
        record.JstfScript = script
        records.append(record)
    table = ot.JSTF()
    table.Version = 0x00010000
    table.JstfScriptCount = len(records)
    table.JstfScriptRecord = records
    return table


def main():
    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder(GLYPHS)
    fb.setupCharacterMap({ord(g): g for g in GLYPHS[1:]})
    fb.setupGlyf({g: box(50, 0, 450, 700) if g != ".notdef" else TTGlyphPen(None).glyph() for g in GLYPHS})
    fb.setupHorizontalMetrics({g: (500, 50) for g in GLYPHS})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "DaegunFuzzReach", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200)
    fb.setupPost()
    bold = {"location": {"wght": 700}, "stylename": "Bold", "postscriptfontname": "DaegunFuzzReach-Bold"}
    fb.setupFvar([("wght", 400, 400, 900, "Weight")], [bold])
    addOpenTypeFeaturesFromString(fb.font, FEA)
    variants = {g: [(g, 700)] for g in GLYPHS}
    parts = [("A", 0, 0, 100, 300), ("B", 1, 100, 100, 300), ("C", 0, 100, 0, 300)]
    assembly = {g: (parts, 0) for g in GLYPHS}
    buildMathTable(fb.font, constants={"AxisHeight": 250}, minConnectorOverlap=20, vertGlyphVariants=variants,
                   horizGlyphVariants=variants, vertGlyphAssembly=assembly, horizGlyphAssembly=assembly)
    fb.font["JSTF"] = newTable("JSTF")
    fb.font["JSTF"].table = jstf()
    fb.save(OUT)
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
