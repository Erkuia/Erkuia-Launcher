# Rendog Launcher 구현 작업 분해

Rendog Launcher(Rust + Slint)를 `RendogLauncher.exe`로 완성하기 위한 작업 분해 문서입니다.
`workflow.md`의 원칙에 따라 각 작업(`L#-#`)은 한 번에 하나씩 완료합니다.

## 전제 조건

| 항목 | 값 |
| --- | --- |
| 구현 언어 | Rust (UI: Slint 1.8) |
| 산출물 | `RendogLauncher.exe` |
| Minecraft 버전 | `1.20.4` (고정) |
| 모드 로더 | Fabric (RendogClient 요구사항) |
| 서버 | `rendog.kr` |
| Java | 21 |
| 로그인 | Microsoft 정식 로그인 (MSA → Xbox Live → XSTS → Minecraft) |
| 1차 범위 | 전체 기능 (로그인 / 업데이트 / 검증 / 모드 관리 / 실행) |
| 디자인 | Figma `Rendog-Launcher` (`node-id=25-2`) |

## 선행 결정 필요 항목

착수 전에 값이 확정되어야 진행 가능한 항목입니다.

- [x] ~~Azure AD 애플리케이션 Client ID~~ → 불필요. Title Auth 방식 채택 (Phase 4 참고)
- [ ] 런처 매니페스트 호스팅 위치 (예: GitHub Releases, 자체 CDN)
- [x] ~~런처 데이터 디렉터리 규약~~ → `%APPDATA%\RendogLauncher` 확정, 인스톨러 반영 완료
- [ ] Java 21 런타임 배포 방식 (번들 다운로드 vs 시스템 Java 요구)
- [ ] Figma MCP 커넥터 연결 (디자인 토큰/레이아웃 추출용)

### 쓰기 권한 (해결됨 — A안 적용)

런처는 실행 시 모드 ON/OFF, 에셋 다운로드, 로그 기록 등 쓰기 작업이 필요한데
`%ProgramFiles%` 아래는 일반 권한으로 쓸 수 없습니다. 바이너리와 가변 데이터를
분리하는 A안을 인스톨러에 적용했습니다.

| 루트 | 경로 | 내용 | 관리자 권한 |
| --- | --- | --- | --- |
| install | `%ProgramFiles%\Rendog Launcher` | `RendogLauncher.exe`, 언인스톨러 | 필요 |
| data | `%APPDATA%\RendogLauncher` | `minecraft\` (게임 파일, 모드, 설정, 로그) | 불필요 |

런처는 게임 디렉터리를 `%APPDATA%\RendogLauncher\minecraft`로 고정하며,
설치 디렉터리에는 쓰지 않습니다 (L9 자체 업데이트만 예외).

---

## Phase 0 — 프로젝트 기반

| ID | 작업 | 산출물 |
| --- | --- | --- |
| L0-1 | Cargo 프로젝트 스캐폴딩 | `Cargo.toml`, `build.rs`, `src/main.rs`, `ui/launcher.slint`, `.gitignore` |
| L0-2 | Windows 리소스 설정 | 아이콘 `.ico`, `winresource` 제품 메타데이터, 창 이름 |
| L0-3 | 에셋 이전 | `assets/` 로고 세트를 인스톨러 기준으로 정리 |
| L0-4 | 공통 골격 모듈 | `error.rs`(anyhow 래핑), `paths.rs`, `logging.rs` |

## Phase 1 — 디자인 시스템 / UI 셸

| ID | 작업 | 내용 |
| --- | --- | --- |
| L1-1 | Figma 토큰 추출 | 색상, 타이포, 간격, 라운드, 아이콘 → `ui/theme.slint` |
| L1-2 | 공통 컴포넌트 | 버튼(primary/ghost), 인풋, 토글/체크박스, 프로그레스 바, 카드, 모달 |
| L1-3 | 커스텀 타이틀바 | frameless 창, 드래그 이동, 최소화/닫기 (인스톨러와 동일 규칙) |
| L1-4 | 앱 셸 + 라우팅 | 사이드바/네비 + 콘텐츠 영역, `Screen` enum 기반 화면 전환 |

> 인스톨러의 벡터 체크마크 방식(폰트 의존 제거)을 런처에도 동일하게 적용합니다.

## Phase 2 — 화면 구현 (Figma 기준)

| ID | 화면 | 주요 요소 |
| --- | --- | --- |
| L2-1 | 로그인 | Microsoft 로그인 버튼, 디바이스 코드 표시, 진행/오류 상태 |
| L2-2 | 홈 | 플레이 버튼, 계정 프로필, 서버 상태, 공지 영역 |
| L2-3 | 모드 관리 | 모드 리스트, ON/OFF 토글, 필수 모드 잠금 표시 |
| L2-4 | 설정 | 최대 RAM, 추가 JVM 인자, Java 경로, 해상도, 실행 후 런처 종료 옵션 |
| L2-5 | 진행률 / 오류 | 다운로드·검증·실행 준비 단계 표시, 재시도/닫기 |

## Phase 3 — 코어 인프라

| ID | 작업 | 내용 |
| --- | --- | --- |
| L3-1 | 디렉터리 부트스트랩 | `paths.rs` — `%APPDATA%\RendogLauncher` 하위 `minecraft/`, `runtime/`, `cache/`, `logs/` (인스톨러 `paths.rs`와 동일 규약) |
| L3-2 | 설정 저장/로드 | `config.json` (serde), 기본값 병합, 원자적 쓰기 |
| L3-3 | 로깅 | 파일 롤링 로그 + 오류 화면 연동용 컨텍스트 |
| L3-4 | 비동기 + UI 브리지 | 백그라운드 워커 스레드 ↔ `slint::invoke_from_event_loop`, 인스톨러 `progress.rs` 이벤트 모델 이식 |
| L3-5 | HTTP 클라이언트 | `reqwest` 공통 클라이언트, 타임아웃, 재시도, User-Agent |

## Phase 4 — Microsoft 인증

레퍼런스: [meteor-client `MicrosoftLogin.java`](https://github.com/MeteorDevelopment/meteor-client/blob/master/src/main/java/meteordevelopment/meteorclient/systems/accounts/MicrosoftLogin.java)
→ 내부적으로 [RaphiMC/MinecraftAuth](https://github.com/RaphiMC/MinecraftAuth)의
`JavaAuthManager` + `DeviceCodeMsaAuthService`를 사용합니다.
런처는 Rust이므로 라이브러리를 그대로 쓸 수 없고, **동일한 흐름을 Rust로 이식**합니다.

### 채택 방식: Title Auth (Device Code)

meteor는 Azure 앱 등록 대신 Minecraft 정식 런처의 Title ID를 사용합니다.
따라서 **Azure AD Client ID 발급이 필요 없습니다.**

| 항목 | 값 | 출처 |
| --- | --- | --- |
| Client ID | `00000000402b5328` | `MsaConstants.JAVA_TITLE_ID` |
| Scope | `service::user.auth.xboxlive.com::MBI_SSL` | `MsaConstants.SCOPE_TITLE_AUTH` |
| Device code 발급 | `https://login.live.com/oauth20_connect.srf` | 레거시 live.com 엔드포인트 |
| 토큰 발급/갱신 | `https://login.live.com/oauth20_token.srf` | 동일 |
| Xbox Live | `https://user.auth.xboxlive.com/user/authenticate` | |
| XSTS | `https://xsts.auth.xboxlive.com/xsts/authorize` (`rp://api.minecraftservices.com/`) | |
| Minecraft 토큰 | `https://api.minecraftservices.com/authentication/login_with_xbox` | |
| 프로필 | `https://api.minecraftservices.com/minecraft/profile` | |

> Title auth는 `XboxLive.signin` 스코프의 v2.0 엔드포인트가 아니라 live.com 레거시
> 엔드포인트를 씁니다. `offline_access` 없이도 리프레시 토큰이 발급됩니다.

### 작업

| ID | 작업 | 내용 |
| --- | --- | --- |
| L4-1 | Device Code 발급 | Title ID + title auth 스코프로 user code / verification URI 획득 |
| L4-2 | 토큰 폴링 | `interval` 준수, `authorization_pending` / `expired_token` / `authorization_declined` 구분 처리, 취소 지원 |
| L4-3 | Xbox Live 인증 | `RpsTicket` 교환, XBL 토큰 획득 |
| L4-4 | XSTS 인증 | userhash(`uhs`) 추출, 오류코드 안내 (`2148916233` 계정 없음 / `2148916238` 미성년자) |
| L4-5 | Minecraft 로그인 | `login_with_xbox` 토큰 교환 + 게임 소유 여부 확인 |
| L4-6 | 프로필 조회 | UUID, 닉네임, 스킨 URL |
| L4-7 | 토큰 체인 캐시 | 리프레시 토큰을 Windows DPAPI로 암호화 저장. meteor의 `getUpToDate()`처럼 만료 시 체인 단계별 자동 갱신 |
| L4-8 | 로그인 UI 연결 | L2-1 화면 바인딩, 코드 표시 + 브라우저 자동 열기, 로그아웃, 세션 자동 복구 |

### Rust 크레이트 후보

- HTTP: `reqwest` (인스톨러와 동일)
- JSON: `serde` / `serde_json`
- DPAPI: `windows` crate의 `CryptProtectData` / `CryptUnprotectData`
- 기존 Rust 구현 참고용: `minecraft-msa-auth` (동작 확인 후 직접 구현과 비교 결정)

## Phase 5 — 파일 / 버전 관리

| ID | 작업 | 내용 |
| --- | --- | --- |
| L5-1 | 런처 매니페스트 스키마 | `launcher-manifest.json` — 런처 버전, 모드 목록, 추가 파일, size + SHA-256 |
| L5-2 | Minecraft 1.20.4 설치 | version.json, 클라이언트 jar, 라이브러리, natives |
| L5-3 | 에셋 인덱스 | asset index 파싱 + `objects/` 다운로드 |
| L5-4 | Fabric 로더 설치 | Fabric Meta API → 로더 버전 고정, 라이브러리 병합 |
| L5-5 | 병렬 다운로더 | 동시성 제한, 재시도, 진행률 이벤트 (인스톨러 `download.rs` 재사용) |
| L5-6 | 무결성 검증 / 복구 | 크기 + SHA-256 검사, 불일치 시 재다운로드 |

## Phase 6 — 모드 관리

| ID | 작업 | 내용 |
| --- | --- | --- |
| L6-1 | 모드 모델 | 필수/선택 구분, `enabled` 상태, 버전 |
| L6-2 | RendogClient 설치 | `%APPDATA%\RendogLauncher\minecraft\mods\RendogClient-Delta.jar` 배치 및 버전 갱신 (인스톨러가 먼저 설치한 파일과 SHA-256 일치 확인) |
| L6-3 | ON/OFF 적용 | `mods/` ↔ `mods-disabled/` 이동 또는 `.disabled` 접미사 방식 |
| L6-4 | 모드 화면 바인딩 | L2-3 연결, 필수 모드 비활성화 차단 |

## Phase 7 — Java 런타임 및 실행

| ID | 작업 | 내용 |
| --- | --- | --- |
| L7-1 | Java 탐지 | 번들 런타임 → 설치 디렉터리 → 시스템 `JAVA_HOME`/PATH 순 |
| L7-2 | 런타임 확보 | Java 21 미존재 시 Adoptium API로 다운로드 후 `runtime/`에 전개 |
| L7-3 | JVM 인자 빌더 | 힙(`-Xmx`), GC 옵션, `-Djava.library.path`, classpath, log4j 설정 |
| L7-4 | Minecraft 인자 빌더 | `accessToken`, `uuid`, `username`, `assetsDir`, `versionType`, 해상도 |
| L7-5 | 프로세스 실행 | 작업 디렉터리 설정, stdout/stderr 로그 수집, 실행 후 런처 종료 정책 |
| L7-6 | 실행 실패 처리 | 조기 종료 감지, exit code + 로그 요약을 오류 화면에 연결 |

## Phase 8 — 서버 자동 접속 연동

| ID | 작업 | 내용 |
| --- | --- | --- |
| L8-1 | 자동 접속 방식 확정 | `--quickPlayMultiplayer rendog.kr` vs RendogClient 모드 설정 파일 |
| L8-2 | 모드 설정 파일 생성 | 서버 주소, 자동 종료 정책 등 런처가 기록 |
| L8-3 | 서버 상태 조회 | Server List Ping으로 온라인 여부/접속자 수 → 홈 화면 표시 |

## Phase 9 — 런처 자체 업데이트

| ID | 작업 | 내용 |
| --- | --- | --- |
| L9-1 | 버전 체크 | 원격 매니페스트의 런처 버전 비교, 업데이트 알림 |
| L9-2 | 자기 교체 구현 | 신규 exe 다운로드 → 검증 → 교체 스크립트 또는 사이드바이사이드 재실행 |

## Phase 10 — 패키징 / 인스톨러 연동

| ID | 작업 | 내용 |
| --- | --- | --- |
| L10-1 | 릴리즈 빌드 | `lto`, `codegen-units=1`, `strip` 프로파일로 `RendogLauncher.exe` 산출 |
| L10-2 | 인스톨러 매니페스트 갱신 | `installer/manifest.json`의 `RendogLauncher.exe`를 `pending` → `ready`로 전환 (URL, size, SHA-256) |
| L10-3 | 배포 문서화 | 릴리즈 절차, 매니페스트 갱신 체크리스트 |

## Phase 11 — 검증

| ID | 작업 | 내용 |
| --- | --- | --- |
| L11-1 | 단위 테스트 | 매니페스트 파싱, 인자 빌더, 해시 검증, 경로 로직 |
| L11-2 | E2E 체크리스트 | 클린 PC: 설치 → 로그인 → 다운로드 → 실행 → 자동 접속 → 종료 |
| L11-3 | 문서 갱신 | `README.md`, `launcher/README.md`, `workflow.md` 현재 구현 상태 반영 |

---

## 의존 관계

```text
L0 ──> L1 ──> L2
 │
 └──> L3 ──┬──> L4 ──┐
           ├──> L5 ──┼──> L7 ──> L8
           └──> L6 ──┘
L5 ──> L9
L7 ──> L10 ──> L11
```

- L2(화면)는 L1(디자인 시스템) 완료 후 진행합니다.
- L7(실행)은 L4(토큰), L5(파일), L6(모드)가 모두 갖춰져야 동작합니다.
- L10-2는 L10-1 완료 후에만 인스톨러 브랜치에서 수행합니다.

## 권장 착수 순서

1. `L0-1` Cargo + Slint 스캐폴딩
2. `L1-1` Figma 디자인 토큰 추출
3. `L1-3` 커스텀 타이틀바
4. `L1-4` 앱 셸 + 라우팅
5. `L2-1` 로그인 화면

## 커밋 메시지 예시

```text
feat: 런처 Cargo 프로젝트와 Slint UI 스캐폴딩 추가
feat: Figma 기반 런처 디자인 토큰 정의
feat: Microsoft 디바이스 코드 로그인 흐름 구현
```
