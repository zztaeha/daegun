//! The fuzzers' shared core: the cross-checks every public call is held to, the input each target reads, the
//! targets, which libFuzzer, AFL++ and the gate's random runner all call, and the heap and stack they're measured by.

macro_rules! must {
    ($cond:expr, $($msg:tt)*) => {
        if !$cond { panic!("invariant: {}", format!($($msg)*)) }
    };
}

mod checks;
mod input;
mod memory;
mod stack;
mod targets;
mod tools;

pub use checks::*;
pub use input::{sections, Choices, Input, CHOICES};
pub use memory::{bounded, ALLOCATION_LIMIT, MEMORY_LIMIT};
pub use stack::{guard_thread, stack_size, STACK_KIB};
pub use targets::*;
pub use tools::*;
