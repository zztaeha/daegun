// Thai and Lao, after Microsoft's spec for them: SARA AM split, its NIKHAHIT before marks typed ahead of it. A font
// with no Thai GSUB gets its marks arranged from the private-use stand-ins such fonts carry.
use super::{Model, Shaper};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::plan::{Plan, Table};
use crate::daecore::daeshaper::run::{text, Run};
use crate::daecore::daeshaper::unicode::{GeneralCategory, Script};

pub(crate) struct Thai;

pub(crate) const MODEL: Thai = Thai;

impl Model for Thai {
    fn kind(&self) -> Shaper {
        Shaper::Thai
    }

    fn takes_fallbacks(&self) -> bool {
        false
    }

    fn prepare(&self, run: &mut Run, plan: &Plan, face: &Face) {
        split_sara_am(run);
        // A font with Thai GSUB arranges these marks itself and wants no stand-in.
        if run.script == Some(Script::THAI) && !plan.features.script(Table::Gsub).found {
            arrange_from_stand_ins(face, run);
        }
    }
}

// Lao's letters lie 0x80 past Thai's, the same letter at the same offset.
fn thai_of(code: u32) -> u32 {
    code & !0x80
}

fn is_sara_am(code: u32) -> bool {
    thai_of(code) == 0x0E33
}

// SARA AM is NIKHAHIT and SARA AA, U+0E4D and U+0E32 (Lao: U+0ECD, U+0EB2).
fn halves(am: u32) -> [u32; 2] {
    [am + (0x0E4D - 0x0E33), am - 1]
}

fn is_above_base(code: u32) -> bool {
    matches!(thai_of(code), 0x0E31 | 0x0E34..=0x0E37 | 0x0E3B | 0x0E47..=0x0E4E)
}

// What shaping can turn `c` into: SARA AM's halves and the stand-ins taken where a font has no Thai GSUB.
pub(crate) fn forms_of(c: char) -> impl Iterator<Item = char> {
    let code = c as u32;
    let split = is_sara_am(code).then(|| halves(code)).into_iter().flatten();
    let stand_ins = STAND_INS.iter().filter(move |s| s.of == code).flat_map(|s| [s.windows, s.mac]);
    split.chain(stand_ins).filter_map(char::from_u32)
}

fn split_sara_am(run: &mut Run) {
    run.start_rewrite();
    while run.cursor < run.len {
        let code = run.current().id;
        let went_on = if is_sara_am(code) { split_one(run, code) } else { run.keep() };
        if !went_on {
            break;
        }
    }
    run.finish_rewrite();
}

// The NIKHAHIT, a mark continuing the grapheme before it, goes before the above-base marks typed ahead of it, to sit
// on the consonant under them. False where the run had no room for the halves.
fn split_one(run: &mut Run, am: u32) -> bool {
    let written = run.written;
    run.replace_span(1, &halves(am));
    if run.written != written + 2 {
        return false;
    }
    let nikhahit = written;
    let out = run.out_mut();
    out[nikhahit].text |= text::CONTINUES;
    out[nikhahit].set_category(GeneralCategory::NonspacingMark);
    let typed_ahead = out[..nikhahit].iter().rev().take_while(|g| is_above_base(g.id)).count();
    let first = nikhahit - typed_ahead;
    if typed_ahead > 0 {
        run.join_out_clusters(first..nikhahit + 2);
        run.out_mut()[first..=nikhahit].rotate_right(1);
    }
    if let Some(before) = first.checked_sub(1) {
        run.join_out_graphemes(before..nikhahit + 2);
    }
    true
}

// A move a stand-in makes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shift {
    Down,
    Left,
    DownLeft,
    // The consonant's, not the mark's: its descender goes to clear a below-base mark.
    NoDescender,
}

// A stand-in for `of` making a move, at its Windows and Mac private-use codes.
struct StandIn {
    of: u32,
    shift: Shift,
    windows: u32,
    mac: u32,
}

const fn stand_in(of: u32, shift: Shift, windows: u32, mac: u32) -> StandIn {
    StandIn { of, shift, windows, mac }
}

// The private-use codes legacy Thai fonts draw these forms at.
const STAND_INS: [StandIn; 27] = [
    stand_in(0x0E48, Shift::Down, 0xF70A, 0xF88B),
    stand_in(0x0E49, Shift::Down, 0xF70B, 0xF88E),
    stand_in(0x0E4A, Shift::Down, 0xF70C, 0xF891),
    stand_in(0x0E4B, Shift::Down, 0xF70D, 0xF894),
    stand_in(0x0E4C, Shift::Down, 0xF70E, 0xF897),
    stand_in(0x0E38, Shift::Down, 0xF718, 0xF89B),
    stand_in(0x0E39, Shift::Down, 0xF719, 0xF89C),
    stand_in(0x0E3A, Shift::Down, 0xF71A, 0xF89D),
    stand_in(0x0E48, Shift::DownLeft, 0xF705, 0xF88C),
    stand_in(0x0E49, Shift::DownLeft, 0xF706, 0xF88F),
    stand_in(0x0E4A, Shift::DownLeft, 0xF707, 0xF892),
    stand_in(0x0E4B, Shift::DownLeft, 0xF708, 0xF895),
    stand_in(0x0E4C, Shift::DownLeft, 0xF709, 0xF898),
    stand_in(0x0E48, Shift::Left, 0xF713, 0xF88A),
    stand_in(0x0E49, Shift::Left, 0xF714, 0xF88D),
    stand_in(0x0E4A, Shift::Left, 0xF715, 0xF890),
    stand_in(0x0E4B, Shift::Left, 0xF716, 0xF893),
    stand_in(0x0E4C, Shift::Left, 0xF717, 0xF896),
    stand_in(0x0E31, Shift::Left, 0xF710, 0xF884),
    stand_in(0x0E34, Shift::Left, 0xF701, 0xF885),
    stand_in(0x0E35, Shift::Left, 0xF702, 0xF886),
    stand_in(0x0E36, Shift::Left, 0xF703, 0xF887),
    stand_in(0x0E37, Shift::Left, 0xF704, 0xF888),
    stand_in(0x0E47, Shift::Left, 0xF712, 0xF889),
    stand_in(0x0E4D, Shift::Left, 0xF711, 0xF899),
    stand_in(0x0E0D, Shift::NoDescender, 0xF70F, 0xF89A),
    stand_in(0x0E10, Shift::NoDescender, 0xF700, 0xF89E),
];

// `code` moved by `shift`: the Windows stand-in where the font maps it, else the Mac one, else `code` as it is.
fn shifted(face: &Face, code: u32, shift: Shift) -> u32 {
    let Some(s) = STAND_INS.iter().find(|s| s.of == code && s.shift == shift) else { return code };
    [s.windows, s.mac].into_iter().find(|&c| face.maps(c)).unwrap_or(code)
}

// What a mark is to the room around its consonant.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    // A vowel above the consonant.
    Vowel,
    // A vowel below it.
    Under,
    Tone,
}

fn mark_of(code: u32) -> Option<Mark> {
    match code {
        0x0E31 | 0x0E34..=0x0E37 | 0x0E47 | 0x0E4D..=0x0E4E => Some(Mark::Vowel),
        0x0E38..=0x0E3A => Some(Mark::Under),
        0x0E48..=0x0E4C => Some(Mark::Tone),
        _ => None,
    }
}

// The room above the consonant: free, beside an ascender, holding one mark shifted clear of the ascender, or past
// arranging.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Above {
    Free,
    Ascender,
    PastAscender,
    Settled,
}

// The room below: free, a descender that can go, or taken.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Below {
    Free,
    Descender,
    Taken,
}

// The room a consonant leaves, from its shape; anything else leaves none to arrange.
fn room_of(code: u32) -> (Above, Below) {
    match code {
        0x0E1B | 0x0E1D | 0x0E1F => (Above::Ascender, Below::Free),
        0x0E0D | 0x0E10 => (Above::Free, Below::Descender),
        0x0E0E | 0x0E0F => (Above::Free, Below::Taken),
        0x0E01..=0x0E2E => (Above::Free, Below::Free),
        _ => (Above::Settled, Below::Taken),
    }
}

// A tone with nothing above goes down; beside an ascender, a mark goes left and a tone down and left; a tone over a
// shifted vowel goes left.
fn above(room: Above, mark: Mark) -> (Option<Shift>, Above) {
    match (room, mark) {
        (_, Mark::Under) | (Above::Settled, _) => (None, room),
        (Above::Free, Mark::Vowel) => (None, Above::Settled),
        (Above::Free, Mark::Tone) => (Some(Shift::Down), Above::Settled),
        (Above::Ascender, Mark::Vowel) => (Some(Shift::Left), Above::PastAscender),
        (Above::Ascender, Mark::Tone) => (Some(Shift::DownLeft), Above::PastAscender),
        (Above::PastAscender, Mark::Vowel) => (None, Above::Settled),
        (Above::PastAscender, Mark::Tone) => (Some(Shift::Left), Above::Settled),
    }
}

// A below-base vowel takes a removable descender's place, or goes down past one that stays.
fn below(room: Below, mark: Mark) -> (Option<Shift>, Below) {
    match (room, mark) {
        (Below::Free, Mark::Under) => (None, Below::Taken),
        (Below::Descender, Mark::Under) => (Some(Shift::NoDescender), Below::Taken),
        (Below::Taken, Mark::Under) => (Some(Shift::Down), Below::Taken),
        _ => (None, room),
    }
}

fn arrange_from_stand_ins(face: &Face, run: &mut Run) {
    let (mut room_above, mut room_below) = room_of(0);
    let mut base = 0;
    for i in 0..run.len {
        let code = run.glyphs[i].id;
        let Some(mark) = mark_of(code) else {
            (room_above, room_below) = room_of(code);
            base = i;
            continue;
        };
        let (up, next_above) = above(room_above, mark);
        let (down, next_below) = below(room_below, mark);
        (room_above, room_below) = (next_above, next_below);
        run.unsafe_to_break(base..i);
        match up.or(down) {
            Some(Shift::NoDescender) => run.glyphs[base].id = shifted(face, run.glyphs[base].id, Shift::NoDescender),
            Some(shift) => run.glyphs[i].id = shifted(face, code, shift),
            None => {}
        }
    }
}
