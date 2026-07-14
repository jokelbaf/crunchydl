use crate::element::{master, uint};
use crate::error::Result;

#[derive(Clone, Copy)]
pub(crate) struct Cue {
    pub(crate) time_ms: u64,
    pub(crate) track: u64,
    pub(crate) cluster_position: u64,
}

pub(crate) fn encode(cues: &[Cue]) -> Result<Vec<u8>> {
    master(
        0x1c53_bb6b,
        cues.iter()
            .map(|cue| {
                master(
                    0xbb,
                    [
                        uint(0xb3, cue.time_ms)?,
                        master(
                            0xb7,
                            [uint(0xf7, cue.track)?, uint(0xf1, cue.cluster_position)?],
                        )?,
                    ],
                )
            })
            .collect::<Result<Vec<_>>>()?,
    )
}
