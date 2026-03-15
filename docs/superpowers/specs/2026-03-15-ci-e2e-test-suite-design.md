# CI-호환 E2E 테스트 스위트 및 벤치마크 설계

## 목표

1. 현재 `#[ignore]`된 E2E 테스트를 GitHub Actions CI에서 실행 가능하게 만든다
2. 다양한 JVM 생태계 라이브러리(Java, Kotlin/JVM, KMP)를 대상으로 E2E 테스트를 확충한다
3. 로컬 개발 편의성(`mise` 워크플로우)을 유지한다
4. `criterion` 벤치마크로 refresh/search 성능을 측정하고 회귀를 감지한다

## JDK 경로 해석

`get_java_home(version)` 함수의 해석 우선순위:

1. `JAVA_{version}_HOME` 환경변수 (예: `JAVA_17_HOME`) — CI에서 버전별 지정
2. `JAVA_HOME` 환경변수 — 단일 JDK 환경
3. `mise where java@temurin-{version}` — 로컬 개발자용 fallback
4. 모두 실패 시 테스트 skip (조기 return + stderr 메시지)

`get_java_home`의 반환 타입을 `PathBuf`에서 `Option<PathBuf>`로 변경한다. 기존에는 JDK를 찾지 못하면 panic했으나, 이제는 `None`을 반환하고 호출자가 skip을 결정한다.

### Skip 매크로

```rust
macro_rules! require_jdk {
    ($version:expr) => {
        match get_java_home($version) {
            Some(home) => home,
            None => {
                eprintln!("JDK {} not available, skipping test", $version);
                return;
            }
        }
    };
}
```

## `#[ignore]` 제거

모든 E2E 테스트에서 `#[ignore]`를 제거한다. 대신 `require_jdk!` 매크로로 JDK가 없으면 graceful skip한다. CI에서는 `cargo test --test e2e`로 전체 실행하고, JDK가 설치되지 않은 조합은 자동 skip된다.

## CI 워크플로우 변경

> **참고**: 현재 CI는 `main` 브랜치에 트리거되지만, 기본 브랜치는 `master`이다. `e2e` job 추가 시 트리거 브랜치를 `master`로 수정하거나, 기본 브랜치를 `main`으로 변경한다.

### 기존 `check` job (변경 없음)

- OS 매트릭스: ubuntu-latest, macos-latest
- fmt → clippy → `cargo test` (비-ignore) → doc

### 새 `e2e` job (병렬 실행)

- `check`와 독립적으로 병렬 실행
- OS 매트릭스: ubuntu-latest, macos-latest
- JDK/Gradle 매트릭스 (경계값 2개):
  - JDK 17 + Gradle 7.6.4 (가장 오래된 지원 조합)
  - JDK 21 + Gradle 8.14 (최신 조합)
- 총 4개 조합 (2 OS × 2 JDK/Gradle)
- 매트릭스의 JDK 버전은 반드시 메이저 버전 정수(`17`, `21`)만 사용한다. `17.0.x` 형태는 환경변수 이름에 `.`이 들어가므로 금지.

Steps:
1. Checkout
2. Rust 1.85 + components 설치
3. Rust cache
4. `actions/setup-java` — `JAVA_{version}_HOME` 환경변수 설정
5. protobuf compiler 설치 (GitHub Actions 러너에 `protoc`가 프리인스톨되어 있지만, 버전 보장을 위해 명시적으로 설치한다)
6. `cargo test --test e2e`

### `actions/setup-java` 설정

```yaml
- uses: actions/setup-java@v4
  with:
    distribution: temurin
    java-version: ${{ matrix.jdk }}
```

설치 후 `JAVA_HOME`이 설정되지만, 매트릭스에서 단일 JDK만 사용하므로 테스트 내부에서 `JAVA_HOME` fallback으로 해당 버전을 찾을 수 있다. 추가로 `JAVA_{version}_HOME`을 명시적으로 설정하는 step을 넣어 안전성을 확보한다:

```yaml
- name: Set JAVA version-specific HOME
  run: echo "JAVA_${{ matrix.jdk }}_HOME=$JAVA_HOME" >> $GITHUB_ENV
```

## Fixture 프로젝트 의존성 확장

`tests/fixtures/gradle-project/app/build.gradle`에 다양한 JVM 생태계 라이브러리를 추가한다.

> **Kotlin 의존성 해석**: fixture 프로젝트는 `id 'java'` 플러그인만 사용한다. Kotlin/JVM 및 KMP 라이브러리는 JAR 형태로 classpath에 포함되므로, 컴파일 없이 심볼 추출만 하는 이 도구에서는 Kotlin 플러그인이 불필요하다. 단, Gradle Module Metadata(GMM)가 variant 선택에 영향을 줄 수 있으므로, KMP 의존성은 명시적으로 `-jvm` suffix가 붙은 아티팩트를 사용한다.
>
> **Gradle 7.6.4 호환성**: 일부 KMP 라이브러리가 Gradle 7.x에서 해석되지 않을 수 있다. 이 경우 해당 라이브러리의 심볼 검증은 Gradle 8.x 매트릭스에서만 수행하고, Gradle 7.x에서는 기존 Java 라이브러리 심볼만 검증한다.

### 의존성 목록

| 카테고리 | 라이브러리 | GAV | 검증 포인트 |
|---------|-----------|-----|-----------|
| Pure Java (기존) | Guava | `com.google.guava:guava:33.4.0-jre` | 기본 Java 심볼, 제네릭 |
| Pure Java (기존) | Gson | `com.google.code.gson:gson:2.11.0` | 클래스/메서드 추출, 소스 JAR |
| Pure Java (기존) | commons-lang3 | `org.apache.commons:commons-lang3:3.17.0` | `lib` 모듈, API scope |
| Kotlin/JVM (기존) | kotlinx-coroutines | `org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0` | suspend 함수, Kotlin 메타데이터 |
| Kotlin/JVM (신규) | kotlinx-serialization-json | `org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3` | sealed class, companion object, 어노테이션 |
| Kotlin/JVM (신규) | Ktor Client | `io.ktor:ktor-client-core:3.0.3` | 복잡한 Kotlin 타입 계층, extension 함수 |
| KMP (신규) | kotlinx-datetime | `org.jetbrains.kotlinx:kotlinx-datetime-jvm:0.6.1` | KMP `-jvm` 아티팩트, expect/actual 패턴 |
| KMP (신규) | kotlinx-io | `org.jetbrains.kotlinx:kotlinx-io-core-jvm:0.6.0` | KMP 런타임 심볼 |
| 어노테이션 프로세서 (신규) | Dagger | `com.google.dagger:dagger:2.52` | 생성 코드 vs 원본, 어노테이션 |
| 대형 라이브러리 (신규) | Spring Core | `org.springframework:spring-core:6.2.2` | 대량 심볼, 성능 |
| 대형 라이브러리 (신규) | OkHttp | `com.squareup.okhttp3:okhttp:4.12.0` | Kotlin/Java 혼합 |
| 순수 인터페이스 (신규) | SLF4J | `org.slf4j:slf4j-api:2.0.16` | 인터페이스/추상 클래스 위주 |
| 순수 인터페이스 (신규) | Jakarta Servlet API | `jakarta.servlet:jakarta.servlet-api:6.1.0` | 인터페이스 심볼 추출 |

### 검증 심볼 예시

각 카테고리별로 검색하여 존재를 확인할 대표 심볼:

- **kotlinx-serialization**: `kotlinx.serialization.json.Json`, `kotlinx.serialization.Serializable` (core 모듈에서 전이 의존성으로 제공)
- **Ktor**: `io.ktor.client.HttpClient`
- **kotlinx-datetime**: `kotlinx.datetime.Instant`, `kotlinx.datetime.Clock`
- **kotlinx-io**: `kotlinx.io.Buffer`, `kotlinx.io.Source`
- **Dagger**: `dagger.Component`, `dagger.Module`, `dagger.Provides`
- **Spring Core**: `org.springframework.core.env.Environment`
- **OkHttp**: `okhttp3.OkHttpClient`, `okhttp3.Request`
- **SLF4J**: `org.slf4j.Logger`, `org.slf4j.LoggerFactory`
- **Jakarta Servlet**: `jakarta.servlet.http.HttpServlet`, `jakarta.servlet.Filter`

## 추가 E2E 테스트 케이스

### 에러 케이스

- `test_search_without_index`: 인덱스 없이 search → exit code 3, `INDEX_NOT_FOUND` error_code
- `test_refresh_invalid_project`: 존재하지 않는 디렉토리에서 refresh → exit code 1, `INVALID_PROJECT_DIR` error_code
- `test_search_no_results`: 존재하지 않는 심볼 검색 → 빈 결과 (에러 아님)

### 출력 모드 검증 (`--agentic`)

- `test_agentic_search_output`: search 결과 JSON 구조 검증 — `query`, `results` 배열, 각 result의 `fqn`, `symbol_kind`, `gav` 필드 존재
- `test_agentic_error_output`: 에러 시 `{"success": false, "error_code": "...", "error": "...", "retryable": bool}` 형태 검증
- `test_agentic_exit_codes`: 에러 종류별 exit code 검증 — `INDEX_NOT_FOUND` → 3, `INVALID_PROJECT_DIR` → 1, usage error → 2

### status/clean E2E

- `test_status_after_refresh`: refresh 후 status → `has_index: true`, `dependency_count > 0`, `indexed_symbols: Some(n)` where `n > 0`, `is_stale: false`
- `test_clean_then_status`: clean 후 status → `has_index: false`
- `test_clean_idempotent`: clean 두 번 실행 → 에러 없음

### 소스 해석

- `test_show_with_source_jar`: source JAR 있는 클래스(예: `com.google.gson.Gson`)의 소스 가져오기 — `ShowOutput`에 소스 텍스트가 포함되어 있고 `class Gson` 문자열이 존재하는지 검증
- `test_show_no_source_no_decompile`: source JAR 없는 클래스에서 `--no-decompile` → 적절한 에러

### 다양한 라이브러리 심볼 검증

- `test_kotlin_jvm_symbols`: kotlinx-serialization, Ktor 심볼 검색 및 Kotlin 메타데이터 검증
- `test_kmp_jvm_symbols`: kotlinx-datetime, kotlinx-io의 `-jvm` 아티팩트 심볼 검증
- `test_annotation_processor_symbols`: Dagger 어노테이션/인터페이스 심볼 검증
- `test_large_library_symbols`: Spring Core, OkHttp 심볼 수 및 대표 심볼 검증
- `test_interface_only_symbols`: SLF4J, Jakarta Servlet 인터페이스 심볼 검증

## 테스트 구조 요약

### CI에서 실행되는 테스트 흐름

```
check job (ubuntu, macos):
  cargo fmt -- --check
  cargo clippy
  cargo test              ← 단위 테스트 + 비-Gradle 통합 테스트
  cargo doc

e2e job (ubuntu×macos × {jdk17+gradle7.6.4, jdk21+gradle8.14}):
  actions/setup-java
  cargo test --test e2e   ← 모든 E2E (JDK 없으면 skip)
```

### 로컬에서 실행

```bash
cargo test                        # 단위 + 비-Gradle 통합 테스트
cargo test --test e2e             # E2E (mise로 JDK 자동 해석)
cargo test --test e2e test_kmp    # 특정 테스트만
```

## 벤치마크

`criterion` 크레이트를 사용하여 refresh와 search 성능을 측정한다.

### 설정

`Cargo.toml`:
```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "refresh"
harness = false

[[bench]]
name = "search"
harness = false
```

벤치마크 파일은 `benches/refresh.rs`, `benches/search.rs`에 위치한다.

### refresh 벤치마크 (`benches/refresh.rs`)

Gradle 실행이 포함되므로 iteration당 수 초~수십 초가 걸린다. `criterion`의 `sample_size`와 `measurement_time`을 조정한다.

측정 대상:
- **`bench_refresh_full`**: 깨끗한 상태에서 전체 refresh (init → refresh). fixture 프로젝트의 모든 의존성을 인덱싱하는 전체 파이프라인 소요 시간.
- **`bench_refresh_incremental`**: 이미 인덱싱된 상태에서 의존성 1개 변경 후 재-refresh. 증분 인덱싱의 효율성 측정.
- **`bench_refresh_noop`**: 변경 없이 refresh 재실행. staleness 검사만 수행하는 최소 비용 측정.

JDK 해석은 E2E 테스트와 동일한 `get_java_home()` 함수를 사용한다. JDK가 없으면 벤치마크를 skip한다.

```rust
fn bench_refresh_full(c: &mut Criterion) {
    let java_home = match get_java_home("21") {
        Some(h) => h,
        None => {
            eprintln!("JDK 21 not available, skipping benchmark");
            return;
        }
    };

    let mut group = c.benchmark_group("refresh");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(120));

    group.bench_function("full", |b| {
        b.iter_with_setup(
            || { /* copy fixture to tempdir, run init */ },
            |project_dir| { /* run refresh */ },
        );
    });

    group.finish();
}
```

### search 벤치마크 (`benches/search.rs`)

사전에 인덱싱된 fixture를 한 번 준비한 뒤, 검색만 반복 측정한다. 검색은 밀리초 단위이므로 기본 `criterion` 설정으로 충분하다.

측정 대상:
- **`bench_search_simple`**: 단순 키워드 검색 (예: `"ImmutableList"`)
- **`bench_search_fqn`**: FQN 정확 매칭 (예: `"com.google.common.collect.ImmutableList"`)
- **`bench_search_regex`**: 정규식 검색 (예: `"Immutable.*"`)
- **`bench_search_with_type_filter`**: 타입 필터 적용 검색 (예: `kind=class`)
- **`bench_search_with_dependency_filter`**: 의존성 필터 적용 검색 (예: `gav="com.google.guava:*"`)

```rust
fn bench_search(c: &mut Criterion) {
    // setup: copy fixture, refresh once, open IndexReader
    let reader = /* ... */;

    let mut group = c.benchmark_group("search");

    group.bench_function("simple", |b| {
        b.iter(|| reader.search("ImmutableList", None, false, false, 20, None));
    });

    group.bench_function("fqn", |b| {
        b.iter(|| reader.search("com.google.common.collect.ImmutableList", None, true, false, 20, None));
    });

    group.bench_function("regex", |b| {
        b.iter(|| reader.search("Immutable.*", None, false, true, 20, None));
    });

    group.finish();
}
```

### 실행 방법

```bash
# 전체 벤치마크
cargo bench

# 특정 벤치마크만
cargo bench --bench search
cargo bench --bench refresh -- "noop"

# HTML 리포트 확인
open target/criterion/report/index.html
```

### CI에서의 벤치마크

벤치마크는 CI에서 **실행하지 않는다**. 이유:
- GitHub Actions 러너의 성능이 일관되지 않아 통계적 비교가 무의미하다
- refresh 벤치마크는 Gradle 네트워크 접근이 필요하여 실행 시간이 길다
- 성능 회귀 감지는 로컬에서 `cargo bench` + `criterion`의 기준선 비교로 수행한다

## 테스트 격리

모든 E2E 테스트는 `tempfile::tempdir()`로 독립적인 임시 디렉토리를 생성하여 fixture 프로젝트를 복사한다. Rust의 기본 테스트 러너는 테스트를 병렬 실행하므로, 각 테스트가 자체 복사본에서 작업하여 충돌을 방지한다.

## 비-목표

- Gradle 데몬 캐싱/최적화 (CI 실행 시간 최적화는 후속 작업)
- 디컴파일러 연동 테스트 (외부 도구 의존성 추가 필요)
- Windows CI 지원
