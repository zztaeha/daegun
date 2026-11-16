/* daegun: a no_std, zero-dependency OpenType engine, from C. What every other header here builds on: the rules
 * every call keeps, statuses, borrowed views, owned lists, tags, bounds, fonts and locations. Include daegun.h,
 * which includes the rest.
 *
 * Everything the Rust API can do, C can do, but for what a C caller writes for itself: building a display
 * list or a paint graph, Rgba's and a matrix's arithmetic, and a color ramp of a gradient no display list
 * holds. These headers are the contract.
 *
 * The C ABI is behind daegun's `capi` feature. Ask for the shape you want:
 *
 *     cargo rustc --release --features capi --crate-type cdylib     # libdaegun.so / .dylib, daegun.dll
 *     cargo rustc --release --features capi --crate-type staticlib  # libdaegun.a / daegun.lib
 *
 * `capi` implies `threading`, which is what makes rule 5 below true. A program that calls daegun-2.dll
 * defines DAEGUN_DLL before including daegun.h, so each call goes straight to the DLL; one linking the
 * static library leaves it undefined.
 *
 * The static library needs the system libraries Rust's standard library links against, as
 * `--print native-static-libs` reports them; the shared library carries its own:
 *
 *     macOS / iOS   cc app.c libdaegun.a
 *     Linux         cc app.c libdaegun.a -lgcc_s -lutil -lrt -lpthread -lm -ldl
 *     Linux, musl   cc app.c libdaegun.a
 *     Android       cc app.c libdaegun.a -llog -lm
 *     Windows       cl /std:c11 /MD app.c daegun.lib kernel32.lib ntdll.lib userenv.lib ws2_32.lib dbghelp.lib
 *     MinGW         gcc app.c libdaegun.a -lkernel32 -lntdll -luserenv -lws2_32 -ldbghelp
 *
 * Windows's list ends in /defaultlib:msvcrt, the DLL C runtime that /MD names. An application built /MT
 * needs the library built with `-C target-feature=+crt-static`, as every object in one link takes one runtime.
 * A shared library of yours linking libdaegun.a exports daegun's calls as its own unless it hides them:
 * -Wl,--exclude-libs,libdaegun.a on Linux, Android and MinGW, -Wl,-hidden-ldaegun on macOS in place of
 * -ldaegun.
 *
 * THE FIVE RULES. Every function here obeys all of them, so there is nothing to remember per call.
 *
 *   1. A fallible call returns daegun_status, and its results come back through out-parameters. One
 *      that fails leaves a number or struct out as it was, so you may preset a fallback, and sets a
 *      handle out to NULL, so freeing it on the way out is always safe.
 *   2. Passing NULL where a pointer is required returns DAEGUN_NULL. It is never dereferenced. A
 *      pointer that may be NULL is marked DAEGUN_NULLABLE, and every other is required. One that
 *      comes with a count may be NULL where the count is 0, unless its call says it may be NULL
 *      whatever the count. A location is never required: NULL is the default instance.
 *   3. daegun allocates and daegun frees. A handle comes back through a non-const `T **` out and
 *      has one _free, which takes NULL as free(NULL) does; anything const is borrowed. Never call
 *      free() on a pointer daegun gave you: this library has its own allocator. No handle borrows
 *      another, so free them in any order: a run, a layout or a location may outlive its font. A
 *      freed handle is kept for the thread's next one of its kind; a library built with
 *      `--cfg daegun_no_spares` frees each at once, so ASan or Valgrind sees a use after the free.
 *   4. A borrowed view is a const pointer plus a count, valid until the handle it came from is
 *      freed or changed by a call that takes it as non-const. Copy it if you need it longer.
 *   5. A handle a call takes as const may be used from several threads at once. Sharing a font scales,
 *      each thread keeping its own working state. A thread keeps what it last hinted with and the
 *      locations it last read at (up to 4 hinting contexts, 4 autohinters and 2 locations), so a freed
 *      font's bytes may stay until it hints or reads at locations of other fonts, ends, or calls
 *      daegun_thread_release.
 *      A handle a call takes as non-const (a path being built, quads being rewound, a packer, a table
 *      map), or one a pen draws into, needs that handle to itself until it returns.
 *      Any handle may be freed on any thread, once no call is using it.
 *
 * UNITS. Every metric, advance, offset, bound and outline is in font units, a double, y up and unrounded,
 * as the font's units per em measure them. Calls that hint or prepare a glyph take a pixel size and
 * answer in pixels.
 *
 * TEXT. Text comes as UTF-8 and a count of its bytes, so a slice of a larger buffer goes in without a
 * copy, and U+0000 is a character like any other. A u8"" literal, whose characters are char8_t from C23
 * and C++20, goes in cast to const char *. A short identifier, such as a BCP 47 language tag, is
 * NUL-terminated instead. Text that isn't UTF-8 answers DAEGUN_INVALID_ARGUMENT, with the byte where it
 * stops being UTF-8 in daegun_last_error(). A position in text is a byte offset where a character starts,
 * as in Rust, so text + start is where what it names begins.
 *
 * BUFFERS. A list of fixed-size items, such as glyph ids, tags or axes, goes into your buffer rather
 * than a handle, so a buffer kept between calls means none allocates; handles hold strings and what
 * nests. The count a result needs is always set: a NULL buffer and a length of 0 ask for it alone and
 * answer DAEGUN_OK, and a buffer too small answers DAEGUN_INVALID_ARGUMENT and writes nothing into it.
 * One counted per byte or per character fits room for the text's byte count, as no character takes
 * less than a byte, and a call whose count has a bound you can know says so. A buffer a call fills,
 * and the count it sets, share no byte with the text or array the call reads or with each other, as
 * with memcpy: buffers that overlap answer DAEGUN_INVALID_ARGUMENT, and nothing is written.
 *
 * OPTIONS. Each options struct opens with `size_t size`, the bytes your build of it holds. Fill one with
 * its _default(&opts, sizeof opts), which zeroes it, sets its defaults and its size, then set the fields
 * you want. daegun reads no byte past `size`, so a struct from an older header works with a newer
 * library; one from a newer header, larger than this library knows, answers DAEGUN_UNSUPPORTED, and one
 * below 2.0's size, as a zeroed struct's 0 is, DAEGUN_INVALID_ARGUMENT. A NULL options pointer is the
 * defaults.
 *
 * YES, NO AND CODES. A yes or no is a `bool`, given or answered, field or argument. A list of them comes
 * as bytes, 1 for yes and 0 for no. A code that names one of several, such as a DAEGUN_* mode, is an
 * int32_t, and one that names none of its constants answers DAEGUN_INVALID_ARGUMENT rather than a guess.
 *
 * PANICS. The public path contains no unwrap, expect or panic!, and a gate step keeps it that way.
 * Were one to occur it would end the process rather than unwind into your stack frame, which across
 * this boundary would be undefined behavior: a release build sets panic = "abort", and in any build Rust
 * aborts a panic that would leave an extern "C" function. Running out of memory ends the process too,
 * as Rust's allocations do; daegun_font_buffer_new alone answers NULL.
 *
 * CALLBACKS. A function you hand daegun (a pen's, a paint visitor's, a word segmenter, a record search's
 * key_at) runs on the thread that called daegun, before that call returns, and nothing of it is kept
 * after. It must return normally. A C++ exception or a longjmp out of one is undefined behavior, as is
 * ending the thread inside one (pthread_exit, a cancellation acted on there, ExitThread). Each call is
 * declared to throw nothing, so a compiler drops your catch around one and an exception ends the
 * process, but MSVC unoptimized (/Od) keeps the catch and lets the exception pass the Rust frames between,
 * which hold memory and locks; a debug build of daegun aborts at its first Rust frame. Declare C++
 * callbacks noexcept, so a throw ends the process inside the callback in every build. Under MSVC each
 * callback type is C's calling convention, DAEGUN_CALLBACK; building with /Gv or /Gz, mark your callbacks
 * DAEGUN_CALLBACK, as one left to that default is refused where it's handed over rather than called wrong.
 *
 * PROCESSES. No call is safe in a signal handler: calls allocate and take locks. The child of a fork from a
 * process with other threads may call nothing here until it execs, as a lock one of them held stays held.
 * Unloading the library (dlclose, FreeLibrary) while a thread that called it runs may leave it loaded, or
 * leak what the thread kept, by platform.
 */

#ifndef DAEGUN_BASE_H
#define DAEGUN_BASE_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

/* How each call is declared: imported from daegun-2.dll where DAEGUN_DLL asks for it, which saves every call
 * a jump through a stub; to GCC, clang and MSVC's C++, throwing nothing, so C++ around a call needs no cleanup
 * for one, under MSVC's /EHs as under /EHsc, which assumes as much of every C call (MSVC's C takes no nothrow);
 * and a warning where a status that carries results is ignored, which a cast to void silences. GCC's own
 * attribute ignores that cast, so GCC warns from C23 and C++17 on; MSVC warns from C23 (/std:clatest) and C++17
 * on, and in C before C23 under /analyze. */
#if defined(__GNUC__) && defined(_WIN32) && defined(DAEGUN_DLL)
#define DAEGUN_API __attribute__((dllimport, nothrow))
#elif defined(__GNUC__)
#define DAEGUN_API __attribute__((nothrow))
#elif defined(_MSC_VER) && defined(__cplusplus) && defined(DAEGUN_DLL)
#define DAEGUN_API __declspec(dllimport) __declspec(nothrow)
#elif defined(_MSC_VER) && defined(__cplusplus)
#define DAEGUN_API __declspec(nothrow)
#elif defined(_WIN32) && defined(DAEGUN_DLL)
#define DAEGUN_API __declspec(dllimport)
#else
#define DAEGUN_API
#endif

/* C's calling convention, pinned under MSVC, whose /Gv and /Gz would make an unmarked function another, and on i386
 * under GCC and clang, whose -mrtd does: on each call, so such a program links, and as DAEGUN_CALLBACK on each
 * callback type. */
#if defined(_MSC_VER)
#define DAEGUN_CALL __cdecl
#elif defined(__i386__) && (defined(__GNUC__) || defined(__clang__))
#define DAEGUN_CALL __attribute__((cdecl))
#else
#define DAEGUN_CALL
#endif
#define DAEGUN_CALLBACK DAEGUN_CALL

#if (defined(__cplusplus) && __cplusplus >= 201703L) || (defined(_MSVC_LANG) && _MSVC_LANG >= 201703L)
#define DAEGUN_NODISCARD [[nodiscard]]
#elif defined(__STDC_VERSION__) && __STDC_VERSION__ > 201710L && defined(__has_c_attribute)
/* A C2x draft's version says nothing of whether the attribute is in: GCC 10's has none. */
#if __has_c_attribute(nodiscard)
#define DAEGUN_NODISCARD [[nodiscard]]
#endif
#endif
#ifndef DAEGUN_NODISCARD
#if defined(__clang__)
#define DAEGUN_NODISCARD __attribute__((warn_unused_result))
#elif defined(_MSC_VER)
#include <sal.h>
#define DAEGUN_NODISCARD _Check_return_
#else
#define DAEGUN_NODISCARD
#endif
#endif

/* What gives an allocator's pointer back, so GCC's analyzer and -Wmismatched-dealloc can follow it; to clang and
 * MSVC, that the pointer aliases nothing. GCC's plain malloc would hide an allocator's size from its overflow
 * checks. */
#if defined(__GNUC__) && !defined(__clang__) && __GNUC__ >= 11
#define DAEGUN_MALLOC(...) __attribute__((__VA_ARGS__))
#elif defined(__clang__)
#define DAEGUN_MALLOC(...) __attribute__((malloc))
#elif defined(_MSC_VER)
#define DAEGUN_MALLOC(...) __declspec(restrict)
#else
#define DAEGUN_MALLOC(...)
#endif

/* An allocator's size, from its argument `len` at index n counted from 1, for GCC's and clang's bounds checks, and
 * to MSVC's /analyze also that it may answer NULL. clang targeting MSVC defines no __GNUC__, so each attribute here
 * asks for clang by name too. */
#if defined(__GNUC__) || defined(__clang__)
#define DAEGUN_ALLOC_SIZE(n, len) __attribute__((alloc_size(n)))
#elif defined(_MSC_VER)
#include <sal.h>
#define DAEGUN_ALLOC_SIZE(n, len) _Ret_maybenull_ _Post_writable_byte_size_(len)
#else
#define DAEGUN_ALLOC_SIZE(n, len)
#endif

/* An answer from the arguments alone, reading no memory, so a compiler may reuse one call's. */
#if defined(__GNUC__) || defined(__clang__)
#define DAEGUN_CONST __attribute__((const))
#else
#define DAEGUN_CONST
#endif

/* A call made once a call has failed, so a compiler lays the path to it out of the way of the one that succeeds. */
#if defined(__GNUC__) || defined(__clang__)
#define DAEGUN_COLD __attribute__((cold))
#else
#define DAEGUN_COLD
#endif

/* An array argument at index p, counted from 1, that a call reads or writes, holding as many items as the argument
 * at index n says, for GCC's bounds checks: an array of yours shorter than the length you pass warns, as does NULL
 * with a length. A buffer you may leave NULL whatever its length carries neither. */
#if defined(__GNUC__) && !defined(__clang__) && __GNUC__ >= 10
#define DAEGUN_READS(p, n) __attribute__((access(read_only, p, n)))
#define DAEGUN_WRITES(p, n) __attribute__((access(write_only, p, n)))
#else
#define DAEGUN_READS(p, n)
#define DAEGUN_WRITES(p, n)
#endif

/* The same array parameter as DAEGUN_READS or DAEGUN_WRITES names, marked on the parameter itself and sized by
 * parameter n, for MSVC's /analyze and for clang's bounds safety (-fbounds-safety, and the attributes alone, which
 * Swift's C interop reads): an array of yours shorter than the length you pass warns, or under -fbounds-safety is
 * an error. A buffer you may leave NULL whatever its length is DAEGUN_OUT_OR_NULL, held to the length where it
 * isn't NULL. */
#if defined(__has_feature)
#if __has_feature(bounds_attributes) || __has_feature(bounds_safety_attributes)
#define DAEGUN_IN(n) __attribute__((__counted_by__(n)))
#define DAEGUN_OUT(n) __attribute__((__counted_by__(n)))
#define DAEGUN_OUT_OR_NULL(n) __attribute__((__counted_by_or_null__(n)))
#endif
#endif
#if !defined(DAEGUN_IN) && defined(_MSC_VER)
#include <sal.h>
#define DAEGUN_IN(n) _In_reads_opt_(n)
#define DAEGUN_OUT(n) _Out_writes_opt_(n)
#define DAEGUN_OUT_OR_NULL(n) _Out_writes_opt_(n)
#endif
#ifndef DAEGUN_IN
#define DAEGUN_IN(n)
#define DAEGUN_OUT(n)
#define DAEGUN_OUT_OR_NULL(n)
#endif

/* A struct's pointer to as many items as its field n holds, for GCC's bounds checks in C and clang's bounds safety:
 * an index of yours past the count warns, or traps under -fsanitize=bounds or -fbounds-safety. DAEGUN_LENT is the
 * same count where GCC reads none: a view a call returns, a visitor's stops, and a string, whose NUL lies past it. */
#if defined(__has_feature)
#if __has_feature(bounds_attributes) || __has_feature(bounds_safety_attributes)
#define DAEGUN_COUNTED_BY(n) __attribute__((__counted_by_or_null__(n)))
#define DAEGUN_LENT(n) __attribute__((__counted_by_or_null__(n)))
#endif
#endif
#if !defined(DAEGUN_COUNTED_BY) && defined(__GNUC__) && !defined(__clang__) && __GNUC__ >= 16 && !defined(__cplusplus)
#define DAEGUN_COUNTED_BY(n) __attribute__((counted_by(n)))
#endif
#ifndef DAEGUN_COUNTED_BY
#define DAEGUN_COUNTED_BY(n)
#endif
#ifndef DAEGUN_LENT
#define DAEGUN_LENT(n)
#endif

/* Which pointers may be NULL, as rule 2 has it, for clang's checks. Between the BEGIN and END each header wraps
 * itself in, a pointer not marked DAEGUN_NULLABLE is required; elsewhere they are empty. Every struct's pointers are
 * nullable, as a struct is yours, zeroed, until a call fills it; one a call filled points somewhere, even with a
 * count of 0. BEGIN and END also pack at 8, the alignment every struct here is laid out at, so a /Zp or a #pragma
 * pack in force at the include can't move a field the library reads. */
#if defined(__clang__) && defined(__has_feature)
#if __has_feature(nullability) && __has_feature(assume_nonnull)
#define DAEGUN_NULLABLE _Nullable
#define DAEGUN_NONNULL _Nonnull
#define DAEGUN_NONNULL_BEGIN _Pragma("clang diagnostic push") \
    _Pragma("clang diagnostic ignored \"-Wnullability-extension\"") _Pragma("clang assume_nonnull begin")
#define DAEGUN_NONNULL_END _Pragma("clang assume_nonnull end") _Pragma("clang diagnostic pop")
#endif
#endif
#ifndef DAEGUN_NULLABLE
#define DAEGUN_NULLABLE
#define DAEGUN_NONNULL
#define DAEGUN_NONNULL_BEGIN
#define DAEGUN_NONNULL_END
#endif
#ifdef _MSC_VER
#define DAEGUN_HEADER_BEGIN DAEGUN_NONNULL_BEGIN __pragma(pack(push, 8))
#define DAEGUN_HEADER_END __pragma(pack(pop)) DAEGUN_NONNULL_END
#else
#define DAEGUN_HEADER_BEGIN DAEGUN_NONNULL_BEGIN _Pragma("pack(push, 8)")
#define DAEGUN_HEADER_END _Pragma("pack(pop)") DAEGUN_NONNULL_END
#endif

#ifdef __cplusplus
extern "C" {
#endif

DAEGUN_HEADER_BEGIN

/* status */

/** What every fallible call returns: OK, a pointer of yours that was NULL, an answer that there is none, or
 * one status for each kind of failure, as Rust's ErrorKind names them.
 *
 * The values are frozen. A caller compares against these constants and a compiled consumer carries
 * the numbers, not the names, so renumbering would break every binary silently. New codes append, so
 * treat one you don't know as a failure. */
typedef int32_t daegun_status;

#define DAEGUN_OK                 0   /* the call did what it says */
#define DAEGUN_NULL              -1   /* a required pointer was NULL, always the caller's bug */
#define DAEGUN_MALFORMED         -2   /* font data the call needs is malformed: truncated, out of range or
                                         inconsistent */
#define DAEGUN_INVALID_ARGUMENT  -3   /* an argument is out of range, not a number, or not UTF-8 */
#define DAEGUN_ABSENT            -4   /* the font has no such glyph, table or value: an answer, not a
                                         failure. Separate from DAEGUN_OK because C has no Option and
                                         zero is a real glyph id (.notdef). */
/* -5 meant unsupported in 1.x, which only the GPU backends returned. It is never reused. */
#define DAEGUN_UNSUPPORTED       -6   /* a format, version or options struct daegun doesn't read */
#define DAEGUN_MISSING           -7   /* a table or record the call needs isn't in the font */
#define DAEGUN_LIMIT_EXCEEDED    -8   /* the font asks for more work or memory than daegun allows one font */
#define DAEGUN_NO_OUTLINE        -9   /* the glyph has no outline to draw */
#define DAEGUN_OVERFLOW         -10   /* a value outgrows what its type can hold once scaled */

/* borrowed views */

/** A run of bytes daegun owns, lent to you. Valid until the handle it came from is freed or changed. */
typedef struct {
    const uint8_t *DAEGUN_NULLABLE data DAEGUN_COUNTED_BY(len);
    size_t         len;
} daegun_bytes;

/** A UTF-8 string daegun owns, lent to you.
 *
 * NUL-terminated and length-carrying both. An interior NUL, which a font's name table may contain,
 * is replaced with U+FFFD, so strlen cannot mislead you and `len` still describes the whole string. */
typedef struct {
    const char *DAEGUN_NULLABLE data DAEGUN_LENT(len);
    size_t      len;
} daegun_str;

/* library */

/** The C ABI this library was built with, as (major << 16) | (minor << 8) | patch. It is the ABI's own
 * version, not the crate's. Its minor moves when calls or constants are added, or an options struct
 * grows at its end; its major whenever an existing call, constant or any other struct changes. Every
 * struct but an options struct is frozen for a major: a call fills it, or you pass arrays of it, so
 * its size is compiled into your program. New data for one comes from a new call. So is each set of
 * codes a call answers, its statuses and every DAEGUN_* a field or a visitor reports: a minor gives an
 * existing call no new one to handle, but where the call says a newer library may answer more
 * (DAEGUN_SHAPER_*, DAEGUN_VO_*). A set you pass in may grow in a minor.
 *
 * Refuse a library whose major differs from DAEGUN_ABI_VERSION's, or whose minor is lower: a lower
 * minor lacks calls you may use. DAEGUN_ABI_COMPATIBLE(daegun_abi_version()) says whether it may be used. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_CONST uint32_t DAEGUN_CALL
daegun_abi_version(void);

#define DAEGUN_ABI_VERSION 0x020000u

/* A version's parts, and whether a library answering `v` serves a program built against this header: the same
 * major and a minor no lower. Each works in #if too. */
#define DAEGUN_ABI_MAJOR(v) ((v) >> 16)
#define DAEGUN_ABI_MINOR(v) (((v) >> 8) & 0xFFu)
#define DAEGUN_ABI_PATCH(v) ((v) & 0xFFu)
#define DAEGUN_ABI_COMPATIBLE(v) \
    (DAEGUN_ABI_MAJOR(v) == DAEGUN_ABI_MAJOR(DAEGUN_ABI_VERSION) && ((v) >> 8) >= (DAEGUN_ABI_VERSION >> 8))

/** The library's release, such as "2.0.0", for a log or a bug report; whether it may be used is the ABI's version's
 * to say. Static, so valid for the program's life. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_CONST daegun_str DAEGUN_CALL
daegun_version(void);

/** The reason the last failure ON THIS THREAD gave, as UTF-8, valid until the next that gives one or the
 * thread ends; empty until one has. Every status but DAEGUN_OK, DAEGUN_NULL and DAEGUN_ABSENT gives one,
 * so read it right after any of them, on the same thread. DAEGUN_NULL, a pointer of your own, and
 * DAEGUN_ABSENT, an answer, give none and leave the slot as it was. Per-thread, like errno, since handles
 * are shared and a global slot would hand one thread another's failure. A failure inside your own
 * thread-exit destructor may give none, as the thread's slot can be gone by then. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_COLD daegun_str DAEGUN_CALL
daegun_last_error(void);

/** Lets go of what the calling thread keeps between calls: the locations it last read at, its hinters, its recent
 * runs, the scratch it reuses and the freed handles it keeps for its next. What it last read and hinted with holds
 * those fonts' bytes, so a pool's worker calls this as it goes idle or once a font is freed. Other threads keep
 * theirs, this thread's next call builds again what it needs, and its last error stays. */
DAEGUN_API DAEGUN_COLD void DAEGUN_CALL
daegun_thread_release(void);

/** A status's name, such as "DAEGUN_INVALID_ARGUMENT", for a log; "unknown" for a code this library never answers.
 * Static, so valid for the program's life. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_CONST DAEGUN_COLD daegun_str DAEGUN_CALL
daegun_status_name(daegun_status status);

/* tags */

/** A four-byte OpenType tag, packed big-endian as the font stores it: DAEGUN_TAG('w','g','h','t') is
 * 0x77676874. Scripts, languages, features, baselines, axes and tables alike; a short one is padded with
 * spaces, as fonts store 'lao ' and 'CFF '. Where a call takes a tag it may go without, 0 is none: no
 * font's tag is four NULs. */
typedef uint32_t daegun_tag;

#ifdef __cplusplus
#define DAEGUN_TAG(a, b, c, d) \
    (static_cast<daegun_tag>(static_cast<uint8_t>(a)) << 24 | static_cast<daegun_tag>(static_cast<uint8_t>(b)) << 16 \
     | static_cast<daegun_tag>(static_cast<uint8_t>(c)) << 8 | static_cast<daegun_tag>(static_cast<uint8_t>(d)))
#else
#define DAEGUN_TAG(a, b, c, d) \
    ((daegun_tag)(((uint32_t)(uint8_t)(a) << 24) | ((uint32_t)(uint8_t)(b) << 16) | \
                  ((uint32_t)(uint8_t)(c) << 8) | (uint32_t)(uint8_t)(d)))
#endif

/** A tag from its text, one to four printable ASCII characters and any spaces after them, as Rust's Tag reads one:
 * for a language that can't call the macro above, as Swift can't. DAEGUN_INVALID_ARGUMENT for anything else. */
DAEGUN_NODISCARD DAEGUN_API DAEGUN_READS(1, 2) daegun_status DAEGUN_CALL
daegun_tag_from_string(DAEGUN_IN(text_len) const char *DAEGUN_NULLABLE text, size_t text_len, daegun_tag *out);

/* A tag's four bytes, first to last, as four char arguments: printf("%c%c%c%c", DAEGUN_UNTAG(tag)). */
#ifdef __cplusplus
#define DAEGUN_UNTAG(t) \
    static_cast<char>(((t) >> 24) & 0xFFu), static_cast<char>(((t) >> 16) & 0xFFu), \
    static_cast<char>(((t) >> 8) & 0xFFu), static_cast<char>((t) & 0xFFu)
#else
#define DAEGUN_UNTAG(t) \
    (char)(((t) >> 24) & 0xFFu), (char)(((t) >> 16) & 0xFFu), (char)(((t) >> 8) & 0xFFu), (char)((t) & 0xFFu)
#endif

/* bounds */

/** A box in font units, y up. A glyph that draws nothing has none, answered DAEGUN_ABSENT, which differs
 * from a box of zero size. */
typedef struct {
    double x_min;
    double y_min;
    double x_max;
    double y_max;
} daegun_bounds;

/* orientation */

/* Which way a metric, an advance or a run of text goes, wherever a call takes one. */
#define DAEGUN_HORIZONTAL 0
#define DAEGUN_VERTICAL   1

/* owned results */

/** Rule 3: daegun allocates and daegun frees. Strings and a font file's bytes come back as one of these,
 * returned with the matching _free, where a list of fixed-size items fills your buffer (BUFFERS). A
 * borrowed view read out of one is valid until that free. */
typedef struct daegun_blob     daegun_blob;          /* a run of bytes: a font file */
typedef struct daegun_str_list daegun_str_list;
typedef struct daegun_text     daegun_text;          /* one owned string */

/** A glyph paired with what a table maps it to.
 *
 * One list of pairs rather than two parallel lists, which is the opposite of what the optional
 * results do. The difference: there the halves answer different questions and a caller reads one
 * without the other, here neither half means anything alone. */
typedef struct {
    uint32_t glyph;
    uint16_t value;
} daegun_glyph_value;

/** The bytes, borrowed. NULL if the blob or the count pointer is NULL. */
DAEGUN_API const uint8_t *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_blob_data(const daegun_blob *blob, size_t *out_count);
DAEGUN_API void DAEGUN_CALL
daegun_blob_free(daegun_blob *DAEGUN_NULLABLE blob);

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_str_list_count(const daegun_str_list *list, size_t *out);
/** DAEGUN_INVALID_ARGUMENT past the end, rather than an empty string, since a font may hold an empty name
 * and you must be able to tell the two apart. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_str_list_at(const daegun_str_list *list, size_t index, daegun_str *out);
DAEGUN_API void DAEGUN_CALL
daegun_str_list_free(daegun_str_list *DAEGUN_NULLABLE list);

DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_text_str(const daegun_text *text, daegun_str *out);
DAEGUN_API void DAEGUN_CALL
daegun_text_free(daegun_text *DAEGUN_NULLABLE text);

/* fonts and locations */

/** Opaque. You never see the layout, so it is free to change without breaking you. */
typedef struct daegun_font     daegun_font;
typedef struct daegun_location daegun_location;

/** One axis value of a location, as you state it: a tag such as DAEGUN_TAG('w','g','h','t') and a value
 * in the axis's own units. */
typedef struct {
    daegun_tag tag;
    double     value;
} daegun_coord;

/* Layout checks, repeated on the Rust side, so a header and library that disagree fail the build rather
 * than a caller. cl's default C mode, a Visual Studio project's, has no _Static_assert, so skips them. */
#ifdef __cplusplus
#define DAEGUN_LAYOUT_ASSERT(cond, why) static_assert(cond, why)
#elif defined(_MSC_VER) && !defined(__STDC_VERSION__)
#define DAEGUN_LAYOUT_ASSERT(cond, why)
#else
#define DAEGUN_LAYOUT_ASSERT(cond, why) _Static_assert(cond, why)
#endif

DAEGUN_LAYOUT_ASSERT(sizeof(daegun_status) == 4, "status must be an int32");
DAEGUN_LAYOUT_ASSERT(sizeof(bool) == 1, "a bool must be one byte, as Rust's is");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_tag) == 4, "a tag is a uint32");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_bytes) == 2 * sizeof(size_t), "bytes view must be two words");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_str) == 2 * sizeof(size_t), "string view must be two words");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_bounds) == 32, "bounds are four doubles");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_glyph_value) == 8, "a glyph-value pair is a uint32 and a padded uint16");
/* Where a double follows a narrower field, its offset holds only where doubles align to 8, as every 64-bit
 * target does; 32-bit x86 aligns them to 4. */
#if UINTPTR_MAX == 0xFFFFFFFFFFFFFFFFu
#define DAEGUN_LAYOUT_64 1
#endif
#ifdef DAEGUN_LAYOUT_64
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_coord) == 16, "a coord is a tag padded to a double, and a double");
DAEGUN_LAYOUT_ASSERT(offsetof(daegun_coord, value) == 8, "coord layout");
#endif

DAEGUN_HEADER_END

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_BASE_H */
