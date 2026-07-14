use crate::element::{binary, master, uint};
use crate::error::Result;

pub(crate) struct Entry {
    pub(crate) id: u32,
    pub(crate) position: u64,
}

pub(crate) fn encode(entries: &[Entry]) -> Result<Vec<u8>> {
    master(
        0x114d_9b74,
        entries
            .iter()
            .map(|entry| {
                master(
                    0x4dbb,
                    [
                        binary(0x53ab, &crate::ebml::id_bytes(entry.id)?)?,
                        uint(0x53ac, entry.position)?,
                    ],
                )
            })
            .collect::<Result<Vec<_>>>()?,
    )
}
