use sha2::{Digest, Sha256};

pub(crate) fn url_identity(url: &str) -> String {
    let without_fragment = url.split('#').next().unwrap_or(url);
    without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment)
        .to_string()
}

pub(crate) fn fingerprint(parts: impl IntoIterator<Item = impl AsRef<[u8]>>) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        let bytes = part.as_ref();
        hasher.update(bytes.len().to_be_bytes());
        hasher.update(bytes);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_query_and_fragment_from_url_identity() {
        assert_eq!(
            url_identity("https://cdn.example/path/a.m4s?token=secret#part"),
            "https://cdn.example/path/a.m4s"
        );
    }
}
