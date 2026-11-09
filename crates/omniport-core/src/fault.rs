use crate::{FrameFlags, SerialEvent};
use bytes::BytesMut;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// High-performance 64-bit seedable PRNG (SplitMix64)
#[derive(Debug, Clone)]
pub struct FastRng {
    state: u64,
}

impl FastRng {
    pub const fn new(seed: u64) -> Self {
        let s = if seed == 0 { 0x853c49e6748fea9b } else { seed };
        Self { state: s }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    /// Uniform float in the open interval (0.0, 1.0)
    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        let v = self.next_u64() >> 11;
        ((v as f64) + 1.0) / (9007199254740992.0 + 2.0)
    }

    /// Box-Muller transform producing standard normal variate Z ~ N(0, 1)
    pub fn next_gaussian(&mut self) -> f64 {
        let u1 = self.next_f64();
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

impl Default for FastRng {
    fn default() -> Self {
        Self::new(0x1234_5678_9ABC_DEF0)
    }
}

/// Gilbert-Elliott 2-state Markov burst error model configuration
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GilbertElliottConfig {
    /// Transition probability from Good to Bad state (P_GB)
    pub p_gb: f64,
    /// Transition probability from Bad to Good state (P_BG)
    pub p_bg: f64,
    /// Bit error rate in Good state (typically 0.0)
    pub p_g: f64,
    /// Bit error rate in Bad state (typically 0.5)
    pub p_b: f64,
}

impl Default for GilbertElliottConfig {
    fn default() -> Self {
        Self {
            p_gb: 0.01,
            p_bg: 0.1,
            p_g: 0.0,
            p_b: 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GilbertElliottState {
    Good,
    Bad,
}

/// Channel fault injection configuration for timing and corruption simulation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaultConfig {
    /// Constant propagation delay
    pub fixed_delay: Duration,
    /// Mean additional delay for Gaussian jitter
    pub jitter_mean: Duration,
    /// Standard deviation for Box-Muller Gaussian jitter
    pub jitter_std_dev: Duration,
    /// Edge throughput bandwidth limit in bytes per second
    pub rate_limit_bps: Option<u64>,
    /// Disconnect status: all frames dropped on this edge when true
    pub disconnected: bool,
    /// Uniform bit error rate (probability of bit flip, 0.0 to 1.0)
    pub bit_error_rate: f64,
    /// Gilbert-Elliott burst error model
    pub gilbert_elliott: Option<GilbertElliottConfig>,
    /// Targeted N-th frame CRC corruption (e.g. Some(10) corrupts every 10th frame)
    pub corrupt_nth_crc: Option<u64>,
}

impl Default for FaultConfig {
    fn default() -> Self {
        Self {
            fixed_delay: Duration::ZERO,
            jitter_mean: Duration::ZERO,
            jitter_std_dev: Duration::ZERO,
            rate_limit_bps: None,
            disconnected: false,
            bit_error_rate: 0.0,
            gilbert_elliott: None,
            corrupt_nth_crc: None,
        }
    }
}

/// Result of fault injection on a single frame
#[derive(Debug, Clone)]
pub struct FaultOutput {
    pub event: SerialEvent,
    pub delay: Duration,
}

/// Stateful channel fault injector
pub struct FaultInjector {
    pub config: FaultConfig,
    rng: FastRng,
    ge_state: GilbertElliottState,
    frame_count: u64,
    next_avail_ns: u64,
}

impl FaultInjector {
    pub fn new(config: FaultConfig) -> Self {
        Self {
            config,
            rng: FastRng::default(),
            ge_state: GilbertElliottState::Good,
            frame_count: 0,
            next_avail_ns: 0,
        }
    }

    pub fn with_seed(config: FaultConfig, seed: u64) -> Self {
        Self {
            config,
            rng: FastRng::new(seed),
            ge_state: GilbertElliottState::Good,
            frame_count: 0,
            next_avail_ns: 0,
        }
    }

    /// Evaluates fault injection for an incoming event. Returns None if dropped (disconnected).
    pub fn inject(&mut self, mut event: SerialEvent, now_ns: u64) -> Option<FaultOutput> {
        if self.config.disconnected {
            return None;
        }

        self.frame_count = self.frame_count.saturating_add(1);

        // 1. Calculate timing delay: Fixed + Box-Muller Jitter + Serialization Rate Limit
        let mut delay = self.config.fixed_delay;

        if self.config.jitter_std_dev > Duration::ZERO {
            let z = self.rng.next_gaussian();
            let mean_ns = self.config.jitter_mean.as_nanos() as f64;
            let std_ns = self.config.jitter_std_dev.as_nanos() as f64;
            let jitter_ns = mean_ns + z * std_ns;
            if jitter_ns > 0.0 {
                delay += Duration::from_nanos(jitter_ns as u64);
            }
        }

        if let Some(rate_bps) = self.config.rate_limit_bps {
            let payload_len = event.payload.len() as u64;
            if let Some(ser_ns) = payload_len
                .saturating_mul(1_000_000_000)
                .checked_div(rate_bps)
            {
                let scheduled_start = self.next_avail_ns.max(now_ns);
                let rate_delay_ns = scheduled_start.saturating_sub(now_ns);
                self.next_avail_ns = scheduled_start.saturating_add(ser_ns);
                delay += Duration::from_nanos(rate_delay_ns);
            }
        }

        // 2. Corruption faults: Zero-copy preservation unless mutation occurs
        let mut mutated = false;
        let mut mutated_payload = BytesMut::new();

        // Check Targeted N-th Frame CRC Corruption
        let mut should_corrupt_crc = false;
        if let Some(nth) = self.config.corrupt_nth_crc
            && nth > 0
            && self.frame_count.is_multiple_of(nth)
        {
            should_corrupt_crc = true;
        }

        if should_corrupt_crc && !event.payload.is_empty() {
            if !mutated {
                mutated_payload.extend_from_slice(&event.payload);
                mutated = true;
            }
            let last_idx = mutated_payload.len() - 1;
            mutated_payload[last_idx] ^= 0xFF;
            event.flags |= FrameFlags::ERR_CRC | FrameFlags::FAULT_INJECT;
        }

        // Check Uniform Bit Error Rate (BER)
        if self.config.bit_error_rate > 0.0 && !event.payload.is_empty() {
            let total_bits = event.payload.len() * 8;
            for bit_idx in 0..total_bits {
                if self.rng.next_f64() < self.config.bit_error_rate {
                    if !mutated {
                        mutated_payload.extend_from_slice(&event.payload);
                        mutated = true;
                    }
                    let byte_idx = bit_idx / 8;
                    let bit_offset = bit_idx % 8;
                    mutated_payload[byte_idx] ^= 1 << bit_offset;
                    event.flags |= FrameFlags::FAULT_INJECT;
                }
            }
        }

        // Check Gilbert-Elliott Burst Errors
        if let Some(ge) = self.config.gilbert_elliott
            && !event.payload.is_empty()
        {
            let total_bits = event.payload.len() * 8;
            for bit_idx in 0..total_bits {
                // State transition
                match self.ge_state {
                    GilbertElliottState::Good => {
                        if self.rng.next_f64() < ge.p_gb {
                            self.ge_state = GilbertElliottState::Bad;
                        }
                    }
                    GilbertElliottState::Bad => {
                        if self.rng.next_f64() < ge.p_bg {
                            self.ge_state = GilbertElliottState::Good;
                        }
                    }
                }

                // Error generation in current state
                let p_err = match self.ge_state {
                    GilbertElliottState::Good => ge.p_g,
                    GilbertElliottState::Bad => ge.p_b,
                };

                if p_err > 0.0 && self.rng.next_f64() < p_err {
                    if !mutated {
                        mutated_payload.extend_from_slice(&event.payload);
                        mutated = true;
                    }
                    let byte_idx = bit_idx / 8;
                    let bit_offset = bit_idx % 8;
                    mutated_payload[byte_idx] ^= 1 << bit_offset;
                    event.flags |= FrameFlags::FAULT_INJECT;
                }
            }
        }

        if mutated {
            event.payload = mutated_payload.freeze();
        }

        Some(FaultOutput { event, delay })
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    pub fn reset(&mut self) {
        self.frame_count = 0;
        self.next_avail_ns = 0;
        self.ge_state = GilbertElliottState::Good;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PinState;
    use bytes::Bytes;

    fn make_test_event(payload: &[u8]) -> SerialEvent {
        SerialEvent {
            timestamp_ns: 1_000_000,
            source_id: 1,
            flags: FrameFlags::empty(),
            pins: PinState::empty(),
            payload: Bytes::copy_from_slice(payload),
        }
    }

    #[test]
    fn test_disconnect_drops_events() {
        let mut injector = FaultInjector::new(FaultConfig {
            disconnected: true,
            ..Default::default()
        });
        assert!(
            injector
                .inject(make_test_event(b"test"), 1_000_000)
                .is_none()
        );
    }

    #[test]
    fn test_zero_fault_is_zero_copy() {
        let mut injector = FaultInjector::new(FaultConfig::default());
        let original = make_test_event(b"hello world");
        let output = injector.inject(original.clone(), 1_000_000).unwrap();
        assert_eq!(output.delay, Duration::ZERO);
        assert_eq!(output.event.payload, original.payload);
        assert_eq!(output.event.flags, FrameFlags::empty());
        // Invariant: underlying buffer pointer is unchanged (zero-copy)
        assert_eq!(output.event.payload.as_ptr(), original.payload.as_ptr());
    }

    #[test]
    fn test_box_muller_gaussian_jitter_statistics() {
        let mean = Duration::from_millis(10);
        let std_dev = Duration::from_millis(2);

        let mut injector = FaultInjector::with_seed(
            FaultConfig {
                fixed_delay: Duration::ZERO,
                jitter_mean: mean,
                jitter_std_dev: std_dev,
                ..Default::default()
            },
            42,
        );

        let n = 10_000;
        let mut sum_ms = 0.0;
        let mut samples = Vec::with_capacity(n);

        for _ in 0..n {
            let out = injector.inject(make_test_event(b"x"), 1_000_000).unwrap();
            let ms = out.delay.as_secs_f64() * 1000.0;
            sum_ms += ms;
            samples.push(ms);
        }

        let sample_mean = sum_ms / n as f64;
        let variance = samples
            .iter()
            .map(|&x| (x - sample_mean).powi(2))
            .sum::<f64>()
            / n as f64;
        let sample_std = variance.sqrt();

        // Statistical tolerance: within 0.1 ms of target mean (10 ms) and target std (2 ms)
        assert!(
            (sample_mean - 10.0).abs() < 0.1,
            "Sample mean {} out of tolerance",
            sample_mean
        );
        assert!(
            (sample_std - 2.0).abs() < 0.1,
            "Sample std {} out of tolerance",
            sample_std
        );
    }

    #[test]
    fn test_uniform_ber_statistics() {
        let ber = 0.02; // 2% bit error rate
        let mut injector = FaultInjector::with_seed(
            FaultConfig {
                bit_error_rate: ber,
                ..Default::default()
            },
            1337,
        );

        let payload_len = 50; // 400 bits per frame
        let frames = 250; // 100,000 total bits
        let original_payload = vec![0xAA; payload_len];

        let mut flipped_bits = 0;
        for _ in 0..frames {
            let out = injector
                .inject(make_test_event(&original_payload), 1_000_000)
                .unwrap();
            for (orig, mutated) in original_payload.iter().zip(out.event.payload.iter()) {
                flipped_bits += (orig ^ mutated).count_ones() as usize;
            }
        }

        let total_bits = payload_len * frames * 8;
        let measured_ber = flipped_bits as f64 / total_bits as f64;

        // 3-sigma tolerance for 100,000 Bernoulli trials with p=0.02 is ~0.0013
        assert!(
            (measured_ber - ber).abs() < 0.003,
            "Measured BER {} deviated too much from {}",
            measured_ber,
            ber
        );
    }

    #[test]
    fn test_targeted_nth_crc_corruption() {
        let mut injector = FaultInjector::new(FaultConfig {
            corrupt_nth_crc: Some(5),
            ..Default::default()
        });

        for i in 1..=20 {
            let out = injector
                .inject(make_test_event(b"DATA_1234"), 1_000_000)
                .unwrap();
            if i % 5 == 0 {
                assert!(
                    out.event
                        .flags
                        .contains(FrameFlags::ERR_CRC | FrameFlags::FAULT_INJECT),
                    "Frame {} should have CRC corruption flags",
                    i
                );
                assert_ne!(&out.event.payload[..], b"DATA_1234");
            } else {
                assert_eq!(out.event.flags, FrameFlags::empty());
                assert_eq!(&out.event.payload[..], b"DATA_1234");
            }
        }
    }
}
