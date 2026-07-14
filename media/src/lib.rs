//! Dependency-light, container-neutral media models and fragmented-MP4 parsing.
//!
//! The reader supports the AVC and AAC fragmented-MP4 layouts used by the
//! downloader. Unknown optional boxes are skipped by their declared size and
//! unsupported required layouts fail with a typed error.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod codec;
mod error;
mod isobmff;
mod packet;
mod probe;
mod time;
mod track;

pub use codec::Codec;
pub use error::Error;
pub use isobmff::{FragmentedMp4, PacketIter};
pub use packet::Packet;
pub use probe::Probe;
pub use time::{TimeBase, Timestamp};
pub use track::{Edit, Track, TrackKind};
