//! Equal jitter is bounded by the exponential ceiling. Explicit server delays
//! bypass jitter and are never shortened to fit the local retry wait budget.
use std::time::Duration;

pub trait RetryJitter: Send + Sync {
    fn sample(&self) -> u32;
}
pub(super) struct RandomJitter;
impl RetryJitter for RandomJitter {
    fn sample(&self) -> u32 {
        uuid::Uuid::new_v4().as_fields().0
    }
}
pub(super) fn jittered(ceiling: Duration, sample: u32) -> Duration {
    let upper = ceiling.as_nanos();
    let lower = upper / 2;
    // Duration::MAX nanoseconds times u32::MAX fits in u128.
    let nanos = lower + (upper - lower) * u128::from(sample) / u128::from(u32::MAX);
    Duration::new(
        (nanos / 1_000_000_000) as u64,
        (nanos % 1_000_000_000) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_jitter_preserves_bounds_even_at_duration_extremes() {
        for ceiling in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_millis(500),
            Duration::MAX,
        ] {
            let lower = jittered(ceiling, 0);
            assert_eq!(lower.as_nanos(), ceiling.as_nanos() / 2);
            assert_eq!(jittered(ceiling, u32::MAX), ceiling);
            for sample in [1, u32::MAX / 2, u32::MAX - 1] {
                let wait = jittered(ceiling, sample);
                assert!(wait >= lower && wait <= ceiling);
            }
        }
    }
}
