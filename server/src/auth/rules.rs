//! Account field rules and their messages (docs/phase1/spec/auth.md §1.1, §1.5, §2.7;
//! docs/phase1/conventions.md §5.4). Shared by sign-up, login, preferences, password change and
//! the email flows.

use std::str::FromStr;
use std::sync::LazyLock;

use regex::Regex;

use crate::text::{char_len, py_strip};

/// The EN DASH (U+2013) is part of the contract with `apiErrors.ts`.
pub const USERNAME_RULE: &str =
    "Username must be 2–32 characters: lowercase letters, numbers, underscore";
pub const PASSWORD_RULE: &str = "Password must be at least 8 characters";
pub const NEW_PASSWORD_RULE: &str = "New password must be at least 8 characters";
pub const INVALID_EMAIL: &str = "Enter a valid email address";
pub const THEME_RULE: &str = "theme must be one of: paper, slate, forest, ink";
pub const COLOR_MODE_RULE: &str = "color_mode must be one of: light, dark, system";
pub const LOCALE_RULE: &str = "locale must be one of: en, de";

/// `users.theme` values.
pub const THEMES: [&str; 4] = ["paper", "slate", "forest", "ink"];
/// `users.color_mode` values.
pub const COLOR_MODES: [&str; 3] = ["light", "dark", "system"];
/// `users.locale` values.
pub const LOCALES: [&str; 2] = ["en", "de"];

/// Shortest password, in characters (code points, like Python's `len`).
pub const MIN_PASSWORD_CHARS: usize = 8;
/// Longest address `users.email` accepts.
const MAX_EMAIL_CHARS: usize = 254;

static USERNAME: LazyLock<Regex> = LazyLock::new(|| Regex::new("^[a-z0-9_]{2,32}$").unwrap());
/// A plain `local@domain` (RFC 5322 dot-atom on both sides), nothing else. The local part takes
/// the atext characters plus non-ASCII letters, never quotes, comments, angle brackets, commas
/// or control and format characters: lettre would read `"a"@b.de`, `(c)a@b.de` or
/// `x<a@b.de>` as the mailbox `a@b.de`, so such a spelling would reach another account's
/// mailbox past `verified_holder`, the verified-address index and the per-address mail limit.
/// The domain has at least two labels of letters, digits and inner hyphens. Stricter than the
/// `users_email_format` CHECK, which it implies.
static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    let atom = r"(?:[A-Za-z0-9!#$%&'*+/=?^_`{|}~-]|[^\x00-\x7F\s\p{Cc}\p{Cf}])+";
    let label = r"[\p{L}\p{N}](?:[\p{L}\p{N}\p{M}-]*[\p{L}\p{N}\p{M}])?";
    Regex::new(&format!(r"^{atom}(?:\.{atom})*@{label}(?:\.{label})+$")).unwrap()
});

/// A username as every lookup and the regex see it: stripped and lowercased (Python's
/// `strip().lower()`; Rust lowercases the Kelvin sign to `k` too).
pub fn normalize_username(raw: &str) -> String {
    py_strip(raw).to_lowercase()
}

/// Whether a normalised username is allowed.
pub fn valid_username(name: &str) -> bool {
    USERNAME.is_match(name)
}

/// Passwords are never trimmed and have no maximum; the minimum counts characters.
pub fn long_enough(password: &str) -> bool {
    char_len(password) >= MIN_PASSWORD_CHARS
}

/// Whether a stripped address is acceptable: a plain address (see `EMAIL`) that lettre also
/// takes as an `Address` unchanged, so the mail goes to exactly the address that was checked.
pub fn valid_email(email: &str) -> bool {
    char_len(email) <= MAX_EMAIL_CHARS
        && EMAIL.is_match(email)
        && lettre::Address::from_str(email).is_ok_and(|address| address.to_string() == email)
}

/// Normalises a preference value like Python's `strip().lower()` and checks it against
/// `allowed`.
pub fn allowed_choice(value: &mut String, allowed: &[&str]) -> bool {
    *value = py_strip(value).to_lowercase();
    allowed.contains(&value.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_are_normalised_before_the_regex() {
        assert_eq!(normalize_username("  Ada_1 "), "ada_1");
        assert_eq!(normalize_username("\u{212A}ate"), "kate");
        assert!(valid_username("ada_1"));
        assert!(valid_username("ab"));
        assert!(!valid_username("a"));
        assert!(!valid_username(&"a".repeat(33)));
        assert!(!valid_username("ada lovelace"));
        assert!(!valid_username("adä"));
        assert!(USERNAME_RULE.contains('\u{2013}'));
    }

    #[test]
    fn passwords_count_characters() {
        assert!(long_enough("pässwörd"));
        assert!(!long_enough("pässwö"));
        assert!(long_enough("        "));
    }

    #[test]
    fn email_addresses() {
        for ok in ["ada@example.test", "a.b+c@mail.example.co.uk", "ä@ü.de"] {
            assert!(valid_email(ok), "{ok}");
        }
        for bad in [
            "",
            "ada",
            "ada@",
            "@example.test",
            "ada@localhost",
            "ada@@example.test",
            "a da@example.test",
            "ada@example..test",
            "ada@.example.test",
            "ada@example.test.",
            "ada.@example.test",
            ".ada@example.test",
            "a..da@example.test",
            "ada@-example.test",
            "ada@example-.test",
            "ada@exa_mple.test",
        ] {
            assert!(!valid_email(bad), "{bad}");
        }
        let long = format!("{}@example.test", "a".repeat(250));
        assert!(!valid_email(&long));
    }

    /// Spellings that a mail library reads as another mailbox (here `victim@example.test`) or
    /// that Postgres cannot store are refused.
    #[test]
    fn email_addresses_are_plain_addr_specs() {
        for bad in [
            "<victim@example.test>",
            "x<victim@example.test>",
            "x <victim@example.test>",
            "\"victim\"@example.test",
            "\"vic tim\"@example.test",
            "(1)victim@example.test",
            "victim(1)@example.test",
            "victim@(1)example.test",
            "a,victim@example.test",
            "a;victim@example.test",
            "a:victim@example.test",
            "vic\\tim@example.test",
            "victim@[127.0.0.1]",
            "a\u{0}b@example.test",
            "a\u{7}b@example.test",
            "a\u{200B}b@example.test",
            "a\u{202E}b@example.test",
            "victim@exam\u{0}ple.test",
        ] {
            assert!(!valid_email(bad), "{bad:?}");
        }
        for ok in [
            "o'brien@example.test",
            "a!#$%&*+/=?^_`{|}~-z@example.test",
            "x-1@sub-domain.example.test",
            "ünïcödé@bücher.de",
        ] {
            assert!(valid_email(ok), "{ok}");
        }
    }

    #[test]
    fn choices_are_stripped_and_lowercased() {
        let mut theme = " Forest ".to_owned();
        assert!(allowed_choice(&mut theme, &THEMES));
        assert_eq!(theme, "forest");
        let mut neon = "neon".to_owned();
        assert!(!allowed_choice(&mut neon, &THEMES));
    }
}
