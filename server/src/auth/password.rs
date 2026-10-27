//! Password hashing with argon2id (docs/phase1/conventions.md §5.4).
//!
//! Hashing and verifying run on `spawn_blocking` behind a semaphore of [`MAX_CONCURRENT`]
//! permits. A stored hash is verified with the parameters in its PHC string, so legacy
//! argon2-cffi hashes (`m=65536,t=3,p=4`) verify unchanged; login rehashes them.

use std::sync::{Arc, OnceLock};

use anyhow::Context;
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine};
use tokio::sync::Semaphore;

use super::random;

/// Concurrent hash or verify operations; legacy hashes use 64 MiB each.
const MAX_CONCURRENT: usize = 4;

/// argon2 parameters plus the semaphore that bounds concurrent hashing. Held in
/// `AppState::passwords`.
pub struct Passwords {
    /// Parameters for new hashes; a stored hash with other parameters is rehashed on login.
    pub params: Params,
    /// Bounds memory: at most [`MAX_CONCURRENT`] hashes run at once. Each operation moves its
    /// permit into the blocking task, so a cancelled request cannot free the permit while its
    /// hash still runs.
    pub permits: Arc<Semaphore>,
    /// The hash unknown accounts verify against, made on first use with `params`.
    dummy: Arc<OnceLock<String>>,
}

/// The result of checking a password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Wrong password, an unknown account, or a stored hash that does not parse.
    Mismatch,
    /// The password is right. `rehash` is true when the stored hash uses other parameters than
    /// [`Passwords::params`] (or another argon2 variant or version).
    Match { rehash: bool },
}

impl Verdict {
    pub fn is_match(self) -> bool {
        matches!(self, Verdict::Match { .. })
    }
}

impl Passwords {
    /// Production parameters: the argon2 crate default (m = 19456 KiB, t = 2, p = 1, OWASP).
    pub fn new() -> Self {
        Self::with_params(Params::DEFAULT)
    }

    /// Cheap parameters (m = 8, t = 1, p = 1) so tests do not hash at production cost.
    pub fn for_tests() -> Self {
        let params = Params::new(8, 1, 1, None).expect("valid argon2 test parameters");
        Self::with_params(params)
    }

    fn with_params(params: Params) -> Self {
        Self {
            params,
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT)),
            dummy: Arc::new(OnceLock::new()),
        }
    }

    /// A new argon2id PHC string for `password` with [`Passwords::params`] and a random salt.
    pub async fn hash(&self, password: &str) -> anyhow::Result<String> {
        let params = self.params.clone();
        let password = password.to_owned();
        self.blocking(move || hash_with(&params, &password)).await
    }

    /// Checks `password` against a stored PHC string. `None` (an unknown account) verifies
    /// against a dummy hash with the current parameters, so the answer takes as long as for a
    /// real account, and is always a mismatch. An unparsable stored hash is a mismatch too.
    pub async fn verify(&self, password: &str, stored: Option<&str>) -> Verdict {
        let password = password.to_owned();
        let params = self.params.clone();
        match stored {
            Some(stored) => {
                let stored = stored.to_owned();
                self.blocking(move || verify_with(&params, &password, &stored))
                    .await
            }
            None => {
                let dummy = self.dummy.clone();
                self.blocking(move || {
                    let hash = dummy.get_or_init(|| dummy_hash(&params));
                    verify_with(&params, &password, hash);
                    Verdict::Mismatch
                })
                .await
            }
        }
    }

    /// Runs `job` on the blocking pool once a permit is free.
    async fn blocking<T: Send + 'static>(&self, job: impl FnOnce() -> T + Send + 'static) -> T {
        let permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .expect("the password semaphore is never closed");
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            job()
        })
        .await
        .expect("password hashing panicked")
    }
}

impl Default for Passwords {
    fn default() -> Self {
        Self::new()
    }
}

fn argon2(params: &Params) -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params.clone())
}

fn hash_with(params: &Params, password: &str) -> anyhow::Result<String> {
    let hash = argon2(params)
        .hash_password(password.as_bytes())
        .map_err(|error| anyhow::anyhow!("{error}"))
        .context("argon2 hashing failed")?;
    Ok(hash.to_string())
}

fn verify_with(params: &Params, password: &str, stored: &str) -> Verdict {
    let Ok(parsed) = PasswordHash::new(stored) else {
        tracing::warn!("a stored password hash is not a PHC string; treating it as a mismatch");
        return Verdict::Mismatch;
    };
    // Verification uses the algorithm, version and parameters stored in the hash.
    match PasswordVerifier::<PasswordHash>::verify_password(
        &argon2(params),
        password.as_bytes(),
        &parsed,
    ) {
        Ok(()) => Verdict::Match {
            rehash: needs_rehash(params, &parsed),
        },
        Err(_) => Verdict::Mismatch,
    }
}

/// True when `hash` was not made with argon2id v19 and exactly `params`' m, t and p.
fn needs_rehash(params: &Params, hash: &PasswordHash) -> bool {
    if hash.algorithm != Algorithm::Argon2id.ident() || hash.version != Some(Version::V0x13.into())
    {
        return true;
    }
    match Params::try_from(hash) {
        Ok(stored) => {
            stored.m_cost() != params.m_cost()
                || stored.t_cost() != params.t_cost()
                || stored.p_cost() != params.p_cost()
        }
        Err(_) => true,
    }
}

fn dummy_hash(params: &Params) -> String {
    let mut secret = [0u8; 24];
    random::fill(&mut secret);
    hash_with(params, &BASE64_URL_SAFE_NO_PAD.encode(secret)).expect("hashing a dummy password")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made by the Python app's `argon2.PasswordHasher()` (argon2-cffi 25.1: argon2id, v=19,
    /// m=65536, t=3, p=4, 32-byte hash, 16-byte salt) for "correct horse battery staple".
    const LEGACY: &str = "$argon2id$v=19$m=65536,t=3,p=4$HriveBINAQZ51LoIBI0BgA$KL9lKfOZE02xEwFAY8SIj1fwtdBbdbNdM6KT3FTsUSw";

    #[test]
    fn production_and_test_parameters() {
        let prod = Passwords::new();
        assert_eq!(
            (
                prod.params.m_cost(),
                prod.params.t_cost(),
                prod.params.p_cost()
            ),
            (19456, 2, 1)
        );
        let test = Passwords::for_tests();
        assert_eq!(
            (
                test.params.m_cost(),
                test.params.t_cost(),
                test.params.p_cost()
            ),
            (8, 1, 1)
        );
        assert_eq!(test.permits.available_permits(), 4);
    }

    #[tokio::test]
    async fn a_new_hash_verifies_and_needs_no_rehash() {
        let passwords = Passwords::for_tests();
        let hash = passwords.hash("pässwörd").await.unwrap();
        assert!(hash.starts_with("$argon2id$v=19$m=8,t=1,p=1$"), "{hash}");
        assert_eq!(
            passwords.verify("pässwörd", Some(&hash)).await,
            Verdict::Match { rehash: false }
        );
        assert_eq!(
            passwords.verify("passwörd", Some(&hash)).await,
            Verdict::Mismatch
        );
        assert_ne!(
            hash,
            passwords.hash("pässwörd").await.unwrap(),
            "fresh salt"
        );
    }

    #[tokio::test]
    async fn a_legacy_hash_verifies_with_its_own_parameters_and_asks_for_a_rehash() {
        let passwords = Passwords::for_tests();
        assert_eq!(
            passwords
                .verify("correct horse battery staple", Some(LEGACY))
                .await,
            Verdict::Match { rehash: true }
        );
        assert_eq!(
            passwords
                .verify("correct horse battery", Some(LEGACY))
                .await,
            Verdict::Mismatch
        );
    }

    #[tokio::test]
    async fn garbage_and_unknown_accounts_are_mismatches() {
        let passwords = Passwords::for_tests();
        for stored in ["hash", "", "$argon2id$v=19$m=8,t=1,p=1$bad", "$2b$12$abc"] {
            assert_eq!(
                passwords.verify("password1", Some(stored)).await,
                Verdict::Mismatch,
                "{stored}"
            );
        }
        assert_eq!(passwords.verify("password1", None).await, Verdict::Mismatch);
        // The dummy hash is made once and reused.
        let first = passwords.dummy.get().cloned().unwrap();
        passwords.verify("other", None).await;
        assert_eq!(passwords.dummy.get(), Some(&first));
    }

    #[test]
    fn other_variants_and_parameters_need_a_rehash() {
        let params = Params::new(8, 1, 1, None).unwrap();
        let argon2i = Argon2::new(Algorithm::Argon2i, Version::V0x13, params.clone())
            .hash_password(b"x")
            .unwrap();
        assert!(needs_rehash(&params, &argon2i));
        let same = argon2(&params).hash_password(b"x").unwrap();
        assert!(!needs_rehash(&params, &same));
        let other = Params::new(16, 1, 1, None).unwrap();
        assert!(needs_rehash(&other, &same));
    }
}
