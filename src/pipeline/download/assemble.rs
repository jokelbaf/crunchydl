//! Clear and CENC-decrypted MP4 track assembly.

use super::*;

pub(crate) fn assemble_track(
    init_path: &Path,
    segments: &[PathBuf],
    output_path: &Path,
    key: Option<&ContentKey>,
    cancellation: &CancellationToken,
) -> Result<(), Error> {
    cancellation.check()?;
    let init = std::fs::read(init_path).map_err(|error| path_error(init_path, error))?;
    let mut output =
        BufWriter::new(File::create(output_path).map_err(|error| path_error(output_path, error))?);
    if let Some(key) = key {
        let decrypter = crate::CencDecrypter::new(&init, key)
            .map_err(|error| Error::Decrypt(error.to_string()))?;
        decrypter
            .assemble(&init, std::iter::empty(), &mut output)
            .map_err(|error| Error::Decrypt(error.to_string()))?;
        for segment in segments {
            cancellation.check()?;
            let encrypted = std::fs::read(segment).map_err(|error| path_error(segment, error))?;
            output
                .write_all(
                    &decrypter
                        .decrypt_fragment(encrypted)
                        .map_err(|error| Error::Decrypt(error.to_string()))?,
                )
                .map_err(|error| path_error(output_path, error))?;
        }
    } else {
        output
            .write_all(&init)
            .map_err(|error| path_error(output_path, error))?;
        for segment in segments {
            cancellation.check()?;
            let mut input = File::open(segment).map_err(|error| path_error(segment, error))?;
            std::io::copy(&mut input, &mut output)
                .map_err(|error| path_error(output_path, error))?;
        }
    }
    output
        .flush()
        .map_err(|error| path_error(output_path, error))?;
    output
        .get_ref()
        .sync_all()
        .map_err(|error| path_error(output_path, error))
}
