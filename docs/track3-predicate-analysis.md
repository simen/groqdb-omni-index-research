# Track 3: GROQ Predicate Analysis

**Author**: hash
**Status**: Draft
**Date**: 2024-12-22

## Executive Summary

Analysis of 2,837 filter predicates across 169 test files in the groqdb test suite reveals:

1. **Equality dominates**: 21.1% of predicates use `==`, with `_type` filters at 10.4%
2. **Range queries are significant**: 14% combined (`>`, `<`, `>=`, `<=`)
3. **Match is rare**: Only 1.4% of predicates use `match`
4. **Most predicates are indexable today**: 84.6% use `_type` or `_id` which already have index support

**Recommendation**: Holodex v1 should focus on equality predicates on arbitrary paths. Range support can be deferred to v2.

---

## Methodology

Extracted all `*[...]` filter expressions from the groq-test-suite test files using pattern matching. Categorized predicates by:
- Operator type (equality, range, match, in, etc.)
- Path characteristics (nested, array traversal, dereference)
- Special fields (_type, _id, _ref)
- Function usage (defined, count, references, etc.)

---

## Predicate Type Distribution

### Operators (sorted by frequency)

| Operator | Count | Percentage | Holodex Support |
|----------|-------|------------|-----------------|
| `==` (equality) | 598 | 21.1% | v1 |
| `>` / `>=` | 202 | 7.1% | v2 |
| `<` / `<=` | 196 | 6.9% | v2 |
| `!=` | 106 | 3.7% | Challenging* |
| `in` | 94 | 3.3% | v1 (array membership) |
| `match` | 39 | 1.4% | v2 (n-gram signatures) |

*Negation is fundamentally hard for probabilistic filters - proving absence requires different techniques.

### Logical Operators

| Operator | Count | Percentage | Notes |
|----------|-------|------------|-------|
| `&&` (AND) | 730 | 25.7% | Intersection of candidate sets |
| `||` (OR) | 5 | 0.2% | Union of candidate sets |
| `!` (NOT) | 122 | 4.3% | Requires complement logic |

### Path Characteristics

| Pattern | Count | Percentage | Notes |
|---------|-------|------------|-------|
| `_id` filter | 2,475 | 87.2% | Already indexed |
| `_type` filter | 296 | 10.4% | Already indexed |
| Nested path (`.`) | 298 | 10.5% | Holodex target |
| Array traversal (`[]`) | 34 | 1.2% | Holodex target |
| Dereference (`->`) | 21 | 0.7% | Cross-document, hard |
| Parent scope (`^`) | 84 | 3.0% | Runtime-dependent |

### Function Usage

| Function | Count | Percentage | Notes |
|----------|-------|------------|-------|
| `references()` | 35 | 1.2% | Deferred |
| `defined()` | 19 | 0.7% | Path-only hashes |
| `count()` | 5 | 0.2% | Aggregation, not filtering |
| `string::*` | 8 | 0.3% | Transformation |

---

## Full Scan Risk Analysis

### Current State

With existing groqdb indexes (`_type`, `_id`):

| Category | Predicate Count | Percentage |
|----------|-----------------|------------|
| Indexable today | 2,400 | 84.6% |
| Needs Holodex | 437 | 15.4% |

### What Causes Full Scans

Risk factors for predicates that would require full scan today:

| Risk Factor | Count | Example |
|-------------|-------|---------|
| Equality on nested path | 236 | `department->_id == "engineering"` |
| Parent scope reference | 84 | `^._id in sources[]._ref` |
| Range on field | 42 | `vote_average > 8.0` |
| Match operator | 39 | `text match "mario"` |
| References function | 35 | `references(^._id)` |
| Array traversal | 30 | `body[].children[].text` |
| Deep nested path | 28 | `metadata.stats.views` |
| Dereference chain | 21 | `author->organization->name` |
| Defined check | 19 | `defined(department->)` |

---

## Realistic Query Patterns (from perf tests)

### High-Value Patterns for Holodex

1. **Type + field filter** (very common)
   ```groq
   *[_type == "movie" && vote_average > 8.0]
   ```
   - `_type` narrows via existing index
   - `vote_average > 8.0` requires scan of all movies
   - Holodex could eliminate non-matching movies

2. **Nested array access**
   ```groq
   *[_type == "movie" && spoken_languages[0] == "nb"]
   ```
   - Array index access, needs normalization

3. **Join with filter**
   ```groq
   *[_type == "person" && _id in ^.cast[].person._ref && gender == "f"]
   ```
   - Parent scope + array traversal + equality
   - `gender == "f"` is the Holodex opportunity

4. **Reference traversal**
   ```groq
   *[_type == "movie"]{poster->{path}, genres[]->name}
   ```
   - Dereference in projection (not filter) - less critical

---

## Priority Ranking for Holodex Optimizations

### Tier 1: Must Have (v1)

| Feature | Impact | Complexity | Rationale |
|---------|--------|------------|-----------|
| Root-level equality | High | Low | 21% of predicates, simple hash |
| `_type` special-casing | High | Low | 10.4% of predicates, exact match |
| Nested path equality | High | Medium | 10.5% of predicates |
| `defined()` support | Medium | Low | Path-only hashes |

### Tier 2: Should Have (v1.1)

| Feature | Impact | Complexity | Rationale |
|---------|--------|------------|-----------|
| Array traversal (`[]`) | Medium | Medium | 1.2% but common in real queries |
| `in` operator (array) | Medium | Medium | 3.3% of predicates |
| Boolean operators (`&&`) | High | Low | Combine candidate sets |

### Tier 3: Nice to Have (v2)

| Feature | Impact | Complexity | Rationale |
|---------|--------|------------|-----------|
| Range queries | Medium | High | 14% but needs different structure |
| `match` operator | Low | High | 1.4%, needs n-gram hashing |
| Dereference chains | Low | Very High | 0.7%, cross-document joins |

### Tier 4: Defer

| Feature | Rationale |
|---------|-----------|
| Negation (`!=`, `!defined()`) | Fundamentally hard for Bloom filters |
| Parent scope (`^`) | Runtime-dependent, can't pre-compute |
| `references()` | Requires join indexes |

---

## Recommendations

1. **Start with equality on any path** - This covers the most ground with minimal complexity

2. **Special-case `_type`** - At 10.4% frequency, `_type` deserves an exact index alongside the Bloom filter

3. **Normalize array indexes early** - `body[0]` → `body[*]` loses precision but enables indexing

4. **Defer range queries** - At 14%, they're significant but require a fundamentally different approach (interval trees, learned indexes). Get equality working first.

5. **Ignore negation for now** - Proving absence is the opposite of what Bloom filters do. Accept false positives on negated predicates.

---

## Cross-Track Dependencies

- **Track 1 (bloom)**: Which filter type provides best FPR for equality-heavy workload?
- **Track 2 (shard)**: Path normalization strategy confirmed - array indexes become `[*]`
- **Track 4 (probe)**: Benchmark framework should prioritize equality + nested path queries

---

## Appendix: Raw Data

### Test Suite Statistics

- Total test files: 169
- Total queries: 1,384
- Total filter predicates: 2,837
- Files with queries: 169

### Path Depth Distribution

| Depth | Count | Percentage |
|-------|-------|------------|
| 1 | 2,727 | 96.1% |
| 2 | 100 | 3.5% |
| 3 | 10 | 0.4% |

Most predicates use shallow paths - deep nesting is rare in the test suite.

### Example Predicates by Category

**Equality**:
- `_type == "movie"`
- `department->_id == "engineering"`
- `metadata.featured == true`

**Match**:
- `text match "mario"`
- `title match "star*"`

**Range**:
- `vote_average > 8.0`
- `runtime > 260`
- `pingedAt < "2017-05-08"`

**Array Traversal**:
- `^._id in sources[]._ref`
- `body[].children[].text`
- `genres[]->name`
