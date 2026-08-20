use anyhow::Context;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use serde_json::json;

/// Xbox signs against the Windows epoch (1601-01-01) in 100 ns ticks.
const WINDOWS_EPOCH_OFFSET_SECS: i64 = 11_644_473_600;
const TICKS_PER_SECOND: i64 = 10_000_000;
const POLICY_VERSION: i32 = 1;

pub fn windows_timestamp(unix_seconds: i64) -> i64 {
    (unix_seconds + WINDOWS_EPOCH_OFFSET_SECS) * TICKS_PER_SECOND
}

/// Byte layout that Xbox expects to be signed.
///
/// Every field is terminated by a zero byte, including the last one. The path
/// and query are concatenated without the `?` separator, matching
/// `SignedXblPostRequest`.
pub fn signature_payload(
    timestamp: i64,
    method: &str,
    path_and_query: &str,
    authorization: Option<&str>,
    body: &[u8],
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(64 + body.len());

    payload.extend_from_slice(&POLICY_VERSION.to_be_bytes());
    payload.push(0);
    payload.extend_from_slice(&timestamp.to_be_bytes());
    payload.push(0);
    payload.extend_from_slice(method.as_bytes());
    payload.push(0);
    payload.extend_from_slice(path_and_query.as_bytes());
    payload.push(0);
    payload.extend_from_slice(authorization.unwrap_or_default().as_bytes());
    payload.push(0);
    payload.extend_from_slice(body);
    payload.push(0);

    payload
}

/// Path and query as the signature expects them: joined with no `?`, matching
/// `getURL().getPath() + (getURL().getQuery() != null ? getQuery() : "")`.
pub fn path_and_query(url: &reqwest::Url) -> String {
    format!("{}{}", url.path(), url.query().unwrap_or_default())
}

/// Device key pair used for `ProofKey` and request signing.
#[derive(Clone)]
pub struct DeviceKey {
    signing: SigningKey,
}

impl DeviceKey {
    pub fn generate() -> Self {
        Self {
            signing: SigningKey::random(&mut rand_core::OsRng),
        }
    }

    /// Restore from the 32-byte private scalar kept in the account store.
    pub fn from_scalar(bytes: &[u8]) -> anyhow::Result<Self> {
        let signing = SigningKey::from_slice(bytes).context("디바이스 키를 복원하지 못했어요.")?;

        Ok(Self { signing })
    }

    pub fn to_scalar(&self) -> [u8; 32] {
        self.signing.to_bytes().into()
    }

    /// Public key as the JWK that Xbox calls `ProofKey`.
    pub fn proof_key(&self) -> serde_json::Value {
        let point = self.signing.verifying_key().to_encoded_point(false);

        json!({
            "kty": "EC",
            "alg": "ES256",
            "crv": "P-256",
            "use": "sig",
            "x": URL_SAFE_NO_PAD.encode(point.x().expect("P-256 point has an x coordinate")),
            "y": URL_SAFE_NO_PAD.encode(point.y().expect("P-256 point has a y coordinate")),
        })
    }

    /// Value for the `Signature` header.
    pub fn signature_header(
        &self,
        timestamp: i64,
        method: &str,
        path_and_query: &str,
        authorization: Option<&str>,
        body: &[u8],
    ) -> String {
        let payload = signature_payload(timestamp, method, path_and_query, authorization, body);
        let signature: Signature = self.signing.sign(&payload);

        let mut header = Vec::with_capacity(4 + 8 + 64);
        header.extend_from_slice(&POLICY_VERSION.to_be_bytes());
        header.extend_from_slice(&timestamp.to_be_bytes());
        header.extend_from_slice(&signature.to_bytes());

        STANDARD.encode(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{signature::Verifier, VerifyingKey};

    #[test]
    fn converts_the_unix_epoch_to_windows_ticks() {
        assert_eq!(windows_timestamp(0), 11_644_473_600 * 10_000_000);
        assert_eq!(
            windows_timestamp(1_700_000_000),
            (1_700_000_000 + 11_644_473_600) * 10_000_000
        );
    }

    #[test]
    fn the_payload_matches_the_documented_layout() {
        let payload = signature_payload(1, "POST", "/authorize", None, b"{}");

        let expected: Vec<u8> = [
            &1_i32.to_be_bytes()[..],
            &[0],
            &1_i64.to_be_bytes()[..],
            &[0],
            b"POST",
            &[0],
            b"/authorize",
            &[0],
            b"",
            &[0],
            b"{}",
            &[0],
        ]
        .concat();

        assert_eq!(payload, expected);
    }

    #[test]
    fn a_missing_authorization_header_still_emits_its_terminator() {
        let without = signature_payload(1, "POST", "/x", None, b"");
        let with = signature_payload(1, "POST", "/x", Some(""), b"");

        assert_eq!(without, with);
    }

    #[test]
    fn the_authorization_header_is_part_of_the_signature() {
        let anonymous = signature_payload(1, "POST", "/x", None, b"");
        let authorized = signature_payload(1, "POST", "/x", Some("XBL3.0 x=uhs;tok"), b"");

        assert_ne!(anonymous, authorized);
    }

    #[test]
    fn every_field_is_zero_terminated() {
        let payload = signature_payload(1, "POST", "/x", Some("A"), b"B");

        assert_eq!(payload.iter().filter(|byte| **byte == 0).count() >= 6, true);
        assert_eq!(payload.last(), Some(&0));
    }

    #[test]
    fn the_header_carries_version_timestamp_and_a_64_byte_signature() {
        let key = DeviceKey::generate();
        let timestamp = windows_timestamp(1_700_000_000);

        let header = key.signature_header(timestamp, "POST", "/authorize", None, b"{}");
        let raw = STANDARD.decode(header).unwrap();

        assert_eq!(raw.len(), 4 + 8 + 64);
        assert_eq!(&raw[..4], &1_i32.to_be_bytes());
        assert_eq!(&raw[4..12], &timestamp.to_be_bytes());
    }

    #[test]
    fn the_signature_verifies_against_the_proof_key() {
        let key = DeviceKey::generate();
        let timestamp = windows_timestamp(1_700_000_000);
        let body = br#"{"Sandbox":"RETAIL"}"#;

        let header = key.signature_header(timestamp, "POST", "/authorize", None, body);
        let raw = STANDARD.decode(header).unwrap();
        let signature = Signature::from_slice(&raw[12..]).unwrap();

        let payload = signature_payload(timestamp, "POST", "/authorize", None, body);
        let verifying: VerifyingKey = *key.signing.verifying_key();

        assert!(verifying.verify(&payload, &signature).is_ok());
    }

    #[test]
    fn a_restored_key_produces_the_same_proof_key() {
        let original = DeviceKey::generate();
        let restored = DeviceKey::from_scalar(&original.to_scalar()).unwrap();

        assert_eq!(original.proof_key(), restored.proof_key());
    }

    #[test]
    fn the_proof_key_coordinates_are_32_bytes_each() {
        let key = DeviceKey::generate();
        let proof = key.proof_key();

        for axis in ["x", "y"] {
            let encoded = proof[axis].as_str().unwrap();
            let decoded = URL_SAFE_NO_PAD.decode(encoded).unwrap();

            assert_eq!(decoded.len(), 32, "{axis} coordinate must be padded to 32 bytes");
            assert!(!encoded.contains('='), "{axis} must be unpadded base64url");
            assert!(!encoded.contains('+') && !encoded.contains('/'));
        }
    }

    #[test]
    fn the_proof_key_declares_the_curve_xbox_expects() {
        let proof = DeviceKey::generate().proof_key();

        assert_eq!(proof["kty"], "EC");
        assert_eq!(proof["alg"], "ES256");
        assert_eq!(proof["crv"], "P-256");
        assert_eq!(proof["use"], "sig");
    }

    #[test]
    fn a_malformed_scalar_is_rejected() {
        assert!(DeviceKey::from_scalar(&[0_u8; 8]).is_err());
    }
}
