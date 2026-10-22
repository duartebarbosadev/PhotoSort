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
    /// Clean a new folder or file name. Idempotent, so already cleaned names
    /// pass through unchanged. Never use it on a name read from disk.
    pub fn sanitize(self, value: &str) -> String {
        let clean: String = value
            .chars()
            .map(|c| if self.replaced(c) { '_' } else { c })
            .collect();
        let clean: String = self.trim(&clean).chars().take(100).collect();
        let clean = self.trim(&clean);
        if clean.is_empty() || clean == "." || clean == ".." {
            "Unsorted".into()
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
        ] {
            assert_eq!(rules.sanitize(input), expected, "{rules:?} {input:?}");
        }
        let long = format!("{}{}", "a".repeat(99), " b");
        for rules in [Windows, Unix] {
            let once = rules.sanitize(&long);
            assert_eq!(once, "a".repeat(99));
            assert_eq!(rules.sanitize(&once), once, "{rules:?} is idempotent");
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
