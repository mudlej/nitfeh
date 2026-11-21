//! Every text the window shows must have its Arabic in lang/ar: a string added to ui/*.slint without one would
//! appear in English in the middle of an Arabic window.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Reads a Slint or PO string literal starting just after its opening quote; returns it and what follows.
fn literal(source: &str) -> (String, &str) {
    let mut text = String::new();
    let mut chars = source.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return (text, &source[i + 1..]),
            '\\' => match chars.next().map(|(_, escaped)| escaped) {
                Some('n') => text.push('\n'),
                // Slint writes a code point as \u{2068}.
                Some('u') => {
                    let rest = &source[i + 3..];
                    let hex = &rest[..rest.find('}').expect("unterminated \\u{...}")];
                    text.push(char::from_u32(u32::from_str_radix(hex, 16).unwrap()).unwrap());
                    chars.nth(hex.len() + 1);
                }
                Some(other) => text.push(other),
                None => break,
            },
            c => text.push(c),
        }
    }
    panic!("unterminated string literal");
}

/// The first string of every `@tr(...)` in a .slint file: that is the key a translation is looked up by.
fn window_texts(slint: &str) -> Vec<String> {
    slint.split("@tr(\"").skip(1).map(|after| literal(after).0).collect()
}

/// msgid → whether it has a translation, from a .po file.
fn translated(po: &str) -> HashMap<String, bool> {
    let mut found = HashMap::new();
    let mut id: Option<String> = None;
    let mut reading_id = false;
    for line in po.lines() {
        if let Some(rest) = line.strip_prefix("msgid \"") {
            id = Some(literal(rest).0);
            reading_id = true;
        } else if let Some(rest) = line.strip_prefix('"') {
            if reading_id && let Some(id) = id.as_mut() {
                id.push_str(&literal(rest).0);
            }
        } else if line.starts_with("msgstr") {
            reading_id = false;
            if let Some(id) = &id {
                let text = literal(&line[line.find('"').unwrap() + 1..]).0;
                *found.entry(id.clone()).or_insert(false) |= !text.is_empty();
            }
        } else {
            reading_id = false;
        }
    }
    found
}

#[test]
fn every_window_text_has_its_arabic() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let arabic = translated(&fs::read_to_string(root.join("lang/ar/LC_MESSAGES/nitfeh.po")).unwrap());

    let mut texts = Vec::new();
    for file in ["ui/app.slint", "ui/widgets.slint"] {
        texts.extend(window_texts(&fs::read_to_string(root.join(file)).unwrap()));
    }
    assert!(texts.len() > 40, "found only {} texts: is the scan broken?", texts.len());

    let missing: Vec<&String> = texts.iter().filter(|text| arabic.get(*text) != Some(&true)).collect();
    assert!(missing.is_empty(), "not translated in lang/ar (run `just strings`): {missing:#?}");
}
