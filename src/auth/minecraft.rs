use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use serde::Deserialize;

use crate::auth::xbox::XblToken;

pub const LAUNCHER_LOGIN_URL: &str = "https://api.minecraftservices.com/launcher/login";
pub const ENTITLEMENTS_URL: &str = "https://api.minecraftservices.com/entitlements/mcstore";
pub const PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinecraftProfile {
    pub id: String,
    pub name: String,
    pub skin_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProfileResponse {
    id: String,
    name: String,
    #[serde(default)]
    skins: Vec<Skin>,
}

#[derive(Debug, Deserialize)]
struct Skin {
    #[serde(default)]
    state: String,
    url: String,
}

pub fn profile(token: &MinecraftToken) -> anyhow::Result<MinecraftProfile> {
    let response = crate::http::send_raw(
        crate::http::client()?
            .get(PROFILE_URL)
            .header("Authorization", token.authorization_header())
            .header("Accept", "application/json"),
    )
    .context("프로필을 불러오지 못했어요.")?;

    // 404 is not a transport failure: the account simply has no Java profile
    // yet, which is a different thing to tell the user about.
    if response.status().as_u16() == 404 {
        bail!("이 계정에는 Minecraft Java Edition 프로필이 없어요. 먼저 게임에서 닉네임을 설정해 주세요.");
    }

    if !response.status().is_success() {
        bail!("{}", describe_failure(response.status().as_u16()));
    }

    let parsed: ProfileResponse = response
        .json()
        .context("프로필 응답을 해석하지 못했어요.")?;

    into_profile(parsed)
}

fn into_profile(response: ProfileResponse) -> anyhow::Result<MinecraftProfile> {
    let skin_url = active_skin(&response.skins);

    log::info!("프로필 확인: {} ({})", response.name, response.id);

    Ok(MinecraftProfile {
        id: dash_uuid(&response.id)?,
        name: response.name,
        skin_url,
    })
}

fn active_skin(skins: &[Skin]) -> Option<String> {
    skins
        .iter()
        .find(|skin| skin.state.eq_ignore_ascii_case("ACTIVE"))
        .or_else(|| skins.first())
        .map(|skin| skin.url.clone())
}

/// The profile endpoint returns the UUID without dashes; the game expects the
/// canonical dashed form. Mirrors `UuidUtil.fromUndashedString`.
pub fn dash_uuid(raw: &str) -> anyhow::Result<String> {
    let compact: String = raw.chars().filter(|c| *c != '-').collect();

    if compact.len() != 32 || !compact.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("계정 UUID 형식이 올바르지 않아요: {raw}");
    }

    let lower = compact.to_ascii_lowercase();

    Ok(format!(
        "{}-{}-{}-{}-{}",
        &lower[0..8],
        &lower[8..12],
        &lower[12..16],
        &lower[16..20],
        &lower[20..32]
    ))
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
    fn adds_dashes_to_the_undashed_profile_id() {
        assert_eq!(
            dash_uuid("069a79f444e94726a5befca90e38aaf5").unwrap(),
            "069a79f4-44e9-4726-a5be-fca90e38aaf5"
        );
    }

    #[test]
    fn an_already_dashed_id_survives_unchanged() {
        let dashed = "069a79f4-44e9-4726-a5be-fca90e38aaf5";

        assert_eq!(dash_uuid(dashed).unwrap(), dashed);
    }

    #[test]
    fn uppercase_ids_are_normalised() {
        assert_eq!(
            dash_uuid("069A79F444E94726A5BEFCA90E38AAF5").unwrap(),
            "069a79f4-44e9-4726-a5be-fca90e38aaf5"
        );
    }

    #[test]
    fn malformed_ids_are_rejected() {
        assert!(dash_uuid("069a79f4").is_err());
        assert!(dash_uuid("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").is_err());
        assert!(dash_uuid("").is_err());
    }

    #[test]
    fn parses_the_profile_response_and_picks_the_active_skin() {
        let json = r#"{
            "id": "069a79f444e94726a5befca90e38aaf5",
            "name": "Notch",
            "skins": [
                { "id": "1", "state": "INACTIVE", "url": "https://old.png", "variant": "CLASSIC" },
                { "id": "2", "state": "ACTIVE", "url": "https://current.png", "variant": "SLIM" }
            ],
            "capes": []
        }"#;

        let parsed: ProfileResponse = serde_json::from_str(json).unwrap();
        let profile = into_profile(parsed).unwrap();

        assert_eq!(profile.id, "069a79f4-44e9-4726-a5be-fca90e38aaf5");
        assert_eq!(profile.name, "Notch");
        assert_eq!(profile.skin_url.as_deref(), Some("https://current.png"));
    }

    #[test]
    fn a_profile_without_skins_has_no_avatar_source() {
        let parsed: ProfileResponse =
            serde_json::from_str(r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch"}"#)
                .unwrap();

        assert_eq!(into_profile(parsed).unwrap().skin_url, None);
    }

    #[test]
    fn the_first_skin_is_used_when_none_is_marked_active() {
        let skins = vec![
            Skin {
                state: "INACTIVE".to_string(),
                url: "https://first.png".to_string(),
            },
            Skin {
                state: "INACTIVE".to_string(),
                url: "https://second.png".to_string(),
            },
        ];

        assert_eq!(active_skin(&skins).as_deref(), Some("https://first.png"));
    }

    #[test]
    fn auth_failures_are_told_apart() {
        assert!(describe_failure(401).contains("다시 로그인"));
        assert!(describe_failure(403).contains("사용할 수 없어요"));
        assert!(describe_failure(500).contains("HTTP 500"));
    }
}
