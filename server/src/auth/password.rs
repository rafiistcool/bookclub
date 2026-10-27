//! Password hashing with argon2id (docs/phase1/conventions.md §5.4).
//!
//! Wave 1 step 3 adds `hash` and `verify` (spawn_blocking behind the semaphore, legacy PHC
//! verification, rehash detection, the dummy verify for unknown accounts).

use tokio::sync::Semaphore;

/// Concurrent hash or verify operations; legacy hashes use 64 MiB each.
const MAX_CONCURRENT: usize = 4;

/// argon2 parameters plus the semaphore that bounds concurrent hashing. Held in
/// `AppState::passwords`.
pub struct Passwords {
    /// Parameters for new hashes; a stored hash with other parameters is rehashed on login.
    pub params: argon2::Params,
    /// Bounds memory: at most [`MAX_CONCURRENT`] hashes run at once.
    pub permits: Semaphore,
}

impl Passwords {
    /// Production parameters: the argon2 crate default (m = 19456 KiB, t = 2, p = 1, OWASP).
    pub fn new() -> Self {
        Self::with_params(argon2::Params::DEFAULT)
    }

    /// Cheap parameters (m = 8, t = 1, p = 1) so tests do not hash at production cost.
    pub fn for_tests() -> Self {
        let params = argon2::Params::new(8, 1, 1, None).expect("valid argon2 test parameters");
        Self::with_params(params)
    }

    fn with_params(params: argon2::Params) -> Self {
        Self {
            params,
            permits: Semaphore::new(MAX_CONCURRENT),
        }
    }
}

impl Default for Passwords {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
