use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use serde::Deserialize;

use crate::auth::xbox::XblToken;

pub const LAUNCHER_LOGIN_URL: &str = "https://api.minecraftservices.com/launcher/login";
pub const ENTITLEMENTS_URL: &str = "https://api.minecraftservices.com/entitlements/mcstore";

/// Entitlement names that mean the account may play Java Edition.
const OWNERSHIP_ITEMS: [&str; 4] = [
    "product_minecraft",
    "game_minecraft",
    "product_game_pass_pc",
    "product_game_pass_ultimate",
];

#[derive(Debug, Clone)]
pub struct MinecraftToken {
    pub access_token: String,
    pub token_type: String,
    pub expires_at: Instant,
}

impl MinecraftToken {
    pub fn authorization_header(&self) -> String {
        format!("{} {}", self.token_type, self.access_token)
    }
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct EntitlementsResponse {
    #[serde(default)]
    items: Vec<EntitlementItem>,
}

#[derive(Debug, Deserialize)]
struct EntitlementItem {
    name: String,
}

pub fn login(xsts: &XblToken) -> anyhow::Result<MinecraftToken> {
    let body = serde_json::json!({
        "platform": "PC_LAUNCHER",
        "xtoken": xsts.authorization_header(),
    });

    let response = crate::http::send_raw(
        crate::http::client()?
            .post(LAUNCHER_LOGIN_URL)
            .header("Accept", "application/json")
            .json(&body),
    )
    .context("Minecraft 로그인에 실패했어요.")?;

    if !response.status().is_success() {
        bail!("{}", describe_failure(response.status().as_u16()));
    }

    let parsed: LoginResponse = response
        .json()
        .context("Minecraft 로그인 응답을 해석하지 못했어요.")?;

    log::info!("Minecraft 토큰 발급 완료");

    Ok(MinecraftToken {
        access_token: parsed.access_token,
        token_type: parsed.token_type,
        expires_at: Instant::now() + Duration::from_secs(parsed.expires_in),
    })
}

pub fn entitlements(token: &MinecraftToken) -> anyhow::Result<Vec<String>> {
    let response = crate::http::send(
        crate::http::client()?
            .get(ENTITLEMENTS_URL)
            .header("Authorization", token.authorization_header())
            .header("Accept", "application/json"),
    )
    .context("게임 소유 여부를 확인하지 못했어요.")?;

    let parsed: EntitlementsResponse = response
        .json()
        .context("소유 정보 응답을 해석하지 못했어요.")?;

    let names: Vec<String> = parsed.items.into_iter().map(|item| item.name).collect();

    log::info!("엔타이틀먼트: {}", names.join(", "));

    Ok(names)
}

pub fn owns_java_edition(items: &[String]) -> bool {
    items
        .iter()
        .any(|name| OWNERSHIP_ITEMS.contains(&name.as_str()))
}

fn describe_failure(status: u16) -> String {
    match status {
        401 => "Minecraft 인증이 거부됐어요. 다시 로그인해 주세요.".to_string(),
        403 => "이 계정은 Minecraft 서비스를 사용할 수 없어요.".to_string(),
        status => format!("Minecraft 로그인에 실패했어요. (HTTP {status})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xsts() -> XblToken {
        XblToken {
            token: "XSTS_TOKEN".to_string(),
            user_hash: "1234567890".to_string(),
            not_after: "2026-08-21T00:00:00.0000000Z".to_string(),
        }
    }

    #[test]
    fn the_xtoken_uses_the_xbl3_format() {
        assert_eq!(
            xsts().authorization_header(),
            "XBL3.0 x=1234567890;XSTS_TOKEN"
        );
    }

    #[test]
    fn the_login_body_matches_the_reference_request() {
        let body = serde_json::json!({
            "platform": "PC_LAUNCHER",
            "xtoken": xsts().authorization_header(),
        });

        assert_eq!(body["platform"], "PC_LAUNCHER");
        assert_eq!(body["xtoken"], "XBL3.0 x=1234567890;XSTS_TOKEN");
    }

    #[test]
    fn parses_the_login_response() {
        let json = r#"{
            "username": "abc",
            "roles": [],
            "access_token": "MC_TOKEN",
            "token_type": "Bearer",
            "expires_in": 86400
        }"#;

        let parsed: LoginResponse = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.access_token, "MC_TOKEN");
        assert_eq!(parsed.token_type, "Bearer");
        assert_eq!(parsed.expires_in, 86_400);
    }

    #[test]
    fn the_authorization_header_joins_type_and_token() {
        let token = MinecraftToken {
            access_token: "MC_TOKEN".to_string(),
            token_type: "Bearer".to_string(),
            expires_at: Instant::now(),
        };

        assert_eq!(token.authorization_header(), "Bearer MC_TOKEN");
    }

    #[test]
    fn parses_the_entitlements_response() {
        let json = r#"{
            "items": [
                { "name": "product_minecraft", "signature": "x" },
                { "name": "game_minecraft", "signature": "y" }
            ],
            "signature": "z",
            "keyId": "1"
        }"#;

        let parsed: EntitlementsResponse = serde_json::from_str(json).unwrap();
        let names: Vec<String> = parsed.items.into_iter().map(|item| item.name).collect();

        assert_eq!(names, vec!["product_minecraft", "game_minecraft"]);
        assert!(owns_java_edition(&names));
    }

    #[test]
    fn an_empty_entitlement_list_means_no_ownership() {
        let parsed: EntitlementsResponse = serde_json::from_str(r#"{"items":[]}"#).unwrap();

        assert!(parsed.items.is_empty());
        assert!(!owns_java_edition(&[]));
    }

    #[test]
    fn game_pass_counts_as_ownership() {
        assert!(owns_java_edition(&["product_game_pass_ultimate".to_string()]));
        assert!(owns_java_edition(&["product_game_pass_pc".to_string()]));
    }

    #[test]
    fn unrelated_entitlements_do_not_count() {
        assert!(!owns_java_edition(&["product_minecraft_bedrock".to_string()]));
    }

    #[test]
    fn auth_failures_are_told_apart() {
        assert!(describe_failure(401).contains("다시 로그인"));
        assert!(describe_failure(403).contains("사용할 수 없어요"));
        assert!(describe_failure(500).contains("HTTP 500"));
    }
}
