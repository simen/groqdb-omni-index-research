# Track 2: v1.1 N-gram Support for `match` Operator

**Agent**: shard
**Status**: Design (v1.1)
**Dependencies**: hash (predicate extraction), bloom (filter capacity), probe (planner cost model)

---

## Executive Summary

This document specifies the fingerprinting changes needed to support the GROQ `match` operator via n-gram hashing. The `match` operator accounts for ~14.4% of production queries that v1 cannot accelerate.

---

## Design Goals

1. **Enable `match` acceleration**: Pre-filter documents for text search queries
2. **Acceptable FPR tradeoff**: 10-20% FPR acceptable for `match` (vs <1% for equality)
3. **Selective indexing**: Only n-gram text fields that benefit from it
4. **Backward compatible**: v1 equality behavior unchanged

---

## N-gram Hashing Algorithm

### Overview

For `match` queries like `*[body match "mario"]`, we:
1. **Index time**: Extract trigrams from qualifying text fields
2. **Query time**: Extract trigrams from match pattern, AND them together

### Trigram Extraction

```rust
/// Extract trigrams from text (lowercase, 3-char windows)
pub fn extract_trigrams(text: &str) -> Vec<String> {
    let clean = text.to_lowercase();
    clean.chars()
        .collect::<Vec<_>>()
        .windows(3)
        .map(|w| w.iter().collect())
        .collect()
}

// Example: "Mario" → ["mar", "ari", "rio"]
```

### Fingerprint Hashing

```rust
/// Hash a (path, ngram) pair for match queries
pub fn hash_path_ngram(path: &str, ngram: &str) -> u64 {
    // Distinct type tag avoids collision with exact string matches
    let combined = format!("{}:ngram:{}", path, ngram.to_lowercase());
    xxh64(combined.as_bytes(), 0)
}

/// Extract all ngram hashes for a text value
pub fn extract_ngram_hashes(path: &str, text: &str) -> Vec<u64> {
    extract_trigrams(text)
        .iter()
        .map(|ngram| hash_path_ngram(path, ngram))
        .collect()
}
```

---

## Selective N-gram Indexing

### Heuristic

Not all string fields benefit from n-gram indexing. We use a simple heuristic:

```rust
/// Determine if a field should have n-gram hashes
fn should_ngram(path: &str, text: &str) -> bool {
    text.len() > 10  // Only index strings > 10 chars
}
```

**Rationale**:
- Short strings (like `_type`, `status`, `_id`) are better served by exact equality
- `match` queries target body text, descriptions - typically longer strings
- Threshold of 10 chars catches meaningful content while excluding metadata

### Alternative considered

Combined heuristic with path patterns:
```rust
fn should_ngram(path: &str, text: &str) -> bool {
    text.len() > 10
    || path.ends_with(".text")      // Portable Text convention
    || path.ends_with(".description")
}
```

**Decision**: Start with length-only heuristic for simplicity. Can add path patterns in v1.2 if needed.

---

## Integration with Document Fingerprinting

### Modified `visit_paths()`

```rust
fn visit_paths<F>(value: &Value, path: String, callback: &mut F)
where
    F: FnMut(&str, &Value, bool),  // (path, value, is_ngram)
{
    match value {
        Value::String(s) => {
            // Always emit exact value hash
            callback(&path, value, false);

            // Conditionally emit ngram hashes
            if should_ngram(&path, s) {
                for ngram in extract_trigrams(s) {
                    // Emit as ngram hash
                    callback(&format!("{}:ngram:{}", path, ngram), value, true);
                }
            }
        }
        // ... other cases unchanged
    }
}
```

### Alternative: Separate ngram collection

```rust
pub fn extract_fingerprints_v1_1(doc: &Value) -> (Vec<u64>, Vec<u64>) {
    let mut exact_hashes = Vec::new();
    let mut ngram_hashes = Vec::new();

    visit_paths(doc, String::new(), &mut |path, value| {
        exact_hashes.push(hash_path_value(path, value));
        exact_hashes.push(hash_path_exists(path));

        if let Value::String(s) = value {
            if should_ngram(path, s) {
                ngram_hashes.extend(extract_ngram_hashes(path, s));
            }
        }
    });

    (exact_hashes, ngram_hashes)
}
```

**Decision**: Keep single hash collection for simplicity. Ngram hashes go into same BinaryFuse8 filter.

---

## Query-time Predicate Matching

### Match Predicate Flow

```
Query: *[body match "mario"]

1. hash extracts: HolodexPredicate::Match {
       path: "body",
       pattern: "mario",
       ngrams: ["mar", "ari", "rio"]
   }

2. shard computes hashes:
   - hash_path_ngram("body", "mar")
   - hash_path_ngram("body", "ari")
   - hash_path_ngram("body", "rio")

3. Holodex filters: document must contain ALL ngram hashes

4. Candidates returned (with ~10-20% FPR)
```

### `candidates_match()` Implementation

```rust
impl Holodex {
    /// Get candidates for a match predicate
    pub fn candidates_match(&self, path: &str, ngrams: &[String]) -> CandidateSet {
        if ngrams.is_empty() {
            return CandidateSet::All;  // Pattern too short
        }

        let normalized = normalize_path(path);

        // Start with all documents
        let mut result = CandidateSet::All;

        // Intersect candidates for each ngram (AND semantics)
        for ngram in ngrams {
            let hash = hash_path_ngram(&normalized, ngram);
            let mut candidates = HashSet::new();

            for (idx, filter) in self.filters.iter().enumerate() {
                if filter.contains(hash) {
                    candidates.insert(idx);
                }
            }

            let ngram_candidates = if candidates.is_empty() {
                CandidateSet::None
            } else {
                CandidateSet::Candidates(candidates)
            };

            result = result.intersect(ngram_candidates);
        }

        result
    }
}
```

---

## FPR Analysis

### Per-trigram FPR

With BinaryFuse8 (~0.4% base FPR), each trigram lookup has ~0.4% false positive rate.

### Combined FPR for Match Queries

For a pattern with N trigrams, all must match (AND semantics):

| Pattern Length | Trigrams | Combined FPR |
|---------------|----------|--------------|
| 3 chars | 1 | 0.4% |
| 4 chars | 2 | ~0.4% (dominated by matches) |
| 5 chars | 3 | ~0.4% |
| 10 chars | 8 | <0.1% |

However, common trigrams may appear in many documents, raising effective FPR:
- Common English trigrams: "the", "ing", "and" → high collision
- Rare trigrams: "xyz", "qwr" → low collision

**Estimated real-world FPR**: 10-20% for typical search patterns.

### Impact on Planner Cost Model

probe's cost model adjustments for v1.1:
- `MATCH_FPR_ESTIMATE = 0.15`  // 15% expected FPR for match
- `MIN_MATCH_REDUCTION = 0.70`  // Require 70% reduction (vs 50% for equality)

---

## Filter Capacity Impact

### Additional hashes per document

Estimate for typical Sanity document:
- 3-5 text fields > 10 chars
- Average text length: 100 chars
- Trigrams per field: ~98 (100 - 2)
- Total ngram hashes: 300-500 additional per document

### Index size increase

| v1 | v1.1 (with ngrams) | Increase |
|----|-------------------|----------|
| ~190 bytes/doc | ~300-400 bytes/doc | 50-110% |

bloom to validate filter capacity with increased hash count.

---

## API Changes

### New public functions in fingerprint.rs

```rust
// v1.1 additions
pub fn hash_path_ngram(path: &str, ngram: &str) -> u64;
pub fn extract_ngram_hashes(path: &str, text: &str) -> Vec<u64>;
pub fn extract_trigrams(text: &str) -> Vec<String>;
fn should_ngram(path: &str, text: &str) -> bool;
```

### Modified `extract_fingerprints()`

```rust
// v1.1: Now includes ngram hashes for qualifying text fields
pub fn extract_fingerprints(doc: &Value) -> Vec<u64>;
```

Backward compatible - returns more hashes but same type.

---

## Testing Plan

### Unit tests

1. `test_extract_trigrams()` - basic trigram extraction
2. `test_extract_trigrams_unicode()` - handle non-ASCII
3. `test_hash_path_ngram_distinct()` - ngram hashes differ from exact
4. `test_should_ngram()` - heuristic correctness
5. `test_candidates_match()` - end-to-end match filtering

### Integration tests

1. `test_holodex_match_basic()` - simple match query
2. `test_holodex_match_portable_text()` - Portable Text body search
3. `test_holodex_match_fpr()` - measure actual FPR on test corpus

### Benchmarks

1. Construction overhead with ngrams
2. Query latency for match predicates
3. Filter size increase measurement

---

## Implementation Tasks

| Task | Owner | Status |
|------|-------|--------|
| `should_ngram()` heuristic | shard | pending |
| `hash_path_ngram()` | shard | pending |
| `extract_ngram_hashes()` | shard | pending |
| `extract_trigrams()` | shard | pending |
| `candidates_match()` | shard | pending |
| `HolodexPredicate::Match` | hash | pending |
| `extract_ngrams()` in predicate.rs | hash | pending |
| Planner cost model for match | probe | pending |
| Filter capacity testing | bloom | pending |

---

## Open Questions

1. **Unicode handling**: Should we normalize Unicode before trigram extraction?
   - Option A: Simple `to_lowercase()` (current design)
   - Option B: Full Unicode normalization (NFD/NFC)
   - **Recommendation**: Start with Option A, add normalization if needed

2. **Minimum pattern length**: Patterns < 3 chars can't produce trigrams
   - Option A: Return `CandidateSet::All` (no filtering)
   - Option B: Use bigrams for 2-char patterns
   - **Recommendation**: Option A for v1.1, consider bigrams in v1.2

3. **Wildcard patterns**: `"mar*"` should only extract from "mar"
   - hash's `extract_ngrams()` should handle this
   - Strip wildcards before trigram extraction

---

*v1 - shard - Initial v1.1 ngram design*
