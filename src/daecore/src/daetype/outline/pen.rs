/// What an outline is drawn onto, a contour at a time, y up.
///
/// # Examples
///
/// A pen writing a glyph as SVG path data, y up as the font draws it:
///
/// ```
/// use core::fmt::Write;
/// use daegun::outline::OutlinePen;
/// use daegun::Location;
///
/// struct Svg(String);
///
/// impl OutlinePen for Svg {
///     fn move_to(&mut self, x: f32, y: f32) {
///         let _ = write!(self.0, "M{x} {y}");
///     }
///     fn line_to(&mut self, x: f32, y: f32) {
///         let _ = write!(self.0, "L{x} {y}");
///     }
///     fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
///         let _ = write!(self.0, "Q{cx} {cy} {x} {y}");
///     }
///     fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
///         let _ = write!(self.0, "C{c1x} {c1y} {c2x} {c2y} {x} {y}");
///     }
///     fn close(&mut self) {
///         self.0.push('Z');
///     }
/// }
///
/// # fn main() -> daegun::Result<()> {
/// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
/// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
/// # let font = daegun::Font::from_bytes(&bytes)?;
/// let glyph = font.glyph_id('a').expect("Inter maps a");
/// let mut svg = Svg(String::new());
/// font.outline(glyph, &Location::default(), &mut svg)?;
/// assert!(svg.0.starts_with('M') && svg.0.ends_with('Z'));
/// # Ok(())
/// # }
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not an outline pen",
    note = "implement `OutlinePen` for it, or draw into a `Path`, which records the outline"
)]
pub trait OutlinePen {
    /// Starts a contour at a point.
    fn move_to(&mut self, x: f32, y: f32);
    /// A line to a point.
    fn line_to(&mut self, x: f32, y: f32);
    /// A quadratic curve through a control to a point.
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32);
    /// A cubic curve through two controls to a point.
    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32);
    /// Closes the contour.
    fn close(&mut self);
}

impl<P: OutlinePen + ?Sized> OutlinePen for &mut P {
    #[inline]
    fn move_to(&mut self, x: f32, y: f32) {
        (**self).move_to(x, y);
    }
    #[inline]
    fn line_to(&mut self, x: f32, y: f32) {
        (**self).line_to(x, y);
    }
    #[inline]
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        (**self).quad_to(cx, cy, x, y);
    }
    #[inline]
    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        (**self).curve_to(c1x, c1y, c2x, c2y, x, y);
    }
    #[inline]
    fn close(&mut self) {
        (**self).close();
    }
}

/// A pen passing what it's drawn on to another through a 2×3 transform `[a, b, c, d, dx, dy]`, reckoned in f64.
/// Over a pen of a known type it calls that pen directly; over `dyn OutlinePen`, any pen.
pub struct TransformPen<'a, P: OutlinePen + ?Sized> {
    inner: &'a mut P,
    t: [f64; 6],
    translate_only: bool,
}

impl<P: OutlinePen + ?Sized> core::fmt::Debug for TransformPen<'_, P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TransformPen").field("transform", &self.t).finish_non_exhaustive()
    }
}

impl<'a, P: OutlinePen + ?Sized> TransformPen<'a, P> {
    /// A pen drawing onto `inner` through `t`.
    pub fn new(inner: &'a mut P, t: [f64; 6]) -> Self {
        let [a, b, c, d, _, _] = t;
        let translate_only = a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0;
        TransformPen { inner, t, translate_only }
    }

    #[inline]
    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let (x, y) = (x as f64, y as f64);
        let [a, b, c, d, dx, dy] = self.t;
        if self.translate_only {
            return ((x + dx) as f32, (y + dy) as f32);
        }
        ((x * a + y * c + dx) as f32, (x * b + y * d + dy) as f32)
    }
}

// A point through `t` as a TransformPen moves it, for a walk that keeps a stack of transforms rather than of pens.
pub(crate) fn transformed(t: [f64; 6], x: f32, y: f32) -> (f32, f32) {
    let [a, b, c, d, dx, dy] = t;
    let (x, y) = (x as f64, y as f64);
    if a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0 {
        return ((x + dx) as f32, (y + dy) as f32);
    }
    ((x * a + y * c + dx) as f32, (x * b + y * d + dy) as f32)
}

impl<P: OutlinePen + ?Sized> OutlinePen for TransformPen<'_, P> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.apply(x, y);
        self.inner.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.apply(x, y);
        self.inner.line_to(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (cx, cy) = self.apply(cx, cy);
        let (x, y) = self.apply(x, y);
        self.inner.quad_to(cx, cy, x, y);
    }
    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let (c1x, c1y) = self.apply(c1x, c1y);
        let (c2x, c2y) = self.apply(c2x, c2y);
        let (x, y) = self.apply(x, y);
        self.inner.curve_to(c1x, c1y, c2x, c2y, x, y);
    }
    fn close(&mut self) { self.inner.close(); }
}
