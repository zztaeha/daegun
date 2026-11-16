// Every slot a thread keeps between calls, read from the sources so a new one can't be missed: each file's
// thread-locals and pooled C handles are emptied by its own `release_thread`, which the public releases reach.
use std::path::{Path, PathBuf};

const SRC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src");

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for path in std::fs::read_dir(dir).expect("a source folder").flatten().map(|e| e.path()) {
        if path.is_dir() && !path.ends_with("tests") {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

// A top-level function's text, from its signature to its closing brace.
fn body<'a>(text: &'a str, signature: &str) -> Option<&'a str> {
    let rest = &text[text.find(signature)?..];
    Some(&rest[..rest.find("\n}\n")?])
}

fn ident(text: &str) -> &str {
    text.trim_start().split(|c: char| !c.is_alphanumeric() && c != '_').next().unwrap_or_default()
}

// The thread-locals a file declares, but those a pooled handle's `spare` holds, named by its kind instead, and the C
// last error, which outlives a release by contract.
fn thread_locals(text: &str) -> Vec<&str> {
    let (mut names, mut block, mut spare_fn) = (Vec::new(), None, false);
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("fn ") || line.contains(" fn ") {
            spare_fn = line.contains("fn spare()");
        }
        if line.contains("thread_local!") {
            block = Some(spare_fn && raw.starts_with(' '));
        } else if line == "}" {
            block = None;
        } else if block == Some(false)
            && let Some(rest) = line.strip_prefix("static ")
        {
            names.push(ident(rest));
        }
    }
    names.retain(|&n| n != "LAST_ERROR");
    names
}

// The handle kinds a C file pools: each `Reused` it implements, by hand or through `reused_box!` and `kept_bytes!`.
fn pooled(text: &str) -> Vec<&str> {
    let by_hand = text.split("Reused for ").skip(1).map(ident);
    let by_macro = ["reused_box!(", "kept_bytes!("].into_iter().flat_map(|m| text.split(m).skip(1));
    by_hand.chain(by_macro.filter_map(|args| args.split(',').nth(1).map(ident))).filter(|k| !k.is_empty()).collect()
}

#[test]
fn every_slot_a_thread_keeps_is_released() {
    let mut files = Vec::new();
    sources(Path::new(SRC), &mut files);
    files.sort();
    let read = |rel: &str| std::fs::read_to_string(Path::new(SRC).join(rel)).expect(rel);
    let core = read("daecore/src/daekit/base/sync.rs");
    let (rust, c) = (read("api/rust/mod.rs"), read("api/c/translation/mod.rs"));
    let core_release = body(&core, "fn release_thread()").expect("the core's release");
    let rust_release = body(&rust, "pub fn release_thread()").expect("the public release");
    let c_release = body(&c, "fn daegun_thread_release()").expect("C's release");
    assert!(rust_release.contains("crate::sync::release_thread();"), "the public release skips the core's");
    assert!(c_release.contains("crate::release_thread();"), "C's release skips Rust's");
    // Where each file's own release is called from, by its module's path below the folder.
    let callers = [("daecore/src/", core_release), ("api/rust/", rust_release), ("api/c/translation/", c_release)];
    let mut released = 0;
    for path in &files {
        let rel = path.strip_prefix(SRC).expect("under src").to_str().expect("a UTF-8 path").replace('\\', "/");
        let text = std::fs::read_to_string(path).expect(&rel);
        let (slots, kinds) = (thread_locals(&text), pooled(&text));
        if slots.is_empty() && kinds.is_empty() {
            continue;
        }
        let own = body(&text, "fn release_thread()");
        let own = own.unwrap_or_else(|| panic!("{rel} keeps {slots:?} and pooled {kinds:?}, unreleased"));
        for slot in &slots {
            assert!(own.contains(&format!("{slot}.try_with(")), "{rel}: {slot} isn't released");
        }
        for kind in &kinds {
            assert!(own.contains(&format!("drop_spare::<{kind}>()")), "{rel}: pooled {kind}s aren't released");
        }
        let (below, caller) = callers.iter().find(|(dir, _)| rel.starts_with(dir)).expect("a folder with a release");
        let module = rel[below.len()..].trim_end_matches(".rs").trim_end_matches("/mod").replace('/', "::");
        assert!(caller.contains(&format!("{module}::release_thread();")), "{rel}'s release isn't called");
        released += slots.len() + kinds.len();
    }
    assert_eq!(released, 43, "the slots and pooled kinds released, each new one counted here as it is");
}
