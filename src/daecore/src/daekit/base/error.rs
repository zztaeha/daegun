use alloc::boxed::Box;
use alloc::string::String;
use core::fmt;

/// What a failure means for the caller, each kind something to act on differently.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum ErrorKind {
    /// Font data the call needs is malformed: truncated, out of range or inconsistent.
    Malformed,
    /// A format or version daegun doesn't read.
    Unsupported,
    /// A table or record the call needs isn't in the font.
    Missing,
    /// An argument is out of range or not a number.
    InvalidArgument,
    /// The font asks for more work or memory than daegun allows one font.
    LimitExceeded,
    /// The glyph has no outline to draw.
    NoOutline,
    /// A value outgrows what its type can hold once scaled.
    Overflow,
}

/// A failure: its kind, and a message naming the table or step and what went wrong there. Two are equal, and hash
/// alike, by their kind and their message's text.
#[derive(Clone)]
#[doc(alias("FontError", "QuadError"))]
pub struct Error {
    kind: ErrorKind,
    message: Message,
}

// Static for nearly every failure, so building one allocates nothing; owned for the few that name a number.
#[derive(Clone)]
enum Message {
    Static(&'static str),
    Owned(Box<str>),
}

/// `core::result::Result` with daegun's `Error`.
pub type Result<T, E = Error> = core::result::Result<T, E>;

impl Error {
    pub(crate) const fn new(kind: ErrorKind, message: &'static str) -> Error {
        Error { kind, message: Message::Static(message) }
    }

    pub(crate) fn owned(kind: ErrorKind, message: String) -> Error {
        Error { kind, message: Message::Owned(message.into_boxed_str()) }
    }

    pub(crate) const fn malformed(message: &'static str) -> Error {
        Error::new(ErrorKind::Malformed, message)
    }

    pub(crate) const fn unsupported(message: &'static str) -> Error {
        Error::new(ErrorKind::Unsupported, message)
    }

    pub(crate) const fn missing(message: &'static str) -> Error {
        Error::new(ErrorKind::Missing, message)
    }

    pub(crate) const fn invalid(message: &'static str) -> Error {
        Error::new(ErrorKind::InvalidArgument, message)
    }

    pub(crate) const fn limit(message: &'static str) -> Error {
        Error::new(ErrorKind::LimitExceeded, message)
    }

    // The same failure, its message led by the step that met it.
    pub(crate) fn in_context(self, context: &str) -> Error {
        Error::owned(self.kind, alloc::format!("{context}: {}", self.message()))
    }

    // What it holds on the heap, as a cache keeping it charges it.
    pub(crate) fn heap_bytes(&self) -> usize {
        match &self.message {
            Message::Static(_) => 0,
            Message::Owned(s) => s.len(),
        }
    }

    /// What kind of failure this is.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// The message: the table or step, then what went wrong.
    pub fn message(&self) -> &str {
        match &self.message {
            Message::Static(s) => s,
            Message::Owned(s) => s,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ErrorKind::Malformed => "malformed font data",
            ErrorKind::Unsupported => "unsupported format or version",
            ErrorKind::Missing => "missing table or record",
            ErrorKind::InvalidArgument => "invalid argument",
            ErrorKind::LimitExceeded => "limit exceeded",
            ErrorKind::NoOutline => "no outline",
            ErrorKind::Overflow => "overflow",
        })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Error").field("kind", &self.kind).field("message", &self.message()).finish()
    }
}

impl PartialEq for Error {
    fn eq(&self, other: &Error) -> bool {
        self.kind == other.kind && self.message() == other.message()
    }
}

impl Eq for Error {}

impl core::hash::Hash for Error {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        (self.kind, self.message()).hash(state);
    }
}

impl core::error::Error for Error {}

/// An `InvalidInput` for an [`InvalidArgument`](ErrorKind::InvalidArgument) and `InvalidData` for the rest, the
/// error kept as its source, so a loader returning `io::Result` takes `?` on both reading a file and opening it.
#[cfg(feature = "std")]
impl From<Error> for std::io::Error {
    fn from(error: Error) -> std::io::Error {
        let kind = match error.kind {
            ErrorKind::InvalidArgument => std::io::ErrorKind::InvalidInput,
            _ => std::io::ErrorKind::InvalidData,
        };
        std::io::Error::new(kind, error)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daekit/base/error.rs"]
mod tests;
