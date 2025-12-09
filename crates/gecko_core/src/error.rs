//! Engine Error Types

use thiserror::Error;

/// Errors that can occur in the audio engine
#[derive(Error, Debug)]
pub enum EngineError {
    #[error("No audio devices found")]
    NoDevicesFound,

    #[error("Device not found: {0}")]
    DeviceNotFound(String),

    #[error("Failed to build audio stream: {0}")]
    StreamBuildError(String),

    #[error("Failed to play audio stream: {0}")]
    StreamPlayError(String),

    #[error("Stream configuration error: {0}")]
    ConfigError(String),

    #[error("Engine already running")]
    AlreadyRunning,

    #[error("Engine not running")]
    NotRunning,

    #[error("Ring buffer overflow - audio thread can't keep up")]
    BufferOverflow,

    #[error("Ring buffer underflow - not enough data available")]
    BufferUnderflow,

    #[error("DSP error: {0}")]
    DspError(#[from] gecko_dsp::DspError),

    #[error("Platform error: {0}")]
    PlatformError(#[from] gecko_platform::PlatformError),

    #[error("Channel send error - receiver dropped")]
    ChannelSendError,

    #[error("Channel receive error - sender dropped")]
    ChannelRecvError,
}

/// Result type alias for engine operations
pub type EngineResult<T> = Result<T, EngineError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = EngineError::NoDevicesFound;
        assert!(err.to_string().contains("No audio devices"));

        let err = EngineError::DeviceNotFound("Test Device".into());
        assert!(err.to_string().contains("Test Device"));
    }

    #[test]
    fn test_error_from_dsp() {
        let dsp_err = gecko_dsp::DspError::InvalidBandIndex(10);
        let engine_err: EngineError = dsp_err.into();
        assert!(matches!(engine_err, EngineError::DspError(_)));
    }
}
