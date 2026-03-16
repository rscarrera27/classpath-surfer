# classpath-surfer

Gradle Java/Kotlin 프로젝트의 의존성 심볼을 인덱싱하고 검색하는 Rust CLI 도구.

## Project Status

- 알파 단계 — API, CLI 인터페이스, 내부 구조, 기술 스택 모두 자유롭게 변경 가능
- 하위 호환성 보장 불필요, 과감한 리팩터링 환영

## Build & Test

### Prerequisites

`protoc` (Protocol Buffers compiler) — Kotlin metadata proto 컴파일에 필요:
```bash
# macOS
brew install protobuf
# Ubuntu/Debian
sudo apt install protobuf-compiler
# 확인
protoc --version
```

### Commands

```bash
cargo build                        # 빌드 (proto 자동 컴파일 포함)
cargo test --test unit             # 유닛 테스트 (JDK 불필요)
cargo test --test integration \
  --test integration_mutation \
  --test cli                       # 인테그레이션 + E2E (JDK 21 필요)
cargo test --test matrix \
  -- --test-threads=1              # JDK/Gradle 호환 매트릭스
cargo clippy                       # 린트
cargo fmt -- --check               # 포맷 검사
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps  # 문서 빌드 검증
```

## Architecture

- Gradle init script(`src/gradle/init_script.rs`)로 classpath를 추출하고, `cafebabe` 크레이트로 JAR 내 `.class` 파일을 파싱하여 심볼을 추출한 뒤, Tantivy 인덱스에 저장한다.
- 증분 인덱싱: 이전 매니페스트와 diff를 계산하여 변경된 GAV만 재인덱싱.
- staleness 검사: lockfile 해시 + 빌드 파일 mtime으로 인덱스 유효성 판단.
- OutputMode 기반 인터페이스 분리: `--agentic` 플래그로 구조화 JSON 출력, TTY에서는 ratatui TUI, 비-TTY에서는 Plain 텍스트 출력을 자동 선택.

## Code Style

- Rust edition 2024, MSRV 1.94
- `anyhow::Result`를 모든 fallible 함수의 반환 타입으로 사용
- 사용자 대면 메시지는 `eprintln!`, 데이터 출력은 `println!` (stdout/stderr 분리)
- CLI 파싱에 clap derive 매크로 사용 (`ValueEnum`으로 열거형 옵션 검증)
- `#![deny(missing_docs)]` 적용 — 모든 `pub` 아이템에 `///` 문서 주석 필수
- CLI 옵션 중 Config 파일로도 설정 가능한 것(`--decompiler`, `--configurations`, `--no-decompile`)은 `Option<T>`로 선언하고 `main.rs`에서 Config 값과 merge

## CLI Design Rules

이 도구의 주 사용자는 AI 에이전트(Claude Code)다. 아래 규칙을 따라 에이전트 친화적 CLI를 유지한다.

### 출력 모드 (OutputMode)
- `--agentic` → 구조화 JSON (stdout), 에이전트/스크립트용
- TTY → ratatui TUI, 사람 인터랙티브용
- 비-TTY → Plain 텍스트, 파이프/리다이렉트용
- 데이터는 stdout, 진행/경고/메시지는 stderr — 예외 없음

### 구조화 출력
- 모든 커맨드는 `XxxOutput` 구조체를 반환하고, `Serialize`를 derive
- JSON 필드명은 snake_case, 타입 일관성 유지
- 에러 시 `{"success": false, "error_code": "...", "error": "..."}`  형태로 출력
- 스트리밍이 필요하면 JSON Lines (JSONL) 사용

### Exit code
- 0 = 성공
- 1 = 일반 실패
- 2 = 사용법 오류 (잘못된 인자)
- 3 = 리소스 없음 (인덱스 없음 등)
- 에러 종류별 exit code를 구분하여 에이전트가 분기할 수 있게 한다

### 에러 메시지
- 실패한 입력을 에코백
- 구체적 다음 행동을 제안 (예: "Run `classpath-surfer index refresh` to build the index.")
- 일시적 에러와 영구적 에러를 구분

### 컨텍스트 효율성
- CLI 출력은 간결하게 — LLM 컨텍스트 윈도우 소비 최소화
- 장식(프로그레스 바, 아스키 아트)은 TUI/stderr로만
- JSON 출력은 flat 구조 선호, 깊은 중첩 회피

### 멱등성
- 같은 명령을 여러 번 실행해도 동일한 결과를 보장 (가능한 한)
- 멱등하지 않은 경우 충돌을 명확히 알린다

## Key Paths

- `proto/kotlin_metadata.proto` — Kotlin 메타데이터 protobuf 스키마 (prost-build로 자동 생성)
- `build.rs` — prost-build 설정 (proto 컴파일)
- `scripts/sync-kotlin-proto.sh` — upstream Kotlin proto 동기화 도우미
- `src/cli/` — 서브커맨드 핸들러 (`search {symbol|dep|pkg}`, `show`, `index {init|refresh|status|clean}`)
- `src/cli/render.rs` — Plain 텍스트 렌더러
- `src/error.rs` — 분류된 CLI 에러 타입 (`CliError`: exit code, error_code, retryable)
- `src/gradle/` — Gradle init script 및 실행기
- `src/index/` — Tantivy 스키마 정의, 읽기/쓰기
- `src/manifest/` — classpath 매니페스트 모델, 병합, 차분
- `src/output.rs` — 출력 모드 enum (`Agentic`/`TUI`/`Plain`), JSON emit 헬퍼
- `src/model/mod.rs` — 도메인 타입 (`SymbolKind`, `AccessLevel`, `SearchResult` 등 — `ValueEnum` derive 포함)
- `src/parser/` — JAR, classfile, descriptor 파싱, Kotlin 메타데이터 디코딩
- `.claude-plugin/` — Claude Code 플러그인 매니페스트 (`plugin.json`, `marketplace.json`)
- `agents/` — Claude Code 에이전트 정의 (`find-symbol`, `show-source`, `list-deps`)
- `skills/` — Claude Code 스킬 정의 (`find-symbol`, `show-source`, `list-deps`, `manage-index`)
- `src/source/decompiler.rs` — `Decompiler` enum (`Cfr`/`Vineflower`) 및 디컴파일 실행
- `src/staleness/` — 인덱스 변경 감지 (lockfile, buildfile)
- `src/tui/` — ratatui 기반 인터랙티브 TUI 렌더러 (search, show, status)
- `tests/common/mod.rs` — 테스트 공유 인프라 (LazyLock 공유 인덱스, 헬퍼, 매크로)
- `tests/unit.rs` — JDK 불필요 유닛 테스트
- `tests/integration.rs` — 공유 인덱스 기반 읽기 전용 인테그레이션 테스트
- `tests/integration_mutation.rs` — 프로젝트 수정이 필요한 인테그레이션 테스트
- `tests/matrix.rs` — JDK/Gradle 호환성 매트릭스 테스트
- `tests/cli.rs` — CLI 바이너리 서브프로세스 E2E 테스트

## Conventions

- 새 서브커맨드 추가 시: `src/cli/`에 모듈 생성 → `Commands` enum에 variant 추가 → `main.rs`에서 매칭
- 새 서브커맨드 추가 시: handler는 `Result<XxxOutput>`을 반환하고, `src/cli/render.rs`에 Plain 렌더러, `src/tui/`에 TUI 렌더러(필요 시), `main.rs`에 OutputMode 분기를 추가
- 인덱스 스키마 변경 시: `src/index/schema.rs` 수정 후 reader/writer 동기화 필수
- 커밋 전 `cargo clippy && cargo fmt -- --check` 통과 확인
- 사용자 대면 문서(README, CONTRIBUTING, GitHub 템플릿 등)는 영어와 한국어를 공동 제1언어로 유지
- CHANGELOG 파일 없음 — 릴리즈 노트는 GitHub Releases로 관리
