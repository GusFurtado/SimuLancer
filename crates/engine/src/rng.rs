//! Random numbers, implemented here rather than taken from a crate so that a
//! seed produces the same results forever, independent of dependency versions.
//!
//! Generator: xoshiro256++. Every random input of the model (a source's
//! arrivals, a server's process times, each failure's uptimes and downtimes)
//! gets its own stream, derived from the experiment seed, the replication
//! number and the input's name. Changing one input therefore doesn't shift
//! the random numbers of the others (common random numbers across scenarios).

use std::f64::consts::TAU;

use crate::spec::Distribution;

#[derive(Debug, Clone)]
pub struct Rng {
    s: [u64; 4],
}

/// SplitMix64 finalizer: a fast, well-mixed 64-bit hash.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// FNV-1a, to turn stream names into numbers.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xCBF2_9CE4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3)
    })
}

impl Rng {
    pub fn from_seed(seed: u64) -> Rng {
        let mut s = [0; 4];
        let mut state = seed;
        for word in &mut s {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            *word = mix(state);
        }
        if s == [0; 4] {
            s[0] = 1; // the one state xoshiro can't leave
        }
        Rng { s }
    }

    /// The stream for one named random input in one replication.
    pub fn stream(seed: u64, replication: u32, name: &str) -> Rng {
        Rng::from_seed(mix(mix(mix(seed) ^ u64::from(replication)) ^ fnv1a(name)))
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }

    /// Uniform in (0, 1]. Never 0, so `ln` is always finite.
    pub fn open01(&mut self) -> f64 {
        1.0 - (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Draws a value in seconds. Inverse-transform sampling wherever there's a
    /// closed form, so each draw uses exactly one number from the stream.
    pub fn sample(&mut self, distribution: &Distribution) -> f64 {
        match *distribution {
            Distribution::Constant(value) => value,
            Distribution::Exponential { mean } => -mean * self.open01().ln(),
            Distribution::Uniform { min, max } => max - (max - min) * self.open01(),
            Distribution::Triangular { min, mode, max } => {
                let u = 1.0 - self.open01(); // [0, 1)
                let split = (mode - min) / (max - min);
                if u < split {
                    min + (u * (max - min) * (mode - min)).sqrt()
                } else {
                    max - ((1.0 - u) * (max - min) * (max - mode)).sqrt()
                }
            }
            Distribution::Lognormal { mean, sd } => {
                let sigma2 = (1.0 + (sd * sd) / (mean * mean)).ln();
                let mu = mean.ln() - sigma2 / 2.0;
                (mu + sigma2.sqrt() * self.standard_normal()).exp()
            }
        }
    }

    /// Box-Muller, using one of the pair. Two numbers from the stream per draw.
    fn standard_normal(&mut self) -> f64 {
        let r = (-2.0 * self.open01().ln()).sqrt();
        r * (TAU * self.open01()).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xoshiro_reference_output() {
        // Reference implementation, state {1, 2, 3, 4}: first output is
        // rotl(1 + 4, 23) + 1.
        let mut rng = Rng { s: [1, 2, 3, 4] };
        assert_eq!(rng.next_u64(), 41_943_041);
    }

    #[test]
    fn streams_are_reproducible_and_distinct() {
        let a = Rng::stream(7, 1, "server/lathe/process").next_u64();
        assert_eq!(a, Rng::stream(7, 1, "server/lathe/process").next_u64());
        assert_ne!(a, Rng::stream(7, 2, "server/lathe/process").next_u64());
        assert_ne!(a, Rng::stream(7, 1, "server/mill/process").next_u64());
        assert_ne!(a, Rng::stream(8, 1, "server/lathe/process").next_u64());
    }

    /// Sample mean and standard deviation.
    fn moments(distribution: Distribution) -> (f64, f64) {
        let mut rng = Rng::from_seed(42);
        let n = 400_000;
        let draws: Vec<f64> = (0..n).map(|_| rng.sample(&distribution)).collect();
        let mean = draws.iter().sum::<f64>() / n as f64;
        let var = draws.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
        (mean, var.sqrt())
    }

    fn assert_close(actual: f64, expected: f64, tolerance: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance * expected.abs(),
            "{what}: got {actual}, expected {expected}"
        );
    }

    #[test]
    fn distribution_moments() {
        // (distribution, mean, sd)
        let cases = [
            (Distribution::Exponential { mean: 60.0 }, 60.0, 60.0),
            (
                Distribution::Uniform {
                    min: 4.0,
                    max: 12.0,
                },
                8.0,
                8.0 / 12f64.sqrt(),
            ),
            (
                Distribution::Triangular {
                    min: 30.0,
                    mode: 48.0,
                    max: 72.0,
                },
                50.0,
                ((30f64 * 30.0 + 48.0 * 48.0 + 72.0 * 72.0
                    - 30.0 * 48.0
                    - 30.0 * 72.0
                    - 48.0 * 72.0)
                    / 18.0)
                    .sqrt(),
            ),
            (
                Distribution::Lognormal {
                    mean: 600.0,
                    sd: 240.0,
                },
                600.0,
                240.0,
            ),
        ];
        for (distribution, mean, sd) in cases {
            let (m, s) = moments(distribution);
            assert_close(m, mean, 0.01, &format!("{distribution:?} mean"));
            assert_close(s, sd, 0.02, &format!("{distribution:?} sd"));
        }
    }

    #[test]
    fn samples_stay_in_range() {
        let mut rng = Rng::from_seed(1);
        let tri = Distribution::Triangular {
            min: 1.0,
            mode: 2.0,
            max: 5.0,
        };
        let uni = Distribution::Uniform { min: 3.0, max: 4.0 };
        for _ in 0..100_000 {
            let t = rng.sample(&tri);
            assert!((1.0..=5.0).contains(&t), "{t}");
            let u = rng.sample(&uni);
            assert!((3.0..=4.0).contains(&u), "{u}");
            assert!(rng.sample(&Distribution::Exponential { mean: 1.0 }) >= 0.0);
        }
    }
}
