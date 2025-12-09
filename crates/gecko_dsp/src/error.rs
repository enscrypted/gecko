//! DSP Error Types

use thiserror::Error;

/// Errors that can occur during DSP operations
#[derive(Error, Debug)]
pub enum DspError {
    #[error("Invalid band index: {0} (must be 0-9)")]
    InvalidBandIndex(usize),

    #[error("Invalid filter coefficients for frequency {frequency}Hz at sample rate {sample_rate}Hz")]
    InvalidCoefficients { frequency: f32, sample_rate: f32 },

    #[error("Sample rate must be positive, got {0}")]
    InvalidSampleRate(f32),

    #[error("Buffer size mismatch: expected {expected}, got {got}")]
    BufferSizeMismatch { expected: usize, got: usize },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = DspError::InvalidBandIndex(15);
        assert!(err.to_string().contains("15"));

        let err = DspError::InvalidCoefficients {
            frequency: 1000.0,
            sample_rate: 48000.0,
        };
        assert!(err.to_string().contains("1000"));
    }
}
