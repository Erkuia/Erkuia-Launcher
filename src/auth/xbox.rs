use anyhow::{bail, Context};
use reqwest::blocking::Response;
use serde::Deserialize;

pub const USER_AUTHENTICATE_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
pub const XSTS_AUTHORIZE_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";

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

#[derive(Debug, Deserialize)]
struct XstsError {
    #[serde(rename = "XErr")]
    x_err: u64,
}

/// Xbox rejects accounts for reasons the user can usually fix, but only says so
/// through a numeric `XErr`. Codes come from `XblRequestException`.
fn describe_xerr(code: u64) -> String {
    let message = match code {
        0x8015_DC03 => "Xbox 커뮤니티 규정 위반으로 계정이 정지됐어요.",
        0x8015_DC04 => "제3자 서비스에서 계정이 정지됐어요.",
        0x8015_DC05 => {
            "보호자 설정으로 온라인 플레이가 제한돼 있어요. https://account.microsoft.com/family/ 에서 보호자가 권한을 변경해야 해요."
        }
        0x8015_DC09 => {
            "Xbox 프로필이 없는 계정이에요. https://www.xbox.com/live 에서 프로필을 먼저 만들어 주세요."
        }
        0x8015_DC0A => {
            "Xbox 서비스 약관에 동의가 필요해요. https://www.xbox.com/live 에서 로그인해 동의해 주세요."
        }
        0x8015_DC0B => "Xbox Live를 사용할 수 없는 국가의 계정이에요.",
        0x8015_DC0C => {
            "계정에 연령 확인이 필요해요. https://login.live.com/login.srf 에서 확인해 주세요."
        }
        0x8015_DC0D => "플레이 시간 제한에 도달해 로그인이 차단됐어요.",
        0x8015_DC0E => {
            "미성년자 계정이라 보호자의 Microsoft 가족 그룹에 추가돼야 해요."
        }
        _ => "Xbox Live 권한 확인에 실패했어요.",
    };

    format!("{message} (XErr {code})")
}

pub fn authorize(user_token: &str, relying_party: &str) -> anyhow::Result<XblToken> {
    let body = serde_json::json!({
        "Properties": {
            "SandboxId": "RETAIL",
            "UserTokens": [user_token],
        },
        "RelyingParty": relying_party,
        "TokenType": "JWT",
    });

    let response = crate::http::send_raw(
        crate::http::client()?
            .post(XSTS_AUTHORIZE_URL)
            .header("x-xbl-contract-version", CONTRACT_VERSION)
            .header("Accept", "application/json")
            .json(&body),
    )
    .context("XSTS 인증에 실패했어요.")?;

    if response.status().is_success() {
        let parsed: AuthenticateResponse =
            response.json().context("XSTS 응답을 해석하지 못했어요.")?;

        return parse(parsed);
    }

    Err(error_from(response, "Xbox Live 권한 확인에 실패했어요."))
}

/// Turn a rejected Xbox response into an actionable error.
///
/// The code arrives either in the `X-Err` header or as `XErr` in the body;
/// `XblResponseHandler` checks both, so this does too.
pub fn error_from(response: Response, fallback: &str) -> anyhow::Error {
    let status = response.status();
    let header_code = response
        .headers()
        .get("X-Err")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok());
    let body_text = response.text().unwrap_or_default();
    let code = header_code.or_else(|| {
        serde_json::from_str::<XstsError>(&body_text)
            .ok()
            .map(|error| error.x_err)
    });

    match code {
        Some(code) => {
            log::error!("Xbox 거부: XErr {code} (HTTP {})", status.as_u16());
            anyhow::anyhow!("{}", describe_xerr(code))
        }
        None => anyhow::anyhow!("{fallback} (HTTP {})", status.as_u16()),
    }
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
    fn the_documented_xerr_codes_get_actionable_messages() {
        // Matches the decimal values recorded in PLAN.md.
        assert_eq!(0x8015_DC09_u64, 2_148_916_233);
        assert_eq!(0x8015_DC0E_u64, 2_148_916_238);

        let no_profile = describe_xerr(2_148_916_233);
        assert!(no_profile.contains("Xbox 프로필"));
        assert!(no_profile.contains("xbox.com/live"));

        let child = describe_xerr(2_148_916_238);
        assert!(child.contains("가족 그룹"));
    }

    #[test]
    fn every_known_xerr_is_distinct_from_the_fallback() {
        let fallback = describe_xerr(1);

        for code in [
            0x8015_DC03,
            0x8015_DC04,
            0x8015_DC05,
            0x8015_DC09,
            0x8015_DC0A,
            0x8015_DC0B,
            0x8015_DC0C,
            0x8015_DC0D,
            0x8015_DC0E,
        ] {
            assert_ne!(
                describe_xerr(code),
                fallback,
                "XErr {code:#x} fell through to the generic message"
            );
        }
    }

    #[test]
    fn the_raw_code_is_always_kept_for_reporting() {
        assert!(describe_xerr(42).contains("XErr 42"));
    }

    #[test]
    fn parses_the_xerr_error_body() {
        let json = r#"{"Identity":"0","XErr":2148916238,"Message":"","Redirect":""}"#;

        let parsed: XstsError = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.x_err, 2_148_916_238);
    }

    #[test]
    fn xsts_reuses_the_user_token_response_shape() {
        let json = r#"{
            "NotAfter": "2026-08-21T00:00:00.0000000Z",
            "Token": "XSTS_TOKEN",
            "DisplayClaims": { "xui": [ { "uhs": "1234567890" } ] }
        }"#;

        let parsed: AuthenticateResponse = serde_json::from_str(json).unwrap();
        let token = parse(parsed).unwrap();

        assert_eq!(token.token, "XSTS_TOKEN");
        assert_eq!(token.user_hash, "1234567890");
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
