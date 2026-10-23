//! Minimal encoding types shared by the standalone GPU probes.
//! Keep rate-control behavior in the production module.
#[path = "../../../../src/media/encoding/rate.rs"]
pub(crate) mod rate;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Codec {
    H264,
}
#[derive(Clone, Copy, Debug)]
pub struct Format {
    pub codec: Codec,
    pub chroma: u8,
    pub depth: u8,
}
impl Format {
    pub const AVC: Self = Self {
        codec: Codec::H264,
        chroma: 1,
        depth: 8,
    };
    pub fn valid(self) -> bool {
        true
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QualityTarget {
    pub bitrate: u32,
    pub fps: u32,
}

#[derive(Clone, Copy)]
pub struct Rate {
    pub target: u32,
    pub peak: u32,
    pub fps: u32,
    pub quality: i32,
    pub quality_target: QualityTarget,
}
pub struct Encoded {
    pub data: Vec<u8>,
    pub keyframe: bool,
    pub timestamp_100ns: i64,
    pub is_new: bool,
    pub timing: Option<()>,
    pub format: Format,
    pub color: (),
}
pub enum Color {
    Sdr,
}
impl Color {
    pub fn space(self, _: Option<()>) {}
}
