use anyhow::Context;
use rand_core::RngCore;
use serde::Deserialize;

use crate::auth::{
    sign::{self, DeviceKey},
    xbox::XBL_AUTH_RELYING_PARTY,
};

pub const DEVICE_AUTHENTICATE_URL: &str = "https://device.auth.xboxlive.com/device/authenticate";
pub const DEVICE_TYPE: &str = "Win32";

const CONTRACT_VERSION: &str = "1";

/// Stable identity of this installation. Both halves are persisted by the
/// account store so Xbox keeps seeing the same device.
#[derive(Clone)]
pub struct DeviceIdentity {
    pub id: String,
    pub key: DeviceKey,
}

impl DeviceIdentity {
    pub fn generate() -> Self {
        Self {
            id: uuid_v4(),
            key: DeviceKey::generate(),
        }
    }

    pub fn restore(id: String, scalar: &[u8]) -> anyhow::Result<Self> {
        Ok(Self {
            id,
            key: DeviceKey::from_scalar(scalar)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceToken {
    pub token: String,
    pub device_id: String,
    pub not_after: String,
}

#[derive(Debug, Deserialize)]
struct DeviceResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "NotAfter")]
    not_after: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DeviceClaims,
}

/// Device auth answers with `xdi`, not the `xui` array used by user auth.
#[derive(Debug, Deserialize)]
struct DeviceClaims {
    xdi: Xdi,
}

#[derive(Debug, Deserialize)]
struct Xdi {
    did: String,
}

fn request_body(identity: &DeviceIdentity) -> serde_json::Value {
    serde_json::json!({
        "Properties": {
            "DeviceType": DEVICE_TYPE,
            "Id": format!("{{{}}}", identity.id),
            "AuthMethod": "ProofOfPossession",
            "ProofKey": identity.key.proof_key(),
        },
        "RelyingParty": XBL_AUTH_RELYING_PARTY,
        "TokenType": "JWT",
    })
}

pub fn authenticate(identity: &DeviceIdentity) -> anyhow::Result<DeviceToken> {
    let url = reqwest::Url::parse(DEVICE_AUTHENTICATE_URL)
        .context("디바이스 인증 주소가 올바르지 않아요.")?;

    // The exact bytes that get signed must be the exact bytes that get sent, so
    // the body is serialized once and reused rather than re-encoded by reqwest.
    let body = serde_json::to_vec(&request_body(identity))
        .context("디바이스 인증 요청을 만들지 못했어요.")?;

    let timestamp = sign::windows_timestamp(now_unix());
    let signature = identity.key.signature_header(
        timestamp,
        "POST",
        &sign::path_and_query(&url),
        None,
        &body,
    );

    let response = crate::http::send_raw(
        crate::http::client()?
            .post(url)
            .header("x-xbl-contract-version", CONTRACT_VERSION)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header("Signature", signature)
            .body(body),
    )
    .context("디바이스 인증에 실패했어요.")?;

    if !response.status().is_success() {
        return Err(crate::auth::xbox::error_from(
            response,
            "디바이스 인증에 실패했어요.",
        ));
    }

    let parsed: DeviceResponse = response
        .json()
        .context("디바이스 인증 응답을 해석하지 못했어요.")?;

    log::info!("디바이스 토큰 발급 완료");

    Ok(DeviceToken {
        token: parsed.token,
        device_id: parsed.display_claims.xdi.did,
        not_after: parsed.not_after,
    })
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

fn uuid_v4() -> String {
    let mut bytes = [0_u8; 16];
    rand_core::OsRng.fill_bytes(&mut bytes);

    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format_uuid(&bytes)
}

fn format_uuid(bytes: &[u8; 16]) -> String {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();

    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes_in_the_canonical_layout() {
        let bytes: [u8; 16] = [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
            0xcd, 0xef,
        ];

        assert_eq!(
            format_uuid(&bytes),
            "01234567-89ab-cdef-0123-456789abcdef"
        );
    }

    #[test]
    fn generated_ids_carry_the_version_and_variant_bits() {
        for _ in 0..32 {
            let id = uuid_v4();

            assert_eq!(id.len(), 36);
            assert_eq!(id.as_bytes()[14] as char, '4', "version nibble");
            assert!(
                matches!(id.as_bytes()[19] as char, '8' | '9' | 'a' | 'b'),
                "variant nibble in {id}"
            );
        }
    }

    #[test]
    fn generated_ids_are_unique() {
        let first = uuid_v4();
        let second = uuid_v4();

        assert_ne!(first, second);
    }

    #[test]
    fn the_body_matches_the_reference_request() {
        let identity = DeviceIdentity::generate();
        let body = request_body(&identity);

        assert_eq!(body["Properties"]["DeviceType"], "Win32");
        assert_eq!(body["Properties"]["AuthMethod"], "ProofOfPossession");
        assert_eq!(body["RelyingParty"], "http://auth.xboxlive.com");
        assert_eq!(body["TokenType"], "JWT");
        assert_eq!(body["Properties"]["ProofKey"]["crv"], "P-256");
    }

    #[test]
    fn the_device_id_is_wrapped_in_braces() {
        let identity = DeviceIdentity::generate();
        let body = request_body(&identity);
        let sent = body["Properties"]["Id"].as_str().unwrap();

        assert_eq!(sent, format!("{{{}}}", identity.id));
        assert!(sent.starts_with('{') && sent.ends_with('}'));
        assert_eq!(sent.len(), 38);
    }

    #[test]
    fn parses_the_xdi_claim_shape() {
        let json = r#"{
            "IssueInstant": "2026-08-20T00:00:00.0000000Z",
            "NotAfter": "2026-08-21T00:00:00.0000000Z",
            "Token": "DEVICE_TOKEN",
            "DisplayClaims": { "xdi": { "did": "DID", "dcs": "0" } }
        }"#;

        let parsed: DeviceResponse = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.token, "DEVICE_TOKEN");
        assert_eq!(parsed.display_claims.xdi.did, "DID");
    }

    #[test]
    fn a_restored_identity_keeps_its_id_and_key() {
        let original = DeviceIdentity::generate();
        let restored =
            DeviceIdentity::restore(original.id.clone(), &original.key.to_scalar()).unwrap();

        assert_eq!(restored.id, original.id);
        assert_eq!(restored.key.proof_key(), original.key.proof_key());
    }

    #[test]
    fn the_signed_path_has_no_question_mark() {
        let url = reqwest::Url::parse(DEVICE_AUTHENTICATE_URL).unwrap();

        assert_eq!(sign::path_and_query(&url), "/device/authenticate");
    }
}
