use anyhow::{bail, Context};
use serde::Deserialize;

pub const USER_AUTHENTICATE_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";

/// `XblConstants.XBL_AUTH_RELYING_PARTY`
pub const XBL_AUTH_RELYING_PARTY: &str = "http://auth.xboxlive.com";

/// `XblConstants.JAVA_XSTS_RELYING_PARTY`
pub const JAVA_XSTS_RELYING_PARTY: &str = "rp://api.minecraftservices.com/";

const CONTRACT_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XblToken {
    pub token: String,
    pub user_hash: String,
    pub not_after: String,
}

#[derive(Debug, Deserialize)]
struct AuthenticateResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "NotAfter")]
    not_after: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}

#[derive(Debug, Deserialize)]
struct DisplayClaims {
    #[serde(default)]
    xui: Vec<Xui>,
}

#[derive(Debug, Deserialize)]
struct Xui {
    uhs: String,
}

/// Legacy title ids are 16 hex characters; Azure application ids are GUIDs.
/// Mirrors `MsaApplicationConfig.isTitleClientId()`.
pub fn is_title_client_id(client_id: &str) -> bool {
    client_id.len() == 16 && client_id.chars().all(|c| c.is_ascii_hexdigit())
}

/// Xbox Live wants the MSA token tagged with where it came from.
///
/// `t=` for title auth, `d=` for an Azure application. Sending the wrong prefix
/// is rejected with an opaque 400, so this is not a detail to guess at.
fn rps_ticket(access_token: &str, client_id: &str) -> String {
    let prefix = if is_title_client_id(client_id) {
        "t="
    } else {
        "d="
    };

    format!("{prefix}{access_token}")
}

pub fn authenticate(access_token: &str, client_id: &str) -> anyhow::Result<XblToken> {
    let body = serde_json::json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": rps_ticket(access_token, client_id),
        },
        "RelyingParty": XBL_AUTH_RELYING_PARTY,
        "TokenType": "JWT",
    });

    let response = crate::http::send(
        crate::http::client()?
            .post(USER_AUTHENTICATE_URL)
            .header("x-xbl-contract-version", CONTRACT_VERSION)
            .header("Accept", "application/json")
            .json(&body),
    )
    .context("Xbox Live 인증에 실패했어요.")?;

    let parsed: AuthenticateResponse = response
        .json()
        .context("Xbox Live 응답을 해석하지 못했어요.")?;

    parse(parsed)
}

fn parse(response: AuthenticateResponse) -> anyhow::Result<XblToken> {
    let Some(claim) = response.display_claims.xui.into_iter().next() else {
        bail!("Xbox Live 응답에 사용자 정보가 없어요.");
    };

    log::info!("Xbox Live 토큰 발급 완료");

    Ok(XblToken {
        token: response.token,
        user_hash: claim.uhs,
        not_after: response.not_after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::msa::CLIENT_ID;

    #[test]
    fn the_launcher_uses_a_title_client_id() {
        assert!(is_title_client_id(CLIENT_ID));
    }

    #[test]
    fn azure_application_ids_are_not_title_ids() {
        assert!(!is_title_client_id("00000000-0000-0000-0000-000000000000"));
        assert!(!is_title_client_id("389b1b32-b5d5-43b2-bddc-84ce938d6737"));
    }

    #[test]
    fn non_hex_ids_of_the_right_length_are_not_title_ids() {
        assert!(!is_title_client_id("zzzzzzzzzzzzzzzz"));
    }

    #[test]
    fn title_auth_uses_the_t_prefix() {
        assert_eq!(rps_ticket("AT", CLIENT_ID), "t=AT");
    }

    #[test]
    fn azure_auth_uses_the_d_prefix() {
        assert_eq!(
            rps_ticket("AT", "389b1b32-b5d5-43b2-bddc-84ce938d6737"),
            "d=AT"
        );
    }

    #[test]
    fn parses_the_documented_response_shape() {
        let json = r#"{
            "IssueInstant": "2026-08-20T00:00:00.0000000Z",
            "NotAfter": "2026-08-21T00:00:00.0000000Z",
            "Token": "XBL_TOKEN",
            "DisplayClaims": { "xui": [ { "uhs": "1234567890" } ] }
        }"#;

        let parsed: AuthenticateResponse = serde_json::from_str(json).unwrap();
        let token = parse(parsed).unwrap();

        assert_eq!(
            token,
            XblToken {
                token: "XBL_TOKEN".to_string(),
                user_hash: "1234567890".to_string(),
                not_after: "2026-08-21T00:00:00.0000000Z".to_string(),
            }
        );
    }

    #[test]
    fn an_empty_claim_list_is_reported() {
        let json = r#"{
            "NotAfter": "2026-08-21T00:00:00.0000000Z",
            "Token": "XBL_TOKEN",
            "DisplayClaims": { "xui": [] }
        }"#;

        let parsed: AuthenticateResponse = serde_json::from_str(json).unwrap();

        assert!(parse(parsed).is_err());
    }

    #[test]
    fn the_first_claim_wins_when_several_are_returned() {
        let json = r#"{
            "NotAfter": "2026-08-21T00:00:00.0000000Z",
            "Token": "XBL_TOKEN",
            "DisplayClaims": { "xui": [ { "uhs": "first" }, { "uhs": "second" } ] }
        }"#;

        let parsed: AuthenticateResponse = serde_json::from_str(json).unwrap();

        assert_eq!(parse(parsed).unwrap().user_hash, "first");
    }
}
