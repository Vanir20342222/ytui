//! Allocation-free PCM16 metering of this app's decoder output only.
pub fn pcm16(bytes: &[u8], channels: usize) -> u64 {
    if channels == 0 || channels > 8 || bytes.len() > 256 * 1024 || bytes.len() < channels * 2 {
        return 0;
    }
    let mut sums = [0.0_f64; 2];
    let mut peak = 0.0_f64;
    let mut frames = 0;
    for frame in bytes.chunks_exact(channels * 2) {
        for (channel, sum) in sums.iter_mut().enumerate() {
            let index = channel.min(channels - 1) * 2;
            let value = i16::from_le_bytes([frame[index], frame[index + 1]]) as f64 / 32768.0;
            *sum += value * value;
            peak = peak.max(value.abs());
        }
        frames += 1;
    }
    let normalize = |amplitude: f64| -> u64 {
        if amplitude <= 0.0 {
            0
        } else {
            (((20.0 * amplitude.log10() + 60.0) / 60.0).clamp(0.0, 1.0) * 65535.0).round() as u64
        }
    };
    normalize((sums[0] / frames as f64).sqrt())
        | normalize((sums[1] / frames as f64).sqrt()) << 16
        | normalize(peak) << 32
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silence_stereo_mono_and_malformed_frames_are_bounded() {
        assert_eq!(pcm16(&[0; 32], 2), 0);
        assert_eq!(pcm16(&[1, 2], 0), 0);
        assert_eq!(pcm16(&[1, 2], 9), 0);
        let mut bytes = vec![];
        for _ in 0..128 {
            bytes.extend(16384_i16.to_le_bytes());
            bytes.extend(1638_i16.to_le_bytes());
        }
        let v = pcm16(&bytes, 2);
        assert!((v & 65535) > ((v >> 16) & 65535) + 15000);
        let m = pcm16(&[0, 64, 0, 64], 1);
        assert_eq!(m & 65535, (m >> 16) & 65535);
    }
}
