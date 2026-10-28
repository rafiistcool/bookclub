//! Password hashing with argon2id (docs/phase1/conventions.md §5.4).
//!
//! Hashing and verifying run on `spawn_blocking` behind a semaphore of [`MAX_CONCURRENT`]
//! permits. A stored hash is verified with the parameters in its PHC string, so legacy
//! argon2-cffi hashes (`m=65536,t=3,p=4`) verify unchanged; login rehashes them.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Context;
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use base64::prelude::{BASE64_URL_SAFE_NO_PAD, Engine};
use tokio::sync::Semaphore;

use super::random;

/// Concurrent hash or verify operations; legacy hashes use 64 MiB each.
const MAX_CONCURRENT: usize = 4;
/// The longest an operation waits for a permit before it gives up with [`Busy`]. At about
/// 30 ms per production hash, ten seconds is a queue of over a thousand operations, which
/// normal load never builds: the queue cannot grow without bound when something floods it.
const MAX_WAIT: Duration = Duration::from_secs(10);

/// The password semaphore stayed full for the whole wait (503 `SERVER_BUSY`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("every password permit stayed busy")]
pub struct Busy;

/// argon2 parameters plus the semaphore that bounds concurrent hashing. Held in
/// `AppState::passwords`.
pub struct Passwords {
    /// Parameters for new hashes; a stored hash with other parameters is rehashed on login.
    pub params: Params,
    /// Bounds memory: at most [`MAX_CONCURRENT`] hashes run at once. Each operation moves its
    /// permit into the blocking task, so a cancelled request cannot free the permit while its
    /// hash still runs.
    pub permits: Arc<Semaphore>,
    /// How long an operation waits for a permit ([`MAX_WAIT`]) before it fails with [`Busy`].
    max_wait: Duration,
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
            max_wait: MAX_WAIT,
            dummy: Arc::new(OnceLock::new()),
        }
    }

    /// A new argon2id PHC string for `password` with [`Passwords::params`] and a random salt.
    /// Fails with [`Busy`] (inside the `anyhow::Error`, which `AppError` answers with 503) when
    /// no permit frees up within the wait.
    pub async fn hash(&self, password: &str) -> anyhow::Result<String> {
        let params = self.params.clone();
        let password = password.to_owned();
        self.blocking(move || hash_with(&params, &password)).await?
    }

    /// Checks `password` against a stored PHC string. `None` (an unknown account) verifies
    /// against a dummy hash with the current parameters, so the answer takes as long as for a
    /// real account, and is always a mismatch. An unparsable stored hash is a mismatch too.
    /// [`Busy`] when no permit frees up within the wait.
    pub async fn verify(&self, password: &str, stored: Option<&str>) -> Result<Verdict, Busy> {
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

    /// Runs `job` on the blocking pool once a permit is free, or gives up with [`Busy`] after
    /// `max_wait`.
    async fn blocking<T: Send + 'static>(
        &self,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, Busy> {
        let permit = tokio::time::timeout(self.max_wait, self.permits.clone().acquire_owned())
            .await
            .map_err(|_| Busy)?
            .expect("the password semaphore is never closed");
        Ok(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            job()
        })
        .await
        .expect("password hashing panicked"))
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
            Ok(Verdict::Match { rehash: false })
        );
        assert_eq!(
            passwords.verify("passwörd", Some(&hash)).await,
            Ok(Verdict::Mismatch)
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
            Ok(Verdict::Match { rehash: true })
        );
        assert_eq!(
            passwords
                .verify("correct horse battery", Some(LEGACY))
                .await,
            Ok(Verdict::Mismatch)
        );
    }

    #[tokio::test]
    async fn garbage_and_unknown_accounts_are_mismatches() {
        let passwords = Passwords::for_tests();
        for stored in ["hash", "", "$argon2id$v=19$m=8,t=1,p=1$bad", "$2b$12$abc"] {
            assert_eq!(
                passwords.verify("password1", Some(stored)).await,
                Ok(Verdict::Mismatch),
                "{stored}"
            );
        }
        assert_eq!(
            passwords.verify("password1", None).await,
            Ok(Verdict::Mismatch)
        );
        // The dummy hash is made once and reused.
        let first = passwords.dummy.get().cloned().unwrap();
        passwords.verify("other", None).await;
        assert_eq!(passwords.dummy.get(), Some(&first));
    }

    /// Security review 2026-10-28 #1 (backstop): an operation that cannot get a permit gives
    /// up instead of queueing until the request deadline, so a flood cannot hold every login of
    /// the instance in an unbounded queue.
    #[tokio::test]
    async fn a_full_semaphore_answers_busy_after_the_wait() {
        let passwords = Passwords {
            max_wait: Duration::from_millis(50),
            ..Passwords::for_tests()
        };
        let held = passwords
            .permits
            .clone()
            .acquire_many_owned(MAX_CONCURRENT as u32)
            .await
            .unwrap();
        assert_eq!(passwords.verify("password1", None).await, Err(Busy));
        let error = passwords.hash("password1").await.unwrap_err();
        assert_eq!(error.downcast_ref::<Busy>(), Some(&Busy));
        drop(held);
        assert_eq!(
            passwords.verify("password1", None).await,
            Ok(Verdict::Mismatch)
        );
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
