//! FFT Spectrum Analyzer
//!
//! Provides real-time FFT analysis for visualization.
//! Uses a ring buffer to accumulate samples and computes FFT
//! at a configurable rate (default ~30fps) without blocking the audio thread.
//!
//! # Architecture
//!
//! The analyzer uses a lock-free ring buffer for the audio thread to write samples,
//! and a separate analysis that can be polled from the UI thread.

use rustfft::{num_complex::Complex, FftPlanner};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// FFT size (must be power of 2)
/// 2048 samples at 48kHz = ~42ms window, ~23Hz resolution
pub const FFT_SIZE: usize = 2048;

/// Number of frequency bins to output (reduced for efficient UI rendering)
/// These are logarithmically spaced to match human hearing
pub const NUM_BINS: usize = 32;

/// Hann window coefficients (pre-computed for efficiency)
/// Hann window reduces spectral leakage in FFT analysis
fn hann_window(n: usize, size: usize) -> f32 {
    0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / (size - 1) as f32).cos())
}

/// Pre-computed Hann window lookup table
struct HannWindow {
    coeffs: [f32; FFT_SIZE],
}

impl HannWindow {
    fn new() -> Self {
        let coeffs = core::array::from_fn(|i| hann_window(i, FFT_SIZE));
        Self { coeffs }
    }

    #[inline]
    fn apply(&self, sample: f32, index: usize) -> f32 {
        sample * self.coeffs[index]
    }
}

/// Spectrum analyzer that computes FFT magnitude spectrum
///
/// Thread-safe design:
/// - Audio thread writes samples via `push_sample()`
/// - UI thread reads spectrum via `get_spectrum()`
/// - Internal atomic flags coordinate when new data is ready
pub struct SpectrumAnalyzer {
    /// Ring buffer for incoming samples (mono, mixed from stereo)
    sample_buffer: Vec<f32>,
    /// Current write position in ring buffer
    write_pos: AtomicU32,
    /// Number of samples written since last FFT
    samples_since_fft: AtomicU32,
    /// Samples needed before computing next FFT (~30fps at 48kHz)
    samples_per_fft: u32,
    /// Flag indicating new spectrum data is available
    spectrum_ready: AtomicBool,
    /// Output spectrum (magnitude in dB, 0.0 to 1.0 normalized)
    spectrum: parking_lot::RwLock<[f32; NUM_BINS]>,
    /// Smoothed spectrum for display (with decay)
    smoothed_spectrum: parking_lot::RwLock<[f32; NUM_BINS]>,
    /// Hann window coefficients
    window: HannWindow,
    /// FFT planner (reused for efficiency)
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
    /// Working buffer for FFT input
    fft_input: parking_lot::Mutex<Vec<Complex<f32>>>,
    /// Working buffer for FFT output
    fft_output: parking_lot::Mutex<Vec<Complex<f32>>>,
}

/// Smoothing factor for spectrum decay (0.0 = instant, 1.0 = no decay)
/// 0.7 gives nice smooth falloff at ~30fps
const SPECTRUM_DECAY: f32 = 0.7;

/// Attack factor for spectrum rise (higher = faster response to new peaks)
/// 0.5 gives responsive feel while still smoothing transients
const SPECTRUM_ATTACK: f32 = 0.5;

impl SpectrumAnalyzer {
    /// Create a new spectrum analyzer
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `fps` - Target update rate for spectrum (default 30)
    pub fn new(sample_rate: f32, fps: u32) -> Self {
        let samples_per_fft = (sample_rate / fps as f32) as u32;

        // Create FFT planner
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);

        Self {
            sample_buffer: vec![0.0; FFT_SIZE],
            write_pos: AtomicU32::new(0),
            samples_since_fft: AtomicU32::new(0),
            samples_per_fft,
            spectrum_ready: AtomicBool::new(false),
            spectrum: parking_lot::RwLock::new([0.0; NUM_BINS]),
            smoothed_spectrum: parking_lot::RwLock::new([0.0; NUM_BINS]),
            window: HannWindow::new(),
            fft,
            fft_input: parking_lot::Mutex::new(vec![Complex::new(0.0, 0.0); FFT_SIZE]),
            fft_output: parking_lot::Mutex::new(vec![Complex::new(0.0, 0.0); FFT_SIZE]),
        }
    }

    /// Push a stereo sample pair to the analyzer
    ///
    /// # Real-time Safety
    /// This function is designed for audio callbacks:
    /// - No allocations
    /// - No locks (uses atomic operations)
    /// - O(1) time complexity
    #[inline]
    pub fn push_sample(&self, left: f32, right: f32) {
        // Mix to mono (average of L+R)
        let mono = (left + right) * 0.5;

        // Write to ring buffer
        let pos = self.write_pos.load(Ordering::Relaxed) as usize;
        // Safety: We only write, and pos is always < FFT_SIZE due to modulo
        // This is technically a race but acceptable for visualization
        unsafe {
            let ptr = self.sample_buffer.as_ptr() as *mut f32;
            *ptr.add(pos) = mono;
        }

        // Advance write position (wrap around)
        let next_pos = ((pos + 1) % FFT_SIZE) as u32;
        self.write_pos.store(next_pos, Ordering::Relaxed);

        // Increment sample counter
        let count = self.samples_since_fft.fetch_add(1, Ordering::Relaxed) + 1;

        // Signal that we have enough samples for a new FFT
        // Note: We don't reset samples_since_fft here - update() handles that
        // This prevents race conditions where update() clears spectrum_ready
        // but samples_since_fft was already reset, causing dropped frames
        if count >= self.samples_per_fft {
            self.spectrum_ready.store(true, Ordering::Release);
        }
    }

    /// Check if new spectrum data is available and compute it if so
    ///
    /// Call this from the UI thread at your desired frame rate.
    /// Returns true if spectrum was updated.
    pub fn update(&self) -> bool {
        let was_ready = self.spectrum_ready.swap(false, Ordering::Acquire);
        if !was_ready {
            return false;
        }

        // Reset the sample counter now that we're processing
        // This ensures we don't skip frames due to race conditions
        self.samples_since_fft.store(0, Ordering::Relaxed);

        // Copy samples to FFT input with windowing
        let mut input = self.fft_input.lock();
        let mut output = self.fft_output.lock();

        let read_pos = self.write_pos.load(Ordering::Relaxed) as usize;
        for i in 0..FFT_SIZE {
            // Read from ring buffer in correct order (oldest first)
            let buf_idx = (read_pos + i) % FFT_SIZE;
            let sample = self.sample_buffer[buf_idx];
            let windowed = self.window.apply(sample, i);
            input[i] = Complex::new(windowed, 0.0);
        }

        // Compute FFT
        output.copy_from_slice(&input);
        self.fft.process(&mut output);

        // Convert to magnitude spectrum with logarithmic frequency bins
        let mut spectrum = self.spectrum.write();
        compute_log_spectrum(&output, &mut spectrum);

        // Apply smoothing to the spectrum for nicer visualization
        // Uses asymmetric attack/decay for snappy response but smooth falloff
        let mut smoothed = self.smoothed_spectrum.write();
        for i in 0..NUM_BINS {
            let raw = spectrum[i];
            let current = smoothed[i];

            if raw > current {
                // Attack: fast response to new peaks
                smoothed[i] = current + (raw - current) * SPECTRUM_ATTACK;
            } else {
                // Decay: smooth falloff
                smoothed[i] = current * SPECTRUM_DECAY + raw * (1.0 - SPECTRUM_DECAY);
            }
        }

        true
    }

    /// Get the current spectrum data (smoothed for display)
    ///
    /// Returns array of NUM_BINS values, each 0.0 to 1.0 representing
    /// magnitude in that frequency range (logarithmically spaced).
    /// Values are smoothed with attack/decay for nice visualization.
    pub fn get_spectrum(&self) -> [f32; NUM_BINS] {
        *self.smoothed_spectrum.read()
    }

    /// Get the raw (unsmoothed) spectrum data
    ///
    /// Use this if you need instantaneous FFT values without smoothing.
    pub fn get_raw_spectrum(&self) -> [f32; NUM_BINS] {
        *self.spectrum.read()
    }

    /// Reset the analyzer state
    pub fn reset(&self) {
        self.write_pos.store(0, Ordering::Relaxed);
        self.samples_since_fft.store(0, Ordering::Relaxed);
        self.spectrum_ready.store(false, Ordering::Relaxed);
        *self.spectrum.write() = [0.0; NUM_BINS];
        *self.smoothed_spectrum.write() = [0.0; NUM_BINS];
    }
}

// Implement Send + Sync manually since we use interior mutability safely
unsafe impl Send for SpectrumAnalyzer {}
unsafe impl Sync for SpectrumAnalyzer {}

/// Convert FFT output to logarithmically-spaced magnitude bins
///
/// Maps the linear FFT bins to logarithmic frequency bands that
/// better match human perception of pitch.
fn compute_log_spectrum(fft_output: &[Complex<f32>], spectrum: &mut [f32; NUM_BINS]) {
    // Only use first half of FFT (positive frequencies)
    let nyquist = FFT_SIZE / 2;

    // Frequency range we care about (roughly 20Hz to 20kHz at 48kHz sample rate)
    // At 48kHz: bin 0 = 0Hz, bin 1 = 23.4Hz, bin 1024 = 24kHz
    let min_bin = 1; // Skip DC
    let max_bin = nyquist;

    // Logarithmically space the output bins
    let log_min = (min_bin as f32).ln();
    let log_max = (max_bin as f32).ln();
    let log_step = (log_max - log_min) / NUM_BINS as f32;

    // FFT normalization factor:
    // For a 2048-point FFT with Hann window, a full-scale sine at one bin
    // would have magnitude ≈ FFT_SIZE * 0.5 (due to Hann window gain).
    // We want typical audio (-20dB to 0dB peaks) to show nice meter range.
    // Reference magnitude = FFT_SIZE / 4 gives good visual results.
    let reference_magnitude = (FFT_SIZE as f32) / 4.0;

    for (i, spectrum_bin) in spectrum.iter_mut().enumerate() {
        // Calculate the range of FFT bins for this output bin
        let log_start = log_min + i as f32 * log_step;
        let log_end = log_min + (i + 1) as f32 * log_step;
        let bin_start = log_start.exp() as usize;
        let bin_end = (log_end.exp() as usize).min(max_bin);

        // Average the magnitudes in this range
        // Rust pattern: use iterator with filter/map instead of manual loop
        let end_idx = (bin_end + 1).min(nyquist);
        let (sum, count): (f32, usize) = fft_output[bin_start..end_idx]
            .iter()
            .map(|c| c.norm())
            .fold((0.0, 0), |(s, c), mag| (s + mag, c + 1));

        let avg_mag = if count > 0 { sum / count as f32 } else { 0.0 };

        // Normalize by reference magnitude and convert to dB
        // This gives us roughly 0dB when signal is at full scale
        let normalized_mag = avg_mag / reference_magnitude;
        let db = 20.0 * (normalized_mag.max(1e-10)).log10();

        // Map -60dB to 0dB range to 0.0 to 1.0
        // -60dB is essentially silence, 0dB is full scale
        let normalized = ((db + 60.0) / 60.0).clamp(0.0, 1.0);

        *spectrum_bin = normalized;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyzer_creation() {
        let analyzer = SpectrumAnalyzer::new(48000.0, 30);
        let spectrum = analyzer.get_spectrum();
        // Should start with all zeros
        for bin in spectrum {
            assert_eq!(bin, 0.0);
        }
    }

    #[test]
    fn test_push_samples() {
        let analyzer = SpectrumAnalyzer::new(48000.0, 30);

        // Push some samples
        for i in 0..1000 {
            let t = i as f32 / 48000.0;
            let sample = (2.0 * std::f32::consts::PI * 1000.0 * t).sin();
            analyzer.push_sample(sample, sample);
        }

        // Should not crash
    }

    #[test]
    fn test_spectrum_update() {
        let analyzer = SpectrumAnalyzer::new(48000.0, 30);

        // Push enough samples to trigger an update
        let samples_needed = (48000.0 / 30.0) as usize + FFT_SIZE;
        for i in 0..samples_needed {
            let t = i as f32 / 48000.0;
            // 1kHz sine wave
            let sample = (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * 0.5;
            analyzer.push_sample(sample, sample);
        }

        // Update should return true
        let updated = analyzer.update();
        assert!(updated, "Spectrum should have been updated");

        // Spectrum should have some non-zero values
        let spectrum = analyzer.get_spectrum();
        let has_signal = spectrum.iter().any(|&v| v > 0.01);
        assert!(has_signal, "Spectrum should show signal presence");
    }

    #[test]
    fn test_reset() {
        let analyzer = SpectrumAnalyzer::new(48000.0, 30);

        // Push samples and update
        for _ in 0..2000 {
            analyzer.push_sample(0.5, 0.5);
        }
        analyzer.update();

        // Reset
        analyzer.reset();

        // Spectrum should be zeros
        let spectrum = analyzer.get_spectrum();
        for bin in spectrum {
            assert_eq!(bin, 0.0);
        }
    }

    #[test]
    fn test_hann_window() {
        // Hann window should be 0 at edges and 1 at center
        let w = HannWindow::new();
        assert!(w.coeffs[0] < 0.01, "Window should be ~0 at start");
        assert!(w.coeffs[FFT_SIZE - 1] < 0.01, "Window should be ~0 at end");
        assert!(
            (w.coeffs[FFT_SIZE / 2] - 1.0).abs() < 0.01,
            "Window should be ~1 at center"
        );
    }
}
