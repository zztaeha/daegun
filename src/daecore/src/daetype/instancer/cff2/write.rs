#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use crate::daecore::daetype::decoder::{read_u16_be, read_u32_be};
use crate::daecore::daetype::format::cff::{
    blend_value, encode_cff_int, push_short_int, subr_bias, subr_index, MAX_SUBR_DEPTH,
};
use crate::daecore::daetype::format::cff::{DictKind, FdSelect};
use crate::daecore::daetype::format::ivs::{data_scalars, ItemVariationStore};
use crate::daecore::daetype::subsetter::subset_budget;
use crate::daecore::Error;

const RETURN: u16 = 11;
const ENDCHAR: u16 = 14;
const CHARSTRINGS: u16 = 17;
const PRIVATE: u16 = 18;
const SUBRS: u16 = 19;
const VSINDEX: u16 = 22;
const BLEND: u16 = 23;
const VSTORE: u16 = 24;
const FD_ARRAY: u16 = 0x0C24;
const FD_SELECT: u16 = 0x0C25;
// An escape byte with nothing after it.
const INVALID: u16 = 0xFFFF;
// StdHW, StdVW, BlueScale, BlueShift, BlueFuzz, ExpansionFactor, LanguageGroup, BlueValues, OtherBlues, FamilyBlues,
// FamilyOtherBlues, StemSnapH and StemSnapV: the Private DICT operators written with their operands.
const HINTS: [u16; 13] = [10, 11, 0x0C09, 0x0C0A, 0x0C0B, 0x0C18, 0x0C11, 6, 7, 8, 9, 0x0C0C, 0x0C0D];
// BlueScale and ExpansionFactor, whose values are fractions: blended, they're kept as they come.
const FRACTIONAL: [u16; 2] = [0x0C09, 0x0C18];

// CFF2's maxstack: the argument stack holds 513 operands.
const ARGS: usize = 513;
const OVER_BUDGET: Error = Error::limit("CFF2: charstrings past the font's work budget");

// A CFF2 table as the instancer reads it, to write it pinned as static CFF2.
pub(crate) struct Cff2Source<'a> {
    top: Vec<(u16, &'a [u8])>,
    global_subrs: Index<'a>,
    charstrings: Index<'a>,
    fds: Vec<Fd<'a>>,
    // Each Private DICT the FDs name, once however many name it.
    privates: Vec<Vec<(u16, &'a [u8])>>,
    // None without FDSelect. A glyph it names no FD for, or one past the FDArray, takes FD 0, as do all where it
    // doesn't read.
    fd_select: Option<(FdSelect, &'a [u8])>,
    store: Option<ItemVariationStore>,
    // The bytes of charstrings and subroutines run, past which the font is refused: bounding each glyph alone would
    // leave a font's output unbounded.
    budget: usize,
    // Charstrings written in place of their glyphs' own, which VARC draws.
    drawn: BTreeMap<u32, Vec<u8>>,
}

// An FD's Font DICT, which of `privates` it names, and that one's local Subrs and vsindex.
struct Fd<'a> {
    font: Vec<(u16, &'a [u8])>,
    private: usize,
    subrs: Index<'a>,
    ivs: u32,
}

impl<'a> Cff2Source<'a> {
    // Fails where the table can't be read, and the font's instance fails with it. Its CharStrings count maxp's
    // glyphs, or fewer beside a `varc` table.
    pub(crate) fn read(cff2: &'a [u8], num_glyphs: usize, varc: bool) -> Result<Cff2Source<'a>, Error> {
        let unread =
            |what: &str| Error::owned(crate::daecore::ErrorKind::Malformed, format!("CFF2: {what} doesn't read"));
        if cff2.len() < 5 || cff2[0] != 2 {
            return Err(unread("the header"));
        }
        let top_at = usize::from(cff2[2]);
        let top_end = top_at + usize::from(read_u16_be(cff2, 3).unwrap_or(0));
        let top = read_dict(cff2.get(top_at..top_end).ok_or_else(|| unread("the Top DICT"))?, Kind::Top)?;
        let global_subrs = index_at(cff2, 0, top_end as i64).ok_or_else(|| unread("the global subroutines"))?;
        let charstrings = index_at(cff2, 0, top.charstrings.into()).ok_or_else(|| unread("CharStrings"))?;
        let fd_dicts = index_at(cff2, 0, top.fd_array.into()).ok_or_else(|| unread("the FDArray"))?;
        if charstrings.len() != num_glyphs && !(charstrings.len() < num_glyphs && varc) {
            return Err(Error::malformed("CFF2: CharStrings count differs from maxp's"));
        }
        let fd_select = usize::try_from(top.fd_select).ok().filter(|&at| at != 0).map(|at| {
            let read = FdSelect::read(cff2, at, charstrings.len(), DictKind::Cff2);
            (read.map_or_else(|_| FdSelect::zero(charstrings.len()), |(select, _)| select), cff2)
        });
        // FDArray elements and Private DICTs may overlap, so FDs each reading theirs whole could read the table's
        // square: each Private DICT is read once, and what's read shares the subset budget.
        let mut left = subset_budget(cff2.len());
        let mut charge = |n: usize| {
            left = left.checked_sub(n).ok_or(Error::limit("CFF2: the FDs' DICTs run past the font's budget"))?;
            Ok::<(), Error>(())
        };
        let (mut fds, mut privates, mut read_once) = (Vec::new(), Vec::new(), BTreeMap::new());
        for font in (0..fd_dicts.len()).filter_map(|i| fd_dicts.get(i)) {
            charge(font.len())?;
            let font = read_dict(font, Kind::Font)?;
            let (size, at) = (font.private.0 as usize, font.private.1 as usize);
            let held = at != 0 && at.checked_add(size).is_some_and(|end| end <= cff2.len());
            if size != 0 && !held {
                return Err(unread("a Private DICT"));
            }
            let span = if held { at..at + size } else { 0..0 };
            let (private, subrs, ivs) = match read_once.get(&(span.start, span.end)) {
                Some(&read) => read,
                None => {
                    charge(span.len())?;
                    let dict = read_dict(&cff2[span.clone()], Kind::Private)?;
                    let subrs = held.then(|| index_at(cff2, at, dict.subrs.into())).flatten().unwrap_or_default();
                    privates.push(dict.ops);
                    *read_once.entry((span.start, span.end)).or_insert((privates.len() - 1, subrs, dict.ivs))
                }
            };
            fds.push(Fd { font: font.ops, private, subrs, ivs });
        }
        Ok(Cff2Source {
            top: top.ops,
            global_subrs,
            charstrings,
            fds,
            privates,
            fd_select,
            store: store_at(cff2, top.vstore.into()),
            budget: cff2.len().saturating_mul(64),
            drawn: BTreeMap::new(),
        })
    }

    pub(crate) fn with_drawn(self, drawn: BTreeMap<u32, Vec<u8>>) -> Cff2Source<'a> {
        Cff2Source { drawn, ..self }
    }

    // Whether the charstrings can blend, which a pinned write resolves.
    pub(crate) fn blends(&self) -> bool {
        self.store.is_some()
    }

    // The table pinned at `coords`, 2.14 values: each slot the charstring of the glyph it names, flattened with its
    // blends rounded, or an empty one; FDs no slot selects dropped, the rest renumbered in order.
    pub(crate) fn write(&self, coords: &[f64], slots: &[Option<u32>]) -> Result<Vec<u8>, Error> {
        self.write_kept(coords, slots, None)
    }

    // As `write`, reading and keeping glyphs in `kept`, which holds glyphs flattened at `coords` alone.
    pub(crate) fn write_kept(
        &self, coords: &[f64], slots: &[Option<u32>], mut kept: Option<&mut Flattened>,
    ) -> Result<Vec<u8>, Error> {
        if slots.first() != Some(&Some(0)) {
            return Err(Error::malformed("CFF2: a subset whose first glyph isn't .notdef"));
        }
        // Each slot's FD, an empty one taking the FD of the glyph its number names; a glyph past CharStrings, read
        // as empty, takes FD 0.
        let fd_of = |glyph: usize| self.fd_select.as_ref().and_then(|(select, cff2)| select.get(cff2, glyph));
        let slot_fds: Vec<usize> = slots.iter().enumerate()
            .map(|(i, slot)| {
                let fd = usize::from(fd_of(slot.map_or(i, |s| s as usize)).unwrap_or(0));
                if fd < self.fds.len() { fd } else { 0 }
            })
            .collect();
        let mut flat = Flattener { src: self, coords, budget: 0, over: false, scalars: Vec::new() };
        // Each glyph's spend charged in turn, failing where one budget across the glyphs would.
        let mut left = self.budget;
        let (mut data, mut ends) = (Vec::new(), Vec::with_capacity(slots.len()));
        for (&slot, &fd) in slots.iter().zip(&slot_fds) {
            if let Some(gid) = slot {
                let fd = self.fds.get(fd).ok_or(Error::malformed("CFF2: FDSelect names an FD past the FDArray"))?;
                let charstring = self.charstrings.get(gid as usize).unwrap_or_default();
                match kept.as_deref_mut() {
                    _ if let Some(drawn) = self.drawn.get(&gid) => data.extend_from_slice(drawn),
                    Some(kept) => {
                        let (glyph, spent) = kept.glyph(gid, |out| flat.glyph_alone(charstring, fd, out));
                        left = left.checked_sub(spent).ok_or(OVER_BUDGET)?;
                        data.extend_from_slice(glyph?);
                    }
                    None => {
                        let (flattened, spent) = flat.glyph_alone(charstring, fd, &mut data);
                        left = left.checked_sub(spent).ok_or(OVER_BUDGET)?;
                        flattened?;
                    }
                }
            }
            ends.push(data.len());
        }

        // FDSelect's ranges over the slots.
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        if self.fd_select.is_some() {
            for (i, &fd) in slot_fds.iter().enumerate() {
                if ranges.last().is_none_or(|&(_, last)| last != fd) {
                    ranges.push((i, fd));
                }
            }
        }
        let kept: Vec<usize> = match self.fd_select {
            Some(_) => ranges.iter().map(|&(_, fd)| fd).collect::<BTreeSet<_>>().into_iter().collect(),
            None => alloc::vec![0],
        };
        let rank = |fd: usize| kept.binary_search(&fd).unwrap_or(0);
        let fd_select = self.fd_select.is_some().then(|| fd_select_bytes(&ranges, kept.len(), slots.len(), rank));

        // Private DICTs in FD order, each written once, one alike byte for byte to one before it naming that one: the
        // smaller table.
        let (mut found, mut written) = (BTreeMap::new(), alloc::vec![None; self.privates.len()]);
        let mut private_of = alloc::vec![None; kept.len()];
        for (new, &fd) in kept.iter().enumerate() {
            let Some(fd) = self.fds.get(fd) else { continue };
            let Some(slot) = written.get_mut(fd.private) else { continue };
            if slot.is_none() {
                let ops = self.privates.get(fd.private).map_or(&[][..], Vec::as_slice);
                let private = write_private(ops, self.store.as_ref(), coords, &mut left)?;
                let next = found.len();
                *slot = Some((!private.is_empty()).then(|| *found.entry(private).or_insert(next)));
            }
            private_of[new] = slot.flatten();
        }
        let mut objects: Vec<(usize, Vec<u8>)> = found.into_iter().map(|(object, i)| (i, object)).collect();
        objects.sort_unstable_by_key(|&(i, _)| i);
        let objects: Vec<Vec<u8>> = objects.into_iter().map(|(_, object)| object).collect();

        // Laid out in turn: the header, Top DICT and global subroutines, the FDArray, FDSelect, the Private DICTs and
        // the charstrings.
        let top_len: usize = self.top.iter().map(|&(op, bytes)| match op {
            CHARSTRINGS | FD_ARRAY | FD_SELECT => 5 + op_len(op),
            VSTORE => 0,
            _ => bytes.len(),
        }).sum();
        let fonts: Vec<&Fd> = kept.iter().filter_map(|&fd| self.fds.get(fd)).collect();
        let font_len = |fd: &Fd| fd.font.iter().map(|&(op, b)| if op == PRIVATE { 9 } else { b.len() }).sum::<usize>();
        let font_ends: Vec<usize> = fonts.iter().scan(0, |end, fd| { *end += font_len(fd); Some(*end) }).collect();
        let fd_array_at = 5 + top_len + 4;
        let fd_select_at = fd_array_at + index_len(&font_ends);
        let mut at = fd_select_at + fd_select.as_ref().map_or(0, Vec::len);
        let mut object_at = Vec::with_capacity(objects.len());
        for object in &objects {
            object_at.push(at);
            at += object.len();
        }
        let charstrings_at = at;
        let fd_select_at = if fd_select.is_some() { fd_select_at } else { 0 };
        let offset = |v: usize| u32::try_from(v).map_err(|_| Error::limit("CFF2: an instance past 4 GB"));
        let top_size = u16::try_from(top_len).map_err(|_| Error::limit("CFF2: a Top DICT past 65,535 bytes"))?;

        let mut out = Vec::with_capacity(charstrings_at + index_len(&ends));
        out.extend([2, 0, 5]);
        out.extend(top_size.to_be_bytes());
        for &(op, bytes) in &self.top {
            let link = match op {
                CHARSTRINGS => charstrings_at,
                FD_ARRAY => fd_array_at,
                FD_SELECT => fd_select_at,
                VSTORE => continue,
                _ => {
                    out.extend_from_slice(bytes);
                    continue;
                }
            };
            out.push(29);
            out.extend(offset(link)?.to_be_bytes());
            push_op(&mut out, op);
        }
        out.extend([0; 4]);
        let mut font_data = Vec::with_capacity(font_ends.last().copied().unwrap_or(0));
        for (fd, private) in fonts.iter().zip(&private_of) {
            for &(op, bytes) in &fd.font {
                if op != PRIVATE {
                    font_data.extend_from_slice(bytes);
                    continue;
                }
                let (size, at) = private.map_or((0, 0), |i| (objects[i].len(), object_at[i]));
                let size = i16::try_from(size).map_err(|_| Error::limit("CFF2: a Private DICT past 32,767 bytes"))?;
                font_data.push(28);
                font_data.extend(size.to_be_bytes());
                font_data.push(29);
                font_data.extend(offset(at)?.to_be_bytes());
                font_data.push(18);
            }
        }
        write_index(&mut out, &font_ends, &font_data)?;
        if let Some(select) = fd_select {
            out.extend(select);
        }
        for object in &objects {
            out.extend_from_slice(object);
        }
        write_index(&mut out, &ends, &data)?;
        Ok(out)
    }
}

// The VariationStore at `at`: an ItemVariationStore within its `length` bytes (CFF2, "VariationStore"); none where
// that length is 0 or the store doesn't read there.
pub(crate) fn store_at(cff2: &[u8], at: i64) -> Option<ItemVariationStore> {
    let at = usize::try_from(at).ok().filter(|&at| at != 0)?;
    let size = usize::from(read_u16_be(cff2, at)?);
    let store = cff2.get(at + 2..)?.get(..size).filter(|store| !store.is_empty())?;
    ItemVariationStore::parse(store, 0).ok()
}

// An INDEX of 32-bit count, each element read where it's asked for.
#[derive(Clone, Copy, Default)]
struct Index<'a> {
    data: &'a [u8],
    count: usize,
    size: usize,
    // Where the offsets start, the byte before the first element, and the end the last offset gives.
    offsets: usize,
    base: usize,
    last: usize,
}

impl<'a> Index<'a> {
    fn len(&self) -> usize {
        self.count
    }

    fn offset(&self, i: usize) -> usize {
        let at = self.offsets + i * self.size;
        self.data[at..at + self.size].iter().fold(0, |v, &b| v << 8 | b as usize)
    }

    // An element whose offsets run backward or past the last is empty.
    fn get(&self, i: usize) -> Option<&'a [u8]> {
        if i >= self.count {
            return None;
        }
        let (start, end) = (self.offset(i), self.offset(i + 1));
        Some(if end < start || end > self.last { &[][..] } else { &self.data[self.base + start..self.base + end] })
    }
}

// An INDEX of 32-bit count `offset` from `base`; None where it reads none, an offset of 0 among them.
fn index_at(cff2: &[u8], base: usize, offset: i64) -> Option<Index<'_>> {
    let at = usize::try_from(base as i64 + offset).ok().filter(|_| offset != 0)?;
    let count = read_u32_be(cff2, at)? as usize;
    if count == 0 {
        return Some(Index::default());
    }
    let size = usize::from(*cff2.get(at + 4).filter(|&&s| (1..=4).contains(&s))?);
    let offsets = at + 5;
    let base = count.checked_add(1)?.checked_mul(size)?.checked_add(offsets).filter(|&end| end <= cff2.len())? - 1;
    let mut index = Index { data: cff2, count, size, offsets, base, last: 0 };
    index.last = index.offset(count);
    cff2.get(base..)?.get(..index.last)?;
    Some(index)
}

fn fd_select_bytes(ranges: &[(usize, usize)], fds: usize, glyphs: usize, rank: impl Fn(usize) -> usize) -> Vec<u8> {
    // Format 3 holds at most 255 FontDICTs and 65,535 glyphs, with 16-bit glyph ids; past either, format 4.
    let wide = fds > 255 || glyphs > 0xFFFF;
    let mut out = alloc::vec![if wide { 4 } else { 3 }];
    let put = |out: &mut Vec<u8>, v: usize, bytes: usize| out.extend(&(v as u32).to_be_bytes()[4 - bytes..]);
    let (gid, fd) = if wide { (4, 2) } else { (2, 1) };
    put(&mut out, ranges.len(), gid);
    for &(first, select) in ranges {
        put(&mut out, first, gid);
        put(&mut out, rank(select), fd);
    }
    put(&mut out, glyphs, gid);
    out
}

// The length of an INDEX of 32-bit count whose elements end at `ends`.
fn index_len(ends: &[usize]) -> usize {
    match ends.last() {
        None => 4,
        Some(&data) => 5 + (ends.len() + 1) * offset_size(data) + data,
    }
}

fn offset_size(data: usize) -> usize {
    (usize::BITS - (data + 1).leading_zeros()).div_ceil(8) as usize
}

fn write_index(out: &mut Vec<u8>, ends: &[usize], data: &[u8]) -> Result<(), Error> {
    let count = u32::try_from(ends.len()).map_err(|_| Error::limit("CFF2: an INDEX past 4 GB"))?;
    out.extend(count.to_be_bytes());
    if ends.is_empty() {
        return Ok(());
    }
    let size = offset_size(data.len());
    if size > 4 {
        return Err(Error::malformed("CFF2: an INDEX past 4 GB"));
    }
    out.push(size as u8);
    for end in core::iter::once(0).chain(ends.iter().copied()) {
        out.extend(&((end + 1) as u32).to_be_bytes()[4 - size..]);
    }
    out.extend_from_slice(data);
    Ok(())
}

fn op_len(op: u16) -> usize {
    if op > 0xFF { 2 } else { 1 }
}

fn push_op(out: &mut Vec<u8>, op: u16) {
    if op > 0xFF {
        out.push(12);
    }
    out.push(op as u8);
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Top,
    Font,
    Private,
}

#[derive(Default)]
struct Dict<'a> {
    ops: Vec<(u16, &'a [u8])>,
    charstrings: i32,
    fd_array: i32,
    fd_select: i32,
    vstore: i32,
    // Size, then offset.
    private: (u32, u32),
    subrs: i32,
    ivs: u32,
}

// Each operator with the bytes since the last: a Private DICT's blend stays with the operator after it, and an
// unknown operator drops its operands.
fn read_dict(bytes: &[u8], kind: Kind) -> Result<Dict<'_>, Error> {
    let mut dict = Dict::default();
    let (mut args, mut at, mut start, mut seen_vsindex) = (Args::default(), 0, 0, false);
    while at < bytes.len() {
        let op = fetch_op(bytes, &mut at);
        if let Some(v) = dict_number(op, bytes, &mut at)? {
            args.push(v)?;
            continue;
        }
        match (kind, op) {
            (Kind::Top, CHARSTRINGS) => dict.charstrings = args.pop_int()?,
            (Kind::Top, FD_ARRAY) => dict.fd_array = args.pop_int()?,
            (Kind::Top, FD_SELECT) => dict.fd_select = args.pop_int()?,
            (Kind::Top, VSTORE) => dict.vstore = args.pop_int()?,
            (Kind::Font, PRIVATE) => {
                let at = args.pop_uint()?;
                dict.private = (args.pop_uint()?, at);
            }
            (Kind::Private, SUBRS) => dict.subrs = args.pop_int()?,
            (Kind::Private, VSINDEX) => {
                if !seen_vsindex {
                    dict.ivs = args.pop_uint()?;
                }
                seen_vsindex = true;
            }
            (Kind::Private, BLEND) => {
                args.clear();
                continue;
            }
            _ => {}
        }
        args.clear();
        dict.ops.push((op, &bytes[start..at]));
        start = at;
    }
    Ok(dict)
}

// A Private DICT pinned at `coords`: each blend summed and rounded half away from 0 but a fraction's, vsindex, blend
// and Subrs left out, the hinting operators' operands written anew, any other operator's dropped. The values each
// blend reads are spent from the write's budget `left`.
fn write_private(
    ops: &[(u16, &[u8])], store: Option<&ItemVariationStore>, coords: &[f64], left: &mut usize,
) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let (mut ivs, mut scalars) = (0, None);
    for &(_, bytes) in ops.iter().filter(|&&(op, _)| op != SUBRS) {
        let (mut args, mut at) = (Args::default(), 0);
        while at < bytes.len() {
            let op = fetch_op(bytes, &mut at);
            if let Some(v) = dict_number(op, bytes, &mut at)? {
                args.push(v)?;
                continue;
            }
            match op {
                VSINDEX => {
                    ivs = args.pop_uint()?;
                    args.clear();
                    continue;
                }
                BLEND => {
                    // Read once a DICT, at the vsindex of its first blend, with no length check on the store.
                    let scalars: &Vec<i64> = scalars.get_or_insert_with(|| {
                        store.and_then(|s| data_scalars(s, ivs as usize, coords)).unwrap_or_default()
                    });
                    let operator = next_operator(bytes, at)?;
                    let read = args.blend(scalars.len(), Some(scalars), !FRACTIONAL.contains(&operator))?;
                    *left = left.checked_sub(read).ok_or(OVER_BUDGET)?;
                    continue;
                }
                op if HINTS.contains(&op) => {}
                _ => args.clear(),
            }
            for &v in &args.0 {
                write_dict_number(&mut out, v)?;
            }
            push_op(&mut out, op);
            args.clear();
        }
    }
    Ok(out)
}

// The glyphs a location's subsets have flattened, each with what it spent of the font's budget: a write that reads
// one charges it again, so it fails where flattening the glyph there would. Each glyph's box is kept as measured.
#[derive(Default)]
pub(crate) struct Flattened {
    glyphs: BTreeMap<u32, (Result<Vec<u8>, Error>, usize)>,
    boxes: BTreeMap<u32, Option<[i16; 4]>>,
    bytes: usize,
}

impl Flattened {
    fn glyph(
        &mut self, gid: u32, flatten: impl FnOnce(&mut Vec<u8>) -> (Result<(), Error>, usize),
    ) -> (Result<&[u8], Error>, usize) {
        let Flattened { glyphs, bytes, .. } = self;
        let (glyph, spent) = glyphs.entry(gid).or_insert_with(|| {
            let mut out = Vec::new();
            let (flattened, spent) = flatten(&mut out);
            out.shrink_to_fit();
            *bytes += out.capacity() + flattened.as_ref().err().map_or(0, Error::heap_bytes)
                + crate::daecore::daekit::base::sync::btree_entry_of::<u32, (Result<Vec<u8>, Error>, usize)>();
            (flattened.map(|()| out), spent)
        });
        (glyph.as_deref().map_err(Error::clone), *spent)
    }

    // Each of `glyphs`' boxes as `drawn_bounds` gives them at `coords`, a glyph drawn here once.
    pub(crate) fn boxes(&mut self, cff2: &[u8], coords: &[f64], glyphs: &[u32]) -> Vec<Option<[i16; 4]>> {
        let missing: Vec<u32> = glyphs.iter().copied().filter(|g| !self.boxes.contains_key(g)).collect();
        if !missing.is_empty() {
            let measured = super::instance::drawn_bounds(cff2, coords, missing.iter().copied());
            self.bytes += missing.len() * crate::daecore::daekit::base::sync::btree_entry_of::<u32, Option<[i16; 4]>>();
            self.boxes.extend(missing.into_iter().zip(measured));
        }
        glyphs.iter().map(|g| self.boxes.get(g).copied().flatten()).collect()
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        self.bytes
    }
}

struct Flattener<'s, 'a> {
    src: &'s Cff2Source<'a>,
    coords: &'s [f64],
    // What the glyph being flattened has left of the font's budget, and whether it ran past it.
    budget: usize,
    over: bool,
    // Each vsindex's region count and, where the location blends, its scalars, kept across glyphs.
    scalars: Vec<(u32, usize, Option<Vec<i64>>)>,
}

// One glyph's state as it's flattened.
#[derive(Default)]
struct Glyph {
    args: Args,
    hstems: usize,
    vstems: usize,
    mask: Option<usize>,
    ivs: u32,
    seen_vsindex: bool,
    // The vsindex blends were first read at, once one has been.
    blended: Option<u32>,
    // The glyph's absolute positions as they're rounded, where it's tried so, or why its operands can't be.
    rounding: Option<Result<Rounding, Error>>,
    // Whether an operator took a blend's operand, and whether there's a flex1, which decide how it's rounded.
    took_blend: bool,
    flex1: bool,
}

impl Flattener<'_, '_> {
    fn spend(&mut self, bytes: usize) -> Result<(), Error> {
        self.budget = self.budget.checked_sub(bytes).ok_or(OVER_BUDGET).inspect_err(|_| self.over = true)?;
        Ok(())
    }

    // A glyph flattened against the whole budget, and what it spent: up to its error where it fails, and all of it
    // where it runs past. Its spend alone decides whether a write's running budget fails first.
    fn glyph_alone(&mut self, charstring: &[u8], fd: &Fd, out: &mut Vec<u8>) -> (Result<(), Error>, usize) {
        (self.budget, self.over) = (self.src.budget, false);
        let flattened = self.glyph(charstring, fd, out);
        (flattened, if self.over { usize::MAX } else { self.src.budget - self.budget })
    }

    // A charstring, subroutines inlined, blends resolved. A glyph whose operators take a blend rounds at absolute
    // positions unless it has a flex1, else each blend alone; the first goes first, as most glyphs vary.
    fn glyph(&mut self, charstring: &[u8], fd: &Fd, out: &mut Vec<u8>) -> Result<(), Error> {
        let (start, budget) = (out.len(), self.budget);
        let g = self.run(charstring, fd, out, Some(Ok(Rounding::default())))?;
        if g.took_blend && !g.flex1 {
            return g.rounding.map_or(Ok(()), |rounded| rounded.map(drop));
        }
        out.truncate(start);
        self.budget = budget;
        self.run(charstring, fd, out, None).map(drop)
    }

    // One pass over a glyph, rounding as `rounding` says: each operator flushes its operands, endchar and return go,
    // and an unknown operator drops its operands.
    fn run(
        &mut self, charstring: &[u8], fd: &Fd, out: &mut Vec<u8>, rounding: Option<Result<Rounding, Error>>,
    ) -> Result<Glyph, Error> {
        let mut g = Glyph { ivs: fd.ivs, rounding, ..Glyph::default() };
        let mut calls: Vec<(&[u8], usize)> = Vec::new();
        let (mut bytes, mut at) = (charstring, 0);
        // Each operator reads a byte the budget charged, so the budget bounds the operators too.
        self.spend(bytes.len())?;
        loop {
            let op = match () {
                _ if at < bytes.len() => fetch_op(bytes, &mut at),
                _ if calls.is_empty() => ENDCHAR,
                _ => RETURN,
            };
            match op {
                RETURN => {
                    (bytes, at) = calls.pop().ok_or(Error::malformed("CFF2 charstring: return outside a subroutine"))?
                }
                ENDCHAR => {}
                255 => {
                    // Under four bytes, nothing is read and they run as operators.
                    if let Some(fixed) = bytes.get(at..at + 4) {
                        g.args.push(f64::from(i32::from_be_bytes([fixed[0], fixed[1], fixed[2], fixed[3]])) / 65536.0)?;
                        at += 4;
                    }
                }
                10 | 29 => {
                    let subrs = if op == 10 { &fd.subrs } else { &self.src.global_subrs };
                    let n = subr_index(g.args.pop_int()?, subr_bias(subrs.len()), subrs.len());
                    let subr = n.and_then(|n| subrs.get(n)).filter(|_| calls.len() < MAX_SUBR_DEPTH);
                    let subr =
                        subr.ok_or(Error::malformed("CFF2 charstring: a subroutine call out of range or too deep"))?;
                    calls.push((bytes, at));
                    (bytes, at) = (subr, 0);
                    self.spend(bytes.len())?;
                }
                1 | 18 => {
                    g.hstems += g.args.0.len() / 2;
                    g.flush(out, op);
                }
                3 | 23 => {
                    g.vstems += g.args.0.len() / 2;
                    g.flush(out, op);
                }
                19 | 20 => {
                    if g.mask.is_none() {
                        g.vstems += g.args.0.len() / 2;
                        g.mask = Some((g.hstems + g.vstems + 7) >> 3);
                    }
                    let size = g.mask.unwrap_or(0);
                    // A mask cut short is skipped, its bytes run as operators.
                    if let Some(mask) = bytes.get(at..at + size) {
                        g.flush(out, op);
                        out.extend_from_slice(mask);
                        at += size;
                    }
                }
                4..=8 | 21 | 22 | 24..=27 | 30 | 31 | 0x0C22..=0x0C25 => g.flush(out, op),
                16 => self.blend(&mut g)?,
                15 => {
                    let ivs = g.args.pop_uint()?;
                    if g.seen_vsindex || g.blended.is_some() {
                        return Err(Error::malformed("CFF2 charstring: vsindex after another or after a blend"));
                    }
                    (g.ivs, g.seen_vsindex) = (ivs, true);
                    g.args.clear();
                }
                _ => match number(op, bytes, &mut at)? {
                    Some(v) => g.args.push(v)?,
                    None => g.args.clear(),
                },
            }
            if op == ENDCHAR {
                return Ok(g);
            }
        }
    }

    // Charstrings blend only where the store's length isn't 0 and the font has axes.
    fn blend(&mut self, g: &mut Glyph) -> Result<(), Error> {
        let ivs = *g.blended.get_or_insert(g.ivs);
        let i = match self.scalars.iter().position(|&(v, ..)| v == ivs) {
            Some(i) => i,
            None => {
                let store = Some(self.src.store.as_ref().ok_or(Error::malformed("CFF2: a blend without a store"))?);
                let k = store.and_then(|s| s.ivd_data.get(ivs as usize)).map_or(0, |d| d.region_indices.len());
                let blends = !self.coords.is_empty();
                let scalars = store.filter(|_| blends).and_then(|s| data_scalars(s, ivs as usize, self.coords));
                self.scalars.push((ivs, k, scalars));
                self.scalars.len() - 1
            }
        };
        let (_, k, scalars) = &self.scalars[i];
        let read = g.args.blend(*k, scalars.as_ref(), g.rounding.is_none())?;
        self.spend(read)
    }
}

impl Glyph {
    fn flush(&mut self, out: &mut Vec<u8>, op: u16) {
        (self.took_blend, self.flex1) = (self.took_blend || self.args.1.is_some(), self.flex1 || op == 0x0C25);
        let failed = match &mut self.rounding {
            Some(Ok(at)) => at.round(op, &mut self.args.0).err(),
            _ => None,
        };
        if let Some(why) = failed {
            self.rounding = Some(Err(why));
        }
        for &v in &self.args.0 {
            write_charstring_number(out, v);
        }
        self.args.clear();
        push_op(out, op);
    }
}

// A glyph's pen as it's rounded, each axis's absolute position and that rounded half up: each operand is added to the
// one and becomes the step between the other and the last one rounded.
#[derive(Default)]
struct Rounding([(f64, f64); 2]);

impl Rounding {
    // The operands of a path or stem operator, failing where their count doesn't fit it. A stem's edges start from 0
    // each operator, a flex's depth is rounded alone, and an hflex returns to the y it started from.
    fn round(&mut self, op: u16, args: &mut [f64]) -> Result<(), Error> {
        let n = args.len();
        let stems = matches!(op, 1 | 3 | 18 | 19 | 20 | 23);
        let fits = match op {
            _ if stems => n.is_multiple_of(2),
            21 => n == 2,
            4 | 22 => n == 1,
            5 => n > 0 && n.is_multiple_of(2),
            6 | 7 => n > 0,
            8 => n > 0 && n.is_multiple_of(6),
            24 => n >= 8 && (n - 2).is_multiple_of(6),
            25 => n >= 8 && n.is_multiple_of(2),
            26 | 27 | 30 | 31 => n >= 4 && n % 4 <= 1,
            0x0C22 => n == 7,
            0x0C23 => n == 13,
            0x0C24 => n == 9,
            _ => return Ok(()),
        };
        if !fits {
            return Err(Error::owned(
                crate::daecore::ErrorKind::Malformed,
                format!("CFF2 charstring: operator {op} with {n} operands, which can't be rounded"),
            ));
        }
        let pen = &mut self.0;
        match op {
            _ if stems => {
                let mut edge = (0.0, 0.0);
                args.iter_mut().for_each(|v| step(&mut edge, v));
            }
            4 | 22 => step(&mut pen[usize::from(op == 4)], &mut args[0]),
            6 | 7 => alternate(pen, args, usize::from(op == 7)),
            26 | 27 => {
                let along = usize::from(op == 26);
                let (first, curves) = args.split_at_mut(n % 2);
                first.iter_mut().for_each(|v| step(&mut pen[1 - along], v));
                for curve in curves.as_chunks_mut::<4>().0 {
                    curve.iter_mut().zip([along, 0, 1, along]).for_each(|(v, axis)| step(&mut pen[axis], v));
                }
            }
            30 | 31 => {
                let mut along = usize::from(op == 30);
                let (curves, last) = args.split_at_mut(n - n % 4);
                for curve in curves.as_chunks_mut::<4>().0 {
                    curve.iter_mut().zip([along, 0, 1, 1 - along]).for_each(|(v, axis)| step(&mut pen[axis], v));
                    along = 1 - along;
                }
                last.iter_mut().for_each(|v| step(&mut pen[1 - along], v));
            }
            0x0C22 | 0x0C24 => {
                let (y, on_y): (_, &[usize]) = (pen[1], if op == 0x0C22 { &[2] } else { &[1, 3, 7] });
                args.iter_mut().enumerate().for_each(|(i, v)| step(&mut pen[usize::from(on_y.contains(&i))], v));
                pen[1] = y;
            }
            0x0C23 => {
                alternate(pen, &mut args[..12], 0);
                args[12] = (args[12] + 0.5).floor();
            }
            _ => alternate(pen, args, 0),
        }
        Ok(())
    }
}

fn alternate(pen: &mut [(f64, f64); 2], args: &mut [f64], first: usize) {
    args.iter_mut().enumerate().for_each(|(i, v)| step(&mut pen[(i + first) % 2], v));
}

fn step((at, rounded): &mut (f64, f64), v: &mut f64) {
    *at += *v;
    let previous = core::mem::replace(rounded, (*at + 0.5).floor());
    *v = *rounded - previous;
}

// The operands, and where the lowest a blend of regions gave lies.
#[derive(Default)]
struct Args(Vec<f64>, Option<usize>);

impl Args {
    fn push(&mut self, v: f64) -> Result<(), Error> {
        if self.0.len() == ARGS {
            return Err(Error::malformed("CFF2: past 513 operands"));
        }
        self.0.push(v);
        Ok(())
    }

    fn pop_int(&mut self) -> Result<i32, Error> {
        // A count or index is whole: truncated toward 0, and past i32 pinned to its range.
        let v = self.0.pop().map(|v| v as i32).ok_or(Error::malformed("CFF2: an operator short of operands"));
        self.1 = self.1.filter(|&lowest| lowest < self.0.len());
        v
    }

    fn pop_uint(&mut self) -> Result<u32, Error> {
        u32::try_from(self.pop_int()?).map_err(|_| Error::malformed("CFF2: a negative count"))
    }

    fn clear(&mut self) {
        self.0.clear();
        self.1 = None;
    }

    // The last count's values, each blended as drawing blends it by `scalars`, rounded half away from 0 where `round`
    // holds. Where `scalars` isn't one a region, the default alone. Gives how many defaults it read: they stay on the
    // stack as the results, where the next blend reads them again, so each is charged as it's read.
    fn blend(&mut self, k: usize, scalars: Option<&Vec<i64>>, round: bool) -> Result<usize, Error> {
        let n = self.pop_uint()? as usize;
        let count = self.0.len();
        let total = (k + 1)
            .checked_mul(n)
            .filter(|&total| total <= count)
            .ok_or(Error::malformed("CFF2: a blend short of operands"))?;
        let start = count - total;
        for i in 0..n {
            let deltas = &self.0[start + n + i * k..start + n + (i + 1) * k];
            let default = self.0[start + i];
            let v = scalars.filter(|s| s.len() == k).map_or(default, |s| blend_value(default, deltas, s));
            self.0[start + i] = if round { f64::from(v.round() as i32) } else { v };
        }
        self.0.truncate(count - k * n);
        self.1 = match k > 0 && n > 0 {
            true => Some(self.1.map_or(start, |lowest| lowest.min(start))),
            false => self.1.filter(|&lowest| lowest < start),
        };
        Ok(n)
    }
}

// The operator the operands from `at` are for, past any numbers and blends between; 0 at the DICT's end.
fn next_operator(bytes: &[u8], mut at: usize) -> Result<u16, Error> {
    while at < bytes.len() {
        let op = fetch_op(bytes, &mut at);
        if dict_number(op, bytes, &mut at)?.is_none() && op != BLEND {
            return Ok(op);
        }
    }
    Ok(0)
}

fn fetch_op(bytes: &[u8], at: &mut usize) -> u16 {
    let b = bytes[*at];
    *at += 1;
    if b != 12 {
        return u16::from(b);
    }
    match bytes.get(*at) {
        Some(&b) => {
            *at += 1;
            0x0C00 | u16::from(b)
        }
        None => INVALID,
    }
}

fn take<const N: usize>(bytes: &[u8], at: &mut usize) -> Result<[u8; N], Error> {
    let got =
        bytes.get(*at..*at + N).and_then(|b| b.try_into().ok()).ok_or(Error::malformed("CFF2: a number cut short"))?;
    *at += N;
    Ok(got)
}

// The numbers DICTs and charstrings share; None for an operator.
fn number(op: u16, bytes: &[u8], at: &mut usize) -> Result<Option<f64>, Error> {
    let v = match op {
        28 => i32::from(i16::from_be_bytes(take::<2>(bytes, at)?)),
        32..=246 => i32::from(op) - 139,
        247..=250 => (i32::from(op) - 247) * 256 + i32::from(take::<1>(bytes, at)?[0]) + 108,
        251..=254 => -(i32::from(op) - 251) * 256 - i32::from(take::<1>(bytes, at)?[0]) - 108,
        _ => return Ok(None),
    };
    Ok(Some(f64::from(v)))
}

fn dict_number(op: u16, bytes: &[u8], at: &mut usize) -> Result<Option<f64>, Error> {
    match op {
        29 => Ok(Some(f64::from(i32::from_be_bytes(take::<4>(bytes, at)?)))),
        30 => parse_real(bytes, at).map(Some),
        _ => number(op, bytes, at),
    }
}

// A real's nibbles, to its end nibble, as the decimal number they spell: digits, a point, E or E- and an exponent,
// a sign, read to the nearest f64. One that spells no number, or none f64 holds, doesn't read.
fn parse_real(bytes: &[u8], at: &mut usize) -> Result<f64, Error> {
    let unread = Error::malformed("CFF2: a real that doesn't read");
    let mut text = String::new();
    while let Some(&byte) = bytes.get(*at) {
        *at += 1;
        for nibble in [byte >> 4, byte & 15] {
            match nibble {
                0..=9 => text.push(char::from(b'0' + nibble)),
                10 => text.push('.'),
                11 => text.push('E'),
                12 => text.push_str("E-"),
                14 => text.push('-'),
                15 => return text.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or(unread),
                _ => return Err(unread),
            }
        }
    }
    Err(unread)
}

fn in_int_range(v: f64) -> bool {
    (-32768.0..=32767.0).contains(&v) && f64::from(v as i16) == v
}

fn write_int(out: &mut Vec<u8>, v: i32) {
    if !push_short_int(out, v) {
        out.push(28);
        out.extend((v.clamp(-32768, 32767) as i16).to_be_bytes());
    }
}

// A whole number in 16 bits as an integer, anything else as 16.16, pinned to its range.
fn write_charstring_number(out: &mut Vec<u8>, v: f64) {
    if in_int_range(v) {
        return write_int(out, v as i32);
    }
    out.push(255);
    out.extend(((v * 65536.0).clamp(i32::MIN.into(), i32::MAX.into()) as i32).to_be_bytes());
}

// The fewest bytes: a whole number in 16 bits as an integer, else a real's fewest digits reading back as `v`, plain
// or with an exponent (plain on a tie), or a long integer for a whole number where that's shorter.
fn write_dict_number(out: &mut Vec<u8>, v: f64) -> Result<(), Error> {
    if in_int_range(v) {
        write_int(out, v as i32);
        return Ok(());
    }
    if !v.is_finite() {
        return Err(Error::malformed("CFF2: a Private DICT value past f64's range"));
    }
    // `v` is `digits` times ten to `exp`.
    let shortest = format!("{:E}", v.abs());
    let (mantissa, exp) = shortest.split_once('E').unwrap_or((&shortest, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exp = exp.parse::<i32>().unwrap_or(0) - (digits.len() as i32 - 1);
    let places = exp.unsigned_abs() as usize;
    let plain = if exp >= 0 {
        format!("{digits}{}", "0".repeat(places))
    } else if places < digits.len() {
        let (whole, fraction) = digits.split_at(digits.len() - places);
        format!("{whole}.{fraction}")
    } else {
        format!(".{}{digits}", "0".repeat(places - digits.len()))
    };
    let scientific = format!("{digits}E{exp}");
    let nibbles = |s: &str| s.len() - s.matches("E-").count();
    let body = if nibbles(&scientific) < nibbles(&plain) { scientific } else { plain };
    let text = if v < 0.0 { format!("-{body}") } else { body };
    let mut nibbles = Vec::with_capacity(text.len() + 2);
    let mut chars = text.bytes().peekable();
    while let Some(c) = chars.next() {
        nibbles.push(match c {
            b'E' if chars.next_if_eq(&b'-').is_some() => 0x0C,
            b'E' => 0x0B,
            b'.' => 0x0A,
            b'-' => 0x0E,
            _ => c - b'0',
        });
    }
    nibbles.push(0x0F);
    if nibbles.len() % 2 == 1 {
        nibbles.push(0x0F);
    }
    if f64::from(v as i32) == v && 5 < 1 + nibbles.len() / 2 {
        out.extend(encode_cff_int(v as i32));
        return Ok(());
    }
    out.push(30);
    out.extend(nibbles.chunks(2).map(|pair| (pair[0] << 4) | pair[1]));
    Ok(())
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/instancer/cff2/write.rs"]
mod tests;
