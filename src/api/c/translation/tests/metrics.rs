use super::*;
use crate::metrics::StatAxisValue;
use crate::Orientation;
use crate::ffi::testing::{Diffs, bits, c_text, each_font, found, lent, str_of};
use crate::metrics::{DecorationLine, ScriptMetrics};
use core::ptr::null_mut;

const ORIENTATIONS: [(i32, Orientation); 2] =
    [(crate::ffi::HORIZONTAL, Orientation::Horizontal), (crate::ffi::VERTICAL, Orientation::Vertical)];

fn line(m: &LineMetricsC) -> [u64; 3] {
    [bits(m.ascent), bits(m.descent), bits(m.line_gap)]
}

// C against Rust over the corpus, as every area's ignored test does: run them with `--ignored`.
#[test]
#[ignore]
fn c_answers_as_rust_over_the_corpus() {
    let mut d = Diffs::default();
    each_font(&mut d, |f, d| unsafe {
        let at = f.name.as_str();
        let mut b = crate::Bounds { x_min: 0.0, y_min: 0.0, x_max: 0.0, y_max: 0.0 };
        let st = daegun_font_bounds(f.c, &mut b);
        let rust = f.rust.bounds();
        let c = (st == Status::Ok).then_some([bits(b.x_min), bits(b.y_min), bits(b.x_max), bits(b.y_max)]);
        let want = rust.map(|b| [bits(b.x_min), bits(b.y_min), bits(b.x_max), bits(b.y_max)]);
        d.eq(at, "bounds", (st, c), (found(&rust), want));
        for size in [6.0, 9.0, 12.0, 48.0, 400.0, -1.0, f64::NAN] {
            for (code, o) in ORIENTATIONS {
                let mut t = 0.0;
                let st = daegun_font_tracking(f.c, size, code, &mut t);
                d.eq(at, "tracking", (st, bits(t)), (Status::Ok, bits(f.rust.tracking(size, o))));
            }
        }

        for place in f.places() {
            let at = &alloc::format!("{at} @{}", place.label);
            let one = |d: &mut Diffs,
                       what,
                       call: unsafe extern "C" fn(*const Font, *const LocationHandle, *mut f64) -> Status,
                       want: f64| {
                let mut v = 0.0;
                let st = call(f.c, place.c, &mut v);
                d.eq(at, what, (st, bits(v)), (Status::Ok, bits(want)));
            };
            one(d, "ascender", daegun_font_ascender, f.rust.ascender(&place.rust));
            one(d, "descender", daegun_font_descender, f.rust.descender(&place.rust));
            one(d, "cap_height", daegun_font_cap_height, f.rust.cap_height(&place.rust));
            one(d, "italic_angle", daegun_font_italic_angle, f.rust.italic_angle(&place.rust));
            for (code, o) in ORIENTATIONS {
                let mut m = LineMetricsC { ascent: 0.0, descent: 0.0, line_gap: 0.0 };
                let st = daegun_font_line_metrics(f.c, place.c, code, &mut m);
                let r = f.rust.line_metrics(&place.rust, o);
                d.eq(
                    at,
                    "line_metrics",
                    (st, line(&m)),
                    (Status::Ok, [bits(r.ascent), bits(r.descent), bits(r.line_gap)]),
                );
                let mut h = 0.0;
                let st = daegun_line_metrics_line_height(&m, &mut h);
                d.eq(at, "line_height", (st, bits(h)), (Status::Ok, bits(r.line_height())));
            }
            let mut flags = 0;
            let st = daegun_font_pdf_flags(f.c, place.c, &mut flags);
            d.eq(at, "pdf_flags", (st, flags), (Status::Ok, f.rust.pdf_flags(&place.rust).bits()));

            let mut o = core::mem::MaybeUninit::<Os2C>::zeroed();
            let st = daegun_font_os2(f.c, place.c, o.as_mut_ptr());
            let rust = f.rust.os2(&place.rust);
            d.eq(at, "os2", st, found(&rust));
            if let (Status::Ok, Some(r)) = (st, rust) {
                let c = o.assume_init();
                let win = r.win_metrics.map(|w| (bits(w.ascent), bits(w.descent)));
                let typo = r.typo_metrics.map(|t| (bits(t.ascent), bits(t.descent), bits(t.line_gap)));
                let c_win = (c.has_win_metrics != 0).then_some((bits(c.win_ascent), bits(c.win_descent)));
                let c_typo = (c.has_typo_metrics != 0).then_some((
                    bits(c.typo_ascender),
                    bits(c.typo_descender),
                    bits(c.typo_line_gap),
                ));
                let c_family = (c.has_family_class != 0).then_some(c.family_class);
                let c_selection = (c.has_selection != 0).then_some(c.selection);
                d.eq(
                    at,
                    "os2 fields",
                    (c.version, c_family, c_selection, c_win, c_typo),
                    (r.version, r.family_class, r.selection, win, typo),
                );
                let says: [(&str, unsafe extern "C" fn(*const Os2C, *mut bool) -> Status, bool); 5] = [
                    ("is_italic", daegun_os2_is_italic, r.is_italic()),
                    ("is_bold", daegun_os2_is_bold, r.is_bold()),
                    ("is_regular", daegun_os2_is_regular, r.is_regular()),
                    ("is_oblique", daegun_os2_is_oblique, r.is_oblique()),
                    ("uses_typo_metrics", daegun_os2_uses_typo_metrics, r.uses_typo_metrics()),
                ];
                for (what, call, want) in says {
                    let mut v = false;
                    let st = call(&c, &mut v);
                    d.eq(at, what, (st, v), (Status::Ok, want));
                }
            }

            let mut t = core::mem::MaybeUninit::<TypographicMetricsC>::zeroed();
            let st = daegun_font_typographic_metrics(f.c, place.c, t.as_mut_ptr());
            let c = t.assume_init();
            let r = f.rust.typographic_metrics(&place.rust);
            let line2 = |has: bool, v: [f64; 2]| has.then_some([bits(v[0]), bits(v[1])]);
            let line4 = |has: bool, v: [f64; 4]| has.then_some(v.map(bits));
            let r_line = |l: Option<DecorationLine>| l.map(|l| [bits(l.position), bits(l.thickness)]);
            let r_script = |s: Option<ScriptMetrics>| {
                s.map(|s| [bits(s.x_size), bits(s.y_size), bits(s.x_offset), bits(s.y_offset)])
            };
            let c_all = (
                st,
                c.has_x_height.then_some(bits(c.x_height)),
                line2(c.has_underline, c.underline),
                line2(c.has_strikeout, c.strikeout),
                line4(c.has_subscript, c.subscript),
                line4(c.has_superscript, c.superscript),
            );
            let r_all = (
                Status::Ok,
                r.x_height.map(bits),
                r_line(r.underline),
                r_line(r.strikeout),
                r_script(r.subscript),
                r_script(r.superscript),
            );
            d.eq(at, "typographic_metrics", c_all, r_all);

            let mut stat = null_mut();
            let st = daegun_font_stat(f.c, place.c, &mut stat);
            let rust = f.rust.stat(&place.rust);
            d.eq(at, "stat", st, found(&rust));
            if let (Status::Ok, Some(r)) = (st, rust) {
                let mut n = 0;
                let axes: Vec<_> =
                    lent(daegun_stat_axes(stat, &mut n), n).iter().map(|a| (a.tag, a.ordering, a.has_name)).collect();
                let want: Vec<_> = r.axes.iter().map(|a| (a.tag.0, a.ordering, a.name.is_some())).collect();
                d.eq(at, "stat axes", axes, want);
                for (i, a) in r.axes.iter().enumerate() {
                    let mut s = Str::EMPTY;
                    let st = daegun_stat_axis_name(stat, i, &mut s);
                    let name = (st == Status::Ok).then(|| str_of(s));
                    d.eq(at, "stat axis name", (st, name), (found(&a.name), a.name.as_deref().map(c_text)));
                }
                let mut s = Str::EMPTY;
                let st = daegun_stat_elided_fallback_name(stat, &mut s);
                let name = (st == Status::Ok).then(|| str_of(s));
                let want = r.elided_fallback_name.as_deref().map(c_text);
                d.eq(at, "stat elided name", (st, name), (found(&r.elided_fallback_name), want));
                let _ = daegun_stat_value_count(stat, &mut n);
                d.eq(at, "stat value count", n, r.values.len());
                let mut k = 0;
                let combos = lent(daegun_stat_combo_values(stat, &mut k), k);
                for (i, v) in r.values.iter().enumerate() {
                    let mut c = core::mem::MaybeUninit::<StatValueC>::zeroed();
                    let st = daegun_stat_value_at(stat, i, c.as_mut_ptr());
                    let c = c.assume_init();
                    let mut combo = Vec::new();
                    let (kind, axis, name, value, min, max, linked, elidable, older) = match v {
                        StatAxisValue::Single { axis_index, name, value, elidable, older_sibling } => {
                            (STAT_SINGLE, *axis_index, name, *value, 0.0, 0.0, 0.0, *elidable, *older_sibling)
                        }
                        StatAxisValue::Range { axis_index, name, nominal, min, max, elidable, older_sibling } => {
                            (STAT_RANGE, *axis_index, name, *nominal, *min, *max, 0.0, *elidable, *older_sibling)
                        }
                        StatAxisValue::Linked { axis_index, name, value, linked_value, elidable, older_sibling } => {
                            (STAT_LINKED, *axis_index, name, *value, 0.0, 0.0, *linked_value, *elidable, *older_sibling)
                        }
                        StatAxisValue::Combo { name, values, elidable, older_sibling } => {
                            combo = values.iter().map(|&(a, v)| (a, bits(v))).collect();
                            (STAT_COMBO, 0, name, 0.0, 0.0, 0.0, 0.0, *elidable, *older_sibling)
                        }
                    };
                    let start = c.combo_start as usize;
                    let c_combo = combos.get(start..start + c.combo_count as usize).unwrap_or(&[]);
                    let c_combo: Vec<_> = c_combo.iter().map(|a| (a.axis_index, bits(a.value))).collect();
                    d.eq(
                        at,
                        "stat value",
                        (st, c.kind, c.axis_index, c.has_name, bits(c.value), bits(c.min), bits(c.max)),
                        (Status::Ok, kind, axis, name.is_some(), bits(value), bits(min), bits(max)),
                    );
                    d.eq(
                        at,
                        "stat value rest",
                        (bits(c.linked_value), c.elidable, c.older_sibling, c_combo),
                        (bits(linked), elidable, older, combo),
                    );
                    let mut s = Str::EMPTY;
                    let st = daegun_stat_value_name(stat, i, &mut s);
                    let got = (st == Status::Ok).then(|| str_of(s));
                    d.eq(at, "stat value name", (st, got), (found(name), name.as_deref().map(c_text)));
                }
            }
            daegun_stat_free(stat);
        }
    });
    d.done("metrics");
}
