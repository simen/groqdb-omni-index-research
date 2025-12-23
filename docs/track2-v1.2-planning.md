# Track 2: v1.2 Planning - `in` Operator & Unicode Support

**Agent**: shard
**Status**: Approved
**Dependencies**: hash (predicate.rs), bloom (mod.rs), probe (planner.rs)

---

## Executive Summary

This document outlines the v1.2 roadmap for Holodex fingerprinting. The primary goals are:

1. **`in` operator support** - Enable pre-filtering for `field in ["a", "b", "c"]` queries (~15.6% of production)
2. **Unicode normalization** - NFKD normalization for improved internationalized content search
3. **Middle wildcard handling** - Safe fallback behavior for patterns like `"hel*rld"`

---

## v1.2 Features (Prioritized)

### 1. `in` Operator Support

**Production Impact**: ~15.6% of queries use `in` operator

#### Design

The `in` operator requires OR semantics (union) rather than AND semantics (intersection):

```rust
// CandidateSet::union() - new method for OR predicates
impl CandidateSet {
    pub fn union(self, other: CandidateSet) -> CandidateSet {
        match (self, other) {
            (CandidateSet::All, _) | (_, CandidateSet::All) => CandidateSet::All,
            (CandidateSet::None, other) | (other, CandidateSet::None) => other,
            (CandidateSet::Candidates(a), CandidateSet::Candidates(b)) => {
                CandidateSet::Candidates(a.union(&b).copied().collect())
            }
        }
    }
}
```

**Symmetry with `intersect()`:**
```
AND (v1):  All ∩ X = X,   None ∩ X = None
OR (v1.2): All ∪ X = All, None ∪ X = X
```

#### FPR Math

OR predicates use **additive** FPR (vs multiplicative for AND):

| Query | FPR Calculation | Combined FPR |
|-------|-----------------|--------------|
| `a == 1 && b == 2` | 1% × 1% | ~0.01% |
| `field in ["a", "b", "c"]` | 1% + 1% + 1% | ~3% |

**Formula**: `FPR_in = min(0.10, n × FPR_single)` where n = array length

**10% cap rationale**: At 10+ values, pre-filter overhead approaches benefit threshold. Capping prevents threshold inflation (50% FPR → threshold 1.0 → pre-filter never used).

#### Implementation Tasks

| Task | Owner | Module |
|------|-------|--------|
| `HolodexPredicate::In { path, values }` | hash | predicate.rs |
| `predicate_to_in_lookups()` helper | hash | predicate.rs |
| `estimated_fpr()` for In variant | hash | predicate.rs |
| `CandidateSet::union()` | bloom | mod.rs |
| `candidates_in()` method | bloom | mod.rs |
| Query routing for In predicates | bloom | mod.rs |
| (none) | probe | planner.rs |
| (none) | shard | fingerprint.rs |

**Note**: Planner and fingerprint modules require no changes - FPR abstraction handles cost model automatically.

---

### 2. Unicode Normalization (NFKD)

**Goal**: Improve search recall for internationalized content

#### Current Behavior

```rust
pub fn extract_trigrams(text: &str) -> Vec<String> {
    let clean = text.to_lowercase();
    // ...
}
```

- `"café"` → `["caf", "afé"]` (accent preserved)
- `"ﬁnd"` → `["ﬁnd"]` (ligature preserved as single char)

#### Proposed Behavior (NFKD + Combining Mark Removal)

```rust
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

pub fn extract_trigrams(text: &str) -> Vec<String> {
    let normalized: String = text
        .nfkd()
        .filter(|c| !is_combining_mark(*c))  // Strip combining marks
        .collect();
    let clean = normalized.to_lowercase();
    // ... rest unchanged
}
```

- `"café"` → `"cafe"` → `["caf", "afe"]` (searchable as "cafe")
- `"naïve"` → `"naive"` → `["nai", "aiv", "ive"]` (diacritics removed)
- `"ﬁnd"` → `"find"` → `["fin", "ind"]` (ligature expanded)
- `"résumé"` → `"resume"` → `["res", "esu", "sum", "ume"]`

**Key insight**: Stripping combining marks after NFKD improves search recall. Users searching "cafe" should find documents containing "café".

#### NFKD Benefits

| Input | NFD | NFKD (chosen) |
|-------|-----|---------------|
| `ﬁ` | `ﬁ` (1 char) | `fi` (2 chars) |
| `é` | `e` + `́` | `e` + `́` |
| `ℌ` | `ℌ` | `H` |
| `①` | `①` | `1` |

NFKD provides both decomposition (é → e + accent) and compatibility mapping (ﬁ → fi).

#### Implementation Tasks

| Task | Owner | Module |
|------|-------|--------|
| Add `unicode-normalization` to Cargo.toml | shard | - |
| NFKD in `extract_trigrams()` | shard | fingerprint.rs |
| NFKD in query-time trigram extraction | hash | predicate.rs |
| (none) | bloom | mod.rs |
| (none) | probe | planner.rs |

**Normalization boundary:**
```
fingerprint.rs: NFKD → lowercase → extract_trigrams → hash
predicate.rs:   NFKD → lowercase → extract_trigrams → lookup
mod.rs/planner.rs: unchanged (FPR unaffected)
```

---

### 3. Middle Wildcard Handling

**Goal**: Safe behavior for patterns like `"hel*rld"`

#### Challenge

Middle wildcards break contiguous trigram assumption:
- `"hello*world"` → segments `["hello", "world"]`
- Each segment produces trigrams independently
- Trigrams from different segments may correlate (same document type)

#### Options Analysis

| Option | FPR | Complexity | Risk |
|--------|-----|------------|------|
| **Option 1: Full scan fallback** | N/A | Low | None |
| Option 2: Segment-based filtering | ~4% (optimistic) | Medium | Correlated segments |

**Option 2 detail:**
- Split on `*`, extract trigrams per segment
- AND all segment results
- `"hel*rld"` → AND(trigrams("hel"), trigrams("rld"))
- Assumes independence; real FPR may be higher due to correlation

#### Decision

**Option 1 (full scan fallback)** for v1.2:
- Detect middle wildcard in `clean_match_pattern()`
- Return `CandidateSet::All` (no filtering)
- Simple, safe, no false negatives

**Revisit in v1.3** if production telemetry shows:
- High middle wildcard usage (>5% of match queries)
- Significant performance impact from full scans

#### Implementation Tasks

| Task | Owner | Module |
|------|-------|--------|
| Detect middle wildcards in pattern | hash | predicate.rs |
| Return empty ngrams for middle wildcards | hash | predicate.rs |
| (none - All returned) | bloom | mod.rs |
| (none) | probe | planner.rs |
| (none) | shard | fingerprint.rs |

---

## Deferred to v1.3

### Bigrams for 2-Character Patterns

**Rationale for deferral:**

Current behavior: Patterns < 3 chars return `CandidateSet::All` (full scan)

Proposed: Add bigram (2-char) hashing for short patterns

**FPR Analysis:**
```
Trigrams (3+ chars): 10-20% FPR → useful filtering
Bigrams (2 chars):   30-40% FPR → marginal benefit
```

**Math:**
- 2-char pattern → 1 bigram → 35% FPR → threshold 0.85
- Even 2 bigrams: ~20% combined FPR
- Planner threshold becomes very conservative

**Filter size impact:**
- Average text field: 100 chars
- Bigrams per field: ~99 (vs ~98 trigrams)
- Nearly doubles ngram hash count with marginal filtering benefit

**Decision**: Defer to v1.3, evaluate with production data showing high-value 2-char patterns.

---

## v1.2 Work Distribution Summary

| Module | Owner | Changes |
|--------|-------|---------|
| fingerprint.rs | shard | NFKD normalization only |
| predicate.rs | hash | `In` variant, middle wildcard detection, NFKD |
| mod.rs | bloom | `CandidateSet::union()`, `candidates_in()`, routing |
| planner.rs | probe | None (FPR abstraction handles it) |

**Observation**: v1.2 work is concentrated in predicate.rs and mod.rs. The FPR abstraction from v1.1 continues to pay dividends - planner needs no changes.

---

## Testing Plan

### Unit Tests

1. `test_candidate_set_union()` - basic union semantics
2. `test_candidate_set_union_with_all()` - All ∪ X = All
3. `test_candidate_set_union_with_none()` - None ∪ X = X
4. `test_extract_trigrams_nfkd()` - ligature/accent handling
5. `test_in_predicate_fpr()` - additive FPR calculation
6. `test_middle_wildcard_fallback()` - returns empty ngrams

### Integration Tests

1. `test_holodex_in_basic()` - simple in query
2. `test_holodex_in_mixed()` - in + equality predicates
3. `test_holodex_unicode_search()` - café/cafe matching
4. `test_holodex_middle_wildcard()` - verifies full scan behavior

---

## Open Questions

1. **NFKD performance**: Should we cache normalized strings for repeated extractions?
   - Recommendation: Profile first, optimize if needed

2. **`in` array size limit**: Should we cap array size for FPR sanity?
   - **Decision**: Cap FPR at 10% via `min(0.10, n × 0.01)`
   - Rationale: Prevents threshold inflation; keeps pre-filter useful for large arrays

3. **Middle wildcard telemetry**: How do we measure usage?
   - Recommendation: Add counter in predicate.rs for rejected middle wildcards

---

*v1 - shard - Initial v1.2 planning*
*v2 - shard - Added 10% FPR cap decision per team review*
