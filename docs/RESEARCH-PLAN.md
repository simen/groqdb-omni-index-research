# Omni-Index Research Plan

## Overview

This document outlines the research plan for developing a probabilistic "omni-index" for GROQ query engines. The goal is to enable efficient query execution for arbitrary predicates without requiring explicit index configuration.

---

## Phase 1: Literature Review & Foundation (Week 1-2)

### 1.1 Survey Probabilistic Data Structures

**Bloom Filters**
- Classic probabilistic membership testing
- Space: ~10 bits per element for 1% FPR
- Limitations: No deletion, no counting, fixed capacity
- Variants: Counting Bloom, Scalable Bloom, Blocked Bloom

**Cuckoo Filters**
- Modern alternative supporting deletion
- Better space efficiency than counting Bloom
- ~12 bits per element for 1% FPR with deletion support
- Good cache locality

**Quotient Filters**
- Space-efficient, supports deletion and counting
- Cache-oblivious design
- Can be merged efficiently

**XOR Filters**
- Read-only, space-optimal (~9 bits/element for 1% FPR)
- Perfect for static datasets (fits groqdb's read-only model!)
- Very fast lookup

### 1.2 Survey Database Indexing Techniques

**Inverted Indexes**
- Standard for text search
- Maps terms → document sets
- Efficient for equality and text matching

**Bitmap Indexes**
- Maps values → bitmaps of matching documents
- Excellent for low-cardinality fields
- Compressible (Roaring bitmaps)

**Signature Files**
- Hash document content into fixed-size signatures
- Query signatures ⊆ document signatures → possible match
- Classic IR technique, largely superseded but relevant for our use case

**Learned Indexes**
- ML models replacing/augmenting traditional indexes
- Promising for predictable access patterns
- May be overkill for our use case

### 1.3 Deliverables
- [ ] LITERATURE-REVIEW.md with detailed analysis
- [ ] Comparison matrix of data structures
- [ ] Initial recommendation for prototype

---

## Phase 2: Design Exploration (Week 3-4)

### 2.1 Core Design Questions

**Q1: What predicates do we need to support?**

GROQ predicates by type:
```groq
// Equality
field == "value"
field == 123

// Text matching
field match "term"
field match "term*"

// Comparisons
field > 100
field >= 100
field < 100
field <= 100

// Array membership
"value" in field
field in ["a", "b", "c"]

// Existence
defined(field)

// Nested paths
parent.child.field == "value"
```

**Q2: Per-document or global structure?**

Option A: **Per-document signatures**
- Each document has a compact signature encoding its content
- Query signature derived from predicate
- Filter: documents where query_sig ⊆ doc_sig
- Pros: Simple mental model, works for arbitrary predicates
- Cons: Must scan all signatures (though very fast)

Option B: **Global probabilistic index**
- Single structure mapping (field, value) → document set approximation
- Pros: Sub-linear lookup for specific predicates
- Cons: Harder to support arbitrary predicates, space overhead

Option C: **Hybrid approach**
- Global index for common patterns (_type, equality)
- Document signatures for arbitrary predicates
- Use query planner to select strategy

**Q3: How to handle path diversity?**

GROQ allows arbitrary paths:
- `title`
- `author.name`
- `posts[].comments[].author.name`

Options:
- Flatten all paths into a single namespace
- Hierarchical signatures (path prefix → signature)
- Path-aware hashing

### 2.2 Candidate Architectures

**Architecture A: Document Fingerprints**
```
For each document:
  fingerprint = Bloom filter of all (path, value) pairs

Query execution:
  query_fingerprint = Bloom filter of predicate conditions
  candidates = docs where query_fingerprint ⊆ doc.fingerprint
  results = evaluate(candidates, full_predicate)
```

**Architecture B: Partitioned Cuckoo Filters**
```
For each field path:
  filter[path] = Cuckoo filter of (value → doc_id)

Query execution:
  For equality predicate on path P with value V:
    candidates = filter[P].lookup(V)
  Intersect candidates across multiple predicates
```

**Architecture C: Signature Trees**
```
Hierarchical structure:
  Level 0: Document signatures (fine-grained)
  Level 1: Block signatures (OR of 64 doc signatures)
  Level 2: Super-block signatures (OR of 64 block signatures)

Query execution:
  Start at top level
  Prune branches where query_sig ⊄ block_sig
  Descend only into possible-match branches
```

### 2.3 Deliverables
- [ ] Design proposal documents for each architecture
- [ ] Space/time complexity analysis
- [ ] Recommendation with rationale

---

## Phase 3: Prototyping (Week 5-8)

### 3.1 Prototype Scope

Build minimal implementations of top 2 candidate architectures:
- Rust implementation for integration with groqdb
- Focus on core data structures, not full integration
- Support basic predicate types: equality, match, defined

### 3.2 Prototype Evaluation Criteria

**Space efficiency**
- Bits per document
- Scaling with document count
- Scaling with field diversity

**Query performance**
- False positive rate (actual vs theoretical)
- Filter throughput (documents/second)
- Memory access patterns

**Build performance**
- Index construction time
- Incremental update cost (if applicable)

### 3.3 Deliverables
- [ ] Prototype implementations in /prototypes
- [ ] Micro-benchmarks for each structure
- [ ] Evaluation report

---

## Phase 4: Integration Design (Week 9-10)

### 4.1 Query Planner Integration

How does the omni-index fit into groqdb's query pipeline?

Current pipeline:
```
Parse → Plan → Execute
              ↓
         Index Scan (type/id/ref)
              ↓
         Filter Evaluation
              ↓
         Projection
```

Proposed pipeline:
```
Parse → Plan → Execute
              ↓
         Index Scan (type/id/ref)
              ↓
         Omni-Index Pre-Filter ← NEW
              ↓
         Filter Evaluation (on candidates only)
              ↓
         Projection
```

### 4.2 API Design

```rust
trait OmniIndex {
    /// Build index from document iterator
    fn build(docs: impl Iterator<Item = Document>) -> Self;

    /// Get candidate document IDs that might match predicate
    fn candidates(&self, predicate: &Expr) -> CandidateSet;

    /// Estimated false positive rate for this predicate
    fn estimated_fpr(&self, predicate: &Expr) -> f64;
}

enum CandidateSet {
    /// All documents might match (predicate not indexable)
    All,
    /// Specific candidates (with possible false positives)
    Candidates(RoaringBitmap),
    /// No documents match (predicate definitely fails)
    None,
}
```

### 4.3 Deliverables
- [ ] Integration design document
- [ ] API specification
- [ ] Query planner modification proposal

---

## Phase 5: Benchmarking & Validation (Week 11-12)

### 5.1 Benchmark Suite

**Synthetic benchmarks**
- Varying document counts (1k, 10k, 100k, 1M)
- Varying field diversity (10, 100, 1000 unique paths)
- Varying value cardinality (low, medium, high)

**Real workload benchmarks**
- GROQ test suite queries
- Common Sanity query patterns
- Edge cases (highly selective, highly unselective)

### 5.2 Metrics

- **Speedup**: Query time with vs without omni-index
- **False positive rate**: Actual vs theoretical
- **Space overhead**: Index size vs document size
- **Build time**: Index construction cost

### 5.3 Deliverables
- [ ] Benchmark suite in /benchmarks
- [ ] Results analysis
- [ ] Final recommendation

---

## Success Criteria

The research is successful if we can demonstrate:

1. **Significant speedup** for queries without specific indexes
   - Target: 10x+ speedup for selective predicates

2. **Acceptable space overhead**
   - Target: <20% of document store size

3. **Bounded false positive rate**
   - Target: <5% false positives

4. **Practical build time**
   - Target: <100ms per 10k documents

---

## Open Questions

1. How do we handle text matching (`match`) efficiently?
   - N-gram signatures?
   - Prefix trees with probabilistic pruning?

2. How do we handle range queries?
   - Can we partition value space and use per-partition filters?
   - Learned indexes for range estimation?

3. How do we handle updates for non-static datasets?
   - XOR filters are read-only but optimal
   - Cuckoo filters support deletion but at space cost
   - Hybrid: XOR for stable data, Cuckoo for recent?

4. How do we handle compound predicates (AND/OR)?
   - AND: Intersect candidate sets
   - OR: Union candidate sets (may defeat pruning)

---

## Timeline Summary

| Phase | Duration | Deliverables |
|-------|----------|--------------|
| 1. Literature Review | 2 weeks | LITERATURE-REVIEW.md |
| 2. Design Exploration | 2 weeks | Design proposals |
| 3. Prototyping | 4 weeks | Prototype code + evaluation |
| 4. Integration Design | 2 weeks | Integration spec |
| 5. Benchmarking | 2 weeks | Final recommendation |

**Total: 12 weeks**

---

## References

- Bloom, B. H. (1970). "Space/time trade-offs in hash coding with allowable errors"
- Fan et al. (2014). "Cuckoo Filter: Practically Better Than Bloom"
- Graf & Lemire (2020). "XOR Filters: Faster and Smaller Than Bloom"
- Chambi et al. (2016). "Better bitmap performance with Roaring bitmaps"
- Kraska et al. (2018). "The Case for Learned Index Structures"

---

*Last updated: December 2024*
