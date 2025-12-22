# Track 5: Holodex Integration Architecture

**Author**: probe
**Status**: Draft v1
**Date**: 2024-12-22

---

## Executive Summary

This document defines how Holodex integrates into groqdb's query pipeline. The key insight is that Holodex acts as a **pre-filter stage** that sits between the query planner and document iteration, providing candidate document IDs that "might match" a predicate before expensive full evaluation.

---

## Current Architecture

### Query Execution Flow (Today)

```
┌─────────────┐     ┌──────────┐     ┌───────────────┐     ┌───────────┐
│   Parser    │────▶│ Planner  │────▶│ DocumentStore │────▶│ Evaluator │
│ (GROQ→AST)  │     │(IndexScan)│    │  (iter_*)     │     │(filter)   │
└─────────────┘     └──────────┘     └───────────────┘     └───────────┘
```

1. **Parser**: Converts GROQ string to AST (`Expr`)
2. **Planner**: Analyzes filter, selects `IndexScan` strategy:
   - `TypeFilter(type)` → use `_type` index
   - `IdLookup(id)` → use `_id` index
   - `RefLookup(id)` → use `_ref` index
   - `FullScan` → iterate all documents
3. **DocumentStore**: Returns iterator based on scan strategy
4. **Evaluator**: Filters iterator by evaluating `remaining_filter` predicate

### Current Limitations

The planner only indexes three fields: `_type`, `_id`, and references. Any other predicate falls through to `FullScan`:

```groq
// Uses _type index, but still scans all posts for title match
*[_type == "post" && title match "mario"]

// Full scan - no index for arbitrary nested paths
*[metadata.stats.views > 1000]

// Full scan - no index for text fields
*[body[].children[].text match "hello"]
```

---

## Proposed Architecture

### Query Execution Flow (With Holodex)

```
┌─────────────┐     ┌──────────┐     ┌─────────┐     ┌───────────────┐     ┌───────────┐
│   Parser    │────▶│ Planner  │────▶│ Holodex │────▶│ DocumentStore │────▶│ Evaluator │
│ (GROQ→AST)  │     │(IndexScan)│    │(prefilter)│   │ (iter_ids)    │     │(validate) │
└─────────────┘     └──────────┘     └─────────┘     └───────────────┘     └───────────┘
```

**Key Change**: Holodex sits between the planner and store, providing candidate IDs:

1. Planner selects initial `IndexScan` (same as today)
2. Planner extracts predicates that Holodex can help with
3. **Holodex queries signature index, returns candidate doc IDs**
4. Store iterates only those candidate documents
5. Evaluator validates candidates with full predicate evaluation

### Integration Point: The `PreFilter` Trait

```rust
/// Result of a pre-filter query
pub struct PreFilterResult {
    /// Candidate document IDs that might match
    pub candidates: BitSet,
    /// Estimated false positive rate (for query planning)
    pub estimated_fpr: f64,
    /// Whether the pre-filter could be applied
    pub applied: bool,
}

/// Trait for probabilistic pre-filtering
pub trait PreFilter {
    /// Check if predicate is supported by this pre-filter
    fn supports(&self, predicate: &Expr) -> bool;

    /// Get candidate documents that might match the predicate
    /// Returns a bitset of document indices
    fn candidates(&self, predicate: &Expr) -> PreFilterResult;

    /// Get candidate documents matching (path, value) equality
    fn candidates_eq(&self, path: &str, value: &Value) -> PreFilterResult;

    /// Get candidate documents matching text pattern
    fn candidates_match(&self, path: &str, pattern: &str) -> PreFilterResult;
}
```

### Integration Point: Extended `QueryPlan`

```rust
/// Extended query execution plan with Holodex support
#[derive(Debug, Clone)]
pub struct QueryPlan {
    /// Primary index scan strategy (existing)
    pub scan: IndexScan,

    /// Pre-filter to apply before iteration (NEW)
    pub pre_filter: Option<PreFilterSpec>,

    /// Remaining filter predicates not covered by index or pre-filter
    pub remaining_filter: Option<Box<Expr>>,

    /// Estimated number of candidates from index scan
    pub estimated_candidates: usize,

    /// Estimated reduction from pre-filter (0.0 = no reduction, 1.0 = full reduction)
    pub pre_filter_selectivity: f64,
}

/// Specification for pre-filter application
#[derive(Debug, Clone)]
pub struct PreFilterSpec {
    /// The predicate to pre-filter on
    pub predicate: Box<Expr>,
    /// Expected false positive rate
    pub expected_fpr: f64,
}
```

### Integration Point: Extended `DocumentStore`

```rust
/// Extended DocumentStore with candidate iteration
pub trait DocumentStore {
    // Existing methods...
    fn iter_all(&self) -> Box<dyn Iterator<Item = Value> + '_>;
    fn get_by_id(&self, id: &str) -> Option<Value>;
    fn iter_by_type(&self, type_name: &str) -> Box<dyn Iterator<Item = Value> + '_>;
    fn iter_by_ref(&self, target_id: &str) -> Box<dyn Iterator<Item = Value> + '_>;

    // NEW: Iterate only specified candidate indices
    fn iter_candidates(&self, candidates: &BitSet) -> Box<dyn Iterator<Item = Value> + '_>;

    // NEW: Combined type filter + candidate intersection
    fn iter_by_type_candidates(
        &self,
        type_name: &str,
        candidates: &BitSet
    ) -> Box<dyn Iterator<Item = Value> + '_>;
}
```

---

## Planner Modifications

### Decision Logic: When to Use Holodex

```rust
impl Planner {
    pub fn plan(&self, filter: &Expr, holodex: Option<&dyn PreFilter>) -> QueryPlan {
        // 1. Extract indexable predicates (existing logic)
        let (scan, remaining) = self.extract_index_scan(filter);

        // 2. If we have remaining predicates and Holodex, check if it helps
        let pre_filter = if let (Some(remaining), Some(holodex)) = (&remaining, holodex) {
            self.extract_pre_filter(remaining, holodex)
        } else {
            None
        };

        // 3. Calculate expected candidates after all filters
        let estimated_candidates = self.estimate_final_candidates(&scan, &pre_filter);

        QueryPlan {
            scan,
            pre_filter,
            remaining_filter: remaining,
            estimated_candidates,
            pre_filter_selectivity: pre_filter.map(|p| p.expected_fpr).unwrap_or(1.0),
        }
    }

    fn extract_pre_filter(
        &self,
        remaining: &Expr,
        holodex: &dyn PreFilter
    ) -> Option<PreFilterSpec> {
        // Walk the expression tree looking for predicates Holodex supports
        match remaining {
            // Equality on any field: field == "value"
            Expr::BinaryOp { op: BinOp::Eq, left, right } => {
                if let Some(path) = self.extract_path(left) {
                    if holodex.supports(remaining) {
                        return Some(PreFilterSpec {
                            predicate: Box::new(remaining.clone()),
                            expected_fpr: 0.05, // ~5% false positive rate target
                        });
                    }
                }
            }

            // Text match: field match "pattern*"
            Expr::BinaryOp { op: BinOp::Match, .. } => {
                if holodex.supports(remaining) {
                    return Some(PreFilterSpec {
                        predicate: Box::new(remaining.clone()),
                        expected_fpr: 0.10, // Higher FPR for text matching
                    });
                }
            }

            // AND compound: try to extract supported sub-predicates
            Expr::BinaryOp { op: BinOp::And, left, right } => {
                // Try each side
                if let Some(spec) = self.extract_pre_filter(left, holodex) {
                    return Some(spec);
                }
                if let Some(spec) = self.extract_pre_filter(right, holodex) {
                    return Some(spec);
                }
            }

            _ => {}
        }
        None
    }
}
```

### Cost Model: When is Holodex Worth It?

Not all pre-filtering is beneficial. The planner should estimate when Holodex helps:

```rust
impl Planner {
    fn should_use_holodex(
        &self,
        scan: &IndexScan,
        pre_filter_result: &PreFilterResult,
    ) -> bool {
        // Cost model:
        // - Holodex lookup cost: O(1) per query (signature comparison)
        // - Benefit: reduced document iteration

        let candidates_from_scan = self.estimate_candidates(scan);
        let candidates_after_holodex = pre_filter_result.candidates.len();

        // Only use if Holodex reduces candidates by >50%
        let reduction_ratio = candidates_after_holodex as f64 / candidates_from_scan as f64;

        // Also skip if candidates are already small (< 100)
        candidates_from_scan > 100 && reduction_ratio < 0.5
    }
}
```

---

## Execution Engine Modifications

### Modified Query Execution

```rust
impl Evaluator {
    pub fn eval_with_holodex(
        &self,
        expr: &Expr,
        scope: &Scope,
        holodex: Option<&dyn PreFilter>,
    ) -> Result<Value, EvalError> {
        // Parse filter from *[predicate] pattern
        let (filter, projection) = self.extract_filter_and_projection(expr)?;

        // Plan the query with Holodex awareness
        let plan = self.planner.plan(&filter, holodex);

        // Execute based on plan
        let candidates = match &plan.scan {
            IndexScan::TypeFilter(t) => {
                // Get type-filtered documents
                let type_docs = self.store.iter_by_type(t);

                // Apply Holodex pre-filter if specified
                if let Some(pre_filter) = &plan.pre_filter {
                    let candidates = holodex.unwrap().candidates(&pre_filter.predicate);
                    self.store.iter_by_type_candidates(t, &candidates.candidates)
                } else {
                    type_docs
                }
            }
            IndexScan::FullScan => {
                // Apply Holodex pre-filter if specified
                if let Some(pre_filter) = &plan.pre_filter {
                    let candidates = holodex.unwrap().candidates(&pre_filter.predicate);
                    self.store.iter_candidates(&candidates.candidates)
                } else {
                    self.store.iter_all()
                }
            }
            // ... other scan types
        };

        // Final evaluation with remaining filter
        self.filter_and_project(candidates, &plan.remaining_filter, projection, scope)
    }
}
```

---

## API Summary

### New Types

| Type | Purpose |
|------|---------|
| `PreFilter` trait | Interface for probabilistic pre-filtering |
| `PreFilterResult` | Candidate bitset + metadata |
| `PreFilterSpec` | Query plan specification for pre-filter |
| `BitSet` | Efficient document ID set |

### New `DocumentStore` Methods

| Method | Purpose |
|--------|---------|
| `iter_candidates(&BitSet)` | Iterate only specified doc indices |
| `iter_by_type_candidates(type, &BitSet)` | Type filter + candidate intersection |

### Modified `Planner`

| Change | Description |
|--------|-------------|
| `plan()` accepts optional `PreFilter` | Planner considers Holodex when planning |
| `QueryPlan.pre_filter` | New field specifying pre-filter to apply |
| Cost model | Decides when Holodex is beneficial |

---

## Example: Query Execution

### Query: `*[_type == "post" && title match "mario"]`

**Without Holodex:**
1. Planner: `IndexScan::TypeFilter("post")`, remaining: `title match "mario"`
2. Store: Iterate all 10,000 posts
3. Evaluator: Filter by `title match "mario"` → 50 results

**With Holodex:**
1. Planner: `IndexScan::TypeFilter("post")`, pre_filter: `title match "mario"`
2. Holodex: Return 500 candidate IDs (5% FPR)
3. Store: Iterate only 500 candidates
4. Evaluator: Validate → 50 results

**Speedup: 20x** (10,000 → 500 documents evaluated)

---

## Open Questions

1. **BitSet representation**: Use `roaring` for compressed bitmaps? Custom?

2. **Holodex location**: Should it be owned by the store or passed separately?
   - Option A: `store.with_holodex(holodex)` - cleaner API
   - Option B: Separate param - more flexible for testing

3. **Predicate decomposition**: How to handle complex predicates?
   - `a && b && c` - should we try all three with Holodex?
   - Intersection of multiple candidate sets?

4. **Dynamic vs static**: Should Holodex be built once (static corpus) or updated?
   - For groqdb v1: static is fine (read-only use case)
   - Future: incremental updates for mutations

5. **Thread safety**: Should `PreFilter` be `Send + Sync`?
   - Yes for multi-threaded query execution

---

## Next Steps

1. **Track 1/2 dependency**: Need data structure choice and fingerprinting algorithm
2. **Prototype**: Build minimal `PreFilter` implementation with in-memory Bloom filter
3. **Benchmark**: Measure overhead and speedup on realistic queries
4. **Iterate**: Refine based on Track 3 (predicate analysis) findings

---

*Draft v1 - awaiting review from @balder and team*
