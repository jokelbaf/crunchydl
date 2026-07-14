use std::io::{Read, Seek, SeekFrom};

use crate::error::{Error, Result};

pub(crate) const MAX_BOX_BUFFER: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_SAMPLES_PER_FRAGMENT: u32 = 1_000_000;

#[derive(Clone, Copy, Debug)]
pub(crate) struct BoxHeader {
    pub(crate) start: u64,
    pub(crate) content_start: u64,
    pub(crate) end: u64,
    pub(crate) kind: [u8; 4],
}

impl BoxHeader {
    pub(crate) fn payload_len(self) -> u64 {
        self.end - self.content_start
    }
}

pub(crate) fn file_end<R: Seek>(reader: &mut R) -> Result<u64> {
    let position = reader.stream_position()?;
    let end = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(position))?;
    Ok(end)
}

pub(crate) fn next_box<R: Read + Seek>(
    reader: &mut R,
    parent_end: u64,
) -> Result<Option<BoxHeader>> {
    let start = reader.stream_position()?;
    if start == parent_end {
        return Ok(None);
    }
    if start > parent_end || parent_end - start < 8 {
        return Err(Error::Truncated {
            context: "box header",
        });
    }
    let size32 = read_u32(reader)?;
    let kind = read_array::<4, _>(reader)?;
    let (size, header_len) = match size32 {
        0 => (parent_end - start, 8_u64),
        1 => (read_u64(reader)?, 16_u64),
        value => (u64::from(value), 8_u64),
    };
    if size < header_len {
        return Err(Error::Invalid("box size is smaller than its header"));
    }
    let end = start.checked_add(size).ok_or(Error::Overflow("box end"))?;
    if end > parent_end {
        return Err(Error::Truncated {
            context: "box payload",
        });
    }
    Ok(Some(BoxHeader {
        start,
        content_start: start + header_len,
        end,
        kind,
    }))
}

pub(crate) fn seek_to<R: Seek>(reader: &mut R, position: u64) -> Result<()> {
    reader.seek(SeekFrom::Start(position))?;
    Ok(())
}

pub(crate) fn read_payload<R: Read + Seek>(reader: &mut R, header: BoxHeader) -> Result<Vec<u8>> {
    if header.payload_len() > MAX_BOX_BUFFER {
        return Err(Error::Unsupported(
            "codec or metadata box exceeds the bounded buffer limit",
        ));
    }
    seek_to(reader, header.content_start)?;
    let len = usize::try_from(header.payload_len()).map_err(|_| Error::Overflow("box buffer"))?;
    let mut bytes = vec![0; len];
    reader
        .read_exact(&mut bytes)
        .map_err(|error| map_eof(error, "box payload"))?;
    Ok(bytes)
}

pub(crate) fn read_array<const N: usize, R: Read>(reader: &mut R) -> Result<[u8; N]> {
    let mut value = [0; N];
    reader
        .read_exact(&mut value)
        .map_err(|error| map_eof(error, "field"))?;
    Ok(value)
}

pub(crate) fn read_u16<R: Read>(reader: &mut R) -> Result<u16> {
    Ok(u16::from_be_bytes(read_array(reader)?))
}

pub(crate) fn read_u32<R: Read>(reader: &mut R) -> Result<u32> {
    Ok(u32::from_be_bytes(read_array(reader)?))
}

pub(crate) fn read_i32<R: Read>(reader: &mut R) -> Result<i32> {
    Ok(i32::from_be_bytes(read_array(reader)?))
}

pub(crate) fn read_u64<R: Read>(reader: &mut R) -> Result<u64> {
    Ok(u64::from_be_bytes(read_array(reader)?))
}

pub(crate) fn read_i64<R: Read>(reader: &mut R) -> Result<i64> {
    Ok(i64::from_be_bytes(read_array(reader)?))
}

pub(crate) fn full_box<R: Read>(reader: &mut R) -> Result<(u8, u32)> {
    let bytes = read_array::<4, _>(reader)?;
    Ok((
        bytes[0],
        u32::from_be_bytes([0, bytes[1], bytes[2], bytes[3]]),
    ))
}

fn map_eof(error: std::io::Error, context: &'static str) -> Error {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        Error::Truncated { context }
    } else {
        Error::Io(error)
    }
}
