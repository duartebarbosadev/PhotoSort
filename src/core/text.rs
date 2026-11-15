//! Comparing names as people read them. An accented letter has two
//! spellings: composed, one character ("é", NFC), and decomposed, the letter
//! and a combining accent ("e" + U+0301, NFD). Files copied from older Mac
//! volumes keep decomposed names, typed text is composed, and APFS takes both
//! for the same name. Nothing here renames anything: these are comparison
//! keys only.
use icu_normalizer::{ComposingNormalizerBorrowed, DecomposingNormalizerBorrowed};
use std::borrow::Cow;

/// `text` in composed form. Borrows when it already is, which plain ASCII
/// always is: the common case costs one scan and no allocation.
pub fn nfc(text: &str) -> Cow<'_, str> {
    if text.is_ascii() {
        return Cow::Borrowed(text);
    }
    ComposingNormalizerBorrowed::new_nfc().normalize(text)
}

/// The key under which names that differ only in letter case, or only in
/// composed and decomposed accents, are one name: lower case, composed.
/// Lowercasing comes first: "J" + caron has no composed capital, "j" + caron
/// does.
pub fn folded(text: &str) -> String {
    let lower = text.to_lowercase();
    match nfc(&lower) {
        Cow::Borrowed(_) => lower,
        Cow::Owned(composed) => composed,
    }
}

/// The key under which a volume keeps one name, for a volume that ignores
/// letter `case`, the composed or decomposed `form` of accents, or both.
pub fn name_key(text: &str, case: bool, form: bool) -> Cow<'_, str> {
    match (case, form) {
        (false, false) => Cow::Borrowed(text),
        (false, true) => nfc(text),
        (true, false) => Cow::Owned(text.to_lowercase()),
        (true, true) => Cow::Owned(folded(text)),
    }
}

/// Whether such a volume takes `a` and `b` for one name. Plain ASCII, the
/// common case, is compared in place.
pub fn same_name(a: &str, b: &str, case: bool, form: bool) -> bool {
    if a == b {
        return true;
    }
    if a.is_ascii() && b.is_ascii() {
        return case && a.eq_ignore_ascii_case(b);
    }
    name_key(a, case, form) == name_key(b, case, form)
}

/// `name` in the other form: composed when it is not, else decomposed.
/// `None` when both forms are the same text, as for plain ASCII.
pub fn other_form(name: &str) -> Option<String> {
    if name.is_ascii() {
        return None;
    }
    let other = match nfc(name) {
        Cow::Owned(composed) => composed,
        Cow::Borrowed(_) => DecomposingNormalizerBorrowed::new_nfd()
            .normalize(name)
            .into_owned(),
    };
    (other != name).then_some(other)
}
