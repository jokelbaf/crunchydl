//! Movie-header timescale parsing.

use super::*;

pub(super) fn parse_mvhd<R: Read + Seek>(reader: &mut R, header: BoxHeader) -> Result<u32> {
    seek_to(reader, header.content_start)?;
    let (version, _) = full_box(reader)?;
    match version {
        0 => {
            seek_to(reader, header.content_start + 12)?;
            read_u32(reader)
        }
        1 => {
            seek_to(reader, header.content_start + 20)?;
            read_u32(reader)
        }
        _ => Err(Error::Unsupported("mvhd version")),
    }
}
