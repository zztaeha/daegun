mod ext;
mod trig;

// A dependent has to forward its own `std` feature here, or this crate takes the `no_std` path
// while the rest of the build takes the fast one.
pub use ext::FloatExt;
pub use trig::{atan2, sin_cos, sin_cos_degrees};
