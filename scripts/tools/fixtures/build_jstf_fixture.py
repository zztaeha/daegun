#!/usr/bin/env python3
# Writes test-fixtures/jstf-levels.ttf: Scheherazade New cut to a few letters, with a JSTF of two priority levels, as
# no vendored font has one and only a font with levels reaches shape_justified. Takes the fontTools Python.
import pathlib

from fontTools import subset
from fontTools.ttLib import TTFont, newTable
from fontTools.ttLib.tables import otTables as ot

FONTS = pathlib.Path(__file__).resolve().parents[3] / "assets/test-fonts"
TEXT = "Aafi المهسـ"


def lookups(kind, indexes):
    mods = kind()
    mods.LookupCount = len(indexes)
    setattr(mods, "GSUBLookupIndex" if kind is ot.JstfGSUBModList else "GPOSLookupIndex", indexes)
    return mods


def priority(gsub, gpos):
    level = ot.JstfPriority()
    level.ShrinkageEnableGSUB = lookups(ot.JstfGSUBModList, gsub)
    level.ShrinkageDisableGSUB = lookups(ot.JstfGSUBModList, gsub)
    level.ShrinkageEnableGPOS = lookups(ot.JstfGPOSModList, gpos)
    level.ShrinkageDisableGPOS = lookups(ot.JstfGPOSModList, gpos)
    level.ShrinkageJstfMax = None
    level.ExtensionEnableGSUB = lookups(ot.JstfGSUBModList, gsub)
    level.ExtensionDisableGSUB = lookups(ot.JstfGSUBModList, gsub)
    level.ExtensionEnableGPOS = lookups(ot.JstfGPOSModList, gpos)
    level.ExtensionDisableGPOS = lookups(ot.JstfGPOSModList, gpos)
    level.ExtensionJstfMax = None
    return level


font = TTFont(FONTS / "scheherazade-new/ScheherazadeNew-Regular.ttf")
options = subset.Options(layout_features=["*"], name_IDs=["*"], notdef_outline=True)
subsetter = subset.Subsetter(options)
subsetter.populate(text=TEXT)
subsetter.subset(font)

extenders = ot.ExtenderGlyph()
extenders.GlyphCount = 1
extenders.ExtenderGlyph = [font.getBestCmap()[0x0640]]
system = ot.JstfLangSys()
system.JstfPriorityCount = 2
system.JstfPriority = [priority([0], [0]), priority([1], [])]
script = ot.JstfScript()
script.ExtenderGlyph = extenders
script.DefJstfLangSys = system
script.JstfLangSysCount = 0
script.JstfLangSysRecord = []
record = ot.JstfScriptRecord()
record.JstfScriptTag = "arab"
record.JstfScript = script
table = ot.JSTF()
table.Version = 0x00010000
table.JstfScriptCount = 1
table.JstfScriptRecord = [record]
font["JSTF"] = newTable("JSTF")
font["JSTF"].table = table
font.save(FONTS / "test-fixtures/jstf-levels.ttf")
