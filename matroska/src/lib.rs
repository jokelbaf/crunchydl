//! Focused, streaming Matroska writer for AVC, AAC, and ASS tracks.
//!
//! The writer consumes already-demuxed packets, writes through `Write + Seek`,
//! and never decodes, re-encodes, or invokes another executable.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod attachment;
/// Block and block-group encoding primitives.
pub mod block;
mod chapter;
mod cue;
/// Low-level EBML variable-integer and element-id encoding primitives.
pub mod ebml;
mod element;
mod error;
mod muxer;
mod seek;
mod track;

pub use attachment::Attachment;
pub use chapter::Chapter;
pub use error::Error;
pub use muxer::{MuxOptions, Muxer, Packet};
pub use track::{
    AudioSettings, Language, Track, TrackCodec, TrackSettings, TrackType, VideoSettings,
};
