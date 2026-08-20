# Rendog Launcher 구현 작업 분해

Rendog Launcher(Rust + Slint)를 `RendogLauncher.exe`로 완성하기 위한 작업 분해 문서입니다.
`workflow.md`의 원칙에 따라 각 작업(`L#-#`)은 한 번에 하나씩 완료합니다.

## 전제 조건

| 항목 | 값 |
| --- | --- |
| 구현 언어 | Rust (UI: Slint 1.8) |
| 산출물 | `RendogLauncher.exe` |
| 창 크기 | 1000 × 640 (frameless, 리사이즈 없음) |
| Minecraft 버전 | `1.20.4` (고정) |
| 모드 로더 | Fabric (RendogClient 요구사항) |
| 서버 | `rendog.kr` |
| Java | 21 |
| 로그인 | Microsoft Title Auth (Device Code) — Phase 4 참고 |
| 데이터 경로 | `%APPDATA%\RendogLauncher` |
| 설치 경로 | `%ProgramFiles%\Rendog Launcher` (읽기 전용 취급) |
| 디자인 | Figma `Rendog-Launcher` / `launcher` 페이지 (`node-id=25-2`) |

## 확정된 결정 사항

- [x] **Azure AD Client ID 불필요** — Title Auth 방식 채택 (Phase 4)
- [x] **데이터 디렉터리** — `%APPDATA%\RendogLauncher`, 인스톨러 반영 완료
- [x] **런처 매니페스트 호스팅** — GitHub (`foliq/Rendog-Launcher` 릴리즈 자산)
- [x] **Java 21 런타임** — 시스템에 없으면 번들 자동 다운로드 (Adoptium)

### 쓰기 권한 (해결됨)

| 루트 | 경로 | 내용 | 관리자 권한 |
| --- | --- | --- | --- |
| install | `%ProgramFiles%\Rendog Launcher` | `RendogLauncher.exe`, 언인스톨러 | 필요 |
| data | `%APPDATA%\RendogLauncher` | `minecraft\` (게임 파일, 모드, 설정, 로그) | 불필요 |

런처는 설치 디렉터리에 쓰지 않습니다 (L9 자체 업데이트만 예외).

---

## 디자인에서 확인된 구조

Figma에는 **전체 화면 전환이 없습니다.** 단일 메인 화면 위에 팝오버와 모달이 얹히는 구조입니다.

```text
창 (1000×640, frameless)
├── 타이틀바 44px          "Rendog Launcher" + [–] [×]
├── 상단 바                프로필 칩(좌) · [Settings](우)
│   └── 프로필 드롭다운     240px 팝오버 (3가지 상태)
├── 히어로 영역            로고 132 · RENDOG LAUNCHER · rendog.kr · Minecraft 1.20.4
│                          [시작] 140×49 · 상태 힌트 문구
├── 푸터                   v1.0.0
└── 설정 모달              460×560 오버레이
```

### 프로필 드롭다운 3가지 상태

| 상태 | 높이 | 내용 |
| --- | --- | --- |
| 미로그인 | 85 | "로그인된 계정이 없어요" + `[로그인]` 버튼 |
| 계정 1개 | 159 | 현재 계정(로그인됨) / `계정 추가` / `로그아웃` |
| 계정 2개+ | 214 | 현재 계정 / 다른 계정 목록(클릭 시 전환) / `계정 추가` / `로그아웃` |

### 설정 모달 구성

| 섹션 | 항목 |
| --- | --- |
| 렌더링 | 목표 FPS 슬라이더 (30–150, 현재값 툴팁) + 설명문 |
| 모드 | `RendogClient` (필수 뱃지, 토글 없음) |
| | `적응형 렌더링` (토글) |
| | 로컬 모드 목록 (파일명 + `삭제` + 토글, 설명 없으면 `(None)`) |
| | `+ 로컬에서 모드 추가` (점선 버튼) |
| 프로그램 디렉토리 | 읽기 전용 경로 필드 + `[열기]` |

> 디자인에 RAM·JVM 인자·해상도·Java 경로 설정이 **없습니다.** 런처가 자동 결정합니다.

### 디자인에 없는 부분 (확정됨)

| 항목 | 결정 |
| --- | --- |
| 다운로드/검증 진행 표시 | **`시작` 버튼을 진행률로 전환.** 140×49 자리를 유지하고 accent로 채우며 퍼센트 표시, 단계 문구는 하단 힌트 영역 |
| 오류 표시 | **별도 오류 모달.** 설정 모달과 같은 시각 언어, `다시 시도` / `닫기` |
| Device Code 로그인 | **코드가 채워진 페이지를 기본 브라우저로 열기.** 인앱 코드 입력 UI 불필요 |

Device Code URL은 meteor가 쓰는 `MinecraftAuth`의 `MsaDeviceCode.getDirectVerificationUri()`와 동일하게 만듭니다.

```text
verificationUri + "?otc=" + userCode
예) https://www.microsoft.com/link?otc=ABCD1234
```

런처는 이 URL을 열고, 폴링하는 동안 `시작` 버튼을 진행 상태로 두고 힌트에 안내를 띄웁니다.

---

## Phase 0 — 프로젝트 기반

| ID | 작업 | 산출물 |
| --- | --- | --- |
| L0-1 | Cargo + Slint 스캐폴딩 | `Cargo.toml`, `build.rs`, `src/main.rs`, `ui/launcher.slint`, `.gitignore` |
| L0-2 | Windows 리소스 | 아이콘 `.ico`, `winresource` 제품 메타데이터 |
| L0-3 | 에셋 이전 | 인스톨러 `assets/` 로고 세트 재사용 |
| L0-4 | 공통 골격 모듈 | `error.rs`, `paths.rs`, `logging.rs` |

## Phase 1 — 디자인 시스템 & 셸

| ID | 작업 | 내용 |
| --- | --- | --- |
| L1-1 | 디자인 토큰 | Figma에서 색상/타이포/간격/라운드 추출 → `ui/theme.slint` (Figma 변수 미정의라 실측값 사용) |
| L1-2 | 타이틀바 | frameless 1000×640, 드래그 이동, `–`/`×` (최대화 없음) — 인스톨러 구현 이식 |
| L1-3 | 기본 컴포넌트 | 버튼(primary/secondary/ghost), 토글 스위치, 뱃지, 구분선, 읽기 전용 인풋 |
| L1-4 | 슬라이더 컴포넌트 | 값 툴팁 + 눈금 라벨(30/60/90/120/150) |
| L1-5 | 팝오버 / 모달 | 바깥 클릭 닫기, `Esc` 닫기, 그림자 오버레이 |
| L1-6 | 앱 셸 | 상단 바 + 히어로 + 푸터 레이아웃, 전역 상태 프로퍼티 정의 |

## Phase 2 — 화면 구현 (디자인 1:1)

| ID | 작업 | 내용 |
| --- | --- | --- |
| L2-1 | 히어로 영역 | 로고, 타이틀, 서버/버전 라벨, `시작` 버튼, 상태 힌트 문구 |
| L2-2 | 프로필 칩 | 아바타 이니셜, 닉네임/`No profile`, `▾` |
| L2-3 | 프로필 드롭다운 | 3가지 상태 전부 (미로그인 / 1개 / 2개+) |
| L2-4 | 설정 모달 셸 | 헤더 + 스크롤 영역 + 프로그램 디렉토리 푸터 |
| L2-5 | 렌더링 섹션 | 목표 FPS 슬라이더 바인딩 |
| L2-6 | 모드 섹션 | 필수/토글/로컬 모드 행, `+ 로컬에서 모드 추가` |
| L2-7 | 진행·오류 표현 | 위 "결정이 필요한 것" 확정 후 구현 |

## Phase 3 — 코어 인프라

| ID | 작업 | 내용 |
| --- | --- | --- |
| L3-1 | 디렉터리 부트스트랩 | `%APPDATA%\RendogLauncher` 하위 `minecraft/`, `runtime/`, `cache/`, `logs/` |
| L3-2 | 설정 저장/로드 | `config.json` — 목표 FPS, 적응형 렌더링, 모드 상태, 선택 계정. 원자적 쓰기 |
| L3-3 | 로깅 | 파일 롤링 로그 + 오류 표시용 컨텍스트 |
| L3-4 | 백그라운드 워커 ↔ UI 브리지 | 인스톨러 `progress.rs` 이벤트 모델 이식, `slint::invoke_from_event_loop` |
| L3-5 | HTTP 클라이언트 | `reqwest` 공통 설정, 타임아웃, 재시도, User-Agent |

## Phase 4 — Microsoft 인증 (멀티 계정)

레퍼런스: [meteor-client `MicrosoftLogin.java`](https://github.com/MeteorDevelopment/meteor-client/blob/master/src/main/java/meteordevelopment/meteorclient/systems/accounts/MicrosoftLogin.java)
→ [RaphiMC/MinecraftAuth](https://github.com/RaphiMC/MinecraftAuth)의 `JavaAuthManager` + `DeviceCodeMsaAuthService` 사용.
런처는 Rust이므로 **같은 흐름을 Rust로 이식**합니다.

### 채택 방식: Title Auth (Device Code) — Azure 앱 등록 불필요

| 항목 | 값 |
| --- | --- |
| Client ID | `00000000402b5328` (`MsaConstants.JAVA_TITLE_ID`) |
| Scope | `service::user.auth.xboxlive.com::MBI_SSL` (`SCOPE_TITLE_AUTH`) |
| Device code 발급 | `https://login.live.com/oauth20_connect.srf` |
| 토큰 발급/갱신 | `https://login.live.com/oauth20_token.srf` |
| 디바이스 토큰 | `https://device.auth.xboxlive.com/device/authenticate` |
| SISU (user+title+XSTS) | `https://sisu.xboxlive.com/authorize` (`rp://api.minecraftservices.com/`) |
| Minecraft 토큰 | `https://api.minecraftservices.com/launcher/login` (`MinecraftLauncherLoginRequest`) |
| 소유 확인 | `https://api.minecraftservices.com/entitlements/mcstore` |
| 프로필 | `https://api.minecraftservices.com/minecraft/profile` |

> Title auth는 v2.0이 아니라 `login.live.com` 레거시 엔드포인트를 씁니다.
> `offline_access` 없이도 리프레시 토큰이 발급됩니다.

### 작업

| ID | 작업 | 내용 |
| --- | --- | --- |
| L4-1 | Device Code 발급 | user code / verification URI 획득, 기본 브라우저 열기 |
| L4-2 | 토큰 폴링 | `interval` 준수, `authorization_pending`/`expired_token`/`authorization_declined` 구분, 취소 지원 |
| L4-3 | Xbox 요청 서명 | ECDSA P-256 키쌍, `ProofKey`(JWK), `Signature` 헤더 (`SignedXblPostRequest` 이식) |
| L4-4 | 디바이스 토큰 | `device.auth.xboxlive.com/device/authenticate`, `AuthMethod: ProofOfPossession` |
| L4-5 | SISU 인증 | `sisu.xboxlive.com/authorize` → UserToken / TitleToken / AuthorizationToken(XSTS). XErr 안내 (`2148916233` 계정 없음 / `2148916238` 미성년자) |
| L4-6 | Minecraft 로그인 | `launcher/login` 교환 (`platform: PC_LAUNCHER`, `xtoken: XBL3.0 x=uhs;token`) + `entitlements/mcstore` 소유 확인 |
| L4-7 | 프로필 조회 | UUID, 닉네임, 스킨 URL |
| L4-7b | 스킨 헤드 아바타 | 스킨 PNG(64×64)에서 얼굴 `(8,8)-(16,16)`과 모자 레이어 `(40,8)-(48,16)`를 합성해 8×8 헤드 생성 → 캐시. 로드 전에는 이니셜 표시 |
| L4-8 | 계정 저장소 | **여러 계정** 리프레시 토큰 + 디바이스 키쌍/ID를 DPAPI로 암호화 저장 |
| L4-9 | 토큰 자동 갱신 | meteor `getUpToDate()`처럼 체인 단계별 만료 시 갱신 |
| L4-10 | UI 연결 | 드롭다운 로그인/계정 추가/계정 전환/로그아웃 (L2-3 바인딩) |

> **title client id는 SISU 경로를 씁니다.** `JavaAuthManager.refreshJavaXstsToken()` 이
> `isTitleClientId()` 로 분기하며, title 이면 `user/authenticate` + `xsts/authorize` 대신
> `sisu.xboxlive.com/authorize` 를 호출합니다. SISU 는 디바이스 토큰과 ECDSA P-256 으로
> 서명된 요청을 요구합니다. 단순 경로는 Azure 앱 등록을 한 경우에만 해당합니다.

### Xbox 요청 서명 형식

`SignedXblPostRequest.appendSignatureHeader()` 를 그대로 옮깁니다.

```text
서명 대상 = i32be(1) ‖ 00 ‖ i64be(windowsTs) ‖ 00 ‖ METHOD ‖ 00
           ‖ path+query ‖ 00 ‖ Authorization ‖ 00 ‖ body ‖ 00
Signature = base64( i32be(1) ‖ i64be(windowsTs) ‖ ECDSA-P256-SHA256 r‖s 64B )
windowsTs = (unix초 + 11644473600) × 10^7
```

### Rust 크레이트 후보

- HTTP `reqwest` · JSON `serde`/`serde_json`
- DPAPI: `windows` crate `CryptProtectData` / `CryptUnprotectData`
- 참고 구현: `minecraft-msa-auth` (직접 구현과 비교 후 결정)

## Phase 5 — 파일 / 버전 관리

| ID | 작업 | 내용 |
| --- | --- | --- |
| L5-1 | 런처 매니페스트 스키마 | `launcher-manifest.json` — 런처 버전, 필수 모드, 추가 파일, size + SHA-256. **GitHub 릴리즈 자산으로 호스팅** |
| L5-2 | Minecraft 1.20.4 설치 | version.json, 클라이언트 jar, 라이브러리, natives |
| L5-3 | 에셋 인덱스 | asset index 파싱 + `objects/` 다운로드 |
| L5-4 | Fabric 로더 설치 | Fabric Meta API → 로더 버전 고정, 라이브러리 병합 |
| L5-5 | 병렬 다운로더 | 동시성 제한, 재시도, 진행률 이벤트 (인스톨러 `download.rs` 재사용) |
| L5-6 | 무결성 검증 / 복구 | 크기 + SHA-256, 불일치 시 재다운로드 |

## Phase 6 — 모드 관리

| ID | 작업 | 내용 |
| --- | --- | --- |
| L6-1 | 모드 모델 | 필수(`RendogClient`) / 내장 기능(`적응형 렌더링`) / 로컬 추가 모드 3분류 |
| L6-2 | RendogClient 동기화 | 인스톨러가 배치한 jar의 SHA-256 확인, 매니페스트와 다르면 갱신 |
| L6-3 | ON/OFF 적용 | `mods/` ↔ `mods-disabled/` 이동, 필수 모드는 비활성화 차단 |
| L6-4 | 로컬 모드 추가 | 파일 선택 → `mods/`로 복사, `fabric.mod.json`에서 설명 추출 (없으면 `(None)`) |
| L6-5 | 로컬 모드 삭제 | 파일 제거 + 목록/설정 갱신 |
| L6-6 | UI 바인딩 | L2-6 연결 |

## Phase 7 — Java 런타임 및 실행

| ID | 작업 | 내용 |
| --- | --- | --- |
| L7-1 | Java 탐지 | `%APPDATA%\RendogLauncher\runtime` → 시스템 `JAVA_HOME`/PATH 순으로 Java 21 탐색 |
| L7-2 | 런타임 자동 확보 | **Java 21이 없으면 Adoptium(Temurin) JRE 21 자동 다운로드 → `runtime/`에 전개 → SHA-256 검증.** 진행률은 시작 흐름에 표시 |
| L7-3 | JVM 인자 빌더 | 힙 자동 산정(시스템 RAM 기반), GC 옵션, `-Djava.library.path`, classpath |
| L7-4 | Minecraft 인자 빌더 | `accessToken`, `uuid`, `username`, `assetsDir`, `versionType` |
| L7-5 | 프로세스 실행 | 작업 디렉터리 `%APPDATA%\RendogLauncher\minecraft`, 로그 수집, 실행 후 런처 종료 |
| L7-6 | 실행 실패 처리 | 조기 종료 감지, exit code + 로그 요약 표시 |

## Phase 8 — 서버 자동 접속 & 렌더링 정책 전달

| ID | 작업 | 내용 |
| --- | --- | --- |
| L8-1 | 자동 접속 방식 확정 | `--quickPlayMultiplayer rendog.kr` vs RendogClient 설정 파일 |
| L8-2 | 모드 설정 파일 생성 | 서버 주소 + **목표 FPS** + **적응형 렌더링 ON/OFF**를 RendogClient가 읽을 형식으로 기록 |
| L8-3 | 설정 ↔ 모드 계약 정의 | 파일 위치·스키마를 RendogClient 쪽과 합의 (모드 수정 필요 여부 확인) |

## Phase 9 — 런처 자체 업데이트

| ID | 작업 | 내용 |
| --- | --- | --- |
| L9-1 | 버전 체크 | 원격 매니페스트와 `v1.0.0` 비교, 푸터/힌트에 알림 |
| L9-2 | 자기 교체 | 설치 디렉터리 쓰기는 관리자 권한 필요 → UAC 승격 후 교체 (인스톨러 `elevation.rs` 방식 재사용) |

## Phase 10 — 패키징 / 인스톨러 연동

| ID | 작업 | 내용 |
| --- | --- | --- |
| L10-1 | 릴리즈 빌드 | `lto`, `codegen-units=1`, `strip` → `RendogLauncher.exe` |
| L10-2 | 인스톨러 매니페스트 갱신 | `rendog-launcher` 컴포넌트를 `pending` → `ready` (url/size/sha256, `targetRoot: install`) |
| L10-3 | 배포 문서화 | 릴리즈 절차, 매니페스트 갱신 체크리스트 |

## Phase 11 — 검증

| ID | 작업 | 내용 |
| --- | --- | --- |
| L11-1 | 단위 테스트 | 매니페스트 파싱, 인자 빌더, 해시 검증, 경로 로직, 토큰 만료 판정 |
| L11-2 | E2E 체크리스트 | 클린 PC: 설치 → 로그인 → 계정 추가/전환 → 다운로드 → 실행 → 자동 접속 → 종료 |
| L11-3 | 문서 갱신 | `README.md`, `launcher/README.md`, `workflow.md` 반영 |

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

- L2-7(진행·오류 표현)은 미결정 항목 확정 후 착수합니다.
- L7(실행)은 L4(토큰) · L5(파일) · L6(모드)가 모두 갖춰져야 동작합니다.
- L8-3은 RendogClient 모드 쪽 확인이 필요해 병렬로 미리 진행할 수 있습니다.
- L10-2는 L10-1 완료 후 `installer` 브랜치에서 수행합니다.

## 권장 착수 순서

1. `L0-1` Cargo + Slint 스캐폴딩
2. `L1-1` 디자인 토큰
3. `L1-2` 타이틀바
4. `L1-6` 앱 셸
5. `L2-1` 히어로 영역
6. `L2-2` / `L2-3` 프로필 칩 + 드롭다운

## Phase 12 — 적응형 렌더러 (`mod/`)

Phase 8은 목표 FPS와 적응형 렌더링 ON/OFF를 **전달**만 한다. 실제로 그 목표를 지키는 일은 여기서 한다. 전부 런처가 소유한 `mod/` 안에서 Java 21 + GLSL로 구현한다.

원칙: **화질을 깎아서 FPS를 산다.** 어떤 단계에서도 게임이 꺼지거나 화면이 깨지면 안 되므로, 모든 단계는 바닐라 경로로 되돌아갈 수 있어야 한다.

| ID | 작업 | 내용 |
| --- | --- | --- |
| L12-1 | 설정 로더 | `config/rendoglauncher.json` 읽기. 없거나 깨졌거나 모르는 `schemaVersion`이면 **기본값으로 진행**(실패 금지) |
| L12-2 | 프레임 측정기 | 프레임 시간 링 버퍼 → 평균 FPS + 1% low. 창 전환·로딩으로 생긴 긴 공백 프레임은 표본에서 제외 |
| L12-3 | 품질 단계 정의 | 0(최고)~N(최저) 단계로 렌더 거리 · 엔티티 거리 · 파티클 · 밉맵 · 구름 · 생물군계 블렌드를 묶음 |
| L12-4 | 정책 엔진 | 목표 FPS 대비 상·하향 판정. **히스테리시스 + 쿨다운**으로 단계가 왕복 진동하지 않게 |
| L12-5 | Java 옵션 적용 | `GameOptions` 반영. 렌더 거리 변경은 청크 재빌드를 유발하므로 하향은 즉시, 상향은 보수적으로 |
| L12-6 | 내장 리소스팩 등록 | `ResourceManagerHelper.registerBuiltinResourcePack`으로 코어 셰이더 오버라이드 슬롯 확보 |
| L12-7 | 코어 셰이더 변형 | `rendertype_*` 셰이더에 품질 스케일 유니폼 도입 (안개 거리, 라이트맵 정밀도 등) |
| L12-8 | 유니폼 배선 | 정책 단계 → 셰이더 유니폼 전달, 프레임당 1회 갱신 |
| L12-9 | 안전장치 | 셰이더 컴파일 실패·드라이버 이상 감지 시 리소스팩을 끄고 바닐라 경로로 폴백, 이후 재시도 안 함 |
| L12-10 | 검증 | 프레임 지연 주입으로 저사양 시뮬레이션, 단계 전환 로그, 목표 FPS별 수렴 확인 |

`adaptiveRendering`이 `false`면 L12-3 이후는 전부 건너뛰고 사용자가 게임 안에서 고른 설정을 그대로 둔다.

## 커밋 메시지 예시

```text
feat: 런처 Cargo 프로젝트와 Slint UI 스캐폴딩 추가
feat: Figma 기반 런처 디자인 토큰 정의
feat: 프로필 드롭다운 3가지 상태 구현
feat: Microsoft 디바이스 코드 로그인 흐름 구현
```
