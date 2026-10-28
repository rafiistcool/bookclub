//! Plain-text email templates in English and German, by `users.locale`
//! (docs/phase1/conventions.md §9, §5.5): the verification link, the reset link, the notice to
//! the old address when the address changes, and the "someone tried to use your address" notice.
//!
//! Token links carry the token in the URL fragment; the notices contain no link that changes
//! anything, only the login and forgot-password pages.

use super::Email;

/// The service a mail comes from.
#[derive(Debug, Clone, Copy)]
pub struct Site<'a> {
    /// `SERVICE_NAME`.
    pub service: &'a str,
    /// `PUBLIC_URL`, an origin without a trailing slash.
    pub public_url: &'a str,
}

/// Who a mail goes to.
#[derive(Debug, Clone, Copy)]
pub struct Recipient<'a> {
    pub to: &'a str,
    /// The account the mail is about.
    pub username: &'a str,
    /// `users.locale`: `de` gets German, everything else English.
    pub locale: &'a str,
}

/// What someone tried with an address a verified account already holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    /// A sign-up named the address.
    SignUp,
    /// Another account's email change named the address.
    EmailChange,
}

/// The verification link of a sign-up or an email change (valid 48 hours).
pub fn verify_email(site: Site, to: Recipient, link: &str) -> Email {
    let (subject, text) = if german(to) {
        (
            format!("Bestätige deine E-Mail-Adresse für {}", site.service),
            format!(
                "Hallo {user},\n\n\
                 bitte bestätige, dass dies deine E-Mail-Adresse für {service} ist:\n\n\
                 {link}\n\n\
                 Der Link gilt 48 Stunden. Wenn du dich nicht bei {service} angemeldet oder deine \
                 Adresse nicht geändert hast, ignoriere diese Mail.\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
            ),
        )
    } else {
        (
            format!("Confirm your email address for {}", site.service),
            format!(
                "Hi {user},\n\n\
                 please confirm that this is your email address for {service}:\n\n\
                 {link}\n\n\
                 The link works for 48 hours. If you did not sign up for {service} or change your \
                 address there, ignore this mail.\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
            ),
        )
    };
    mail(to, subject, text)
}

/// The password reset link (valid one hour, once).
pub fn reset_password(site: Site, to: Recipient, link: &str) -> Email {
    let (subject, text) = if german(to) {
        (
            format!("Setze dein Passwort für {} zurück", site.service),
            format!(
                "Hallo {user},\n\n\
                 jemand möchte das Passwort deines Kontos bei {service} zurücksetzen. Über diesen \
                 Link wählst du ein neues Passwort:\n\n\
                 {link}\n\n\
                 Der Link gilt eine Stunde und nur einmal. Wenn du das nicht warst, ignoriere \
                 diese Mail; dein Passwort bleibt, wie es ist.\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
            ),
        )
    } else {
        (
            format!("Reset your {} password", site.service),
            format!(
                "Hi {user},\n\n\
                 someone asked to reset the password of your {service} account. Open this link \
                 to choose a new one:\n\n\
                 {link}\n\n\
                 The link works for one hour and only once. If this was not you, ignore this \
                 mail; your password stays as it is.\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
            ),
        )
    };
    mail(to, subject, text)
}

/// To the old, verified address when the account asks to change it to `new_address`. The old
/// address stays until the new one is verified.
pub fn email_changed(site: Site, to: Recipient, new_address: &str) -> Email {
    let login = format!("{}/login", site.public_url);
    let forgot = format!("{}/forgot-password", site.public_url);
    let (subject, text) = if german(to) {
        (
            format!(
                "Die E-Mail-Adresse deines Kontos bei {} wird geändert",
                site.service
            ),
            format!(
                "Hallo {user},\n\n\
                 jemand möchte die E-Mail-Adresse deines Kontos bei {service} in {new_address} \
                 ändern. Dein Konto behält diese Adresse, bis die neue bestätigt ist.\n\n\
                 Wenn du das warst, musst du nichts tun.\n\
                 Wenn nicht, melde dich an und ändere dein Passwort: {login}\n\
                 Falls du dich nicht anmelden kannst, setze dein Passwort zurück: {forgot}\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
            ),
        )
    } else {
        (
            format!("Your {} email address is being changed", site.service),
            format!(
                "Hi {user},\n\n\
                 someone asked to change the email address of your {service} account to \
                 {new_address}. Your account keeps this address until the new one is \
                 confirmed.\n\n\
                 If that was you, there is nothing to do.\n\
                 If it was not you, sign in and change your password: {login}\n\
                 If you cannot sign in, reset your password: {forgot}\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
            ),
        )
    };
    mail(to, subject, text)
}

/// To a verified address instead of a verification link, when a sign-up or another account's
/// email change named it (§5.5: register and email change never reveal that an address is
/// taken). It links to login and password reset only.
pub fn address_in_use(site: Site, to: Recipient, attempt: Attempt) -> Email {
    let login = format!("{}/login", site.public_url);
    let forgot = format!("{}/forgot-password", site.public_url);
    let (subject, text) = if german(to) {
        let tried = match attempt {
            Attempt::SignUp => format!("ein neues Konto bei {} anzulegen", site.service),
            Attempt::EmailChange => {
                format!("sie einem anderen Konto bei {} hinzuzufügen", site.service)
            }
        };
        (
            format!(
                "Jemand wollte deine E-Mail-Adresse bei {} verwenden",
                site.service
            ),
            format!(
                "Hallo {user},\n\n\
                 gerade hat jemand versucht, mit deiner E-Mail-Adresse {tried}. Wir haben keinen \
                 Bestätigungslink verschickt: Die Adresse bleibt bei deinem Konto „{user}“.\n\n\
                 Wenn du das warst, melde dich mit deinem Benutzernamen oder dieser Adresse an: \
                 {login}\n\
                 Wenn du dein Passwort vergessen hast, kannst du es hier zurücksetzen: {forgot}\n\n\
                 Wenn du das nicht warst, musst du nichts tun.\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
            ),
        )
    } else {
        let tried = match attempt {
            Attempt::SignUp => format!("sign up for a new {} account", site.service),
            Attempt::EmailChange => format!("add it to another {} account", site.service),
        };
        (
            format!(
                "Someone tried to use your email address on {}",
                site.service
            ),
            format!(
                "Hi {user},\n\n\
                 someone just tried to {tried} with your email address. We did not send them a \
                 confirmation link: the address stays with your account \"{user}\".\n\n\
                 If that was you, sign in with your username or this address: {login}\n\
                 If you forgot your password, you can reset it here: {forgot}\n\n\
                 If it was not you, there is nothing to do.\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
            ),
        )
    };
    mail(to, subject, text)
}

fn german(to: Recipient) -> bool {
    to.locale == "de"
}

fn mail(to: Recipient, subject: String, text: String) -> Email {
    Email {
        to: to.to.to_owned(),
        subject,
        text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SITE: Site = Site {
        service: "Bookclub",
        public_url: "https://books.example.com",
    };

    fn to(locale: &'static str) -> Recipient<'static> {
        Recipient {
            to: "ada@example.test",
            username: "ada",
            locale,
        }
    }

    const LINK: &str = "https://books.example.com/verify-email#token=abc";

    #[test]
    fn links_appear_in_both_languages() {
        for locale in ["en", "de"] {
            let verify = verify_email(SITE, to(locale), LINK);
            assert_eq!(verify.to, "ada@example.test");
            assert!(verify.text.contains(LINK), "{locale}");
            assert!(verify.text.contains("ada"));
            assert!(verify.subject.contains("Bookclub"));
            let reset = reset_password(SITE, to(locale), LINK);
            assert!(reset.text.contains(LINK), "{locale}");
        }
    }

    #[test]
    fn the_locale_picks_the_language() {
        assert!(
            verify_email(SITE, to("de"), LINK)
                .subject
                .starts_with("Bestätige")
        );
        assert!(
            verify_email(SITE, to("en"), LINK)
                .subject
                .starts_with("Confirm")
        );
        assert!(
            verify_email(SITE, to("fr"), LINK)
                .subject
                .starts_with("Confirm")
        );
        assert!(
            reset_password(SITE, to("de"), LINK)
                .text
                .starts_with("Hallo ada")
        );
    }

    #[test]
    fn notices_link_only_to_login_and_password_reset() {
        for locale in ["en", "de"] {
            for attempt in [Attempt::SignUp, Attempt::EmailChange] {
                let notice = address_in_use(SITE, to(locale), attempt);
                assert!(!notice.text.contains("#token="), "{locale}");
                assert!(notice.text.contains("https://books.example.com/login"));
                assert!(
                    notice
                        .text
                        .contains("https://books.example.com/forgot-password")
                );
            }
            let changed = email_changed(SITE, to(locale), "new@example.test");
            assert!(changed.text.contains("new@example.test"));
            assert!(!changed.text.contains("#token="));
            assert!(changed.text.contains("https://books.example.com/login"));
        }
        let sign_up = address_in_use(SITE, to("en"), Attempt::SignUp);
        assert!(sign_up.text.contains("sign up for a new Bookclub account"));
        let change = address_in_use(SITE, to("en"), Attempt::EmailChange);
        assert!(change.text.contains("add it to another Bookclub account"));
    }
}
