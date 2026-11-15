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

/// The system's own name for the place deleted files go: "the Trash", and
/// "the Recycle Bin" on Windows. Every phrase that says where files end up
/// takes the name from here, so one system never reads both. The Trash
/// mark, its key and its button are PhotoSort's own and keep their name.
pub const TRASH: &str = if cfg!(windows) {
    "Recycle Bin"
} else {
    "Trash"
};
/// The same after "to" in a short label, where macOS and Linux leave "the"
/// out ("3 photos going to Trash"). Windows never does.
pub const TO_TRASH: &str = if cfg!(windows) {
    "to the Recycle Bin"
} else {
    "to Trash"
};
/// The same where it is told apart from PhotoSort's own Trash mark.
pub const SYSTEM_TRASH: &str = if cfg!(windows) {
    "Recycle Bin"
} else {
    "System Trash"
};

/// A path as it is shown to the user. A folder is opened by its canonical
/// path, which Windows spells with a prefix that says "take this path as it
/// is": `\\?\C:\Photos`, and `\\?\UNC\server\share` on a network share.
/// File work keeps that spelling, which is what lets it reach long paths;
/// nobody should have to read it. Everything that prints a path for a
/// person goes through here, or through [`plain_paths`] for a finished
/// sentence.
pub fn shown(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    match plain_paths(&text) {
        Cow::Borrowed(_) => text,
        Cow::Owned(plain) => plain,
    }
}

/// `text` with every path in it spelled the way File Explorer spells it:
/// `\\?\C:\Photos` as `C:\Photos`, `\\?\UNC\server\share` as
/// `\\server\share`. Other `\\?\` paths (a volume by its identifier) have no
/// shorter spelling and stay. This is text work only, the same on every
/// system; a path on macOS or Linux never starts this way.
pub fn plain_paths(text: &str) -> Cow<'_, str> {
    const PREFIX: &str = r"\\?\";
    const SHARE: &str = r"UNC\";
    if !text.contains(PREFIX) {
        return Cow::Borrowed(text);
    }
    let mut plain = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(PREFIX) {
        plain.push_str(&rest[..at]);
        let after = &rest[at + PREFIX.len()..];
        let drive = {
            let mut letters = after.chars();
            letters.next().is_some_and(|c| c.is_ascii_alphabetic()) && letters.next() == Some(':')
        };
        if let Some(share) = after.strip_prefix(SHARE) {
            plain.push_str(r"\\");
            rest = share;
        } else {
            if !drive {
                plain.push_str(PREFIX);
            }
            rest = after;
        }
    }
    plain.push_str(rest);
    Cow::Owned(plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_paths_drop_the_windows_prefix_and_keep_everything_else() {
        for (path, shown_as) in [
            (r"\\?\C:\Photos\2024\a.jpg", r"C:\Photos\2024\a.jpg"),
            (r"\\?\f:\", r"f:\"),
            (r"\\?\UNC\nas\photos\Trip", r"\\nas\photos\Trip"),
            // No shorter spelling exists for these: they stay as they are.
            (
                r"\\?\Volume{4c1b02c1-d990-11dc-99ae-806e6f6e6963}\a.jpg",
                r"\\?\Volume{4c1b02c1-d990-11dc-99ae-806e6f6e6963}\a.jpg",
            ),
            (r"C:\Photos\a.jpg", r"C:\Photos\a.jpg"),
            (r"\\nas\photos", r"\\nas\photos"),
            ("/Volumes/Photos/What?/a.jpg", "/Volumes/Photos/What?/a.jpg"),
        ] {
            assert_eq!(shown(std::path::Path::new(path)), shown_as);
        }
        // A finished sentence is cleaned the same way, every path in it.
        assert_eq!(
            plain_paths(
                r"Rename file \\?\C:\a\x.jpg → \\?\UNC\nas\p\y.jpg: Destination already exists: \\?\C:\a\y.jpg"
            ),
            r"Rename file C:\a\x.jpg → \\nas\p\y.jpg: Destination already exists: C:\a\y.jpg"
        );
        assert!(matches!(plain_paths("/library/a.jpg"), Cow::Borrowed(_)));
    }
}
