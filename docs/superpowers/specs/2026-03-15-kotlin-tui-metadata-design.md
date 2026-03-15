# TUI Kotlin/Source Metadata Visibility

**Date:** 2026-03-15
**Status:** Approved

## Problem

TUI에서 심볼의 원본 언어(Java/Kotlin)와 소스 출처(원본 소스 JAR/디컴파일)가 충분히 보이지 않는다.

- 검색 테이블: `[kt]` 배지가 Dependency 컬럼에 붙어있을 뿐, 별도 컬럼 없음. `has_source` 정보는 아예 미표시.
- Show 뷰: 타이틀 바에 `fqn — original (kotlin)` 형태로만 표시. GAV 정보 없음.

## Solution

### 1. 검색 테이블 — Lang, Src 컬럼 추가

**Non-compact 모드:**

| Symbol (30%) | Kind (8) | Lang (6) | Src (8) | Signature (Fill) | Dependency (20%) |
|---|---|---|---|---|---|
| `com.example.Foo` | class | Kotlin | Original | `data class Foo` | `com.example:lib:1.0` |
| `com.example.Bar` | method | Java | Decompile | `public void bar()` | `com.example:lib:1.0` |

- `Lang`: `source_language` 기반. `Java` / `Kotlin`. None이면 `Java`.
- `Src`: `has_source` 기반. `Original`(true) / `Decompile`(false).
  - "Decompile"은 "원본 소스 없음, 디컴파일 필요"를 의미.
- 기존 Dependency 컬럼의 `[kt]` 배지 제거 (별도 컬럼으로 이동했으므로).
- Signature 컬럼은 `Constraint::Fill(1)`로 나머지 공간을 채움 (기존 40%에서 변경).

**Compact 모드 (side-by-side):**

기존 `[kt]` 배지에 `[src]` 배지 추가: `Foo [kt][src]`.

### 2. Show 뷰 — 메타데이터 패널

소스 코드 블록 위에 테두리가 있는 메타데이터 블록 추가:

**원본 소스가 있을 때:**
```
┌─ Metadata ─────────────────────────────────────────────────────┐
│ Language: Kotlin    Source: Original                            │
│ GAV: org.jetbrains.kotlinx:kotlinx-coroutines-core:1.7.3       │
│ Path: kotlinx/coroutines/CoroutineScope.kt                     │
└────────────────────────────────────────────────────────────────┘
```

**디컴파일된 소스일 때 (path 없음):**
```
┌─ Metadata ─────────────────────────────────────────────────────┐
│ Language: Java      Source: Decompiled                          │
│ GAV: com.google.guava:guava:32.1.3-jre                         │
└────────────────────────────────────────────────────────────────┘
```

- 메타데이터 패널 높이: `path`가 있으면 5줄, 없으면 4줄 (가변).
- Tab으로 secondary 뷰 전환 시: Language, Source, Path가 변경됨. GAV는 동일하게 유지.

### 3. 데이터 파이프라인 — GAV 전달

`ShowOutput`에 `gav` 필드가 없어 메타데이터 패널에 GAV를 표시할 수 없다.

- `ResolvedSource`에 `gav: String` 추가
- `source/resolver.rs`의 `resolve_source`에서 매칭된 dependency의 GAV 반환
- `cli/show.rs`에서 `ShowOutput.gav`로 전달
- `gav` 타입은 `String` (not `Option<String>`): `find_dependency_for_class`가 매칭 실패 시 에러를 반환하므로, `ResolvedSource` 생성 시점에서 GAV는 항상 존재함.

## Changed Files

| File | Change |
|---|---|
| `src/model.rs` | `ShowOutput`에 `gav: String` 추가, `ResolvedSource`에 `gav: String` 추가 |
| `src/source/resolver.rs` | `resolve_source` 반환값에 GAV 포함 |
| `src/cli/show.rs` | GAV를 `ShowOutput`까지 파이프 |
| `src/tui/search.rs` | 검색 테이블에 `Lang`, `Src` 컬럼 추가 + compact 배지 강화 |
| `src/tui/show.rs` | 메타데이터 패널 렌더링 (가변 높이) |
| `src/cli/render.rs` | Plain 텍스트 검색 결과에 `Lang`, `Src` 컬럼 추가 (ASCII 테이블 형식) |

모든 새로운 `pub` 필드와 함수에 `///` 문서 주석 필수 (`#![deny(missing_docs)]`).

## Not Changed

- 인덱스 스키마 — `has_source`, `source_language`는 이미 인덱싱됨
- Kotlin 메타데이터 파싱 로직
- Agentic JSON 출력 — `SearchResult`는 이미 `source_language`, `has_source` 필드를 직렬화함. `ShowOutput`에 `gav` 추가 시 `Serialize` derive로 자동 포함.
- 인덱스 재빌드 불필요
