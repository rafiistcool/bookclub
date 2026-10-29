//! Plain-text email templates in English and German, by `users.locale`
//! (docs/phase1/conventions.md §9, §5.5): the verification link, the reset link, the notice to
//! the old address when the address changes, the "someone tried to use your address" notice,
//! the instance admins' notices of new reports and catalog requests (#54), and the decision
//! notices of a report: the decision to the reporter and the statement of reasons to whoever
//! uploaded or added what was removed (#54, Q12). Every mail ends with the legal footer (#53).
//!
//! Token links carry the token in the URL fragment; the notices contain no link that changes
//! anything, only the login and forgot-password pages, the admin page and the legal pages.

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
    mail(site, to, subject, text)
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
    mail(site, to, subject, text)
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
    mail(site, to, subject, text)
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
    mail(site, to, subject, text)
}

/// What waits in an admin queue, for [`admin_reports`] and [`admin_requests_digest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueNews {
    /// Open items that arrived since the last mail of this queue (at least 1).
    pub new: i64,
    /// Open items in all.
    pub open: i64,
    /// Whole hours the oldest open item has waited.
    pub oldest_hours: i64,
}

/// To an instance admin: new reports of pictures wait (at most one an hour). Counts only; the
/// link opens the reports tab.
pub fn admin_reports(site: Site, to: Recipient, news: QueueNews) -> Email {
    let link = format!("{}/admin?tab=reports", site.public_url);
    let (subject, text) = if german(to) {
        let new = plural_de(news.new, "neue Meldung", "neue Meldungen");
        (
            format!("{new} bei {}", site.service),
            format!(
                "Hallo {user},\n\n\
                 bei {service} {is} {new} eingegangen. Offen {are} insgesamt {open}; die \
                 älteste wartet seit {age}.\n\n\
                 Hier entscheidest du: {link}\n\n\
                 Du bekommst diese Mail als Admin von {service}, höchstens einmal pro Stunde, \
                 solange neue Meldungen eingehen.\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
                is = if news.new == 1 { "ist" } else { "sind" },
                are = if news.open == 1 { "ist" } else { "sind" },
                open = plural_de(news.open, "Meldung", "Meldungen"),
                age = age_de(news.oldest_hours),
            ),
        )
    } else {
        let new = plural_en(news.new, "new report", "new reports");
        (
            format!("{new} on {}", site.service),
            format!(
                "Hi {user},\n\n\
                 there {is} {new} on {service}. {open} open in all; the oldest has waited \
                 {age}.\n\n\
                 Decide here: {link}\n\n\
                 You get this mail as an admin of {service}, at most once an hour while new \
                 reports arrive.\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
                is = if news.new == 1 { "is" } else { "are" },
                open = plural_en(news.open, "report is", "reports are"),
                age = age_en(news.oldest_hours),
            ),
        )
    };
    mail(site, to, subject, text)
}

/// To an instance admin: the daily digest of new catalog requests (corrections, cover photos,
/// merges). Counts only; the link opens the requests tab.
pub fn admin_requests_digest(site: Site, to: Recipient, news: QueueNews) -> Email {
    let link = format!("{}/admin?tab=requests", site.public_url);
    let (subject, text) = if german(to) {
        let new = plural_de(news.new, "neue Katalog-Anfrage", "neue Katalog-Anfragen");
        (
            format!("{new} bei {}", site.service),
            format!(
                "Hallo {user},\n\n\
                 seit der letzten Übersicht {is} bei {service} {new} eingegangen: Korrekturen, \
                 Cover-Fotos oder doppelte Bücher. Offen {are} insgesamt {open}; die älteste \
                 wartet seit {age}.\n\n\
                 Hier entscheidest du: {link}\n\n\
                 Du bekommst diese Übersicht als Admin von {service}, höchstens einmal am Tag, \
                 wenn neue Anfragen eingegangen sind.\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
                is = if news.new == 1 { "ist" } else { "sind" },
                are = if news.open == 1 { "ist" } else { "sind" },
                open = plural_de(news.open, "Anfrage", "Anfragen"),
                age = age_de(news.oldest_hours),
            ),
        )
    } else {
        let new = plural_en(news.new, "new catalog request", "new catalog requests");
        (
            format!("{new} on {}", site.service),
            format!(
                "Hi {user},\n\n\
                 there {is} {new} on {service} since the last digest: corrections, cover \
                 photos or books entered twice. {open} open in all; the oldest has waited \
                 {age}.\n\n\
                 Decide here: {link}\n\n\
                 You get this digest as an admin of {service}, at most once a day when new \
                 requests arrived.\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
                is = if news.new == 1 { "is" } else { "are" },
                open = plural_en(news.open, "request is", "requests are"),
                age = age_en(news.oldest_hours),
            ),
        )
    };
    mail(site, to, subject, text)
}

/// "1 new report", "3 new reports".
fn plural_en(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// "1 neue Meldung", "3 neue Meldungen".
fn plural_de(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// How long the oldest item has waited: "less than an hour", "71 hours", "3 days" (hours below
/// the 72-hour target, as the admin page shows them).
fn age_en(hours: i64) -> String {
    match hours {
        ..=0 => "less than an hour".to_owned(),
        1 => "1 hour".to_owned(),
        2..72 => format!("{hours} hours"),
        _ => format!("{} days", hours / 24),
    }
}

/// The same after "seit": "weniger als einer Stunde", "5 Stunden", "3 Tagen".
fn age_de(hours: i64) -> String {
    match hours {
        ..=0 => "weniger als einer Stunde".to_owned(),
        1 => "einer Stunde".to_owned(),
        2..72 => format!("{hours} Stunden"),
        _ => format!("{} Tagen", hours / 24),
    }
}

/// What a decision notice ([`report_decision`], [`statement_of_reasons`]) is about, named as
/// it was when it was reported (a removed book's title is gone from the catalog by then).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reported<'a> {
    /// A cover picture, with the title of the book that showed it when known.
    Cover(Option<&'a str>),
    /// A profile picture.
    Avatar,
    /// A profile picture its owner had replaced or deleted while it was reported (held for the
    /// decision, `media/avatar.rs`): the owner's current picture is another one, or none.
    /// Reporters read it as [`Reported::Avatar`]; the owner's statement says that it was an
    /// earlier picture, so they do not take their current one for removed.
    EarlierAvatar,
    /// A book a reader added, by its title.
    Work(&'a str),
    /// An edition a reader added, by its book's title.
    Edition(&'a str),
}

/// What came of a report, for its reporter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// An admin removed it.
    Removed,
    /// An admin kept it: no breach found.
    Kept,
    /// It no longer shows as reported, so there was nothing left to decide: a picture its owner
    /// replaced or deleted, or a book merged into another entry of the same book.
    Gone,
}

/// To the reporter, after an admin decided their report (DSA Art. 16 (5)): removed, kept, or no
/// longer online. Names neither the uploader nor what the report said. `contact` is
/// `CONTACT_EMAIL`.
pub fn report_decision(
    site: Site,
    to: Recipient,
    what: Reported,
    outcome: Outcome,
    contact: Option<&str>,
) -> Email {
    let book = matches!(what, Reported::Work(_) | Reported::Edition(_));
    // German: "es" for das Coverbild, das Profilbild, das Buch; "sie" for die Ausgabe.
    let (it, it_upper) = if matches!(what, Reported::Edition(_)) {
        ("sie", "Sie")
    } else {
        ("es", "Es")
    };
    let (subject, text) = if german(to) {
        let what = match what {
            Reported::Cover(Some(title)) => {
                format!("das Coverbild von „{title}“, das du gemeldet hast,")
            }
            Reported::Cover(None) => "das Coverbild, das du gemeldet hast,".to_owned(),
            Reported::Avatar | Reported::EarlierAvatar => {
                "das Profilbild, das du gemeldet hast,".to_owned()
            }
            Reported::Work(title) => format!("das Buch „{title}“, das du gemeldet hast,"),
            Reported::Edition(title) => {
                format!("die Ausgabe von „{title}“, die du gemeldet hast,")
            }
        };
        let (outcome, decision, next) = match outcome {
            Outcome::Removed => (
                "entfernt",
                format!(
                    "Ein Admin von {} hat {what} geprüft und entfernt.",
                    site.service
                ),
                "Wer den Inhalt hochgeladen oder eingetragen hat, erfährt, dass und warum er \
                 entfernt wurde, aber nicht, von wem die Meldung kam."
                    .to_owned(),
            ),
            Outcome::Kept => (
                "nicht entfernt",
                format!(
                    "Ein Admin von {} hat {what} geprüft und nicht entfernt: Ein Verstoß gegen \
                     Gesetze oder die Nutzungsbedingungen wurde nicht festgestellt.",
                    site.service
                ),
                format!(
                    "Wenn du anderer Meinung bist, schreib an {}.",
                    contact_de(site, contact)
                ),
            ),
            Outcome::Gone if book => (
                "zusammengeführt",
                format!(
                    "Ein Admin von {} hat {what} geprüft: {it_upper} wird nicht mehr eigenständig \
                     angezeigt, weil {it} mit einem anderen Eintrag desselben Buchs \
                     zusammengeführt wurde.",
                    site.service
                ),
                format!(
                    "Wenn du dort noch ein Problem siehst, schreib an {}.",
                    contact_de(site, contact)
                ),
            ),
            Outcome::Gone => (
                "nicht mehr online",
                format!(
                    "Ein Admin von {} hat {what} geprüft: {it_upper} ist nicht mehr online, weil {it} \
                     ersetzt oder gelöscht wurde.",
                    site.service
                ),
                format!("Wenn {it} wieder auftaucht, kannst du {it} erneut melden."),
            ),
        };
        (
            format!("Deine Meldung bei {}: {outcome}", site.service),
            format!(
                "Hallo {user},\n\n\
                 danke für deine Meldung. {decision}\n\n\
                 {next}\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
            ),
        )
    } else {
        let what = match what {
            Reported::Cover(Some(title)) => {
                format!("the cover picture of \"{title}\" you reported")
            }
            Reported::Cover(None) => "the cover picture you reported".to_owned(),
            Reported::Avatar | Reported::EarlierAvatar => {
                "the profile picture you reported".to_owned()
            }
            Reported::Work(title) => format!("the book \"{title}\" you reported"),
            Reported::Edition(title) => format!("the edition of \"{title}\" you reported"),
        };
        let (outcome, decision, next) = match outcome {
            Outcome::Removed => (
                "removed",
                format!(
                    "An admin of {} reviewed {what} and removed it.",
                    site.service
                ),
                "Whoever uploaded or added it is told that it was removed and why, but not who \
                 reported it."
                    .to_owned(),
            ),
            Outcome::Kept => (
                "not removed",
                format!(
                    "An admin of {} reviewed {what} and did not remove it: the admin found no \
                     breach of the law or of the terms of use.",
                    site.service
                ),
                format!("If you disagree, write to {}.", contact_en(site, contact)),
            ),
            Outcome::Gone if book => (
                "merged",
                format!(
                    "An admin of {} reviewed {what}: it no longer shows on its own, because it \
                     was merged with another entry of the same book.",
                    site.service
                ),
                format!(
                    "If you still see a problem there, write to {}.",
                    contact_en(site, contact)
                ),
            ),
            Outcome::Gone => (
                "no longer online",
                format!(
                    "An admin of {} reviewed {what}: it is no longer online, because it was \
                     replaced or deleted.",
                    site.service
                ),
                "If it shows up again, you can report it again.".to_owned(),
            ),
        };
        (
            format!("Your report on {}: {outcome}", site.service),
            format!(
                "Hi {user},\n\n\
                 thank you for your report. {decision}\n\n\
                 {next}\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
            ),
        )
    };
    mail(site, to, subject, text)
}

/// To whoever uploaded or added content an admin removed: the statement of reasons of DSA
/// Art. 17 (what was removed, the ground, that a person decided, and how to object). `reason`
/// is the `reports.reason` of the report or notice it was removed on, `None` when an admin
/// removed it without one. Never names the reporter or repeats what the report said. `contact`
/// is `CONTACT_EMAIL`.
pub fn statement_of_reasons(
    site: Site,
    to: Recipient,
    what: Reported,
    reason: Option<&str>,
    contact: Option<&str>,
) -> Email {
    let terms = format!("{}/terms", site.public_url);
    let (subject, text) = if german(to) {
        let (subject, what, effect) = match what {
            Reported::Cover(title) => (
                format!("Dein Cover-Foto bei {} wurde entfernt", site.service),
                match title {
                    Some(title) => {
                        format!("das Cover-Foto, das du für „{title}“ hochgeladen hast,")
                    }
                    None => "ein Cover-Foto, das du hochgeladen hast,".to_owned(),
                },
                "Das Bild erscheint bei keinem Buch mehr, und dasselbe Bild kann nicht noch \
                 einmal hochgeladen werden.",
            ),
            Reported::Avatar => (
                format!("Dein Profilbild bei {} wurde entfernt", site.service),
                "dein Profilbild".to_owned(),
                "Das Bild erscheint nicht mehr in deinem Profil, und dasselbe Bild kann nicht \
                 noch einmal hochgeladen werden. In den Einstellungen kannst du ein anderes \
                 wählen.",
            ),
            Reported::EarlierAvatar => (
                format!(
                    "Ein früheres Profilbild von dir bei {} wurde entfernt",
                    site.service
                ),
                "ein früheres Profilbild von dir".to_owned(),
                "Du hattest dieses Bild schon ersetzt oder gelöscht; dein Profil, wie es jetzt \
                 ist, ändert sich nicht. Das entfernte Bild kann nicht noch einmal hochgeladen \
                 werden.",
            ),
            Reported::Work(title) => (
                format!(
                    "Ein Buch, das du bei {} eingetragen hast, wurde entfernt",
                    site.service
                ),
                format!("das Buch „{title}“, das du eingetragen hast,"),
                "Titel, Untertitel, Autorin / Autor, Beschreibung und Themen wurden entfernt. \
                 Das Buch bleibt in den Regalen, mit dem Titel „[entfernt]“ (in manchen Listen \
                 „[removed]“), damit niemandem seine Lesehistorie verloren geht.",
            ),
            Reported::Edition(title) => (
                format!(
                    "Eine Ausgabe, die du bei {} eingetragen hast, wurde entfernt",
                    site.service
                ),
                format!("die Ausgabe von „{title}“, die du eingetragen hast,"),
                "Titel, Untertitel und Verlag wurden entfernt. Die Ausgabe bleibt, unter dem \
                 Titel des Buchs.",
            ),
        };
        let why = match reason {
            Some(reason) => format!(
                "Der Inhalt wurde gemeldet, weil {}, und der Admin, der die Meldung geprüft hat, \
                 hat zugestimmt.",
                match reason {
                    "copyright" => "er die Urheberrechte anderer verletzt",
                    "wrong_book" => "er zu einem anderen Buch gehört",
                    "offensive" => "er anstößig oder beleidigend ist",
                    _ => "er gegen Gesetze oder die Nutzungsbedingungen verstößt",
                }
            ),
            None => "Ein Admin hat festgestellt, dass der Inhalt gegen Gesetze oder die \
                     Nutzungsbedingungen verstößt."
                .to_owned(),
        };
        (
            subject,
            format!(
                "Hallo {user},\n\n\
                 ein Admin von {service} hat {what} entfernt.\n\n\
                 Warum: {why} Die Entscheidung hat ein Mensch getroffen, kein automatisches \
                 System.\n\n\
                 Was sich ändert: {effect}\n\n\
                 Wenn du die Entscheidung für falsch hältst, schreib an {contact}; sie wird dann \
                 noch einmal geprüft. Die Nutzungsbedingungen: {terms}\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
                contact = contact_de(site, contact),
            ),
        )
    } else {
        let (subject, what, effect) = match what {
            Reported::Cover(title) => (
                format!("Your cover photo on {} was removed", site.service),
                match title {
                    Some(title) => format!("the cover photo you uploaded for \"{title}\""),
                    None => "a cover photo you uploaded".to_owned(),
                },
                "The picture no longer shows on any book, and the same picture cannot be \
                 uploaded again.",
            ),
            Reported::Avatar => (
                format!("Your profile picture on {} was removed", site.service),
                "your profile picture".to_owned(),
                "The picture no longer shows on your profile, and the same picture cannot be \
                 uploaded again. You can choose another one in Settings.",
            ),
            Reported::EarlierAvatar => (
                format!(
                    "An earlier profile picture of yours on {} was removed",
                    site.service
                ),
                "an earlier profile picture of yours".to_owned(),
                "You had already replaced or deleted this picture, so your profile as it is now \
                 does not change. The removed picture cannot be uploaded again.",
            ),
            Reported::Work(title) => (
                format!("A book you added on {} was removed", site.service),
                format!("the book \"{title}\" you added"),
                "Its title, subtitle, authors, description and subjects were removed. The book \
                 stays on shelves, titled \"[removed]\", so nobody loses their reading history.",
            ),
            Reported::Edition(title) => (
                format!("An edition you added on {} was removed", site.service),
                format!("the edition of \"{title}\" you added"),
                "Its title, subtitle and publisher were removed. The edition stays, under the \
                 book's own title.",
            ),
        };
        let why = match reason {
            Some(reason) => format!(
                "it was reported because {}, and the admin who reviewed the report agreed.",
                match reason {
                    "copyright" => "it infringes someone else's copyright",
                    "wrong_book" => "it belongs to a different book",
                    "offensive" => "it is offensive",
                    _ => "it breaks the law or the terms of use",
                }
            ),
            None => "an admin found that it breaks the law or the terms of use.".to_owned(),
        };
        (
            subject,
            format!(
                "Hi {user},\n\n\
                 an admin of {service} removed {what}.\n\n\
                 Why: {why} A person made this decision, not an automated system.\n\n\
                 What changed: {effect}\n\n\
                 If you think the decision is wrong, write to {contact}; it will be reviewed \
                 again. The terms of use: {terms}\n\n\
                 – {service}\n",
                user = to.username,
                service = site.service,
                contact = contact_en(site, contact),
            ),
        )
    };
    mail(site, to, subject, text)
}

/// Where to object: `CONTACT_EMAIL`, else the admins.
fn contact_en(site: Site, contact: Option<&str>) -> String {
    contact.map_or_else(|| format!("the admins of {}", site.service), str::to_owned)
}

/// The same after "schreib an".
fn contact_de(site: Site, contact: Option<&str>) -> String {
    contact.map_or_else(|| format!("die Admins von {}", site.service), str::to_owned)
}

fn german(to: Recipient) -> bool {
    to.locale == "de"
}

/// The legal footer every mail ends with (#53): links to the imprint and the privacy policy, in
/// the recipient's language, after a signature separator.
fn footer(site: Site, to: Recipient) -> String {
    let url = site.public_url;
    if german(to) {
        format!("\n-- \nImpressum: {url}/imprint · Datenschutz: {url}/privacy\n")
    } else {
        format!("\n-- \nImprint: {url}/imprint · Privacy: {url}/privacy\n")
    }
}

/// A template's mail: `text`, then the legal footer.
fn mail(site: Site, to: Recipient, subject: String, text: String) -> Email {
    Email {
        to: to.to.to_owned(),
        subject,
        text: text + &footer(site, to),
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

    const NEWS: QueueNews = QueueNews {
        new: 2,
        open: 3,
        oldest_hours: 50,
    };

    #[test]
    fn admin_notices_link_to_their_tab_in_both_languages() {
        for locale in ["en", "de"] {
            let reports = admin_reports(SITE, to(locale), NEWS);
            assert_eq!(reports.to, "ada@example.test");
            assert!(
                reports
                    .text
                    .contains("https://books.example.com/admin?tab=reports"),
                "{locale}"
            );
            assert!(reports.subject.contains("Bookclub"), "{locale}");
            assert!(reports.text.contains("ada"), "{locale}");
            assert!(!reports.text.contains("#token="));
            let digest = admin_requests_digest(SITE, to(locale), NEWS);
            assert!(
                digest
                    .text
                    .contains("https://books.example.com/admin?tab=requests"),
                "{locale}"
            );
        }
    }

    #[test]
    fn admin_notices_count_in_the_recipients_language() {
        let en = admin_reports(SITE, to("en"), NEWS);
        assert_eq!(en.subject, "2 new reports on Bookclub");
        assert!(en.text.contains("there are 2 new reports on Bookclub."));
        assert!(
            en.text
                .contains("3 reports are open in all; the oldest has waited 50 hours.")
        );
        let de = admin_reports(SITE, to("de"), NEWS);
        assert_eq!(de.subject, "2 neue Meldungen bei Bookclub");
        assert!(de.text.starts_with("Hallo ada"));
        assert!(
            de.text
                .contains("bei Bookclub sind 2 neue Meldungen eingegangen.")
        );
        assert!(
            de.text
                .contains("Offen sind insgesamt 3 Meldungen; die älteste wartet seit 50 Stunden.")
        );
        let one = QueueNews {
            new: 1,
            open: 1,
            oldest_hours: 0,
        };
        let en = admin_requests_digest(SITE, to("en"), one);
        assert_eq!(en.subject, "1 new catalog request on Bookclub");
        assert!(
            en.text
                .contains("there is 1 new catalog request on Bookclub since")
        );
        assert!(
            en.text
                .contains("1 request is open in all; the oldest has waited less than an hour.")
        );
        let de = admin_requests_digest(SITE, to("de"), one);
        assert_eq!(de.subject, "1 neue Katalog-Anfrage bei Bookclub");
        assert!(
            de.text
                .contains("ist bei Bookclub 1 neue Katalog-Anfrage eingegangen")
        );
        assert!(de.text.contains(
            "Offen ist insgesamt 1 Anfrage; die älteste wartet seit weniger als einer Stunde."
        ));
    }

    #[test]
    fn ages_read_in_hours_then_days() {
        assert_eq!(age_en(1), "1 hour");
        assert_eq!(age_en(71), "71 hours");
        assert_eq!(age_en(72), "3 days");
        assert_eq!(age_de(1), "einer Stunde");
        assert_eq!(age_de(5), "5 Stunden");
        assert_eq!(age_de(80), "3 Tagen");
    }

    const REPORTED: [Reported<'static>; 5] = [
        Reported::Cover(Some("Circe")),
        Reported::Cover(None),
        Reported::Avatar,
        Reported::Work("Circe"),
        Reported::Edition("Circe"),
    ];

    /// One mail of every template in `locale`, by template name.
    fn every_template(locale: &'static str) -> Vec<(&'static str, Email)> {
        let to = to(locale);
        let mut mails = vec![
            ("verify_email", verify_email(SITE, to, LINK)),
            ("reset_password", reset_password(SITE, to, LINK)),
            ("email_changed", email_changed(SITE, to, "new@example.test")),
            ("address_in_use", address_in_use(SITE, to, Attempt::SignUp)),
            (
                "address_in_use",
                address_in_use(SITE, to, Attempt::EmailChange),
            ),
            ("admin_reports", admin_reports(SITE, to, NEWS)),
            (
                "admin_requests_digest",
                admin_requests_digest(SITE, to, NEWS),
            ),
        ];
        for what in REPORTED {
            for outcome in [Outcome::Removed, Outcome::Kept, Outcome::Gone] {
                let mail = report_decision(SITE, to, what, outcome, Some(CONTACT));
                mails.push(("report_decision", mail));
            }
            for reason in [Some("copyright"), None] {
                let mail = statement_of_reasons(SITE, to, what, reason, Some(CONTACT));
                mails.push(("statement_of_reasons", mail));
            }
        }
        mails
    }

    const CONTACT: &str = "abuse@books.example.com";

    /// #53: every template ends with one line linking the imprint and the privacy policy, in the
    /// recipient's language, after a signature separator. The list covers every `pub fn … ->
    /// Email` of this file, so a new template fails here until it is added.
    #[test]
    fn every_template_ends_with_the_legal_footer() {
        let declared: std::collections::BTreeSet<String> =
            regex::Regex::new(r"(?s)pub fn (\w+)\([^{]*?\) -> Email \{")
                .unwrap()
                .captures_iter(include_str!("templates.rs"))
                .map(|captures| captures[1].to_owned())
                .collect();
        let covered: std::collections::BTreeSet<String> = every_template("en")
            .into_iter()
            .map(|(name, _)| name.to_owned())
            .collect();
        assert_eq!(declared, covered);
        assert!(declared.len() >= 8, "{declared:?}");
        for (locale, line) in [
            (
                "en",
                "\n\n-- \nImprint: https://books.example.com/imprint · Privacy: \
                 https://books.example.com/privacy\n",
            ),
            (
                "de",
                "\n\n-- \nImpressum: https://books.example.com/imprint · Datenschutz: \
                 https://books.example.com/privacy\n",
            ),
        ] {
            for (name, mail) in every_template(locale) {
                assert!(
                    mail.text.ends_with(line),
                    "{name} ({locale}): {}",
                    mail.text
                );
                assert_eq!(mail.text.matches("-- \n").count(), 1, "{name} ({locale})");
            }
        }
    }

    /// Q12 (DSA Art. 16 (5)): the reporter learns the decision in their language, and where to
    /// object to a kept report; nothing names the uploader.
    #[test]
    fn the_reporter_learns_the_decision() {
        let removed = report_decision(
            SITE,
            to("en"),
            Reported::Work("Circe"),
            Outcome::Removed,
            None,
        );
        assert_eq!(removed.subject, "Your report on Bookclub: removed");
        assert!(
            removed
                .text
                .starts_with("Hi ada,\n\nthank you for your report.")
        );
        assert!(removed.text.contains(
            "An admin of Bookclub reviewed the book \"Circe\" you reported and removed it."
        ));
        assert!(removed.text.contains("but not who reported it"));
        let kept = report_decision(
            SITE,
            to("en"),
            Reported::Avatar,
            Outcome::Kept,
            Some(CONTACT),
        );
        assert_eq!(kept.subject, "Your report on Bookclub: not removed");
        assert!(
            kept.text
                .contains("reviewed the profile picture you reported and did not")
        );
        assert!(
            kept.text
                .contains("If you disagree, write to abuse@books.example.com.")
        );
        let kept = report_decision(SITE, to("en"), Reported::Cover(None), Outcome::Kept, None);
        assert!(kept.text.contains("write to the admins of Bookclub."));

        let removed = report_decision(
            SITE,
            to("de"),
            Reported::Edition("Circe"),
            Outcome::Removed,
            None,
        );
        assert_eq!(removed.subject, "Deine Meldung bei Bookclub: entfernt");
        assert!(
            removed
                .text
                .starts_with("Hallo ada,\n\ndanke für deine Meldung.")
        );
        assert!(removed.text.contains(
            "Ein Admin von Bookclub hat die Ausgabe von „Circe“, die du gemeldet hast, geprüft \
             und entfernt."
        ));
        let kept = report_decision(
            SITE,
            to("de"),
            Reported::Cover(Some("Circe")),
            Outcome::Kept,
            None,
        );
        assert_eq!(kept.subject, "Deine Meldung bei Bookclub: nicht entfernt");
        assert!(
            kept.text
                .contains("hat das Coverbild von „Circe“, das du gemeldet hast, geprüft und nicht")
        );
        assert!(kept.text.contains("schreib an die Admins von Bookclub."));

        // Nothing left to decide: a profile picture its owner replaced, a book merged away.
        let gone = report_decision(
            SITE,
            to("en"),
            Reported::Avatar,
            Outcome::Gone,
            Some(CONTACT),
        );
        assert_eq!(gone.subject, "Your report on Bookclub: no longer online");
        assert!(gone.text.contains(
            "reviewed the profile picture you reported: it is no longer online, because it was \
             replaced or deleted."
        ));
        assert!(
            gone.text
                .contains("If it shows up again, you can report it again.")
        );
        let merged = report_decision(SITE, to("en"), Reported::Work("Zine"), Outcome::Gone, None);
        assert_eq!(merged.subject, "Your report on Bookclub: merged");
        assert!(merged.text.contains(
            "reviewed the book \"Zine\" you reported: it no longer shows on its own, because it \
             was merged with another entry of the same book."
        ));
        assert!(merged.text.contains("write to the admins of Bookclub."));
        let gone = report_decision(SITE, to("de"), Reported::Avatar, Outcome::Gone, None);
        assert_eq!(
            gone.subject,
            "Deine Meldung bei Bookclub: nicht mehr online"
        );
        assert!(gone.text.contains(
            "hat das Profilbild, das du gemeldet hast, geprüft: Es ist nicht mehr online, weil \
             es ersetzt oder gelöscht wurde."
        ));
        assert!(
            gone.text
                .contains("Wenn es wieder auftaucht, kannst du es erneut melden.")
        );
        let merged = report_decision(
            SITE,
            to("de"),
            Reported::Edition("Zine"),
            Outcome::Gone,
            None,
        );
        assert_eq!(
            merged.subject,
            "Deine Meldung bei Bookclub: zusammengeführt"
        );
        assert!(merged.text.contains(
            "die du gemeldet hast, geprüft: Sie wird nicht mehr eigenständig angezeigt, weil sie \
             mit einem anderen Eintrag desselben Buchs zusammengeführt wurde."
        ));
        for mail in [&gone, &merged] {
            assert!(!mail.text.contains("Verstoß"), "{}", mail.text);
        }
    }

    /// Q12 (DSA Art. 17): the uploader learns what was removed, the reported ground, that a
    /// person decided and how to object, with the terms linked. The reporter's words never
    /// appear: the template does not take them.
    #[test]
    fn the_uploader_gets_a_statement_of_reasons() {
        let cases = [
            (
                Reported::Cover(Some("Circe")),
                "Your cover photo on Bookclub was removed",
                "an admin of Bookclub removed the cover photo you uploaded for \"Circe\".",
            ),
            (
                Reported::Avatar,
                "Your profile picture on Bookclub was removed",
                "an admin of Bookclub removed your profile picture.",
            ),
            (
                Reported::Work("Circe"),
                "A book you added on Bookclub was removed",
                "titled \"[removed]\"",
            ),
            (
                Reported::Edition("Circe"),
                "An edition you added on Bookclub was removed",
                "the edition of \"Circe\" you added",
            ),
        ];
        for (what, subject, line) in cases {
            let mail = statement_of_reasons(SITE, to("en"), what, Some("offensive"), Some(CONTACT));
            assert_eq!(mail.subject, subject);
            assert!(mail.text.contains(line), "{}", mail.text);
            assert!(mail.text.contains("because it is offensive"));
            assert!(
                mail.text
                    .contains("A person made this decision, not an automated system.")
            );
            assert!(mail.text.contains("write to abuse@books.example.com;"));
            assert!(
                mail.text
                    .contains("The terms of use: https://books.example.com/terms")
            );
        }
        for (reason, en, de) in [
            (
                "copyright",
                "it infringes someone else's copyright",
                "er die Urheberrechte anderer verletzt",
            ),
            (
                "wrong_book",
                "it belongs to a different book",
                "er zu einem anderen Buch gehört",
            ),
            (
                "other",
                "it breaks the law or the terms of use",
                "er gegen Gesetze oder die Nutzungsbedingungen verstößt",
            ),
        ] {
            let mail = statement_of_reasons(SITE, to("en"), Reported::Avatar, Some(reason), None);
            assert!(mail.text.contains(&format!("because {en},")), "{reason}");
            assert!(mail.text.contains("write to the admins of Bookclub;"));
            let mail = statement_of_reasons(SITE, to("de"), Reported::Avatar, Some(reason), None);
            assert!(mail.text.contains(&format!("weil {de},")), "{reason}");
        }
        // Removed without a report or notice: the admin's own finding, not "it was reported".
        let own = statement_of_reasons(SITE, to("en"), Reported::Avatar, None, None);
        assert!(own.text.contains(
            "Why: an admin found that it breaks the law or the terms of use. A person made this \
             decision, not an automated system."
        ));
        assert!(!own.text.contains("reported"));
        let own = statement_of_reasons(SITE, to("de"), Reported::Avatar, None, None);
        assert!(own.text.contains(
            "Warum: Ein Admin hat festgestellt, dass der Inhalt gegen Gesetze oder die \
             Nutzungsbedingungen verstößt. Die Entscheidung hat ein Mensch getroffen"
        ));
        assert!(!own.text.contains("gemeldet"));
        let de = statement_of_reasons(
            SITE,
            to("de"),
            Reported::Work("Circe"),
            Some("copyright"),
            Some(CONTACT),
        );
        assert_eq!(
            de.subject,
            "Ein Buch, das du bei Bookclub eingetragen hast, wurde entfernt"
        );
        assert!(de.text.starts_with(
            "Hallo ada,\n\nein Admin von Bookclub hat das Buch „Circe“, das du eingetragen \
             hast, entfernt."
        ));
        assert!(
            de.text
                .contains("Die Nutzungsbedingungen: https://books.example.com/terms")
        );
        assert!(de.text.contains("schreib an abuse@books.example.com;"));
        // The title the book page shows in German (`report.removedTitle`), and the stored one
        // that lists still show.
        assert!(
            de.text
                .contains("mit dem Titel „[entfernt]“ (in manchen Listen „[removed]“)")
        );
    }

    /// A held profile picture (its owner had replaced or deleted it while it was reported) is
    /// named as an earlier picture, so the owner does not read that their current one is gone.
    /// Its reporters read it like any profile picture.
    #[test]
    fn the_statement_for_an_earlier_profile_picture_says_so() {
        let en = statement_of_reasons(
            SITE,
            to("en"),
            Reported::EarlierAvatar,
            Some("offensive"),
            None,
        );
        assert_eq!(
            en.subject,
            "An earlier profile picture of yours on Bookclub was removed"
        );
        assert!(
            en.text
                .contains("an admin of Bookclub removed an earlier profile picture of yours.")
        );
        assert!(en.text.contains(
            "You had already replaced or deleted this picture, so your profile as it is now does \
             not change."
        ));
        assert!(!en.text.contains("no longer shows on your profile"));
        let de = statement_of_reasons(
            SITE,
            to("de"),
            Reported::EarlierAvatar,
            Some("offensive"),
            None,
        );
        assert_eq!(
            de.subject,
            "Ein früheres Profilbild von dir bei Bookclub wurde entfernt"
        );
        assert!(
            de.text
                .contains("ein Admin von Bookclub hat ein früheres Profilbild von dir entfernt.")
        );
        assert!(!de.text.contains("erscheint nicht mehr in deinem Profil"));
        let reporter = report_decision(
            SITE,
            to("en"),
            Reported::EarlierAvatar,
            Outcome::Removed,
            None,
        );
        let plain = report_decision(SITE, to("en"), Reported::Avatar, Outcome::Removed, None);
        assert_eq!(reporter.text, plain.text);
    }
}
