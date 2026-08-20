use anyhow::{bail, Context};
use serde::Deserialize;

use crate::auth::{
    device::{DeviceIdentity, DeviceToken},
    msa::CLIENT_ID,
    sign,
    xbox::{self, XblToken, XblTokenResponse},
};

pub const SISU_AUTHORIZE_URL: &str = "https://sisu.xboxlive.com/authorize";

/// One signed call returns the user, title and XSTS tokens together. Title
/// client ids have to take this path — the plain `xsts/authorize` route is for
/// Azure applications.
#[derive(Debug, Clone)]
pub struct SisuTokens {
    pub user: XblToken,
    pub title: String,
    pub authorization: XblToken,
}

#[derive(Debug, Deserialize)]
struct SisuResponse {
    #[serde(rename = "UserToken")]
    user_token: XblTokenResponse,
    #[serde(rename = "TitleToken")]
    title_token: TitleTokenResponse,
    #[serde(rename = "AuthorizationToken")]
    authorization_token: XblTokenResponse,
}

/// The title token carries `xti.tid` rather than a user hash, so it does not
/// share the user-token shape.
#[derive(Debug, Deserialize)]
struct TitleTokenResponse {
    #[serde(rename = "Token")]
    token: String,
}

fn request_body(
    access_token: &str,
    device_token: &str,
    identity: &DeviceIdentity,
    relying_party: &str,
) -> serde_json::Value {
    serde_json::json!({
        "Sandbox": "RETAIL",
        "UseModernGamertag": true,
        "AppId": CLIENT_ID,
        "AccessToken": xbox::rps_ticket(access_token, CLIENT_ID),
        "DeviceToken": device_token,
        "ProofKey": identity.key.proof_key(),
        "RelyingParty": relying_party,
    })
}

pub fn authorize(
    access_token: &str,
    device_token: &DeviceToken,
    identity: &DeviceIdentity,
    relying_party: &str,
) -> anyhow::Result<SisuTokens> {
    if !xbox::is_title_client_id(CLIENT_ID) {
        bail!("SISU 인증은 title 클라이언트 ID에서만 사용할 수 있어요.");
    }

    let url =
        reqwest::Url::parse(SISU_AUTHORIZE_URL).context("SISU 주소가 올바르지 않아요.")?;

    let body = serde_json::to_vec(&request_body(
        access_token,
        &device_token.token,
        identity,
        relying_party,
    ))
    .context("SISU 요청을 만들지 못했어요.")?;

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
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .header("Signature", signature)
            .body(body),
    )
    .context("SISU 인증에 실패했어요.")?;

    if !response.status().is_success() {
        return Err(xbox::error_from(response, "SISU 인증에 실패했어요."));
    }

    let parsed: SisuResponse = response
        .json()
        .context("SISU 응답을 해석하지 못했어요.")?;

    let tokens = SisuTokens {
        user: parsed.user_token.into_token()?,
        title: parsed.title_token.token,
        authorization: parsed.authorization_token.into_token()?,
    };

    log::info!("SISU 인증 완료 (user / title / XSTS 토큰 확보)");

    Ok(tokens)
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::xbox::JAVA_XSTS_RELYING_PARTY;

    fn body() -> serde_json::Value {
        request_body(
            "ACCESS",
            "DEVICE",
            &DeviceIdentity::generate(),
            JAVA_XSTS_RELYING_PARTY,
        )
    }

    #[test]
    fn the_body_matches_the_reference_request() {
        let body = body();

        assert_eq!(body["Sandbox"], "RETAIL");
        assert_eq!(body["UseModernGamertag"], true);
        assert_eq!(body["AppId"], CLIENT_ID);
        assert_eq!(body["DeviceToken"], "DEVICE");
        assert_eq!(body["RelyingParty"], JAVA_XSTS_RELYING_PARTY);
        assert_eq!(body["ProofKey"]["crv"], "P-256");
    }

    #[test]
    fn the_access_token_carries_the_title_prefix() {
        assert_eq!(body()["AccessToken"], "t=ACCESS");
    }

    #[test]
    fn parses_all_three_tokens() {
        let json = r#"{
            "DeviceToken": "DEV",
            "TitleToken": {
                "NotAfter": "2026-08-21T00:00:00.0000000Z",
                "Token": "TITLE",
                "DisplayClaims": { "xti": { "tid": "1234" } }
            },
            "UserToken": {
                "NotAfter": "2026-08-21T00:00:00.0000000Z",
                "Token": "USER",
                "DisplayClaims": { "xui": [ { "uhs": "UHS" } ] }
            },
            "AuthorizationToken": {
                "NotAfter": "2026-08-21T00:00:00.0000000Z",
                "Token": "XSTS",
                "DisplayClaims": { "xui": [ { "uhs": "UHS", "xid": "999" } ] }
            }
        }"#;

        let parsed: SisuResponse = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.user_token.into_token().unwrap().token, "USER");
        assert_eq!(parsed.title_token.token, "TITLE");

        let xsts = parsed.authorization_token.into_token().unwrap();
        assert_eq!(xsts.token, "XSTS");
        assert_eq!(xsts.user_hash, "UHS");
    }

    #[test]
    fn a_response_missing_a_token_is_rejected() {
        let json = r#"{
            "UserToken": {
                "NotAfter": "x", "Token": "USER",
                "DisplayClaims": { "xui": [ { "uhs": "UHS" } ] }
            }
        }"#;

        assert!(serde_json::from_str::<SisuResponse>(json).is_err());
    }

    #[test]
    fn the_signed_path_has_no_question_mark() {
        let url = reqwest::Url::parse(SISU_AUTHORIZE_URL).unwrap();

        assert_eq!(sign::path_and_query(&url), "/authorize");
    }

    #[test]
    fn the_launcher_client_id_is_allowed_to_use_sisu() {
        assert!(xbox::is_title_client_id(CLIENT_ID));
    }
}
