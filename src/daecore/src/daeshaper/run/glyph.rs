use super::settings::{Fact, Facts};
use crate::daecore::daeshaper::unicode::{self, GeneralCategory, SpaceFallback};

pub(crate) type Mask = u32;

// What a shaped run reports of each glyph, in the mask's low bits; every feature's bits lie above them.
pub(crate) mod flag {
    use super::Mask;

    pub(crate) const UNSAFE_TO_BREAK: Mask = 0b001;
    pub(crate) const UNSAFE_TO_CONCAT: Mask = 0b010;
    pub(crate) const SAFE_TO_INSERT_TATWEEL: Mask = 0b100;
    pub(crate) const ALL: Mask = 0b111;
}

// GDEF's class in the bits a lookup flag names to skip it, so one AND tells, then what substitution made of the glyph.
pub(crate) mod class {
    pub(crate) const BASE: u8 = 0x02;
    pub(crate) const LIGATURE: u8 = 0x04;
    pub(crate) const MARK: u8 = 0x08;
    pub(crate) const SUBSTITUTED: u8 = 0x10;
    pub(crate) const LIGATED: u8 = 0x20;
    pub(crate) const MULTIPLIED: u8 = 0x40;
    pub(crate) const HISTORY: u8 = SUBSTITUTED | LIGATED | MULTIPLIED;
}

// What the text says of a glyph's character. The last four describe format characters, so they go with the category.
pub(crate) mod text {
    pub(crate) const IGNORABLE: u8 = 0x01;
    pub(crate) const HIDDEN: u8 = 0x02;
    pub(crate) const CONTINUES: u8 = 0x04;
    pub(crate) const ZWJ: u8 = 0x08;
    pub(crate) const ZWNJ: u8 = 0x10;
    pub(crate) const SELECTOR: u8 = 0x20;
    pub(crate) const DELETED: u8 = 0x40;
    pub(crate) const OF_FORMAT: u8 = ZWJ | ZWNJ | SELECTOR | DELETED;
}

const COMBINING_GRAPHEME_JOINER: u32 = 0x034F;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct Ligature {
    pub(crate) id: u8,
    // The ligature glyph itself, rather than a mark or a component of it.
    pub(crate) whole: bool,
    // The whole's component count; for the rest, the component from 1 they sit on, 0 for none.
    pub(crate) count: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Glyph {
    // The character until glyphs are mapped, then the glyph.
    pub(crate) id: u32,
    pub(crate) cluster: u32,
    pub(crate) mask: Mask,
    // The font's glyph for the character, found while normalizing.
    pub(crate) mapped: u32,
    pub(crate) ligature: Ligature,
    pub(crate) category: GeneralCategory,
    pub(crate) text: u8,
    // A mark's sort class or a space's fallback width, which the category tells apart.
    pub(crate) detail: u8,
    pub(crate) class: u8,
    pub(crate) attach_class: u8,
    pub(crate) syllable: u8,
    pub(crate) script_class: u8,
    pub(crate) script_slot: u8,
}

impl Glyph {
    pub(crate) const fn of_char(id: u32, cluster: u32) -> Glyph {
        Glyph {
            id,
            cluster,
            mask: 0,
            mapped: 0,
            ligature: Ligature { id: 0, whole: false, count: 0 },
            category: GeneralCategory::Unassigned,
            text: 0,
            detail: 0,
            class: 0,
            attach_class: 0,
            syllable: 0,
            script_class: 0,
            script_slot: 0,
        }
    }

    // Takes the character's Unicode facts afresh and gives back what they tell of the run.
    pub(crate) fn read_char(&mut self) -> Facts {
        let Some(c) = char::from_u32(self.id) else { return Facts::default() };
        self.category = unicode::general_category(c);
        (self.text, self.detail) = (0, 0);
        if c.is_ascii() {
            return Facts::default();
        }
        let mut facts = Facts::default().with(Fact::NonAscii);
        if unicode::is_default_ignorable(c) {
            facts.note(Fact::Ignorables);
            self.text = text::IGNORABLE
                | match self.id {
                    0x200C => text::ZWNJ,
                    0x200D => text::ZWJ,
                    0xE0020..=0xE007F => text::HIDDEN,
                    COMBINING_GRAPHEME_JOINER => {
                        facts.note(Fact::Cgj);
                        text::HIDDEN
                    }
                    id if unicode::is_free_variation_selector(id) => text::HIDDEN,
                    _ => 0,
                };
        }
        if self.category.is_mark() {
            self.text |= text::CONTINUES;
            self.detail = unicode::mark_class(c);
        }
        facts
    }

    pub(crate) fn set_category(&mut self, category: GeneralCategory) {
        self.category = category;
        self.text &= !text::OF_FORMAT;
        self.detail = 0;
    }

    pub(crate) fn is_mark_char(&self) -> bool {
        self.category.is_mark()
    }

    pub(crate) fn is_space_char(&self) -> bool {
        self.category == GeneralCategory::SpaceSeparator
    }

    pub(crate) fn mark_class(&self) -> u8 {
        if self.is_mark_char() { self.detail } else { 0 }
    }

    pub(crate) fn set_mark_class(&mut self, class: u8) {
        self.detail = class;
    }

    pub(crate) fn space_fallback(&self) -> Option<SpaceFallback> {
        self.is_space_char().then(|| SpaceFallback::from_byte(self.detail)).flatten()
    }

    pub(crate) fn set_space_fallback(&mut self, fallback: SpaceFallback) {
        if self.is_space_char() {
            self.detail = fallback.to_byte();
        }
    }

    // Whether the character carries any of `bits`, from `text`.
    pub(crate) fn char_is(&self, bits: u8) -> bool {
        self.text & bits != 0
    }

    // Whether the glyph carries any of `bits`, from `class`: its GDEF class or what substitution made of it.
    pub(crate) fn glyph_is(&self, bits: u8) -> bool {
        self.class & bits != 0
    }

    // A default ignorable the font hasn't substituted for something it draws.
    pub(crate) fn is_ignorable(&self) -> bool {
        self.char_is(text::IGNORABLE) && !self.glyph_is(class::SUBSTITUTED)
    }

    // A ligature no later lookup split again.
    pub(crate) fn only_ligated(&self) -> bool {
        self.class & (class::LIGATED | class::MULTIPLIED) == class::LIGATED
    }

    // A variation selector the font can't apply is drawn alone, a format character until its stand-in replaces it;
    // then a mark again.
    pub(crate) fn set_selector_fallback(&mut self, on: bool) {
        match on {
            true => self.become_format(text::SELECTOR),
            false => self.set_category(GeneralCategory::NonspacingMark),
        }
    }

    // A glyph morx deletes: a hidden format character with no mark class.
    pub(crate) fn mark_deleted(&mut self) {
        self.become_format(text::DELETED | text::HIDDEN);
    }

    fn become_format(&mut self, bits: u8) {
        self.set_category(GeneralCategory::Format);
        self.text |= bits;
    }

    pub(crate) fn ligature_id(&self) -> u8 {
        self.ligature.id
    }

    pub(crate) fn component(&self) -> u16 {
        if self.ligature.whole { 0 } else { self.ligature.count }
    }

    pub(crate) fn components(&self) -> u16 {
        match self.ligature.whole && self.glyph_is(class::LIGATURE) {
            true => self.ligature.count,
            false => 1,
        }
    }

    // What the glyph adds to a ligature formed over it: none for a piece of a glyph a sequence split.
    pub(crate) fn components_joined(&self) -> u16 {
        if self.glyph_is(class::MULTIPLIED) && self.component() != 0 { 0 } else { self.components() }
    }

    pub(crate) fn make_ligature(&mut self, id: u8, count: u16) {
        self.ligature = Ligature { id, whole: true, count };
    }

    pub(crate) fn attach_to_ligature(&mut self, id: u8, component: u16) {
        self.ligature = Ligature { id, whole: false, count: component };
    }

    pub(crate) fn make_component(&mut self, component: u16) {
        self.attach_to_ligature(0, component);
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) enum AttachKind {
    #[default]
    None,
    Mark,
    Cursive,
}

// A link to the glyph `to` places away that this one hangs from, resolved once positioning is done; 0 for none.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct Attachment {
    pub(crate) to: i16,
    pub(crate) kind: AttachKind,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct Placement {
    pub(crate) advance_x: i32,
    pub(crate) advance_y: i32,
    pub(crate) offset_x: i32,
    pub(crate) offset_y: i32,
    pub(crate) attach: Attachment,
}

impl Placement {
    // No advance, the offsets first moved back by it where asked so the glyph hangs over what precedes it.
    // Saturating, as an extreme advance mustn't wrap the offset.
    pub(crate) fn zero_advance(&mut self, shift_offsets: bool) {
        if shift_offsets {
            self.offset_x = self.offset_x.saturating_sub(self.advance_x);
            self.offset_y = self.offset_y.saturating_sub(self.advance_y);
        }
        (self.advance_x, self.advance_y) = (0, 0);
    }

    // The advance and offset along a horizontal run or a vertical one.
    pub(crate) fn along_mut(&mut self, horizontal: bool) -> (&mut i32, &mut i32) {
        match horizontal {
            true => (&mut self.advance_x, &mut self.offset_x),
            false => (&mut self.advance_y, &mut self.offset_y),
        }
    }

    pub(crate) fn across_mut(&mut self, horizontal: bool) -> &mut i32 {
        match horizontal {
            true => &mut self.offset_y,
            false => &mut self.offset_x,
        }
    }
}
