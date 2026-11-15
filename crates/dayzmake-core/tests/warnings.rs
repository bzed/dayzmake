//! Warnings are raised into the sink a call is given; nothing is shared between calls.

use std::path::PathBuf;

use dayzmake_core::preprocess::preprocess;
use dayzmake_core::warning::{Warning, WarningKind, WarningSink, Warnings};

fn raise(w: &mut Warnings, count: u32, name: Option<&'static str>) {
    for _ in 0..count {
        w.record(Warning::new(name, "m"));
    }
}

#[test]
fn display_matches_the_cli_line() {
    assert_eq!(
        Warning::new(Some("x"), "m")
            .in_file("f.cpp")
            .at_line(3)
            .to_string(),
        "In file f.cpp:3: warning: m [x]"
    );
    assert_eq!(Warning::new(None, "m").to_string(), "warning: m");
    assert_eq!(
        Warning::new(None, "m").at_line(2).to_string(),
        "In line 2: warning: m"
    );
    assert_eq!(
        Warning::new(None, "m").in_file("f").to_string(),
        "In file f: warning: m"
    );
}

#[test]
fn quota_keeps_ten_and_counts_the_rest() {
    let mut w = Warnings::new();
    raise(&mut w, 12, Some("n"));
    assert_eq!(w.warnings().len(), 10);
    assert_eq!(w.suppressed(), vec![("n".to_string(), 2)]);
    assert!(w.is_suppressed(Some("n")));
    assert!(!w.is_suppressed(Some("other")));
}

#[test]
fn muted_names_are_dropped_and_not_summarised() {
    let mut w = Warnings::new();
    w.mute("n");
    raise(&mut w, 12, Some("n"));
    assert_eq!(w.warnings(), []);
    assert_eq!(w.suppressed(), Vec::<(String, u32)>::new());
    assert!(w.is_suppressed(Some("n")));
}

#[test]
fn unlimited_keeps_everything() {
    let mut w = Warnings::new();
    w.unlimited();
    raise(&mut w, 12, Some("n"));
    assert_eq!(w.warnings().len(), 12);
    assert_eq!(w.suppressed(), Vec::<(String, u32)>::new());
}

#[test]
fn unnamed_warnings_have_no_quota() {
    let mut w = Warnings::new();
    raise(&mut w, 12, None);
    assert_eq!(w.warnings().len(), 12);
    assert!(!w.is_suppressed(None));
}

/// A macro call with the wrong number of arguments, on line 3 of the source.
fn arity_source(lines: usize) -> String {
    let mut s = String::from("#define QUOTE(x) #x\n");
    for _ in 0..lines {
        s.push_str("t = QUOTE(a,b);\n");
    }
    s
}

#[test]
fn preprocess_reports_into_the_given_sink() {
    let mut w = Warnings::new();
    preprocess(arity_source(1), None, &[], &mut w).unwrap();
    let kept = w.warnings();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].name, Some("macro-arg-count"));
    assert_eq!(kept[0].line, Some(2));
}

#[test]
fn warnings_stay_with_their_own_sink_across_threads() {
    let handles: Vec<_> = (0..8_usize)
        .map(|i| {
            std::thread::spawn(move || {
                let mut w = Warnings::new();
                let file = PathBuf::from(format!("thread{i}.cpp"));
                preprocess(arity_source(i + 1), Some(file), &[], &mut w).unwrap();
                (i, w.into_warnings())
            })
        })
        .collect();
    for handle in handles {
        let (i, kept) = handle.join().unwrap();
        assert_eq!(kept.len(), i + 1);
        assert!(
            kept.iter()
                .all(|w| w.file.as_deref() == Some(&format!("thread{i}.cpp"))),
            "{kept:?}"
        );
    }
}

#[test]
fn notes_are_kept_past_the_quota_and_despite_mutes() {
    let mut w = Warnings::new();
    w.mute("x");
    for i in 0..25 {
        w.record(Warning::note(format!("n{i}")));
    }
    raise(&mut w, 12, Some("y"));
    let notes = w
        .warnings()
        .iter()
        .filter(|w| w.kind == WarningKind::Note)
        .count();
    assert_eq!(notes, 25);
    // notes are not counted: only the 2 excess "y" warnings are suppressed
    assert_eq!(w.suppressed(), [("y".to_string(), 2)]);
    assert_eq!(Warning::note("plain").to_string(), "plain");
    assert!(!w.is_suppressed(None));
}

#[test]
fn macro_arity_warnings_do_not_leak_into_the_next_call() {
    let mut first = Warnings::new();
    preprocess(
        "#define F(a) a\nx = F(1,2);\n".to_string(),
        None,
        &[],
        &mut first,
    )
    .unwrap();
    assert!(
        first
            .warnings()
            .iter()
            .any(|w| w.name == Some("macro-arg-count")),
        "{:?}",
        first.warnings()
    );
    let mut second = Warnings::new();
    preprocess("y = 1;\n".to_string(), None, &[], &mut second).unwrap();
    assert!(second.warnings().is_empty(), "{:?}", second.warnings());
}

#[test]
fn warning_location_is_escaped() {
    let w = Warning::new(None, "m").in_file("a\u{9b}b.cpp").at_line(1);
    let text = w.to_string();
    assert!(text.contains("a\\u{9b}b.cpp"), "{text:?}");
    assert!(!text.contains('\u{9b}'), "{text:?}");
    assert!(!w.location().contains('\u{9b}'));
    assert_eq!(w.file.as_deref(), Some("a\u{9b}b.cpp"));
}
