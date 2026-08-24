# Erkuia Launcher

Erkuia Launcher는 `erkuia.kr` 서버에 접속하기 위한 Minecraft 런처 프로젝트입니다.
일반적인 범용 Minecraft 런처가 아니라, Erkuia 서버에 필요한 클라이언트 구성, 업데이트, 실행 흐름을 가볍게 제공하는 것을 목표로 합니다.

이 루트 디렉터리는 같은 원격 저장소의 두 브랜치를 각각 별도 폴더로 체크아웃해 함께 관리하는 작업 공간입니다.

## 프로젝트 구성

| 리드미 | 역할 |
| --- | --- |
| [`launcher`](https://github.com/Erkuia/Erkuia-Launcher/tree/launcher#erkuia-launcher) | 실제 Erkuia Launcher 앱을 구현하는 프로젝트 |
| [`installer`](https://github.com/Erkuia/Erkuia-Launcher/tree/installer#erkuia-launcher-installer) | Erkuia Launcher를 설치하는 온라인 인스톨러 프로젝트 |

두 프로젝트는 하나의 제품 흐름을 나누어 담당합니다.

- `installer`는 사용자의 PC에 필요한 파일을 내려받고 검증한 뒤 설치합니다.
- `launcher`는 설치된 환경에서 Minecraft 1.21.4 클라이언트를 준비하고 Erkuia 서버 접속 흐름을 실행합니다.

## 전체 실행 흐름

```text
Erkuia-Launcher-Installer.exe
  -> 시작 화면
  -> 설치 경로 선택
  -> 설치 버튼에서 관리자 권한 요청 (UAC)
  -> 승격된 프로세스가 같은 창 위치에서 설치 단계를 이어받음
  -> 온라인 매니페스트 기준으로 파일 다운로드
  -> 파일 크기와 SHA-256 검증
  -> Erkuia Launcher 및 클라이언트 구성 파일 설치
  -> 바로가기와 제거 정보 등록
  -> 완료 화면에서 바탕화면 바로가기 / 종료 후 런처 실행 선택
  -> 종료 시 선택에 따라 Erkuia-Launcher.exe 실행

Erkuia-Launcher.exe
  -> 런처와 클라이언트 파일 상태 확인
  -> 모드 ON/OFF 상태 적용
  -> Java 런타임과 JVM 옵션 준비
  -> Minecraft 1.21.4 실행
  -> 런처 프로세스 종료
```

## 공통 대상

- Publisher / company name: `Erkuia`
- Minecraft 버전: `1.21.4`
- 서버 주소: `erkuia.kr`
- 런처 제품명: `Erkuia Launcher`
- 인스톨러 제품명: `Erkuia Launcher Installer`
- 인스톨러 실행 파일명: `Erkuia-Launcher-Installer.exe`
- 런처 실행 파일명: `Erkuia-Launcher.exe`
- 설치 방식: 온라인 설치 전용

## 기술 스택

| 영역 | 기술 | 설명 |
| --- | --- | --- |
| Installer | Rust + Slint | 네이티브 설치 UI, 다운로드, 검증, 파일 배치 |
| Launcher | Rust | 런처 UI, 업데이트, 모드 관리, JVM 실행 |
| Client mod | Java 21 | Fabric/Mixin 기반 서버 자동 접속 및 클라이언트 제어 |
| Rendering optimization | Java 21 + GLSL | FPS 목표, LOD, 셰이더 품질 조정 |
| Build / Packaging | Cargo, Gradle | Rust 실행 파일과 Minecraft 모드 빌드 |

## 브랜치 구조

이 저장소는 기능별 브랜치를 폴더 단위로 나누어 작업합니다.

- `installer/`: [`installer` 브랜치](https://github.com/Erkuia/Erkuia-Launcher/tree/installer)
- `launcher/`: [`launcher` 브랜치](https://github.com/Erkuia/Erkuia-Launcher/tree/launcher)

루트 디렉터리 자체는 두 프로젝트를 함께 보기 위한 작업 공간이며, 각 폴더 안에서 별도의 git 작업을 수행합니다.

## 현재 구현 상태

### Installer

- Rust + Slint 기반 인스톨러 UI 구현 (Figma 디자인 반영)
- 시작 / 설치 경로 / 설치 / 완료 4단계 화면과 오류 화면 구성
- `manifest.json` 기반 설치 컴포넌트 정의
- 준비, 다운로드, 검증, 파일 설치, 완료 단계의 진행률 이벤트 구조 구현
- 준비 완료 컴포넌트 다운로드 및 SHA-256 검증 흐름 구현
- 설치 시작 시 UAC 승격 후 같은 창에서 설치를 이어가는 흐름 구현
- 바탕화면 / 시작 메뉴 바로가기 생성, HKLM 제거 정보 등록 구현
- 완료 화면 옵션(바탕화면 바로가기, 종료 후 런처 실행) 적용 및 런처 실행 구현
- `Erkuia-Launcher.exe` 아티팩트는 아직 `pending` 상태라 관련 단계는 자동으로 건너뜀

### Launcher

- Erkuia 서버 런처 방향 정의
- Minecraft `1.21.4` 고정 실행 흐름 설계
- 서버 자동 접속/종료 흐름 계획
- 경량 런처, 파일 검증, 모드 관리, JVM 실행 기능 구현 예정

## 작업 원칙

작업은 한 번에 하나의 구체적인 단위로 진행합니다.

1. 요청을 하나의 명확한 작업으로 나눕니다.
2. 해당 작업만 완료합니다.
3. 완료 후 상태를 확인합니다.
4. 커밋 메시지는 한국어로 작성합니다.

커밋 메시지 형식:

```text
type: 한국어로 된 작업 내용
```

주요 타입:

- `feat`: 새 기능 또는 프로젝트 기능 추가
- `fix`: 버그 수정 또는 빌드 문제 수정
- `chore`: 저장소, 도구, 유지보수 작업
- `docs`: 문서만 변경
- `refactor`: 동작 변화 없는 코드 구조 개선
