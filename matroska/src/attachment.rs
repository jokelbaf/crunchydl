use crate::element::{binary, master, string, uint};
use crate::error::{Error, Result};

/// A font or other file attached to the Matroska segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    /// Output filename stored in the container.
    pub filename: String,
    /// MIME type.
    pub mime_type: String,
    /// Stable nonzero UID, or zero to derive one.
    pub uid: u64,
    /// Exact attachment bytes.
    pub data: Vec<u8>,
}

pub(crate) fn encode(attachments: &[Attachment]) -> Result<Option<Vec<u8>>> {
    if attachments.is_empty() {
        return Ok(None);
    }
    let mut files = Vec::new();
    for (index, attachment) in attachments.iter().enumerate() {
        if attachment.filename.is_empty()
            || attachment.mime_type.is_empty()
            || attachment.data.is_empty()
        {
            return Err(Error::Invalid("attachment fields must not be empty"));
        }
        files.push(master(
            0x61a7,
            [
                string(0x466e, &attachment.filename)?,
                string(0x4660, &attachment.mime_type)?,
                binary(0x465c, &attachment.data)?,
                uint(
                    0x46ae,
                    if attachment.uid == 0 {
                        index as u64 + 1
                    } else {
                        attachment.uid
                    },
                )?,
            ],
        )?);
    }
    Ok(Some(master(0x1941_a469, files)?))
}
