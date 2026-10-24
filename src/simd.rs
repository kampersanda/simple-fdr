//! 128-bit mask accumulation using baseline NEON or SSE2 instructions.
use clap::ValueEnum;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    Integer,
    Neon,
    Sse2,
}
impl Backend {
    pub fn is_supported(self) -> bool {
        match self {
            Self::Integer => true,
            Self::Neon => cfg!(target_arch = "aarch64"),
            Self::Sse2 => cfg!(target_arch = "x86_64"),
        }
    }
}

/// Accumulate src << shift with zero fill and arbitrary word boundaries.
pub(crate) fn or_shifted(dst: &mut [u64], src: &[u64], shift: usize, backend: Backend) {
    let words = shift / 64;
    let bits = shift % 64;
    let source = |d: usize| {
        d.checked_sub(words)
            .and_then(|s| src.get(s))
            .copied()
            .unwrap_or(0)
    };
    let mut d = 0;
    if backend != Backend::Integer {
        while d + 1 < dst.len() {
            let low = [source(d), source(d + 1)];
            let carry = [if d == 0 { 0 } else { source(d - 1) }, source(d)];
            // SAFETY: Arrays hold two initialized u64s, and dst has two writable
            // words. Loads/stores allow unaligned addresses. These target
            // intrinsics are baseline features on aarch64 and x86_64.
            unsafe {
                #[cfg(target_arch = "aarch64")]
                {
                    use std::arch::aarch64::*;
                    let mut value = vshlq_u64(vld1q_u64(low.as_ptr()), vdupq_n_s64(bits as i64));
                    if bits != 0 {
                        value = vorrq_u64(
                            value,
                            vshlq_u64(vld1q_u64(carry.as_ptr()), vdupq_n_s64(bits as i64 - 64)),
                        );
                    }
                    vst1q_u64(
                        dst.as_mut_ptr().add(d),
                        vorrq_u64(vld1q_u64(dst.as_ptr().add(d)), value),
                    );
                }
                #[cfg(target_arch = "x86_64")]
                {
                    use std::arch::x86_64::*;
                    let mut value = _mm_sll_epi64(
                        _mm_loadu_si128(low.as_ptr().cast()),
                        _mm_cvtsi64_si128(bits as i64),
                    );
                    if bits != 0 {
                        value = _mm_or_si128(
                            value,
                            _mm_srl_epi64(
                                _mm_loadu_si128(carry.as_ptr().cast()),
                                _mm_cvtsi64_si128((64 - bits) as i64),
                            ),
                        );
                    }
                    _mm_storeu_si128(
                        dst.as_mut_ptr().add(d).cast(),
                        _mm_or_si128(_mm_loadu_si128(dst.as_ptr().add(d).cast()), value),
                    );
                }
            }
            d += 2;
        }
    }
    for (index, word) in dst.iter_mut().enumerate().skip(d) {
        *word |= source(index) << bits;
        if bits != 0 && index > 0 {
            *word |= source(index - 1) >> (64 - bits);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vector_shifts_match_independent_bit_oracle() {
        let backend = if cfg!(target_arch = "aarch64") {
            Backend::Neon
        } else {
            Backend::Sse2
        };
        if !backend.is_supported() {
            return;
        }
        for length in 0..=9 {
            let src: Vec<u64> = (0..length)
                .map(|i| 0x91e27c5a30db864fu64.rotate_left(i * 7))
                .collect();
            for output_len in 0..=10 {
                for shift in 0..=520 {
                    let mut expected = vec![0x123456789abcdef0; output_len];
                    for bit in 0..length as usize * 64 {
                        let destination = bit + shift;
                        if destination < output_len * 64 && src[bit / 64] & (1 << (bit % 64)) != 0 {
                            expected[destination / 64] |= 1 << (destination % 64);
                        }
                    }
                    for mode in [Backend::Integer, backend] {
                        let mut actual = vec![0x123456789abcdef0; output_len];
                        or_shifted(&mut actual, &src, shift, mode);
                        assert_eq!(
                            actual, expected,
                            "len={length} out={output_len} shift={shift} mode={mode:?}"
                        );
                    }
                }
            }
        }
    }
}
