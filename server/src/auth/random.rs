//! The one source of randomness for secrets and identifiers: session and email tokens, public
//! ids, invite codes and slugs (docs/phase1/conventions.md §5.1).

use rand::TryRng;
use rand::rngs::SysRng;

/// Fills `buf` from the operating system's random source.
///
/// # Panics
///
/// When the OS source fails, which does not happen on a working Linux system. The panic
/// becomes a 500 through the `CatchPanicLayer`; a background job logs it.
pub fn fill(buf: &mut [u8]) {
    SysRng
        .try_fill_bytes(buf)
        .expect("the operating system's random source failed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_with_fresh_bytes() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        fill(&mut a);
        fill(&mut b);
        assert_ne!(a, [0u8; 32]);
        assert_ne!(a, b);
    }
}
