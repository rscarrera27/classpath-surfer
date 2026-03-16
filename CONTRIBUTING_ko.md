# classpath-surfer 기여 가이드

classpath-surfer에 관심을 가져주셔서 감사합니다! 이 가이드는 기여를 시작하는 데 필요한 정보를 제공합니다.

## 개발 환경 설정

- **Rust 1.94+** (MSRV) — [rustup](https://rustup.rs/)으로 설치
- **protoc** (Protocol Buffers 컴파일러) — 빌드 시 필요 (`brew install protobuf` / `apt install protobuf-compiler`)
- **JDK** (선택) — Gradle을 호출하는 E2E 테스트 실행 시 필요
- **[mise](https://mise.jdx.dev/)** (선택) — 자동 툴체인 관리

## 빌드 & 테스트

```bash
cargo build              # 빌드 (proto 자동 컴파일 포함)
cargo clippy             # 린트
cargo fmt -- --check     # 포맷 검사
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps  # 문서 빌드 검증
```

### 테스트 실행

```bash
# 유닛 + 순수 로직 (JDK 불필요, 모든 환경)
cargo test --test unit

# 인테그레이션 (JDK 21 필요)
cargo test --test integration --test integration_mutation --test cli

# JDK/Gradle 호환성 매트릭스 (CI에서 자동 실행)
JAVA_17_HOME=/path/to/jdk17 cargo test --test matrix -- --test-threads=1
```

PR을 제출하기 전에 모든 검사가 통과해야 합니다.

## 아키텍처 개요

classpath-surfer는 파이프라인 아키텍처를 따릅니다:

1. **Gradle init script** (`src/gradle/`) — 대상 프로젝트의 Gradle 빌드에 주입되어 전체 classpath(GAV 좌표와 JAR 경로를 포함한 모든 해결된 의존성)를 추출합니다.
2. **Classpath 추출** (`src/manifest/`) — 추출된 classpath는 매니페스트로 모델링됩니다. GAV 수준에서 이전 매니페스트와 diff를 계산하여 증분 업데이트를 수행합니다.
3. **JAR 파싱** (`src/parser/`) — 각 JAR를 스캔하고, `cafebabe` 크레이트를 사용하여 `.class` 파일을 파싱한 뒤 클래스명, 메서드 시그니처, 필드 선언을 추출합니다. Kotlin 클래스의 경우 `@kotlin.Metadata` 어노테이션을 protobuf(prost)로 디코딩하여 Kotlin 네이티브 시그니처를 생성합니다. `SourceFile` 속성으로 소스 언어를 판별합니다.
4. **Tantivy 인덱싱** (`src/index/`) — 추출된 심볼을 Tantivy 전문 검색 인덱스에 기록합니다. 텍스트, FQN(정규화된 이름), 정규식 쿼리를 지원합니다.
5. **소스 해석** (`src/source/`) — 소스 JAR이 있으면 원본 소스를 사용하고, 없으면 CFR 또는 Vineflower로 즉석 디컴파일합니다.
6. **출력 렌더링** (`src/cli/`, `src/tui/`, `src/output.rs`) — `--agentic` 플래그와 TTY 감지에 따라 세 가지 출력 모드(Agentic JSON, syntect 기반 구문 강조가 적용된 인터랙티브 TUI, Plain 텍스트)를 선택합니다.
7. **변경 감지** (`src/staleness/`) — lockfile 해시와 빌드 파일 수정 시간을 인덱싱 당시의 스냅샷과 비교하여 `refresh`가 필요한지 감지합니다.
8. **에러 처리** (`src/error.rs`) — `CliError`가 분류된 종료 코드(0/1/2/3), 기계 판독 가능한 에러 코드, retryable 플래그를 제공하여 에이전트 통합을 지원합니다.

## 새 서브커맨드 추가

1. `src/cli/`에 새 모듈을 생성합니다 (예: `src/cli/my_command.rs`).
2. `src/cli/mod.rs`의 `Commands` enum에 variant를 추가합니다.
3. `src/model/output.rs`에 `#[derive(Serialize)]`로 출력 구조체 `XxxOutput`을 정의하고, 핸들러가 `Result<XxxOutput>`을 반환하도록 합니다.
4. `src/cli/render.rs`에 Plain 텍스트 렌더러를 추가합니다.
5. `src/tui/`에 TUI 렌더러를 추가합니다 (인터랙티브 표시가 필요한 경우).
6. `main.rs`에 OutputMode 분기를 추가합니다 (Agentic → JSON, TUI → ratatui, Plain → render).

## 인덱스 스키마 변경

`src/index/schema.rs`의 Tantivy 인덱스 스키마를 수정하는 경우, **반드시** reader(`src/index/reader.rs`), writer(`src/index/writer.rs`), 그리고 `src/index/compat.rs`의 `REQUIRED_FIELDS` 상수를 함께 업데이트해야 합니다.

## 코드 스타일

- **Rust edition 2024**, MSRV 1.94
- 실패 가능한 모든 함수의 반환 타입으로 `anyhow::Result`를 사용합니다.
- 사용자 대면 메시지는 **stderr** (`eprintln!`), 데이터 출력은 **stdout** (`println!`)로 분리합니다.
- CLI 인자 파싱에는 **clap derive 매크로**를 사용합니다.
- `#![deny(missing_docs)]` 적용 — 모든 `pub` 아이템에 `///` 문서 주석이 필요합니다.
- 사용자 대면 문서(README, CONTRIBUTING, GitHub 템플릿 등)는 영어와 한국어를 공동으로 유지합니다.

## PR 프로세스

1. 저장소를 포크하고 기능 브랜치를 생성합니다.
2. 변경 사항을 작성합니다.
3. 모든 검사를 통과하는지 확인합니다: `cargo fmt -- --check && cargo clippy -- -D warnings && cargo test --test unit`.
4. `master` 브랜치를 대상으로 PR을 엽니다.

## 프로젝트 구조

```
project root
├── .claude-plugin/      # Claude Code 플러그인 매니페스트 및 마켓플레이스 설정
├── agents/              # Claude Code 에이전트 정의 (find-symbol, show-source)
├── skills/              # Claude Code 스킬 정의 (SKILL.md)
├── build.rs             # Proto 컴파일 (prost-build)
├── proto/               # Kotlin 메타데이터 protobuf 스키마
├── scripts/             # 도우미 스크립트 (proto 동기화)
├── vendor/              # Kotlin 구문 정의 (syntect)
└── src/
    ├── main.rs          # CLI 진입점 (clap)
    ├── cli/             # 서브커맨드 핸들러 + Plain 텍스트 렌더러
    ├── config.rs        # .classpath-surfer/config.json
    ├── error.rs         # 분류된 CLI 에러 타입 (종료 코드, 에러 코드)
    ├── gradle/          # Init script 및 Gradle 실행기
    ├── index/           # Tantivy 스키마, 리더, 라이터
    ├── manifest/        # Classpath 매니페스트 모델, 병합, 차분
    ├── model/           # 핵심 타입 (SymbolDoc, SearchResult, SourceProvider, *Output)
    ├── output.rs        # 출력 모드 감지 (Agentic/TUI/Plain)
    ├── parser/          # JAR / .class / 디스크립터 / Kotlin 메타데이터 파싱
    ├── source/          # 소스 리졸버 및 디컴파일러 통합
    ├── staleness/       # Lockfile 및 빌드 파일 변경 감지
    └── tui/             # 인터랙티브 TUI 렌더러 (ratatui + syntect)
```

## 프로젝트 루트 파일

| 경로 | 설명 |
|------|------|
| `build.rs` | prost-build를 통한 proto 컴파일 (`cargo build` 시 자동 실행) |
| `proto/` | Kotlin 메타데이터 protobuf 스키마 (`kotlin_metadata.proto`) |
| `scripts/` | 도우미 스크립트 (예: upstream proto 동기화용 `sync-kotlin-proto.sh`) |
| `vendor/` | 벤더링된 에셋 (syntect용 Kotlin 구문 정의) |

## 종료 코드 규칙

| 코드 | 의미 |
|------|------|
| 0 | 성공 |
| 1 | 일반 실패 |
| 2 | 사용법 오류 (잘못된 인자) |
| 3 | 리소스 없음 (예: 인덱스가 존재하지 않음) |
