# Index Structure Optimization for Glob Search Performance

## Problem

Glob search on `simple_name`, `package`, and `fqn` fields uses Tantivy `RegexQuery` which scans the FST term dictionary. Prefix patterns (`Foo*`) are already fast via FST prefix traversal, but suffix patterns (`*List`, `*.collect`) require a full term dictionary scan. Additionally, `build_package_filter` manually iterates all segment term dictionaries instead of leveraging FST automata.

## Solution

Three complementary improvements:

1. **Reversed fields** — `simple_name_rev` and `package_rev` turn suffix globs into prefix globs on the reversed term, enabling FST prefix traversal.
2. **FAST fields** — `symbol_kind` and `access_level` get columnar storage. No immediate query-time benefit (existing `TermQuery` filters use the inverted index, not columnar access), but prepares the schema for future aggregation/sorting/faceting use cases. Cost is negligible.
3. **Package filter rewrite** — Replace manual term iteration with `RegexQuery` (FST automata) and reversed-field routing.

## Schema Changes (`src/index/schema.rs`)

New fields:

| Field | Options | Purpose |
|-------|---------|---------|
| `simple_name_rev` | `TEXT` (not stored) | Reversed simple_name for suffix glob acceleration |
| `package_rev` | `STRING` (not stored) | Reversed package for suffix glob acceleration |

Modified fields:

| Field | Before | After |
|-------|--------|-------|
| `symbol_kind` | `STRING \| STORED` | `STRING \| STORED \| FAST` |
| `access_level` | `STRING \| STORED` | `STRING \| STORED \| FAST` |

`is_schema_compatible` adds `simple_name_rev` and `package_rev` to the required field list, triggering automatic index rebuild for existing indexes. Note: the required fields list is duplicated in `writer.rs` (`is_schema_compatible`) and `reader.rs` (`IndexReader::open` validation). Both must be updated; ideally extract into a shared constant during this change.

### Why `package_rev` is STRING, not TEXT

The default TEXT tokenizer splits on dots, which would break the dotted package structure:

- TEXT: `"tcelloc.nommoc.elgoog.moc"` → tokens `["tcelloc", "nommoc", "elgoog", "moc"]` → regex across segments fails
- STRING: full value `"tcelloc.nommoc.elgoog.moc"` preserved → regex matches correctly

`simple_name_rev` uses TEXT because JVM identifiers are single tokens with no splitting characters (no dots, whitespace, or punctuation) — the default tokenizer only lowercases. `char`-level reversal (`chars().rev()`) is sufficient for JVM identifiers which consist of BMP characters without combining marks.

## Writer Changes (`src/index/writer.rs`)

`SchemaFields` gains two new field handles: `simple_name_rev` and `package_rev`.

`add_symbol_doc` computes reversed values at index time:

```rust
fn reverse_str(s: &str) -> String {
    s.chars().rev().collect()
}

// In add_symbol_doc:
f.simple_name_rev => reverse_str(&doc_data.simple_name),
f.package_rev => reverse_str(&doc_data.package),
```

Cost is paid once at indexing; zero cost at query time.

## Query Builder Changes (`src/index/reader.rs`)

### Glob Pattern Classification

New utility to classify glob shape by `*`/`?` position:

```rust
enum GlobShape {
    Prefix,  // "Foo*", "com.google.*" — trailing glob only
    Suffix,  // "*Foo", "*.collect" — leading glob only
    Mixed,   // "*Foo*", "F*o*" — interior or multiple globs
}
```

Lives in `src/model/mod.rs` alongside existing glob utilities.

Classification rule: scan the pattern for glob characters (`*`/`?`). If all glob characters are at the end → `Prefix`. If all are at the start → `Suffix`. Otherwise → `Mixed`. Only called after confirming the presence of glob characters.

Edge cases:

| Pattern | Shape | Rationale |
|---------|-------|-----------|
| `Foo*` | Prefix | Trailing `*` only |
| `Foo?` | Prefix | Trailing `?` only |
| `*Foo` | Suffix | Leading `*` only |
| `?Foo` | Suffix | Leading `?` only |
| `*` | Suffix | All-match, treated as suffix (reversed `.*` is still `.*`) |
| `*Foo*` | Mixed | Glob chars at both ends |
| `F*o` | Mixed | Interior glob |

### Reversed Glob Pattern Construction Algorithm

For suffix patterns, the query-side transformation to build a regex on the reversed field:

1. **Split**: separate leading glob characters from the literal suffix (e.g., `*List` → glob=`*`, literal=`List`; `*.collect` → glob=`*`, literal=`.collect`)
2. **Lowercase** (for `simple_name_rev` only): lowercase the literal (`List` → `list`)
3. **Reverse** the literal: `list` → `tsil`, `.collect` → `tcelloc.`
4. **Append wildcard**: `tsil` + `*` → `tsil*`, `tcelloc.` + `*` → `tcelloc.*`
5. **Convert to regex**: apply `glob_to_tantivy_regex()` which escapes special chars then converts `*`→`.*`: `tsil*` → `tsil.*`, `tcelloc.*` → `tcelloc\..*`

Examples:

| Input | Field | Step 1 (literal) | Step 2 (lower) | Step 3 (reverse) | Step 4+5 (regex) |
|-------|-------|-------------------|----------------|------------------|------------------|
| `*List` | `simple_name_rev` | `List` | `list` | `tsil` | `tsil.*` |
| `*Exception` | `simple_name_rev` | `Exception` | `exception` | `noitpecxe` | `noitpecxe.*` |
| `*.collect` | `package_rev` | `.collect` | (no lowercase) | `tcelloc.` | `tcelloc\..*` |
| `*Service?` | — | — | — | — | Mixed (not suffix) |

Note: leading `?` globs (e.g., `?List`) follow the same algorithm: strip `?`, reverse literal, append `?` (single-char wildcard), convert to regex. Result: `tsiL?` → `tsiL.` (regex).

### `build_base_query` — simple_name glob routing

Current: all simple_name globs → `RegexQuery` on `simple_name`.

Changed:
- **Suffix** (`*List`) → `RegexQuery` on `simple_name_rev` with reversed prefix pattern (`tsil.*`)
- **Prefix** (`Immutable*`) → unchanged, `RegexQuery` on `simple_name` (FST prefix already fast)
- **Mixed** (`*List*`) → unchanged, `RegexQuery` on `simple_name` (no optimization possible)

### `build_package_filter` — eliminate manual term iteration

Current: glob patterns trigger manual iteration of all segment term dictionaries, applying `matches_glob_pattern` per term, then assembling an OR `BooleanQuery`.

Changed:
- **Exact** (no glob) → `TermQuery` on `package` (unchanged)
- **Suffix** (`*.collect`) → `RegexQuery` on `package_rev` with reversed prefix pattern
- **Prefix/Mixed** (`com.google.*`, `com.*.collect`) → `RegexQuery` on `package` (FST automata replaces manual iteration)

This eliminates ~30 lines of manual segment/term iteration code.

With this rewrite, `build_package_filter` always returns `Some(query)` for glob patterns (the `RegexQuery` naturally returns zero results when nothing matches). The `Option` return type can be simplified, or the caller's `None` short-circuit path becomes dead code for glob inputs.

## Not Changed

- **GAV glob filter** (`build_gav_filter`): GAV count is typically small (tens to hundreds); current `list_gavs()` + in-memory filter approach is adequate.
- **`list_packages()` / `list_packages_for_dependency()`**: These enumerate all packages or aggregate by dependency — term iteration is appropriate for full enumeration, not a glob filter optimization target.
- **`src/cli/pkgs.rs` client-side filtering**: Uses `matches_glob_pattern` for in-memory post-query filtering, separate from the index-level `build_package_filter`. Not affected by this change.
- **n-gram indexing**: Excluded from scope due to index size tradeoff (~200-300% increase).
- **`fqn_rev`**: FQN suffix glob is rare; can be added later if needed.

## Testing

- Unit tests for `classify_glob` — verify Prefix/Suffix/Mixed classification
- Unit tests for `reverse_str` — basic reversal, Unicode safety
- Integration tests: suffix glob queries (`*List`, `*.collect`) return correct results via reversed fields
- Schema migration: existing indexes auto-rebuild (covered by `is_schema_compatible` check)

## Index Size Impact

- Two additional fields per document (reversed strings, similar size to originals)
- Two FAST columns (low cardinality, minimal overhead)
- Estimated total increase: ~20-30%
