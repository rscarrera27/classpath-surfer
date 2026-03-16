# Index Glob Search Optimization Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Accelerate suffix glob searches (`*List`, `*.collect`) by adding reversed index fields and optimize package filtering by replacing manual term iteration with RegexQuery.

**Architecture:** Add `simple_name_rev` (TEXT) and `package_rev` (STRING) reversed fields to the Tantivy schema. At query time, detect suffix patterns via `classify_glob()` and route to reversed fields with prefix regex. Replace `build_package_filter`'s manual segment iteration with `RegexQuery`. Add FAST option to `symbol_kind` and `access_level` for future columnar access.

**Tech Stack:** Rust, Tantivy 0.25, existing `glob_to_tantivy_regex()` utility

**Spec:** `docs/superpowers/specs/2026-03-16-index-glob-optimization-design.md`

---

## Chunk 1: Schema, Writer, and Model Utilities

### Task 1: Extract shared required fields constant

**Files:**
- Create: `src/index/compat.rs`
- Modify: `src/index/mod.rs`
- Modify: `src/index/writer.rs:164-179`
- Modify: `src/index/reader.rs:40-53`

- [ ] **Step 1: Create `src/index/compat.rs` with shared constant**

```rust
//! Index schema compatibility checking.

/// Required field names for schema compatibility validation.
///
/// When new fields are added to the schema, append them here.
/// Both `open_or_create_index` and `IndexReader::open` use this list
/// to detect outdated indexes that need rebuilding.
pub const REQUIRED_FIELDS: &[&str] = &[
    "gav",
    "symbol_kind",
    "fqn",
    "simple_name",
    "name_parts",
    "signature_java",
    "signature_kotlin",
    "access_flags",
    "access_level",
    "source",
    "source_language",
    "classpaths",
    "simple_name_rev",
    "package_rev",
];
```

- [ ] **Step 2: Add `pub mod compat;` to `src/index/mod.rs`**

Add `pub mod compat;` with a doc comment:
```rust
/// Schema compatibility constants shared between reader and writer.
pub mod compat;
```

- [ ] **Step 3: Update `src/index/writer.rs` to use shared constant**

Replace the inline `required` array in `is_schema_compatible` (lines 164-179):
```rust
fn is_schema_compatible(schema: &Schema) -> bool {
    super::compat::REQUIRED_FIELDS
        .iter()
        .all(|&name| schema.get_field(name).is_ok())
}
```

- [ ] **Step 4: Update `src/index/reader.rs` to use shared constant**

Replace the inline `required_fields` array in `IndexReader::open` (lines 40-53):
```rust
let missing: Vec<&str> = super::compat::REQUIRED_FIELDS
    .iter()
    .filter(|&&name| schema.get_field(name).is_err())
    .copied()
    .collect();
```

- [ ] **Step 5: Run `cargo test --test unit` and `cargo clippy`**

Expected: all pass (no behavioral change)

- [ ] **Step 6: Commit**

```bash
git add src/index/compat.rs src/index/mod.rs src/index/writer.rs src/index/reader.rs
git commit -m "refactor: extract shared REQUIRED_FIELDS constant for schema compat"
```

---

### Task 2: Add `classify_glob` and `reverse_str` to model

**Files:**
- Modify: `src/model/mod.rs`
- Modify: `tests/unit.rs`

- [ ] **Step 1: Write failing tests for `classify_glob` in `tests/unit.rs`**

Add after the existing glob pattern tests section:

```rust
use classpath_surfer::model::{GlobShape, classify_glob};

// ---------------------------------------------------------------------------
// Glob shape classification
// ---------------------------------------------------------------------------

#[test]
fn classify_glob_prefix() {
    assert_eq!(classify_glob("Foo*"), GlobShape::Prefix);
    assert_eq!(classify_glob("com.google.*"), GlobShape::Prefix);
    assert_eq!(classify_glob("Foo?"), GlobShape::Prefix);
    assert_eq!(classify_glob("Foo*?"), GlobShape::Prefix);
}

#[test]
fn classify_glob_suffix() {
    assert_eq!(classify_glob("*List"), GlobShape::Suffix);
    assert_eq!(classify_glob("*.collect"), GlobShape::Suffix);
    assert_eq!(classify_glob("?Foo"), GlobShape::Suffix);
    assert_eq!(classify_glob("*"), GlobShape::Suffix);
    assert_eq!(classify_glob("*?Foo"), GlobShape::Suffix);
}

#[test]
fn classify_glob_mixed() {
    assert_eq!(classify_glob("*Foo*"), GlobShape::Mixed);
    assert_eq!(classify_glob("F*o"), GlobShape::Mixed);
    assert_eq!(classify_glob("*Foo?"), GlobShape::Mixed);
    assert_eq!(classify_glob("F*o*"), GlobShape::Mixed);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test unit classify_glob`
Expected: FAIL — `GlobShape` and `classify_glob` not found

- [ ] **Step 3: Implement `GlobShape` and `classify_glob` in `src/model/mod.rs`**

Add after the existing `glob_to_tantivy_regex` function:

```rust
/// Shape of a glob pattern based on wildcard positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobShape {
    /// Glob characters only at the end (e.g., `Foo*`, `com.google.*`).
    Prefix,
    /// Glob characters only at the start (e.g., `*List`, `*.collect`).
    Suffix,
    /// Glob characters in multiple positions or interior (e.g., `*Foo*`, `F*o`).
    Mixed,
}

/// Classify a glob pattern by the position of its wildcard characters.
///
/// Only meaningful when the pattern contains at least one `*` or `?`.
///
/// ```
/// use classpath_surfer::model::{GlobShape, classify_glob};
/// assert_eq!(classify_glob("Foo*"), GlobShape::Prefix);
/// assert_eq!(classify_glob("*List"), GlobShape::Suffix);
/// assert_eq!(classify_glob("*Foo*"), GlobShape::Mixed);
/// ```
pub fn classify_glob(pattern: &str) -> GlobShape {
    let chars: Vec<char> = pattern.chars().collect();
    let first_literal = chars.iter().position(|c| *c != '*' && *c != '?');
    let last_literal = chars.iter().rposition(|c| *c != '*' && *c != '?');
    let first_glob = chars.iter().position(|c| *c == '*' || *c == '?');
    let last_glob = chars.iter().rposition(|c| *c == '*' || *c == '?');

    match (first_literal, last_literal, first_glob, last_glob) {
        (Some(fl), Some(ll), Some(fg), Some(lg)) => {
            if fg > ll {
                // All globs come after all literals: Foo*, com.google.*
                GlobShape::Prefix
            } else if lg < fl {
                // All globs come before all literals: *List, *.collect
                GlobShape::Suffix
            } else {
                // Globs and literals are interleaved: *Foo*, F*o, *Foo?
                GlobShape::Mixed
            }
        }
        // All globs, no literals (e.g., "*", "**", "*?")
        (None, None, Some(_), Some(_)) => GlobShape::Suffix,
        _ => GlobShape::Mixed,
    }
}
```

- [ ] **Step 4: Export `GlobShape` and `classify_glob` from `src/model/mod.rs`**

Ensure they are `pub` (they already are from step 3). No additional re-export needed since `model` module is already public.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --test unit classify_glob`
Expected: PASS

- [ ] **Step 6: Write failing test for `reverse_str`**

Add to `tests/unit.rs`:

```rust
use classpath_surfer::model::reverse_str;

#[test]
fn reverse_str_basic() {
    assert_eq!(reverse_str("ImmutableList"), "tsiLelbatummI");
    assert_eq!(reverse_str("com.google.common.collect"), "tcelloc.nommoc.elgoog.moc");
    assert_eq!(reverse_str(""), "");
    assert_eq!(reverse_str("A"), "A");
}
```

- [ ] **Step 7: Run test to verify it fails**

Run: `cargo test --test unit reverse_str_basic`
Expected: FAIL — `reverse_str` not found

- [ ] **Step 8: Implement `reverse_str` in `src/model/mod.rs`**

Add near the other utility functions:

```rust
/// Reverse a string by Unicode code points.
///
/// Sufficient for JVM identifiers which consist of BMP characters
/// without combining marks.
///
/// ```
/// use classpath_surfer::model::reverse_str;
/// assert_eq!(reverse_str("Hello"), "olleH");
/// ```
pub fn reverse_str(s: &str) -> String {
    s.chars().rev().collect()
}
```

- [ ] **Step 9: Run tests to verify they pass**

Run: `cargo test --test unit reverse_str_basic && cargo test --test unit classify_glob`
Expected: PASS

- [ ] **Step 10: Run clippy**

Run: `cargo clippy`
Expected: no warnings

- [ ] **Step 11: Commit**

```bash
git add src/model/mod.rs tests/unit.rs
git commit -m "feat: add classify_glob and reverse_str utilities"
```

---

### Task 3: Update schema with new fields

**Files:**
- Modify: `src/index/schema.rs`

- [ ] **Step 1: Add reversed fields and FAST options to `build_schema`**

In `src/index/schema.rs`, add after the `name_parts` field (line 36):

```rust
// Search fields — reversed (for suffix glob acceleration)
builder.add_text_field("simple_name_rev", TEXT);
builder.add_text_field("package_rev", STRING);
```

And modify the existing `symbol_kind` and `access_level` lines:

```rust
builder.add_text_field("symbol_kind", STRING | STORED | FAST);
// ...
builder.add_text_field("access_level", STRING | STORED | FAST);
```

- [ ] **Step 2: Update the module-level doc comment table (lines 1-9)**

Add `simple_name_rev` and `package_rev` to the **Search** row in the field categories table.

- [ ] **Step 3: Run `cargo build`**

Expected: compiles successfully

- [ ] **Step 4: Commit**

```bash
git add src/index/schema.rs
git commit -m "feat: add simple_name_rev, package_rev fields and FAST columns"
```

---

### Task 4: Update writer to populate reversed fields

**Files:**
- Modify: `src/index/writer.rs`

- [ ] **Step 1: Add field handles to `SchemaFields`**

Add to the struct (after `name_parts`):

```rust
/// Reversed simple name for suffix glob acceleration.
pub simple_name_rev: Field,
/// Reversed package name for suffix glob acceleration.
pub package_rev: Field,
```

- [ ] **Step 2: Resolve new fields in `SchemaFields::new`**

Add to the `Self { .. }` block:

```rust
simple_name_rev: schema.get_field("simple_name_rev").unwrap(),
package_rev: schema.get_field("package_rev").unwrap(),
```

- [ ] **Step 3: Populate reversed fields in `add_symbol_doc`**

Add to the `doc!()` macro call, after `f.name_parts`:

```rust
f.simple_name_rev => crate::model::reverse_str(&doc_data.simple_name),
f.package_rev => crate::model::reverse_str(&doc_data.package),
```

- [ ] **Step 4: Run `cargo build`**

Expected: compiles successfully

- [ ] **Step 5: Run `cargo test --test unit`**

Expected: all pass

- [ ] **Step 6: Commit**

```bash
git add src/index/writer.rs
git commit -m "feat: populate simple_name_rev and package_rev at index time"
```

---

## Chunk 2: Query Builder Changes and Integration Tests

### Task 5: Route suffix simple_name globs to reversed field

**Files:**
- Modify: `src/index/reader.rs`

- [ ] **Step 1: Add `reverse_suffix_to_regex` helper function**

Add after the existing `regex_escape_term` function at the bottom of `reader.rs`:

```rust
/// Build a Tantivy regex for the reversed field from a suffix glob pattern.
///
/// Algorithm:
/// 1. Strip leading glob characters (`*`/`?`) to get the literal suffix
/// 2. Optionally lowercase the literal (for tokenized TEXT fields)
/// 3. Reverse the literal
/// 4. Append the stripped glob characters (reversed) as trailing wildcards
/// 5. Convert to Tantivy regex via `glob_to_tantivy_regex`
fn reverse_suffix_to_regex(pattern: &str, lowercase: bool) -> String {
    // Split: leading globs | literal suffix
    let first_literal = pattern
        .find(|c: char| c != '*' && c != '?')
        .unwrap_or(pattern.len());
    let glob_prefix = &pattern[..first_literal];
    let literal = &pattern[first_literal..];

    let literal = if lowercase {
        literal.to_lowercase()
    } else {
        literal.to_string()
    };

    let reversed_literal: String = literal.chars().rev().collect();
    // Trailing wildcard: reverse the glob prefix chars so "*?" becomes "?*"
    let trailing_glob: String = glob_prefix.chars().rev().collect();

    glob_to_tantivy_regex(&format!("{reversed_literal}{trailing_glob}"))
}
```

Add `glob_to_tantivy_regex` to the import from `crate::model`:

```rust
use crate::model::{
    AccessLevel, GlobShape, SearchQuery, SearchResult, SignatureDisplay, SourceLanguage,
    SymbolKind, classify_glob, glob_to_tantivy_regex, matches_glob_pattern,
};
```

- [ ] **Step 2: Modify the simple_name glob branch in `build_base_query`**

Replace the current simple_name glob block (lines 487-494):

```rust
if has_glob {
    // Glob on simple_name (lowercase)
    let simple_name_field = schema.get_field("simple_name").unwrap();
    return Ok(Box::new(RegexQuery::from_pattern(
        &glob_to_tantivy_regex(&query_str.to_lowercase()),
        simple_name_field,
    )?));
}
```

With:

```rust
if has_glob {
    // Glob on simple_name — route suffix patterns to reversed field
    match classify_glob(query_str) {
        GlobShape::Suffix => {
            let rev_field = schema.get_field("simple_name_rev").unwrap();
            let regex = reverse_suffix_to_regex(query_str, true);
            return Ok(Box::new(RegexQuery::from_pattern(&regex, rev_field)?));
        }
        _ => {
            let simple_name_field = schema.get_field("simple_name").unwrap();
            return Ok(Box::new(RegexQuery::from_pattern(
                &glob_to_tantivy_regex(&query_str.to_lowercase()),
                simple_name_field,
            )?));
        }
    }
}
```

- [ ] **Step 3: Run `cargo build`**

Expected: compiles successfully

- [ ] **Step 4: Commit**

```bash
git add src/index/reader.rs
git commit -m "feat: route suffix simple_name globs to reversed field"
```

---

### Task 6: Rewrite package filter to use RegexQuery

**Files:**
- Modify: `src/index/reader.rs`

- [ ] **Step 1: Rewrite `build_package_filter`**

Replace the entire `build_package_filter` method (lines 264-307) with:

```rust
/// Build a package filter query from a package pattern.
///
/// - Exact match (no glob) → `TermQuery` on `package`.
/// - Suffix glob (`*.collect`) → `RegexQuery` on `package_rev` (reversed prefix).
/// - Prefix/mixed glob → `RegexQuery` on `package` (FST automata).
fn build_package_filter(
    &self,
    schema: &Schema,
    pattern: &str,
) -> Result<Option<Box<dyn tantivy::query::Query>>> {
    if !pattern.contains('*') && !pattern.contains('?') {
        // Exact match
        let pkg_field = schema.get_field("package").unwrap();
        return Ok(Some(Box::new(TermQuery::new(
            tantivy::Term::from_field_text(pkg_field, pattern),
            IndexRecordOption::Basic,
        ))));
    }

    match classify_glob(pattern) {
        GlobShape::Suffix => {
            let rev_field = schema.get_field("package_rev").unwrap();
            let regex = reverse_suffix_to_regex(pattern, false);
            Ok(Some(Box::new(RegexQuery::from_pattern(&regex, rev_field)?)))
        }
        _ => {
            // Prefix or mixed — use RegexQuery on the forward package field
            let pkg_field = schema.get_field("package").unwrap();
            let regex = glob_to_tantivy_regex(pattern);
            Ok(Some(Box::new(RegexQuery::from_pattern(&regex, pkg_field)?)))
        }
    }
}
```

- [ ] **Step 2: Run `cargo build`**

Expected: compiles successfully

- [ ] **Step 3: Run `cargo clippy`**

Expected: no warnings. Some previously-used imports (`BTreeSet`) may now be unused — remove if flagged.

- [ ] **Step 4: Commit**

```bash
git add src/index/reader.rs
git commit -m "feat: rewrite package filter with RegexQuery and reversed field routing"
```

---

### Task 7: Unit tests for `reverse_suffix_to_regex`

**Files:**
- Modify: `tests/unit.rs`

- [ ] **Step 1: Add tests for the reversed regex construction**

Since `reverse_suffix_to_regex` is private, test it indirectly through `classify_glob` + `reverse_str` + `glob_to_tantivy_regex`. Add to `tests/unit.rs`:

```rust
use classpath_surfer::model::glob_to_tantivy_regex;

#[test]
fn suffix_glob_reversed_regex_construction() {
    // Simulate the reverse_suffix_to_regex algorithm for simple_name (lowercase=true)
    // *List → strip "*" → "List" → lowercase → "list" → reverse → "tsil" → append "*" → "tsil*"
    let regex = glob_to_tantivy_regex("tsil*");
    assert!(regex::Regex::new(&regex).unwrap().is_match("tsilelbatummi"));
    assert!(!regex::Regex::new(&regex).unwrap().is_match("xyzabc"));

    // Simulate for package (lowercase=false)
    // *.collect → strip "*" → ".collect" → reverse → "tcelloc." → append "*" → "tcelloc.*"
    let regex = glob_to_tantivy_regex("tcelloc.*");
    assert!(regex::Regex::new(&regex).unwrap().is_match("tcelloc.nommoc.elgoog.moc"));
    assert!(!regex::Regex::new(&regex).unwrap().is_match("gnirts.modnar"));
}
```

- [ ] **Step 2: Run test**

Run: `cargo test --test unit suffix_glob_reversed`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add tests/unit.rs
git commit -m "test: add unit tests for suffix glob reversed regex construction"
```

---

### Task 8: Integration tests for suffix glob search

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: Add suffix glob test on simple_name**

Add to `tests/integration.rs`:

```rust
#[test]
fn search_suffix_glob_simple_name() {
    let project = require_indexed_project!();
    let reader = IndexReader::open(&project.index_dir()).unwrap();

    // *List should match classes ending in "List" (e.g., ImmutableList, ArrayList)
    let (results, count, _) = reader
        .search(&SearchQuery {
            limit: 50,
            ..SearchQuery::with_types("*List", &[SymbolKind::Class])
        })
        .unwrap();

    assert!(count > 0, "suffix glob *List should match some classes");
    assert!(
        results.iter().all(|r| r.simple_name.ends_with("List")),
        "all results should end with 'List', got: {:?}",
        results.iter().map(|r| &r.simple_name).collect::<Vec<_>>()
    );
}
```

- [ ] **Step 2: Add suffix glob test on package filter**

```rust
#[test]
fn search_suffix_glob_package_filter() {
    let project = require_indexed_project!();
    let reader = IndexReader::open(&project.index_dir()).unwrap();

    // *.collect should match packages ending in ".collect"
    let (results, count, _) = reader
        .search(&SearchQuery {
            limit: 50,
            package: Some("*.collect"),
            ..SearchQuery::with_types("Immutable", &[SymbolKind::Class])
        })
        .unwrap();

    assert!(count > 0, "suffix package glob *.collect should match");
    assert!(
        results.iter().all(|r| r.fqn.contains(".collect.")),
        "all results should be in a .collect package"
    );
}
```

- [ ] **Step 3: Add prefix glob test to verify no regression**

```rust
#[test]
fn search_prefix_glob_still_works() {
    let project = require_indexed_project!();
    let reader = IndexReader::open(&project.index_dir()).unwrap();

    // Immutable* should still work (prefix glob, no reversed field)
    let (results, count, _) = reader
        .search(&SearchQuery {
            limit: 50,
            ..SearchQuery::with_types("Immutable*", &[SymbolKind::Class])
        })
        .unwrap();

    assert!(count > 0, "prefix glob Immutable* should match");
    assert!(
        results
            .iter()
            .all(|r| r.simple_name.to_lowercase().starts_with("immutable")),
        "all results should start with 'Immutable'"
    );
}
```

- [ ] **Step 4: Run integration tests**

Run: `cargo test --test integration search_suffix_glob search_prefix_glob_still_works`
Expected: PASS (requires JDK 21 and indexed test project)

- [ ] **Step 5: Commit**

```bash
git add tests/integration.rs
git commit -m "test: add integration tests for suffix glob on simple_name and package"
```

---

### Task 9: Final verification

- [ ] **Step 1: Run full unit test suite**

Run: `cargo test --test unit`
Expected: all pass

- [ ] **Step 2: Run full integration test suite**

Run: `cargo test --test integration --test integration_mutation --test cli`
Expected: all pass

- [ ] **Step 3: Run clippy and fmt**

Run: `cargo clippy && cargo fmt -- --check`
Expected: clean

- [ ] **Step 4: Run doc build**

Run: `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`
Expected: no warnings (all new `pub` items have doc comments)
