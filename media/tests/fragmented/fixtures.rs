//! Synthetic fragmented-MP4 construction helpers.

#![allow(missing_docs)]

pub fn atom(kind: &[u8; 4], payload: impl AsRef<[u8]>) -> Vec<u8> {
    let payload = payload.as_ref();
    let mut output = Vec::with_capacity(payload.len() + 8);
    output.extend_from_slice(
        &u32::try_from(payload.len() + 8)
            .expect("small fixture")
            .to_be_bytes(),
    );
    output.extend_from_slice(kind);
    output.extend_from_slice(payload);
    output
}

pub fn extended_atom(kind: &[u8; 4], payload: impl AsRef<[u8]>) -> Vec<u8> {
    let payload = payload.as_ref();
    let mut output = Vec::with_capacity(payload.len() + 16);
    output.extend_from_slice(&1_u32.to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(
        &u64::try_from(payload.len() + 16)
            .expect("small fixture")
            .to_be_bytes(),
    );
    output.extend_from_slice(payload);
    output
}

pub fn full_atom(kind: &[u8; 4], version: u8, flags: u32, payload: impl AsRef<[u8]>) -> Vec<u8> {
    let mut body = vec![
        version,
        (flags >> 16) as u8,
        (flags >> 8) as u8,
        flags as u8,
    ];
    body.extend_from_slice(payload.as_ref());
    atom(kind, body)
}

pub fn container(kind: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
    atom(kind, children.concat())
}

pub fn mvhd() -> Vec<u8> {
    let mut payload = vec![0; 8];
    payload.extend_from_slice(&1000_u32.to_be_bytes());
    payload.extend_from_slice(&300_u32.to_be_bytes());
    full_atom(b"mvhd", 0, 0, payload)
}

pub fn tkhd(track_id: u32) -> Vec<u8> {
    let mut payload = vec![0; 8];
    payload.extend_from_slice(&track_id.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    full_atom(b"tkhd", 0, 0, payload)
}

pub fn mdhd(timescale: u32, duration: u32) -> Vec<u8> {
    let mut payload = vec![0; 8];
    payload.extend_from_slice(&timescale.to_be_bytes());
    payload.extend_from_slice(&duration.to_be_bytes());
    full_atom(b"mdhd", 0, 0, payload)
}

pub fn hdlr(kind: &[u8; 4]) -> Vec<u8> {
    let mut payload = vec![0; 4];
    payload.extend_from_slice(kind);
    full_atom(b"hdlr", 0, 0, payload)
}

pub fn stsd(entry: Vec<u8>) -> Vec<u8> {
    let mut payload = 1_u32.to_be_bytes().to_vec();
    payload.extend_from_slice(&entry);
    full_atom(b"stsd", 0, 0, payload)
}

pub fn avc1() -> Vec<u8> {
    let mut payload = vec![0; 78];
    payload[24..26].copy_from_slice(&1920_u16.to_be_bytes());
    payload[26..28].copy_from_slice(&1080_u16.to_be_bytes());
    payload.extend_from_slice(&atom(b"avcC", [1, 100, 0, 40, 0xff]));
    atom(b"avc1", payload)
}

pub fn mp4a() -> Vec<u8> {
    let mut payload = vec![0; 28];
    payload[16..18].copy_from_slice(&2_u16.to_be_bytes());
    payload[24..28].copy_from_slice(&(48_000_u32 << 16).to_be_bytes());
    let mut esds = vec![0, 0, 0, 0];
    esds.extend_from_slice(&[5, 2, 0x12, 0x10]);
    payload.extend_from_slice(&atom(b"esds", esds));
    atom(b"mp4a", payload)
}

pub fn edit_list() -> Vec<u8> {
    let mut payload = 2_u32.to_be_bytes().to_vec();
    payload.extend_from_slice(&100_u32.to_be_bytes());
    payload.extend_from_slice(&(-1_i32).to_be_bytes());
    payload.extend_from_slice(&1_i16.to_be_bytes());
    payload.extend_from_slice(&0_i16.to_be_bytes());
    payload.extend_from_slice(&200_u32.to_be_bytes());
    payload.extend_from_slice(&50_i32.to_be_bytes());
    payload.extend_from_slice(&1_i16.to_be_bytes());
    payload.extend_from_slice(&0_i16.to_be_bytes());
    container(b"edts", &[full_atom(b"elst", 0, 0, payload)])
}

pub fn trak(id: u32, handler: &[u8; 4], entry: Vec<u8>, edits: bool) -> Vec<u8> {
    let stbl = container(b"stbl", &[stsd(entry)]);
    let minf = container(b"minf", &[stbl]);
    let mdia = container(
        b"mdia",
        &[
            mdhd(if handler == b"soun" { 48_000 } else { 1000 }, 300),
            hdlr(handler),
            minf,
        ],
    );
    let mut children = vec![tkhd(id)];
    if edits {
        children.push(edit_list());
    }
    children.push(mdia);
    container(b"trak", &children)
}

pub fn trex(id: u32, duration: u32) -> Vec<u8> {
    let mut payload = id.to_be_bytes().to_vec();
    payload.extend_from_slice(&1_u32.to_be_bytes());
    payload.extend_from_slice(&duration.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    payload.extend_from_slice(&0_u32.to_be_bytes());
    full_atom(b"trex", 0, 0, payload)
}

pub fn init_video() -> Vec<u8> {
    let ftyp = atom(b"ftyp", b"iso6\0\0\0\0iso6");
    let moov = container(
        b"moov",
        &[
            mvhd(),
            trak(1, b"vide", avc1(), true),
            container(b"mvex", &[trex(1, 40)]),
        ],
    );
    [ftyp, moov].concat()
}

pub fn init_audio() -> Vec<u8> {
    let ftyp = atom(b"ftyp", b"iso6\0\0\0\0iso6");
    let moov = container(
        b"moov",
        &[
            mvhd(),
            trak(2, b"soun", mp4a(), false),
            container(b"mvex", &[trex(2, 1024)]),
        ],
    );
    [ftyp, moov].concat()
}

pub fn fragment(
    track_id: u32,
    decode_time: u32,
    rows: &[(u32, u32, u32, i32)],
    data: &[u8],
) -> Vec<u8> {
    fn build_trun(rows: &[(u32, u32, u32, i32)], data_offset: i32) -> Vec<u8> {
        let mut payload = u32::try_from(rows.len())
            .expect("small fixture")
            .to_be_bytes()
            .to_vec();
        payload.extend_from_slice(&data_offset.to_be_bytes());
        for (duration, size, flags, composition) in rows {
            payload.extend_from_slice(&duration.to_be_bytes());
            payload.extend_from_slice(&size.to_be_bytes());
            payload.extend_from_slice(&flags.to_be_bytes());
            payload.extend_from_slice(&composition.to_be_bytes());
        }
        full_atom(b"trun", 1, 0x000f01, payload)
    }
    let tfhd = full_atom(b"tfhd", 0, 0x020000, track_id.to_be_bytes());
    let tfdt = full_atom(b"tfdt", 0, 0, decode_time.to_be_bytes());
    let placeholder = container(
        b"moof",
        &[container(
            b"traf",
            &[tfhd.clone(), tfdt.clone(), build_trun(rows, 0)],
        )],
    );
    let offset = i32::try_from(placeholder.len() + 8).expect("small fixture");
    let moof = container(
        b"moof",
        &[container(b"traf", &[tfhd, tfdt, build_trun(rows, offset)])],
    );
    [moof, atom(b"mdat", data)].concat()
}
