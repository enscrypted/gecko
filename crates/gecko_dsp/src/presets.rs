//! Built-in EQ Presets

/// Named EQ preset with 10 band gains
pub type Preset = (&'static str, [f32; 10]);

/// List of built-in presets
pub const PRESETS: &[Preset] = &[
    ("Flat", [0.0; 10]),
    ("Bass Boost", [6.0, 5.0, 3.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Treble Boost", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 6.0, 6.0]),
    ("Vocal Clarity", [-2.0, -1.0, 0.0, 2.0, 4.0, 4.0, 3.0, 2.0, 1.0, 0.0]),
    ("Bass Reduce", [-6.0, -4.0, -2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Loudness", [4.0, 3.0, 0.0, -1.0, -1.0, 0.0, 1.0, 2.0, 3.0, 4.0]),
    ("Game (FPS)", [-2.0, -1.0, 0.0, 2.0, 4.0, 6.0, 4.0, 2.0, 0.0, -2.0]), // Emphasize footsteps
    ("Electronic", [4.0, 3.0, 1.0, 0.0, -2.0, -2.0, 0.0, 1.0, 3.0, 4.0]),
];
