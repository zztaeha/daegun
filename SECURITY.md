# Security

## Reporting a vulnerability

Use **[private vulnerability reporting](https://github.com/zztaeha/daegun/security/advisories/new)**
on this repository. It is private between you and the maintainer until a fix ships.

Please do not open a public issue for a security problem.

What helps, roughly in order of usefulness:

- The font file, or the bytes, that triggers it. A crash without an input is a report nobody can act on.
- Which entry point was called, and with what arguments.
- Whether you were using the Rust API or the C ABI, and with which features enabled.
- The commit or released version.

Expect an acknowledgement within a week. daegun is maintained by one person, so a fix takes as long
as it takes, and you will be told which it is rather than left waiting.

## Supported versions

| Version | Supported |
|---|---|
| 1.2.x | Yes |
| 1.0.x, 1.1.x | No – upgrade to 1.2 for fixes |
| < 1.0 | No – `0.0.1` was a name reservation and contains no engine |

## The threat model

**A font is untrusted input.** That is the whole of it. daegun is built on the assumption that the
bytes it is handed are hostile: downloaded from a page, embedded in a document, uploaded by a user.
Every table offset, length and count in a font file is attacker-controlled, and none of them is
believed without being checked.

So the questions this project treats as security-relevant are:

- Can a font make it read outside a buffer?
- Can a font make it panic? A panic in a library is a denial of service, and with `panic = "abort"`
  it takes the process.
- Can a font make it allocate without bound, or loop without terminating?
- Can a font make it produce wrong output silently, where wrong output is a security property –
  a subsetter that keeps a glyph it was told to drop, for instance.

## What the code does about it

These are properties the build enforces, not intentions:

**Unsafe is denied by default, and exactly one subtree opts back in.** The crate root is
`#![deny(unsafe_code)]`. `daecore` – the whole engine, every parser, every table reader, the shaper –
is `#![forbid(unsafe_code)]`, which no inner `#[allow]` can override; the compiler rejects it as
`E0453`. The one that allows it says so in its own files: `ffi`, which turns C pointers back into
references. `unsafe_op_in_unsafe_fn` is denied crate-wide, so an `unsafe fn` gets no implicit unsafe
body.

**Nothing is read without a bounds check.** Every integer read from a font goes through a checked
reader that answers `None` rather than reading past the end. Offset arithmetic uses checked
addition, so an offset near `usize::MAX` declines instead of wrapping.

**Every unbounded thing has a bound.** Component recursion depth, charstring step count, points per
glyph, curves per glyph, closure passes, combining marks per cluster, and so on down: each is a named
constant in the source rather than a number inline. A table directory that claims to extract more
than four times its own file size, or 64 KB, whichever is larger, is refused outright, which is what
stops overlapping table entries from being a memory amplifier.

**The public path does not panic.** `clippy::unwrap_used` and `expect_used` are warned on every
non-test build and clippy runs in the gate. Where an `expect` survives, it carries a `reason`
explaining why the `None` arm is unreachable rather than merely unlikely.

**Fuzzed on every build.** The gate runs 500 mutated fonts, deterministically, each reproducible by
seed – a regression check rather than a search. `sh scripts/tools/fuzz/run.sh 40000` is the search.

**No dependencies.** daegun has none at all, so its supply chain is the Rust toolchain and nothing
else. There is no transitive crate to audit and none to be compromised.

## The C ABI, which is different

`daegun.h` is memory-safe on daegun's side and cannot be on yours. C has no borrow checker, so the
ABI is built so that a caller who follows five rules cannot get memory wrong, and a caller who does
not, can:

- A fallible call returns `daegun_status` and hands its result back through an out-parameter, so
  there is no in-band error value to mistake for a pointer.
- Every entry point validates its pointers and answers `DAEGUN_NULL` rather than dereferencing.
- daegun allocates and daegun frees. Calling C's `free()` on a daegun pointer is undefined behavior.
- A borrowed view is valid until the handle it came from is freed, or changed by a call that takes it
  as non-const, as a table map's set and remove do. Using it afterwards is use-after-free, and nothing
  can catch that for you.
- A handle a call takes as const may be shared by threads at once. One a call changes, a path being
  built or a pen's target, needs that call to itself.

**A crash caused by breaking those rules is a bug in the calling code, not a vulnerability in
daegun.** A crash caused by following them is a vulnerability, and worth reporting.

`daegun_font_open_owned` deserves a specific note: it reconstructs an allocation from a raw pointer,
so passing it anything other than a buffer from `daegun_font_buffer_new`, at exactly the length that
call was given, is heap corruption. That path is exercised under AddressSanitizer in the gate.

## What is not a vulnerability

- Wrong output that crosses no security property above, such as a misplaced glyph or a badly hinted
  outline. That is a bug; report it as an issue.
- A malformed font that is refused. Declining to parse hostile input is the intended behavior.
- Slow shaping or outlining on a pathological font, where the work is bounded and simply large.
- A crash from breaking the C ABI's documented rules, as above.
