//! Audio Processor Trait
//!
//! Defines the interface for chainable audio processors.
//! Allows building modular DSP pipelines (EQ -> Compressor -> Limiter).

/// Context passed to processors containing stream metadata
#[derive(Debug, Clone, Copy)]
pub struct ProcessContext {
    pub sample_rate: f32,
    pub channels: usize,
    pub buffer_size: usize,
}

impl ProcessContext {
    pub fn new(sample_rate: f32, channels: usize, buffer_size: usize) -> Self {
        Self {
            sample_rate,
            channels,
            buffer_size,
        }
    }
}

/// Trait for audio processors in the DSP chain
///
/// # Real-time Safety Contract
///
/// Implementors MUST follow these rules in `process()`:
/// - NO heap allocations (no Vec::push, no Box::new, no String)
/// - NO syscalls (no file I/O, no network, no mutex locks)
/// - NO unbounded loops
/// - Constant or O(n) time complexity where n = buffer size
///
/// Violating these rules causes audio dropouts ("glitches").
pub trait AudioProcessor: Send {
    /// Process audio buffer in-place
    ///
    /// Buffer format is interleaved: [L0, R0, L1, R1, ...]
    fn process(&mut self, buffer: &mut [f32], context: &ProcessContext);

    /// Reset internal state (delay lines, envelopes, etc.)
    fn reset(&mut self);

    /// Human-readable name for debugging/UI
    fn name(&self) -> &'static str;

    /// Whether this processor is currently enabled
    fn is_enabled(&self) -> bool {
        true
    }
}

/// A chain of processors applied sequentially
/// Note: This will be used when we implement the full DSP pipeline in the engine
#[allow(dead_code)]
pub struct ProcessorChain {
    processors: Vec<Box<dyn AudioProcessor>>,
    context: ProcessContext,
}

#[allow(dead_code)]
impl ProcessorChain {
    pub fn new(sample_rate: f32, channels: usize, buffer_size: usize) -> Self {
        Self {
            processors: Vec::new(),
            context: ProcessContext::new(sample_rate, channels, buffer_size),
        }
    }

    /// Add a processor to the end of the chain
    ///
    /// Note: This allocates. Only call during setup, not in audio callback.
    pub fn add<P: AudioProcessor + 'static>(&mut self, processor: P) {
        self.processors.push(Box::new(processor));
    }

    /// Process buffer through all enabled processors
    #[inline]
    pub fn process(&mut self, buffer: &mut [f32]) {
        for processor in &mut self.processors {
            if processor.is_enabled() {
                processor.process(buffer, &self.context);
            }
        }
    }

    /// Reset all processors
    pub fn reset(&mut self) {
        for processor in &mut self.processors {
            processor.reset();
        }
    }

    /// Update context (e.g., when buffer size changes)
    pub fn set_context(&mut self, context: ProcessContext) {
        self.context = context;
    }

    /// Get number of processors in chain
    pub fn len(&self) -> usize {
        self.processors.len()
    }

    /// Check if chain is empty
    pub fn is_empty(&self) -> bool {
        self.processors.is_empty()
    }
}

// Implement AudioProcessor for Equalizer so it can be added to chain
impl AudioProcessor for crate::Equalizer {
    fn process(&mut self, buffer: &mut [f32], _context: &ProcessContext) {
        self.process_interleaved(buffer);
    }

    fn reset(&mut self) {
        crate::Equalizer::reset(self);
    }

    fn name(&self) -> &'static str {
        "10-Band Equalizer"
    }

    fn is_enabled(&self) -> bool {
        self.config().enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Equalizer;

    /// Test processor that just inverts audio (for testing chain)
    struct InvertProcessor;

    impl AudioProcessor for InvertProcessor {
        fn process(&mut self, buffer: &mut [f32], _context: &ProcessContext) {
            for sample in buffer.iter_mut() {
                *sample = -*sample;
            }
        }

        fn reset(&mut self) {}

        fn name(&self) -> &'static str {
            "Inverter"
        }
    }

    #[test]
    fn test_empty_chain() {
        let mut chain = ProcessorChain::new(48000.0, 2, 512);
        assert!(chain.is_empty());

        let mut buffer = vec![0.5, -0.5];
        chain.process(&mut buffer);

        // Empty chain should not modify buffer
        assert_eq!(buffer[0], 0.5);
        assert_eq!(buffer[1], -0.5);
    }

    #[test]
    fn test_single_processor() {
        let mut chain = ProcessorChain::new(48000.0, 2, 512);
        chain.add(InvertProcessor);

        let mut buffer = vec![0.5, -0.5];
        chain.process(&mut buffer);

        assert_eq!(buffer[0], -0.5);
        assert_eq!(buffer[1], 0.5);
    }

    #[test]
    fn test_processor_chain_order() {
        let mut chain = ProcessorChain::new(48000.0, 2, 512);

        // Two inverters should cancel out
        chain.add(InvertProcessor);
        chain.add(InvertProcessor);

        let mut buffer = vec![0.5, -0.5];
        chain.process(&mut buffer);

        assert_eq!(buffer[0], 0.5);
        assert_eq!(buffer[1], -0.5);
    }

    #[test]
    fn test_eq_in_chain() {
        let mut chain = ProcessorChain::new(48000.0, 2, 512);
        chain.add(Equalizer::new(48000.0));

        assert_eq!(chain.len(), 1);

        let mut buffer = vec![0.5, -0.5, 0.3, -0.3];
        chain.process(&mut buffer);

        // Should process without panic
        for sample in &buffer {
            assert!(sample.is_finite());
        }
    }

    #[test]
    fn test_chain_reset() {
        let mut chain = ProcessorChain::new(48000.0, 2, 512);
        chain.add(Equalizer::new(48000.0));

        // Process some data
        let mut buffer = vec![0.5; 100];
        chain.process(&mut buffer);

        // Reset should not panic
        chain.reset();
    }

    #[test]
    fn test_process_context() {
        let ctx = ProcessContext::new(48000.0, 2, 512);
        assert_eq!(ctx.sample_rate, 48000.0);
        assert_eq!(ctx.channels, 2);
        assert_eq!(ctx.buffer_size, 512);
    }
}
