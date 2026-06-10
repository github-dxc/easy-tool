use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub struct TencentSignatureInput<'a> {
    pub secret_id: &'a str,
    pub secret_key: &'a str,
    pub service: &'a str,
    pub host: &'a str,
    pub action: &'a str,
    pub version: &'a str,
    pub region: &'a str,
    pub timestamp: i64,
    pub date: &'a str,
    pub payload: &'a str,
}

pub fn hashed_payload(payload: &str) -> String {
    hex::encode(Sha256::digest(payload.as_bytes()))
}

pub fn canonical_headers(headers: &[(&str, &str)]) -> (String, String) {
    let mut normalized: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    normalized.sort_by(|left, right| left.0.cmp(&right.0));

    let canonical = normalized
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect();
    let signed_headers = normalized
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");

    (canonical, signed_headers)
}

fn hmac_sha256(key: &[u8], message: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts keys of any size");
    mac.update(message.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

pub fn sign_v3(input: TencentSignatureInput<'_>) -> String {
    let _ = (input.action, input.version, input.region);
    let (canonical_headers, signed_headers) = canonical_headers(&[
        ("content-type", "application/json; charset=utf-8"),
        ("host", input.host),
    ]);
    let canonical_request = format!(
        "POST\n/\n\n{canonical_headers}\n{signed_headers}\n{}",
        hashed_payload(input.payload)
    );
    let credential_scope = format!("{}/{}/tc3_request", input.date, input.service);
    let string_to_sign = format!(
        "TC3-HMAC-SHA256\n{}\n{}\n{}",
        input.timestamp,
        credential_scope,
        hashed_payload(&canonical_request)
    );
    let secret_date = hmac_sha256(format!("TC3{}", input.secret_key).as_bytes(), input.date);
    let secret_service = hmac_sha256(&secret_date, input.service);
    let secret_signing = hmac_sha256(&secret_service, "tc3_request");
    let signature = hex::encode(hmac_sha256(&secret_signing, &string_to_sign));

    format!(
        "TC3-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
        input.secret_id, credential_scope, signed_headers, signature
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_headers_lowercases_trims_and_sorts_by_header_name() {
        let (canonical, signed) = canonical_headers(&[
            ("X-TC-Timestamp", "  1700000000 "),
            ("Content-Type", " application/json; charset=utf-8 "),
            ("Host", " tmt.tencentcloudapi.com "),
        ]);

        assert_eq!(
            canonical,
            "content-type:application/json; charset=utf-8\nhost:tmt.tencentcloudapi.com\nx-tc-timestamp:1700000000\n"
        );
        assert_eq!(signed, "content-type;host;x-tc-timestamp");
    }

    #[test]
    fn hashed_payload_returns_lowercase_sha256_hex() {
        assert_eq!(
            hashed_payload("{}"),
            "44136fa355b3678a1146ad16f7e8649e94fb4f7c4dbe2dc8aebe1c9e820d0d5"
        );
    }
}
