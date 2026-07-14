//! Defensive ISO-BMFF box traversal helpers.

use crate::Error;

pub(super) fn validate_boxes(data: &[u8], child_offset: usize) -> Result<(), Error> {
    walk_boxes(data, child_offset, &mut |_, _| Ok(()))
}

pub(super) fn count_box(
    data: &[u8],
    target: &[u8; 4],
    child_offset: usize,
) -> Result<usize, Error> {
    let mut count = 0;
    walk_boxes(data, child_offset, &mut |name, _| {
        if name == target {
            count += 1;
        }
        Ok(())
    })?;
    Ok(count)
}

pub(super) fn find_box_payload<'a>(
    data: &'a [u8],
    target: &[u8; 4],
    child_offset: usize,
) -> Result<Option<&'a [u8]>, Error> {
    let mut found = None;
    walk_boxes(data, child_offset, &mut |name, payload| {
        if name == target && found.is_none() {
            found = Some(payload);
        }
        Ok(())
    })?;
    Ok(found)
}

fn walk_boxes<'a>(
    data: &'a [u8],
    child_offset: usize,
    visit: &mut impl FnMut(&[u8; 4], &'a [u8]) -> Result<(), Error>,
) -> Result<(), Error> {
    if child_offset > data.len() {
        return Err(Error::UnsupportedLayout("invalid child offset".to_string()));
    }
    let mut position = child_offset;
    while position < data.len() {
        let (size, header) = box_size(data, position)?;
        let end = position
            .checked_add(size)
            .ok_or_else(|| Error::UnsupportedLayout("box size overflow".to_string()))?;
        if end > data.len() {
            return Err(Error::UnsupportedLayout(
                "box exceeds parent bounds".to_string(),
            ));
        }
        let name: &[u8; 4] = data[position + 4..position + 8]
            .try_into()
            .expect("box type is four bytes");
        let payload = &data[position + header..end];
        visit(name, payload)?;
        if let Some(offset) = children_offset(name) {
            walk_boxes(payload, offset, visit)?;
        }
        position = end;
    }
    Ok(())
}

pub(super) fn walk_boxes_mut(
    data: &mut [u8],
    child_offset: usize,
    visit: &mut impl FnMut(&mut [u8; 4]),
) -> Result<(), Error> {
    if child_offset > data.len() {
        return Err(Error::UnsupportedLayout("invalid child offset".to_string()));
    }
    let mut position = child_offset;
    while position < data.len() {
        let (size, header) = box_size(data, position)?;
        let end = position
            .checked_add(size)
            .ok_or_else(|| Error::UnsupportedLayout("box size overflow".to_string()))?;
        if end > data.len() {
            return Err(Error::UnsupportedLayout(
                "box exceeds parent bounds".to_string(),
            ));
        }
        let original: [u8; 4] = data[position + 4..position + 8]
            .try_into()
            .expect("box type is four bytes");
        let offset = children_offset(&original);
        let name: &mut [u8; 4] = (&mut data[position + 4..position + 8])
            .try_into()
            .expect("box type is four bytes");
        visit(name);
        if let Some(offset) = offset {
            walk_boxes_mut(&mut data[position + header..end], offset, visit)?;
        }
        position = end;
    }
    Ok(())
}

pub(super) fn box_size(data: &[u8], position: usize) -> Result<(usize, usize), Error> {
    if data.len().saturating_sub(position) < 8 {
        return Err(Error::UnsupportedLayout("truncated box header".to_string()));
    }
    let short = u32::from_be_bytes(
        data[position..position + 4]
            .try_into()
            .expect("box size is four bytes"),
    );
    match short {
        0 => Ok((data.len() - position, 8)),
        1 => {
            if data.len().saturating_sub(position) < 16 {
                return Err(Error::UnsupportedLayout(
                    "truncated extended box header".to_string(),
                ));
            }
            let size = u64::from_be_bytes(
                data[position + 8..position + 16]
                    .try_into()
                    .expect("extended box size is eight bytes"),
            );
            let size = usize::try_from(size)
                .map_err(|_| Error::UnsupportedLayout("box size overflow".to_string()))?;
            if size < 16 {
                return Err(Error::UnsupportedLayout(
                    "invalid extended box size".to_string(),
                ));
            }
            Ok((size, 16))
        }
        size if size < 8 => Err(Error::UnsupportedLayout("invalid box size".to_string())),
        size => Ok((size as usize, 8)),
    }
}

pub(super) fn children_offset(name: &[u8; 4]) -> Option<usize> {
    match name {
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"sinf" | b"schi" | b"moof" | b"traf" => {
            Some(0)
        }
        b"stsd" => Some(8),
        b"encv" => Some(78),
        b"enca" => Some(28),
        _ => None,
    }
}
