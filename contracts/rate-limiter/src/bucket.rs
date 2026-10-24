#![no_std]

use soroban_sdk::contracttype;

/// Per-caller token bucket persisted in contract storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenBucket {
    pub tokens: u32,
    pub last_refill: u32,
}

impl TokenBucket {
    pub fn new(capacity: u32, current_ledger: u32) -> Self {
        Self {
            tokens: capacity,
            last_refill: current_ledger,
        }
    }

    /// Refill from ledger-sequence delta, then try to spend `amount` tokens.
    /// Arithmetic is saturating / overflow-safe.
    pub fn consume(
        &mut self,
        amount: u32,
        capacity: u32,
        refill_rate: u32,
        current_ledger: u32,
    ) -> bool {
        self.refill(capacity, refill_rate, current_ledger);
        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }

    pub fn refill(&mut self, capacity: u32, refill_rate: u32, current_ledger: u32) {
        let elapsed = current_ledger.saturating_sub(self.last_refill);
        let added = elapsed.saturating_mul(refill_rate);
        self.tokens = core::cmp::min(capacity, self.tokens.saturating_add(added));
        self.last_refill = current_ledger;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_bucket_is_full() {
        let b = TokenBucket::new(10, 50);
        assert_eq!(b.tokens, 10);
        assert_eq!(b.last_refill, 50);
    }

    #[test]
    fn consume_until_empty() {
        let mut b = TokenBucket::new(3, 1);
        assert!(b.consume(1, 3, 1, 1));
        assert!(b.consume(1, 3, 1, 1));
        assert!(b.consume(1, 3, 1, 1));
        assert!(!b.consume(1, 3, 1, 1));
        assert_eq!(b.tokens, 0);
    }

    #[test]
    fn refill_one_token_per_ledger() {
        let mut b = TokenBucket::new(5, 100);
        assert!(b.consume(5, 5, 1, 100));
        assert!(!b.consume(1, 5, 1, 100));
        assert!(b.consume(1, 5, 1, 101));
        assert_eq!(b.tokens, 0);
    }

    #[test]
    fn refill_does_not_exceed_capacity() {
        let mut b = TokenBucket::new(5, 10);
        b.tokens = 4;
        b.refill(5, 10, 20);
        assert_eq!(b.tokens, 5);
        assert_eq!(b.last_refill, 20);
    }

    #[test]
    fn elapsed_overflow_is_saturating() {
        let mut b = TokenBucket {
            tokens: 0,
            last_refill: u32::MAX,
        };
        b.refill(8, 1, 0);
        assert_eq!(b.tokens, 0);
    }

    #[test]
    fn refill_rate_zero_never_replenishes() {
        let mut b = TokenBucket::new(2, 1);
        assert!(b.consume(2, 2, 0, 1));
        assert!(!b.consume(1, 2, 0, 50));
    }
}
