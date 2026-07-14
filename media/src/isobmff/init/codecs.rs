//! AVC and AAC sample-entry parsing.

use super::*;

pub(super) fn parse_avc1<R: Read + Seek>(
    reader: &mut R,
    entry: BoxHeader,
    builder: &mut TrackBuilder,
) -> Result<()> {
    if entry.payload_len() < 78 {
        return Err(Error::Truncated {
            context: "avc1 sample entry",
        });
    }
    seek_to(reader, entry.content_start + 24)?;
    builder.dimensions = Some((u32::from(read_u16(reader)?), u32::from(read_u16(reader)?)));
    seek_to(reader, entry.content_start + 78)?;
    while let Some(child) = next_box(reader, entry.end)? {
        if &child.kind == b"avcC" {
            builder.codec = Some(Codec::Avc {
                configuration: read_payload(reader, child)?,
            });
        }
        seek_to(reader, child.end)?;
    }
    Ok(())
}

pub(super) fn parse_mp4a<R: Read + Seek>(
    reader: &mut R,
    entry: BoxHeader,
    builder: &mut TrackBuilder,
) -> Result<()> {
    if entry.payload_len() < 28 {
        return Err(Error::Truncated {
            context: "mp4a sample entry",
        });
    }
    seek_to(reader, entry.content_start + 16)?;
    builder.channels = Some(read_u16(reader)?);
    seek_to(reader, entry.content_start + 24)?;
    builder.sample_rate = Some(read_u32(reader)? >> 16);
    seek_to(reader, entry.content_start + 28)?;
    while let Some(child) = next_box(reader, entry.end)? {
        if &child.kind == b"esds" {
            let payload = read_payload(reader, child)?;
            builder.codec = Some(Codec::Aac {
                audio_specific_config: decoder_specific_info(&payload)?,
            });
        }
        seek_to(reader, child.end)?;
    }
    Ok(())
}

fn decoder_specific_info(esds: &[u8]) -> Result<Vec<u8>> {
    if esds.len() < 6 {
        return Err(Error::Truncated { context: "esds" });
    }

    for index in 4..esds.len() {
        if esds[index] != 5 {
            continue;
        }
        if let Ok((length, consumed)) = descriptor_length(&esds[index + 1..]) {
            let start = index + 1 + consumed;
            if let Some(end) = start.checked_add(length)
                && end <= esds.len()
                && length > 0
            {
                return Ok(esds[start..end].to_vec());
            }
        }
    }
    Err(Error::Invalid("esds has no DecoderSpecificInfo"))
}

fn descriptor_length(bytes: &[u8]) -> Result<(usize, usize)> {
    let mut value = 0_usize;
    for (index, byte) in bytes.iter().copied().take(4).enumerate() {
        value = value
            .checked_shl(7)
            .ok_or(Error::Overflow("descriptor length"))?
            | usize::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    Err(Error::Invalid("unterminated descriptor length"))
}
