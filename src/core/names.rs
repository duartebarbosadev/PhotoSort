//! Folder and file name rules for Organize.
//!
//! Names read from the library are used byte-for-byte: they already exist, so
//! they are legal on this filesystem, and "cleaning" one would rename a folder
//! or file the user never touched. Only names PhotoSort creates (generated
//! groupings such as cameras and places, and names the user types) are cleaned.
use std::path::{Component, Path};

/// Which characters a new name may contain, by platform. A parameter rather
/// than `cfg!` so every rule set is testable on any host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameRules {
    Windows,
    /// macOS and Linux.
    Unix,
}
impl NameRules {
    pub const CURRENT: Self = if cfg!(windows) {
        Self::Windows
    } else {
        Self::Unix
    };
    /// Characters the filesystem refuses inside one name.
    fn illegal(self, c: char) -> bool {
        match self {
            Self::Windows => c.is_control() || "/\\:*?\"<>|".contains(c),
            Self::Unix => c == '/' || c == '\0',
        }
    }
    /// Characters replaced in names PhotoSort creates. Existing names keep
    /// them all; this only decides what a new name may contain.
    /// - Control characters are legal on macOS and Linux but invisible.
    /// - A `\` is legal on macOS and Linux but is a folder separator on
    ///   Windows, so a new name with one would read as two folders there.
    /// - A `:` is legal on APFS and Linux, but Finder shows it as `/`, classic
    ///   macOS APIs treat it as a separator, and exFAT, SMB and Windows refuse
    ///   it, so new names never use it and generated names match everywhere.
    fn replaced(self, c: char) -> bool {
        c.is_control() || c == '\\' || c == ':' || self.illegal(c)
    }
    fn trim(self, value: &str) -> &str {
        let value = value.trim();
        // Windows drops trailing dots and spaces, so a name ending in one
        // would not round-trip. Elsewhere a trailing dot is an ordinary name.
        if self == Self::Windows {
            value.trim_end_matches(|c: char| c == '.' || c.is_whitespace())
        } else {
            value
        }
    }
    /// Windows device names (`CON`, `nul.txt`, `com1`, `LPT²`...) cannot be
    /// files or folders, whatever the case or extension.
    fn reserved(self, name: &str) -> bool {
        if self != Self::Windows {
            return false;
        }
        let base = name.split('.').next().unwrap_or(name).trim_end();
        let base = base.to_ascii_uppercase();
        let mut chars = base.chars();
        let port = chars.by_ref().take(3).collect::<String>();
        matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (matches!(port.as_str(), "COM" | "LPT")
                && chars
                    .next()
                    .is_some_and(|c| c.is_ascii_digit() || "¹²³".contains(c))
                && chars.next().is_none())
    }
    /// Which rule a typed name breaks, in a few words for the status bar:
    /// "Windows reserves the name “CON”". `what` is the kind of name, "folder
    /// name". `None` for a name that is taken as typed, which is exactly a
    /// name `sanitize` leaves alone.
    pub fn refusal(self, name: &str, what: &str) -> Option<String> {
        if name.trim().is_empty() {
            return Some(format!("A {what} cannot be empty"));
        }
        if let Some(c) = name.chars().find(|c| self.replaced(*c)) {
            return Some(if c.is_control() {
                format!("A {what} cannot contain tabs or line breaks")
            } else {
                format!("A {what} cannot contain “{c}”")
            });
        }
        if name.trim() != name {
            return Some(format!("A {what} cannot start or end with a space"));
        }
        if name == "." || name == ".." {
            return Some(format!("A {what} cannot be “{name}”"));
        }
        if self.trim(name) != name {
            return Some(format!("On Windows a {what} cannot end with a dot"));
        }
        if name.chars().count() > 100 {
            return Some(format!("A {what} can have up to 100 characters"));
        }
        if self.reserved(name) {
            let base = name.split('.').next().unwrap_or(name).trim_end();
            return Some(format!("Windows reserves the name “{base}”"));
        }
        // Whatever else `sanitize` would change: a name too long in bytes.
        (self.sanitize(name) != name).then(|| format!("This {what} is too long"))
    }
    /// Clean a new folder or file name. Idempotent, so already cleaned names
    /// pass through unchanged. Never use it on a name read from disk.
    pub fn sanitize(self, value: &str) -> String {
        /// Most filesystems allow 255 bytes per name; keep room for a
        /// collision suffix and an extension in any script.
        const MAX_BYTES: usize = 200;
        let clean: String = value
            .chars()
            .map(|c| if self.replaced(c) { '_' } else { c })
            .collect();
        let mut bytes = 0;
        let clean: String = self
            .trim(&clean)
            .chars()
            .take(100)
            .take_while(|c| {
                bytes += c.len_utf8();
                bytes <= MAX_BYTES
            })
            .collect();
        let clean = self.trim(&clean);
        if clean.is_empty() || clean == "." || clean == ".." {
            "Unsorted".into()
        } else if self.reserved(clean) {
            // Before the extension: `nul_.txt`, since `nul.txt_` is still NUL.
            let (base, extension) = clean.split_at(clean.find('.').unwrap_or(clean.len()));
            format!("{base}_{extension}")
        } else {
            clean.into()
        }
    }
}

/// Whether one label component names exactly one folder, as every component
/// read from the library does. Anything else (`.`, `..`, a Windows drive or a
/// second separator) cannot be an existing name and is cleaned as a new one.
pub fn is_single_component(value: &str) -> bool {
    let mut parts = Path::new(value).components();
    !value.contains('\0')
        && matches!(parts.next(), Some(Component::Normal(_)))
        && parts.next().is_none()
}

/// Folder label for a path relative to the library root, components joined
/// by `/`. A `\` is part of a name on macOS and Linux, so it is never treated
/// as a separator there. `None` when a component is not valid Unicode: a lossy
/// label would name a different folder, so such files stay where they are.
pub fn relative_label(path: &Path) -> Option<String> {
    let mut label = String::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if !label.is_empty() {
                    label.push('/');
                }
                label.push_str(part.to_str()?);
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_names_follow_each_platform() {
        use NameRules::*;
        for (rules, input, expected) in [
            (Windows, "23:Dec", "23_Dec"),
            (Unix, "23:Dec", "23_Dec"),
            (Windows, r#"a?b*c"d<e>f|g\h"#, "a_b_c_d_e_f_g_h"),
            (Unix, r#"a?b*c"d<e>f|g\h"#, r#"a?b*c"d<e>f|g_h"#),
            (Windows, "a/b", "a_b"),
            (Unix, "a/b", "a_b"),
            (Unix, "nul\0tab\t", "nul_tab_"),
            (Windows, "Mr. ", "Mr"),
            (Unix, " Mr. ", "Mr."),
            (Windows, "..", "Unsorted"),
            (Unix, "..", "Unsorted"),
            (Unix, ".", "Unsorted"),
            (Unix, "  ", "Unsorted"),
            (Unix, ".hidden", ".hidden"),
            (Windows, "con", "con_"),
            (Windows, "Nul.txt", "Nul_.txt"),
            (Windows, "COM1", "COM1_"),
            (Windows, "lpt²", "lpt²_"),
            (Windows, "console", "console"),
            (Windows, "COM10", "COM10"),
            (Unix, "con", "con"),
        ] {
            assert_eq!(rules.sanitize(input), expected, "{rules:?} {input:?}");
        }
        let long = format!("{}{}", "a".repeat(99), " b");
        for rules in [Windows, Unix] {
            let once = rules.sanitize(&long);
            assert_eq!(once, "a".repeat(99));
            assert_eq!(rules.sanitize(&once), once, "{rules:?} is idempotent");
        }
        // 100 characters of a three-byte script would pass 255 bytes.
        let wide = Unix.sanitize(&"写".repeat(100));
        assert_eq!(wide.len(), 198);
        for clean in ["aux_", "Nul_.txt"] {
            assert_eq!(Windows.sanitize(clean), clean, "idempotent");
        }
        // A typed name that would be changed is refused instead, with the
        // one rule it breaks, short enough for the status bar.
        for (rules, typed, why) in [
            (Windows, "CON", "Windows reserves the name “CON”"),
            (Windows, "nul.txt", "Windows reserves the name “nul”"),
            (
                Windows,
                "Trip.",
                "On Windows a folder name cannot end with a dot",
            ),
            (Windows, "What?", "A folder name cannot contain “?”"),
            (Windows, "a*b", "A folder name cannot contain “*”"),
            (Unix, "Trip/2024", "A folder name cannot contain “/”"),
            (Unix, "23:Dec", "A folder name cannot contain “:”"),
            (
                Unix,
                "a\tb",
                "A folder name cannot contain tabs or line breaks",
            ),
            (
                Unix,
                " Trip",
                "A folder name cannot start or end with a space",
            ),
            (Windows, "..", "A folder name cannot be “..”"),
            (Unix, "  ", "A folder name cannot be empty"),
            (Unix, &"写".repeat(70), "This folder name is too long"),
        ] {
            let said = rules.refusal(typed, "folder name");
            assert_eq!(said.as_deref(), Some(why), "{rules:?} {typed:?}");
            assert!(why.chars().count() <= 50, "{why}");
        }
        assert_eq!(
            Unix.refusal(&"a".repeat(101), "file name").as_deref(),
            Some("A file name can have up to 100 characters")
        );
        // Refused exactly when cleaning would change it.
        for typed in ["CON", "Trip.", "What?", "console", "Mr.", ".hidden", "a b"] {
            for rules in [Windows, Unix] {
                assert_eq!(
                    rules.refusal(typed, "folder name").is_none(),
                    rules.sanitize(typed) == typed,
                    "{rules:?} {typed:?}"
                );
            }
        }
    }

    #[test]
    fn existing_components_and_labels_keep_their_bytes() {
        for name in ["23:Dec", "?*\"<>|", "trailing.", "trailing ", " leading"] {
            assert!(is_single_component(name), "{name:?}");
        }
        for name in [".", "..", "", "nul\0"] {
            assert!(!is_single_component(name), "{name:?}");
        }
        assert_eq!(
            relative_label(Path::new("Archive/toEdit/23:Dec/DCIM")).as_deref(),
            Some("Archive/toEdit/23:Dec/DCIM")
        );
        assert_eq!(relative_label(Path::new("")).as_deref(), Some(""));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            assert!(is_single_component(r"a\b"));
            assert_eq!(
                relative_label(Path::new(r"x/a\b")).as_deref(),
                Some(r"x/a\b")
            );
            let bytes = std::ffi::OsStr::from_bytes(b"bad-\xff/x");
            assert_eq!(relative_label(Path::new(bytes)), None);
        }
        #[cfg(windows)]
        {
            assert!(!is_single_component(r"a\b"));
            assert!(!is_single_component("C:"));
            assert_eq!(relative_label(Path::new(r"x\y")).as_deref(), Some("x/y"));
        }
    }
}
