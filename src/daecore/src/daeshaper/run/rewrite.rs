// A rewrite reads the run's glyphs in order at `cursor` and writes a new sequence of `written` glyphs. Writes land
// over the glyphs already read for as long as they stay behind the cursor, so most rewrites copy nothing.
use super::{Glyph, Output, Run};

impl Run {
    pub(crate) fn start_rewrite(&mut self) {
        (self.output, self.cursor, self.written) = (Output::Overlaid, 0, 0);
        self.positioned = false;
    }

    // The unread glyphs follow the written ones, and the written sequence becomes the run. A run that failed keeps the
    // glyphs it read where its output was kept apart; written over them, they can't be kept, and a glyph read and left
    // standing would draw its text twice, so it keeps what it wrote and the unread, which always fit.
    pub(crate) fn finish_rewrite(&mut self) {
        debug_assert!(self.is_rewriting() && self.cursor <= self.len);
        let unread = self.len - self.cursor;
        if (self.output == Output::Overlaid || !self.failed) && self.keep_n(unread) {
            if self.output == Output::Apart {
                core::mem::swap(&mut self.glyphs, &mut self.apart);
            }
            self.len = self.written;
        }
        (self.output, self.cursor, self.written) = (Output::Off, 0, 0);
    }

    pub(crate) fn is_rewriting(&self) -> bool {
        self.output != Output::Off
    }

    // The written glyphs, wherever a rewrite keeps them; without one, the run's own.
    pub(crate) fn out(&self) -> &[Glyph] {
        if self.output == Output::Apart { &self.apart } else { &self.glyphs }
    }

    pub(crate) fn out_mut(&mut self) -> &mut [Glyph] {
        if self.output == Output::Apart { &mut self.apart } else { &mut self.glyphs }
    }

    pub(crate) fn current(&self) -> &Glyph {
        &self.glyphs[self.cursor]
    }

    pub(crate) fn current_mut(&mut self) -> &mut Glyph {
        &mut self.glyphs[self.cursor]
    }

    // The glyph written last; the first written where none is yet.
    pub(crate) fn last_out(&self) -> &Glyph {
        &self.out()[self.written.max(1) - 1]
    }

    // Glyphs before the cursor: those written during a rewrite, else the run's own.
    pub(crate) fn behind(&self) -> usize {
        match self.output {
            Output::Off => self.cursor,
            _ => self.written,
        }
    }

    // The run's length as the rewrite stands: the glyphs behind the cursor and those still unread.
    pub(crate) fn live_len(&self) -> usize {
        self.behind() + (self.len - self.cursor)
    }

    // Room for an edit reading `reading` glyphs and writing `writing`; output that would overtake the cursor moves
    // apart first.
    #[must_use]
    fn room_to_write(&mut self, reading: usize, writing: usize) -> bool {
        let end = self.written + writing;
        let room = self.room_for(end);
        if room && self.output == Output::Overlaid && end > self.cursor + reading {
            self.move_apart();
        }
        room
    }

    // Within the room `room_for` keeps, so this allocates nothing.
    fn move_apart(&mut self) {
        let size = self.glyphs.len().max(self.apart.len());
        self.apart.resize(size, Glyph::of_char(0, 0));
        self.apart[..self.written].copy_from_slice(&self.glyphs[..self.written]);
        self.output = Output::Apart;
    }

    // After `room_to_write`, which leaves the room.
    fn write(&mut self, glyph: Glyph) {
        let at = self.written;
        self.out_mut()[at] = glyph;
        self.written += 1;
    }

    // Whether a glyph kept now already sits where it would be written.
    fn in_place(&self) -> bool {
        self.output == Output::Overlaid && self.written == self.cursor
    }

    // Writes the current glyph as it is and reads on; false, the cursor unmoved, where the run had no room.
    pub(crate) fn keep(&mut self) -> bool {
        self.copy_on(1)
    }

    // The next `n` glyphs as they are, all or none.
    pub(crate) fn keep_n(&mut self, n: usize) -> bool {
        debug_assert!(self.cursor + n <= self.len);
        self.copy_on(n)
    }

    fn copy_on(&mut self, n: usize) -> bool {
        let from = self.cursor;
        match self.output {
            Output::Off => {}
            _ if self.in_place() => self.written += n,
            _ => {
                if !self.room_to_write(n, n) {
                    return false;
                }
                let to = self.written;
                if self.output == Output::Apart {
                    self.apart[to..to + n].copy_from_slice(&self.glyphs[from..from + n]);
                } else {
                    self.glyphs.copy_within(from..from + n, to);
                }
                self.written += n;
            }
        }
        self.cursor = from + n;
        true
    }

    // Reads past the current glyph without writing it.
    pub(crate) fn pass_over(&mut self) {
        self.cursor += 1;
    }

    // Writes the current glyph as `id` and reads on.
    pub(crate) fn replace(&mut self, id: u32) -> bool {
        let mut glyph = *self.current();
        glyph.id = id;
        if self.in_place() {
            self.glyphs[self.cursor] = glyph;
            self.written += 1;
        } else {
            if !self.room_to_write(1, 1) {
                return false;
            }
            self.write(glyph);
        }
        self.cursor += 1;
        true
    }

    // Reads `n` glyphs and writes one copy of the first for each of `ids`, their clusters joined.
    pub(crate) fn replace_span(&mut self, n: usize, ids: &[u32]) {
        debug_assert!(self.cursor + n <= self.len);
        if !self.room_to_write(n, ids.len()) {
            return;
        }
        self.join_clusters(self.cursor..self.cursor + n);
        let first = *self.current();
        for &id in ids {
            self.write(Glyph { id, ..first });
        }
        self.cursor += n;
    }

    // Writes a copy of the current glyph, or past the end of the last written one, as `id`; the cursor stays.
    pub(crate) fn insert(&mut self, id: u32) {
        let model = match self.cursor < self.len {
            true => Some(*self.current()),
            false => self.written.checked_sub(1).map(|last| self.out()[last]),
        };
        if let Some(model) = model
            && self.room_to_write(0, 1)
        {
            self.write(Glyph { id, ..model });
        }
    }

    pub(crate) fn insert_glyph(&mut self, glyph: Glyph) {
        if self.room_to_write(0, 1) {
            self.write(glyph);
        }
    }

    // Moves the line between written and unread glyphs to `at` written glyphs: forward copies unread ones out, back
    // returns written ones to be read again. Each glyph moved is spent.
    pub(crate) fn seek(&mut self, at: usize) -> bool {
        match self.output {
            Output::Off => {
                debug_assert!(at <= self.len);
                self.cursor = at;
                true
            }
            _ if self.failed => false,
            _ if at > self.written => self.seek_forward(at - self.written),
            _ => self.seek_back(self.written - at),
        }
    }

    fn seek_forward(&mut self, count: usize) -> bool {
        let allowed = self.cursor + count <= self.len && self.room_for(self.written + count) && self.spend(count);
        if allowed {
            // A copy that fails has failed the run, which every later edit sees; the move itself was allowed.
            self.copy_on(count);
        }
        allowed
    }

    fn seek_back(&mut self, count: usize) -> bool {
        if count == 0 {
            return true;
        }
        // Only output kept apart can hold more glyphs than the cursor has behind it.
        let short = count.saturating_sub(self.cursor);
        if (short > 0 && !self.open_before_cursor(short)) || !self.spend(count) {
            return false;
        }
        (self.written, self.cursor) = (self.written - count, self.cursor - count);
        let (from, to) = (self.written, self.cursor);
        match self.output {
            Output::Apart => self.glyphs[to..to + count].copy_from_slice(&self.apart[from..from + count]),
            _ => self.glyphs.copy_within(from..from + count, to),
        }
        true
    }

    // Shifts the unread glyphs `count` further on, spending each shifted.
    fn open_before_cursor(&mut self, count: usize) -> bool {
        if !self.room_for(self.len + count) || !self.spend(self.len - self.cursor) {
            return false;
        }
        self.glyphs.copy_within(self.cursor..self.len, self.cursor + count);
        self.len += count;
        self.cursor += count;
        true
    }

    // Reads past the current glyph, dropping it. A cluster no neighbor shares lives on in one: in the written glyphs
    // before it where it is the lower, else in the next glyph.
    pub(crate) fn delete_current(&mut self) {
        let gone = *self.current();
        let next_shares = self.glyphs[self.cursor + 1..self.len].first().is_some_and(|g| g.cluster == gone.cluster);
        let last_shares = self.written > 0 && self.last_out().cluster == gone.cluster;
        if !next_shares && !last_shares {
            if self.written > 0 {
                let written = self.written;
                super::clusters::hand_down(&mut self.out_mut()[..written], gone.cluster, gone.mask);
            } else if self.cursor + 1 < self.len {
                self.join_clusters(self.cursor..self.cursor + 2);
            }
        }
        self.pass_over();
    }
}
