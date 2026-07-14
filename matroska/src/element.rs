use crate::ebml::{id_bytes, size_bytes, unsigned};
use crate::error::Result;

pub(crate) fn raw(id: u32, payload: &[u8]) -> Result<Vec<u8>> {
    let mut output = id_bytes(id)?;
    output.extend(size_bytes(payload.len() as u64, None)?);
    output.extend(payload);
    Ok(output)
}

pub(crate) fn master(id: u32, children: impl IntoIterator<Item = Vec<u8>>) -> Result<Vec<u8>> {
    let payload = children.into_iter().flatten().collect::<Vec<_>>();
    raw(id, &payload)
}

pub(crate) fn uint(id: u32, value: u64) -> Result<Vec<u8>> {
    raw(id, &unsigned(value))
}

pub(crate) fn string(id: u32, value: &str) -> Result<Vec<u8>> {
    raw(id, value.as_bytes())
}

pub(crate) fn binary(id: u32, value: &[u8]) -> Result<Vec<u8>> {
    raw(id, value)
}

pub(crate) fn float64(id: u32, value: f64) -> Result<Vec<u8>> {
    raw(id, &value.to_be_bytes())
}
