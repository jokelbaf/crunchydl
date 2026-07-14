use std::time::Duration;

use crate::element::{master, string, uint};
use crate::error::{Error, Result};
use crate::track::Language;

/// One named chapter point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
    /// Chapter start time.
    pub start: Duration,
    /// Display title.
    pub title: String,
    /// Display language.
    pub language: Language,
}

pub(crate) fn encode(chapters: &[Chapter]) -> Result<Option<Vec<u8>>> {
    if chapters.is_empty() {
        return Ok(None);
    }
    let mut atoms = Vec::new();
    let mut previous = None;
    for (index, chapter) in chapters.iter().enumerate() {
        if chapter.title.is_empty() || previous.is_some_and(|start| start >= chapter.start) {
            return Err(Error::Invalid(
                "chapters must be nonempty and strictly ordered",
            ));
        }
        previous = Some(chapter.start);
        let nanos = u64::try_from(chapter.start.as_nanos())
            .map_err(|_| Error::Overflow("chapter timestamp"))?;
        atoms.push(master(
            0xb6,
            [
                uint(0x73c4, index as u64 + 1)?,
                uint(0x91, nanos)?,
                uint(0x4598, 1)?,
                master(
                    0x80,
                    [
                        string(0x85, &chapter.title)?,
                        string(0x437c, &chapter.language.legacy)?,
                        string(0x437d, &chapter.language.ietf)?,
                    ],
                )?,
            ],
        )?);
    }
    Ok(Some(master(0x1043_a770, [master(0x45b9, atoms)?])?))
}
