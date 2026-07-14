/// Codec configuration extracted from a sample entry.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Codec {
    /// AVC/H.264 with the exact `avcC` payload required as codec private data.
    Avc {
        /// AVC decoder configuration record.
        configuration: Vec<u8>,
    },
    /// AAC with the AudioSpecificConfig extracted from the `esds` descriptor.
    Aac {
        /// MPEG-4 AudioSpecificConfig bytes.
        audio_specific_config: Vec<u8>,
    },
}

impl Codec {
    /// Return the codec-private bytes consumed by a container writer.
    #[must_use]
    pub fn private_data(&self) -> &[u8] {
        match self {
            Self::Avc { configuration } => configuration,
            Self::Aac {
                audio_specific_config,
            } => audio_specific_config,
        }
    }
}
