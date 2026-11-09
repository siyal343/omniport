use crate::SerialConfig;
use std::time::Duration;

/// Calculates theoretical byte rate: R_bytes = B / (1 + D + P + S) per FR-102
pub fn calculate_byte_rate(cfg: &SerialConfig) -> f64 {
    let start_bits = 1.0;
    let data_bits = cfg.data_bits.clamp(5, 8) as f64;
    let parity_bits = if cfg.parity != 0 { 1.0 } else { 0.0 };
    let stop_bits = cfg.stop_bits.clamp(1, 2) as f64;

    let bits_per_byte = start_bits + data_bits + parity_bits + stop_bits;
    cfg.baud_rate as f64 / bits_per_byte
}

/// Token-Bucket software baud rate throttler for virtual serial ports
#[derive(Debug, Clone)]
pub struct TokenBucketThrottler {
    config: SerialConfig,
    rate_bytes_per_sec: f64,
    capacity: f64,
    tokens: f64,
    last_refill_ns: u64,
    total_bytes_throttled: u64,
}

impl TokenBucketThrottler {
    pub fn new(config: SerialConfig) -> Self {
        let rate = calculate_byte_rate(&config);
        // Default bucket capacity: 50 ms worth of tokens (min 64 bytes)
        let capacity = (rate * 0.05).max(64.0);
        Self {
            config,
            rate_bytes_per_sec: rate,
            capacity,
            tokens: capacity, // Start full
            last_refill_ns: 0,
            total_bytes_throttled: 0,
        }
    }

    pub fn config(&self) -> &SerialConfig {
        &self.config
    }

    pub fn byte_rate(&self) -> f64 {
        self.rate_bytes_per_sec
    }

    pub fn capacity(&self) -> f64 {
        self.capacity
    }

    pub fn update_config(&mut self, new_config: SerialConfig) {
        self.config = new_config;
        self.rate_bytes_per_sec = calculate_byte_rate(&new_config);
        self.capacity = (self.rate_bytes_per_sec * 0.05).max(64.0);
        self.tokens = self.tokens.min(self.capacity);
    }

    /// Refills tokens based on elapsed nanoseconds since last call
    pub fn refill(&mut self, now_ns: u64) {
        if self.last_refill_ns == 0 {
            self.last_refill_ns = now_ns;
            return;
        }
        let elapsed_ns = now_ns.saturating_sub(self.last_refill_ns);
        let added_tokens = (elapsed_ns as f64 / 1_000_000_000.0) * self.rate_bytes_per_sec;
        self.tokens = (self.tokens + added_tokens).min(self.capacity);
        self.last_refill_ns = now_ns;
    }

    /// Consumes tokens for `byte_count` bytes and returns the duration the caller must delay
    pub fn throttle(&mut self, byte_count: usize, now_ns: u64) -> Duration {
        self.refill(now_ns);
        let needed = byte_count as f64;
        self.total_bytes_throttled += byte_count as u64;

        if self.tokens >= needed {
            self.tokens -= needed;
            Duration::ZERO
        } else {
            let deficit = needed - self.tokens;
            self.tokens = 0.0;
            let wait_secs = deficit / self.rate_bytes_per_sec;
            let wait_ns = (wait_secs * 1_000_000_000.0) as u64;
            self.last_refill_ns = now_ns.saturating_add(wait_ns);
            Duration::from_nanos(wait_ns)
        }
    }

    /// Async helper that sleeps for the required throttling duration
    pub async fn throttle_async(&mut self, byte_count: usize) {
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let delay = self.throttle(byte_count, now_ns);
        if delay > Duration::ZERO {
            tokio::time::sleep(delay).await;
        }
    }

    pub fn total_bytes_throttled(&self) -> u64 {
        self.total_bytes_throttled
    }

    pub fn reset_stats(&mut self) {
        self.total_bytes_throttled = 0;
        self.tokens = self.capacity;
        self.last_refill_ns = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_byte_rate_calculation() {
        let cfg_115200_8n1 = SerialConfig {
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };
        assert_eq!(calculate_byte_rate(&cfg_115200_8n1), 11_520.0);

        let cfg_9600_8e2 = SerialConfig {
            baud_rate: 9600,
            data_bits: 8,
            stop_bits: 2,
            parity: 1,
        };
        assert_eq!(calculate_byte_rate(&cfg_9600_8e2), 800.0);

        let cfg_9600_7n1 = SerialConfig {
            baud_rate: 9600,
            data_bits: 7,
            stop_bits: 1,
            parity: 0,
        };
        let expected = 9600.0 / 9.0;
        assert!((calculate_byte_rate(&cfg_9600_7n1) - expected).abs() < 1e-6);
    }

    #[test]
    fn test_token_bucket_throttling_delay() {
        let cfg = SerialConfig {
            baud_rate: 9600,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        };
        let mut throttler = TokenBucketThrottler::new(cfg);

        let cap = throttler.capacity as usize;
        let d0 = throttler.throttle(cap, 1_000_000_000);
        assert_eq!(d0, Duration::ZERO);

        let delay = throttler.throttle(96, 1_000_000_000);
        assert_eq!(delay.as_millis(), 100);

        let delay2 = throttler.throttle(48, 1_100_000_000);
        assert_eq!(delay2.as_millis(), 50);
    }
}
