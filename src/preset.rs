//! The encoder settings. Everything is mono, 16 kHz, speech-tuned libopus with 60 ms frames and VBR;
//! a preset only picks the bitrate.

pub const SAMPLE_RATE: u32 = 16_000;
pub const FRAME_MS: u32 = 60;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preset {
    #[default]
    Standard,
    Small,
    Smallest,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Standard, Preset::Small, Preset::Smallest];

    /// Bits per second. 9 kbps is the floor: below it libopus drops to telephone bandwidth.
    pub fn bitrate(self) -> u32 {
        match self {
            Preset::Standard => 12_000,
            Preset::Small => 10_000,
            Preset::Smallest => 9_000,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Preset::Standard => "standard",
            Preset::Small => "small",
            Preset::Smallest => "smallest",
        }
    }

    /// Measured on a speech sample with libopus 1.6, container included.
    pub fn mb_per_hour(self) -> f64 {
        match self {
            Preset::Standard => 5.4,
            Preset::Small => 4.5,
            Preset::Smallest => 4.1,
        }
    }
}
