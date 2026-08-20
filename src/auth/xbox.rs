use anyhow::bail;
use reqwest::blocking::Response;
use serde::Deserialize;

/// `XblConstants.XBL_AUTH_RELYING_PARTY`
pub const XBL_AUTH_RELYING_PARTY: &str = "http://auth.xboxlive.com";

/// `XblConstants.JAVA_XSTS_RELYING_PARTY`
pub const JAVA_XSTS_RELYING_PARTY: &str = "rp://api.minecraftservices.com/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XblToken {
    pub token: String,
    pub user_hash: String,
    pub not_after: String,
}

#[derive(Debug, Deserialize)]
pub struct XblTokenResponse {
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

impl XblTokenResponse {
    pub fn into_token(self) -> anyhow::Result<XblToken> {
        let Some(claim) = self.display_claims.xui.into_iter().next() else {
            bail!("Xbox Live 응답에 사용자 정보가 없어요.");
        };

        Ok(XblToken {
            token: self.token,
            user_hash: claim.uhs,
            not_after: self.not_after,
        })
    }
}

/// Legacy title ids are 16 hex characters; Azure application ids are GUIDs.
/// Mirrors `MsaApplicationConfig.isTitleClientId()`.
pub fn is_title_client_id(client_id: &str) -> bool {
    client_id.len() == 16 && client_id.chars().all(|c| c.is_ascii_hexdigit())
}

/// Xbox wants the MSA token tagged with where it came from: `t=` for title
/// auth, `d=` for an Azure application.
pub fn rps_ticket(access_token: &str, client_id: &str) -> String {
    let prefix = if is_title_client_id(client_id) {
        "t="
    } else {
        "d="
    };

    format!("{prefix}{access_token}")
}

#[derive(Debug, Deserialize)]
struct XErrBody {
    #[serde(rename = "XErr")]
    x_err: u64,
}

/// Xbox rejects accounts for reasons the user can usually fix, but only says so
/// through a numeric `XErr`. Codes come from `XblRequestException`.
pub fn describe_xerr(code: u64) -> String {
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
        0x8015_DC0E => "미성년자 계정이라 보호자의 Microsoft 가족 그룹에 추가돼야 해요.",
        _ => "Xbox Live 권한 확인에 실패했어요.",
    };

    format!("{message} (XErr {code})")
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
        serde_json::from_str::<XErrBody>(&body_text)
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
    fn parses_the_documented_token_shape() {
        let json = r#"{
            "IssueInstant": "2026-08-20T00:00:00.0000000Z",
            "NotAfter": "2026-08-21T00:00:00.0000000Z",
            "Token": "XBL_TOKEN",
            "DisplayClaims": { "xui": [ { "uhs": "1234567890" } ] }
        }"#;

        let parsed: XblTokenResponse = serde_json::from_str(json).unwrap();

        assert_eq!(
            parsed.into_token().unwrap(),
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

        let parsed: XblTokenResponse = serde_json::from_str(json).unwrap();

        assert!(parsed.into_token().is_err());
    }

    #[test]
    fn the_first_claim_wins_when_several_are_returned() {
        let json = r#"{
            "NotAfter": "2026-08-21T00:00:00.0000000Z",
            "Token": "XBL_TOKEN",
            "DisplayClaims": { "xui": [ { "uhs": "first" }, { "uhs": "second" } ] }
        }"#;

        let parsed: XblTokenResponse = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.into_token().unwrap().user_hash, "first");
    }

    #[test]
    fn the_documented_xerr_codes_get_actionable_messages() {
        assert_eq!(0x8015_DC09_u64, 2_148_916_233);
        assert_eq!(0x8015_DC0E_u64, 2_148_916_238);

        let no_profile = describe_xerr(2_148_916_233);
        assert!(no_profile.contains("Xbox 프로필"));
        assert!(no_profile.contains("xbox.com/live"));

        assert!(describe_xerr(2_148_916_238).contains("가족 그룹"));
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

        let parsed: XErrBody = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.x_err, 2_148_916_238);
    }
}
