# Track 4: Range Query Strategies

**Author**: bloom
**Status**: Complete
**Last Updated**: 2024-12-22

---

## Executive Summary

**Recommendation: Defer range query support to Holodex v2.**

Range predicates (`>`, `<`, `>=`, `<=`) and text matching (`match`) require fundamentally different data structures than equality testing. Based on @hash's predicate analysis showing only 9.4% range + 15.6% match predicates in the test suite, the ROI for v1 doesn't justify the complexity.

Focus v1 on equality predicates where probabilistic filters excel.

---

## The Challenge

Bloom-style filters answer: "Is X a member of set S?"

Range queries ask: "Are there any members of S where X > threshold?"

These are fundamentally different questions. Membership filters cannot be directly adapted for ordering relationships.

---

## Approaches Surveyed

### 1. Bucketed Bloom Filters

**Approach**: Divide numeric range into buckets, insert bucket IDs into filter.

```
Value: 1500
Buckets: [0-999, 1000-1999, 2000-2999, ...]
Insert: hash("views:bucket_1")  // bucket containing 1500
```

**Query**: `views > 1000` → check all buckets > bucket_1

**Problems**:
- Bucket granularity vs filter size trade-off
- Range queries check many buckets → high FPR
- Different fields need different bucketing schemes
- Schema-less: we don't know value distributions upfront

**Verdict**: Impractical for arbitrary fields

### 2. bloomRF (Range Bloom Filters)

**Paper**: [bloomRF: On Performing Range-Queries with Bloom-Filters](https://arxiv.org/abs/2012.15596)

**Approach**: Dyadic interval decomposition + piecewise-monotone hash functions

**Pros**:
- Unified structure for point and range queries
- 4x faster than previous point-range filters

**Cons**:
- Complex implementation
- Higher space overhead than standard Bloom
- No Rust implementation exists
- Designed for integer keys, needs adaptation for JSON paths

**Verdict**: Interesting research, but too immature for v1

### 3. Separate Interval Trees

**Approach**: Maintain interval trees for numeric fields alongside Holodex

**Rust crates available**:
- [`rust-lapper`](https://github.com/sstadick/rust-lapper) - Fast, serde support
- [`intervaltree`](https://docs.rs/intervaltree) - Simple, immutable
- [`unbounded-interval-tree`](https://crates.io/crates/unbounded-interval-tree) - Based on CLRS

**Pros**:
- O(log n + m) query time
- Well-understood data structure
- Good Rust implementations exist

**Cons**:
- Requires knowing which fields have numeric values
- Separate index per field → space explosion
- Doesn't integrate with probabilistic filtering
- Still need to track (doc_id, field, value) triples

**Verdict**: Works, but doesn't fit Holodex's "one index for everything" philosophy

### 4. Learned Indexes

**Paper**: [The Case for Learned Index Structures](https://arxiv.org/abs/1712.01208) (Kraska et al.)

**Approach**: Train neural network to predict value position, use for range estimation

**Pros**:
- Can achieve 3x speedup over B-trees
- Adapts to data distribution

**Cons**:
- 400x slower lookups than Bloom on CPU
- Requires training data (query workload)
- Complex to implement and maintain
- Overkill for our use case

**Verdict**: Not suitable for v1

### 5. Text Matching: N-gram Bloom Filters

**Used by**: [StarRocks](https://docs.starrocks.io/docs/table_design/indexes/Ngram_Bloom_Filter_Index/), [Apache Doris](https://doris.apache.org/docs/table-design/index/ngram-bloomfilter-index/), [ClickHouse](https://www.tinybird.co/blog/using-bloom-filter-text-indexes-in-clickhouse)

**Approach**: Tokenize strings into n-grams, insert each into filter

```
Text: "hello world"
3-grams: ["hel", "ell", "llo", "lo ", "o w", " wo", "wor", "orl", "rld"]
Insert all into Bloom filter
```

**Query**: `match "world*"` → check ["wor", "orl", "rld"] are present

**Pros**:
- Well-established technique
- Production-proven (ClickHouse reports 88x speedup)
- Fits Bloom filter model

**Cons**:
- Inflates filter size significantly (each value becomes many n-grams)
- Wildcard position matters (prefix vs suffix vs infix)
- GROQ `match` semantics need careful mapping

**Verdict**: Viable for v2, but adds complexity and space

---

## Why Defer to v2

### 1. Low Priority Based on Predicate Analysis

From @hash's Track 3 analysis:
- 75% of predicates are equality (Holodex v1 target)
- 9.4% are range comparisons
- 15.6% are match/text operations

Equality is the clear priority.

### 2. Different Index Structures Needed

Range queries fundamentally need ordered structures (B-trees, interval trees). Probabilistic filters are designed for membership testing, not ordering.

Bolting range support onto Holodex would either:
- Compromise the clean design
- Require maintaining parallel structures anyway

### 3. Complexity Cost

Each approach adds:
- New data structures to build/serialize/query
- Edge cases (null handling, type coercion)
- Integration complexity with query planner

For 25% of predicates, the ROI is poor.

### 4. Baseline is Acceptable

Without range index, queries like `*[views > 1000]` fall back to:
1. Use `_type` index (if present) for initial filtering
2. Full scan remaining candidates

This is the status quo. We're not regressing.

---

## Recommendation for v2

When we do add range support, consider:

### For Numeric Ranges

**Option A**: Sorted arrays + binary search per field
- Simple, compact, fast enough for moderate cardinality
- Build: extract (doc_id, value) pairs, sort by value
- Query: binary search for threshold, return doc_ids

**Option B**: Interval trees for high-cardinality numeric fields
- Use `rust-lapper` for its serde support and performance
- Build index per frequently-queried numeric path

### For Text Matching

**Option A**: N-gram Bloom filters
- 3-grams are standard
- Insert all n-grams from text values
- Query: extract n-grams from pattern, check all present

**Option B**: Separate full-text index
- May be out of scope for Holodex
- Point to existing solutions (tantivy, meilisearch)

---

## Conclusion

| Query Type | v1 Support | v2 Recommendation |
|------------|------------|-------------------|
| Equality (`==`) | Yes (Holodex core) | - |
| Range (`>`, `<`) | No (full scan) | Sorted arrays or interval trees |
| Text (`match`) | No (full scan) | N-gram Bloom filters |
| Negation (`!=`) | No (full scan) | Inverted membership check* |

*Negation is hard: "not in filter" has high FPR because filter might contain element by chance.

---

## References

- [bloomRF Paper](https://arxiv.org/abs/2012.15596) - Range-query Bloom filters
- [N-gram Bloom Filter Index - StarRocks](https://docs.starrocks.io/docs/table_design/indexes/Ngram_Bloom_Filter_Index/)
- [N-gram Bloom Filter Index - Apache Doris](https://doris.apache.org/docs/table-design/index/ngram-bloomfilter-index/)
- [The Case for Learned Index Structures](https://arxiv.org/abs/1712.01208)
- [rust-lapper](https://github.com/sstadick/rust-lapper) - Fast interval tree in Rust
- [Trigram Indexing for Regex](https://swtch.com/~rsc/regexp/regexp4.html) - Russ Cox
