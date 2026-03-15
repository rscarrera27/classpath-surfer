# Indexing Process

> Living document — 인덱싱 관련 코드 변경 시 함께 업데이트할 것.

## 전체 흐름

```
DependencyInfo { jar_path, source_jar_path }
  │
  ├─ source_jar_path ──▶ build_source_table()
  │                        각 .kt/.java 파일의 package 선언 파싱
  │                        ──▶ SourceTable: (package, filename) → SourceEntry
  │
  └─ jar_path ──▶ process_class_files()
                    │
                    └─ 각 .class 파일:
                        extract_symbols(bytes, gav) ──▶ Vec<SymbolDoc>
                          │
                          └─ (package, source_file_name)으로 SourceTable 조회
                              → has_source / source_path 설정
                        │
                        add_symbol_doc() ──▶ Tantivy document
```

## 핵심 데이터 구조

### SourceTable (`src/parser/jar.rs`)

```rust
type SourceTable = HashMap<(String, String), SourceEntry>;
//                          ^package  ^filename

struct SourceEntry {
    path: String,            // JAR 내 전체 경로 (예: "commonMain/CoroutineScope.kt")
    language: SourceLanguage, // Java | Kotlin
}
```

**조인 키**: `(package, filename)` — classfile의 `package` + `SourceFile` 속성으로 구성.
디렉토리 구조(KMP source set, 표준 패키지 경로 등)에 의존하지 않으므로 모든 JAR 레이아웃에서 동일하게 동작.

```
Source JAR:  commonMain/CoroutineScope.kt  →  파싱: "package kotlinx.coroutines"
Binary JAR:  CoroutineScope.class          →  package=kotlinx.coroutines, SourceFile=CoroutineScope.kt

Key: ("kotlinx.coroutines", "CoroutineScope.kt") → match
```

**충돌 처리**: 같은 (package, filename) 쌍이 여러 경로에 있으면 `jvmMain/` 우선.

### SymbolDoc (`src/model.rs`)

```rust
struct SymbolDoc {
    gav: String,                    // "com.google.guava:guava:33.0-jre"
    symbol_kind: SymbolKind,        // Class | Method | Field
    fqn: String,                    // "com.google.common.collect.ImmutableList"
    package: String,                // "com.google.common.collect"
    class_name: String,             // "ImmutableList"
    simple_name: String,            // "ImmutableList" (method면 메서드명)
    name_parts: String,             // "Immutable List" (CamelCase 분리)
    descriptor: String,             // JVM descriptor
    signature_display: String,      // 사람 읽기용 시그니처
    access_flags: String,           // "public final"
    has_source: bool,               // SourceTable에서 매칭 성공 여부
    source_path: Option<String>,    // SourceTable에서 찾은 JAR 내 경로
    source_language: Option<SourceLanguage>,
    source_file_name: Option<String>, // classfile SourceFile 속성
    kotlin_signature_display: Option<String>,
}
```

### Tantivy 스키마 (`src/index/schema.rs`)

| 카테고리 | 필드 | 옵션 | 용도 |
|---------|------|------|------|
| **Identity** | `gav`, `symbol_kind`, `fqn`, `package`, `class_name` | `STRING \| STORED` | exact match 필터링 |
| **Search** | `simple_name`, `name_parts` | `TEXT \| STORED` / `TEXT` | 토큰화 검색 |
| **Metadata** | `descriptor`, `signature_display`, `access_flags`, `has_source`, `source_path`, `source_language`, `source_file_name`, `kotlin_signature_display` | `STORED` | 결과에 포함, 검색 불가 |

## 모듈별 역할

| 파일 | 역할 |
|------|------|
| `parser/jar.rs` | `build_source_table()`: source JAR → SourceTable 구축 |
| `parser/jar.rs` | `extract_package_declaration()`: 소스 파일에서 package 선언 파싱 |
| `parser/jar.rs` | `process_class_files()`: binary JAR의 .class 순회 |
| `parser/classfile.rs` | `extract_symbols()`: .class → `Vec<SymbolDoc>` |
| `parser/classfile.rs` | `source_file_name_from_bytes()`: SourceFile 속성 추출 |
| `index/writer.rs` | `index_dependency()`: SourceTable 구축 + 심볼 추출 + 인덱싱 통합 |
| `index/writer.rs` | `add_symbol_doc()`: SymbolDoc → Tantivy document 변환 |
| `index/schema.rs` | `build_schema()`: Tantivy 필드 정의 |

## package 선언 파싱 (`extract_package_declaration`)

블록 주석, 라인 주석, `@file:` 어노테이션, BOM을 처리하는 상태 머신.

- `in_block_comment` 플래그로 `/* */` 추적
- 빈 줄, `//`, `@` 줄은 건너뜀
- `package ` 발견 시 세미콜론 제거 후 반환
- `import`, `class` 등 코드 시작 줄 도달 시 빈 문자열 반환

## 증분 인덱싱

`refresh` 커맨드에서 `ManifestDiff`를 통해 GAV 단위로 증분 처리:

```
compute_diff(current_manifest, previous_manifest)
  → added:     새로 추가된 GAV → index_dependency()
  → removed:   제거된 GAV     → delete_gav()
  → unchanged: 변경 없음       → skip
```

## 스키마 마이그레이션

`open_or_create_index()`에서 기존 인덱스의 스키마가 필수 필드를 포함하지 않으면
인덱스 디렉토리를 삭제하고 새로 생성 (파괴적 마이그레이션).
