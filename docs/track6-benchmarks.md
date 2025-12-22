# Track 6: Holodex Benchmark Framework

**Author**: probe
**Status**: Draft v1
**Date**: 2024-12-22

---

## Executive Summary

This document defines the benchmarking methodology for evaluating Holodex effectiveness. The framework measures three key metrics: **false positive rate (FPR)**, **query speedup**, and **index overhead**. We leverage groqdb's existing benchmark infrastructure and extend it with Holodex-specific measurements.

---

## Goals

1. **Validate correctness**: Zero false negatives (never miss matching documents)
2. **Measure FPR**: False positive rate for different predicate types
3. **Quantify speedup**: End-to-end query time with/without Holodex
4. **Assess overhead**: Index build time, memory usage, storage size

---

## Datasets

### 1. Synthetic Realistic Dataset

Use the existing `generate_realistic_data.rs` generator which produces:

| Size | Documents | Characteristics |
|------|-----------|-----------------|
| Small | 1,000 | Fast iteration during development |
| Medium | 10,000 | Representative for most benchmarks |
| Large | 100,000 | Stress test for scalability |

**Distribution** (matches real Sanity data):
- 40% `sanity.imageAsset`
- 15% API docs (`api.export`, `api.symbol`, `api.interface`)
- 8% content types (`article`, `post`, `author`, `category`)
- 12% stress test types (`eventLog`, `commentThread`, `organization`, `country`)
- 25% long tail of other types

**Key features**:
- Portable Text (nested `body[].children[].text`)
- Multi-hop references (`author->organization->country`)
- Large arrays (`eventLog.events[]` with 100-500 items)
- Cross-document references

### 2. GROQ Test Suite Data

Located in `groqdb-proto/groq-test-suite/test/`:
- Multiple NDJSON fixtures with diverse document structures
- Official Sanity test cases for edge case coverage

### 3. Real-World Dataset (Future)

If available: anonymized Sanity dataset sample for production validation.

---

## Query Workloads

### Workload 1: Simple Equality (Baseline)

Tests where Holodex should provide maximum benefit:

```groq
// Root-level field equality
*[title == "Hello World"]
*[status == "published"]

// Nested path equality
*[slug.current == "hello-world"]
*[metadata.featured == true]
*[metadata.stats.views == 1234]

// Deep nested equality
*[author._ref == "author-123"]
```

**Expected outcome**: 95%+ reduction in scanned documents

### Workload 2: Text Matching

Tests `match` operator support:

```groq
// Simple prefix match
*[title match "mario*"]
*[name match "john*"]

// Portable Text search (the motivating use case!)
*[body[].children[].text match "welcome*"]
*[content[].children[].text match "groq*"]

// Multiple field match
*[_type == "post" && title match "react*"]
```

**Expected outcome**: 80-90% reduction (higher FPR due to n-gram matching)

### Workload 3: Combined Predicates

Tests compound AND expressions:

```groq
// Type + equality
*[_type == "post" && author._ref == "author-1"]
*[_type == "article" && category._ref == "cat-5"]

// Type + nested equality
*[_type == "author" && organization._ref == "org-1"]

// Type + text match
*[_type == "post" && title match "react*"]
*[_type == "article" && body[].children[].text match "api*"]
```

**Expected outcome**: Holodex on non-type predicate, type index for initial filter

### Workload 4: Full Scan Scenarios (Control)

Queries where Holodex cannot help (control group):

```groq
// Range queries (not supported in v1)
*[views > 1000]
*[publishedAt > "2024-01-01"]

// Negation (cannot prove absence)
*[!defined(metadata)]
*[status != "draft"]

// OR expressions (require union, not intersection)
*[_type == "post" || _type == "article"]

// Parameter-dependent (runtime value)
*[_id == $docId]
```

**Expected outcome**: Holodex disabled, baseline performance

### Workload 5: Stress Tests

Edge cases that stress Holodex limits:

```groq
// Many array elements (Portable Text with 50+ spans)
*[body[].children[].text match "keyword*"]

// Deep array slicing
*[_type == "eventLog"].events[100..200]

// Multi-hop reference chains
*[author->organization->country->name == "USA"]

// High cardinality field (many unique values)
*[_id == "specific-doc-id"]
```

---

## Metrics

### 1. False Positive Rate (FPR)

```
FPR = (candidates_returned - true_matches) / candidates_returned
```

- **Target**: < 5% for equality, < 10% for text match
- **Measurement**: Compare Holodex candidates vs full eval results
- **Per-query and aggregate reporting**

### 2. Query Speedup

```
Speedup = time_without_holodex / time_with_holodex
```

- **Target**: > 10x for qualifying queries
- **Measurement**: End-to-end query time (parse + plan + filter + eval)
- **Warm cache measurements** (exclude cold start)

### 3. Candidate Reduction

```
Reduction = 1 - (candidates_returned / total_documents)
```

- **Target**: > 95% for point queries
- **Measurement**: Holodex candidate count vs full scan count

### 4. Index Overhead

| Metric | Target | Measurement |
|--------|--------|-------------|
| Build time | < 10s per 100k docs | Wall clock time for index construction |
| Memory usage | < 1KB per doc | Peak RSS during indexing |
| Storage size | < 100 bytes per doc | Serialized index size on disk |
| Query overhead | < 1ms | Holodex lookup time (excluding iteration) |

---

## Benchmark Implementation

### Existing Infrastructure

The project already has a Criterion benchmark suite in `benches/`:
- `query_bench.rs` - Query performance across patterns
- `lmdb_bench.rs` - Storage backend benchmarks
- `stress_bench.rs` - Stress test scenarios
- `generate_realistic_data.rs` - Synthetic data generator

### New Benchmark Module: `holodex_bench.rs`

```rust
//! Holodex Benchmark Suite
//!
//! Measures:
//! - False positive rate per query type
//! - End-to-end speedup vs baseline
//! - Index build time and size
//!
//! Run with: cargo bench --bench holodex_bench

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use groqdb::store::MemoryStore;
use groqdb::holodex::{Holodex, HolodexConfig};
use std::time::Instant;

/// Generate test data using realistic generator
fn setup_data(size: usize) -> (MemoryStore, Vec<Value>) {
    let config = GeneratorConfig { total_docs: size, seed: 42 };
    let mut gen = RealisticDataGenerator::new(config);
    let docs = gen.generate();
    let store = MemoryStore::from_documents(docs.clone());
    (store, docs)
}

/// Measure index build time
fn bench_index_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("holodex_build");

    for size in [1000, 10000, 100000] {
        group.bench_with_input(
            BenchmarkId::new("build", size),
            &size,
            |b, &size| {
                let (_, docs) = setup_data(size);
                b.iter(|| {
                    let config = HolodexConfig::default();
                    Holodex::build(black_box(&docs), config)
                });
            }
        );
    }
    group.finish();
}

/// Measure query speedup
fn bench_query_speedup(c: &mut Criterion) {
    let (store, docs) = setup_data(10000);
    let holodex = Holodex::build(&docs, HolodexConfig::default());

    let queries = [
        ("eq_root", "*[title == \"Post post-100\"]"),
        ("eq_nested", "*[slug.current == \"article-50\"]"),
        ("match_title", "*[title match \"Post*\"]"),
        ("match_body", "*[body[].children[].text match \"Sample*\"]"),
        ("compound", "*[_type == \"post\" && title match \"Post*\"]"),
    ];

    let mut group = c.benchmark_group("holodex_speedup");

    for (name, query) in queries {
        // Baseline: without Holodex
        group.bench_function(BenchmarkId::new("baseline", name), |b| {
            b.iter(|| run_query(black_box(&store), query))
        });

        // With Holodex
        group.bench_function(BenchmarkId::new("holodex", name), |b| {
            b.iter(|| run_query_with_holodex(black_box(&store), &holodex, query))
        });
    }
    group.finish();
}

/// Measure false positive rate
fn bench_fpr(c: &mut Criterion) {
    let (store, docs) = setup_data(10000);
    let holodex = Holodex::build(&docs, HolodexConfig::default());

    // Measure FPR for each query type
    let queries = [
        "*[title == \"Post post-100\"]",
        "*[slug.current == \"article-50\"]",
        "*[body[].children[].text match \"Sample*\"]",
    ];

    for query in queries {
        // Get Holodex candidates
        let candidates = holodex.candidates(parse_predicate(query));

        // Get true matches via full eval
        let true_matches: Vec<_> = run_query(&store, query).as_array().unwrap().clone();

        let fpr = (candidates.len() - true_matches.len()) as f64 / candidates.len() as f64;
        println!("Query: {} | Candidates: {} | True: {} | FPR: {:.2}%",
                 query, candidates.len(), true_matches.len(), fpr * 100.0);
    }
}

criterion_group!(
    benches,
    bench_index_build,
    bench_query_speedup,
);
criterion_main!(benches);
```

### FPR Measurement Tool

Separate binary for detailed FPR analysis:

```rust
//! FPR Analysis Tool
//!
//! Run with: cargo run --bin fpr_analysis -- --queries queries.txt

fn main() {
    let args = Args::parse();
    let docs = load_dataset(&args.dataset);
    let holodex = Holodex::build(&docs, HolodexConfig::default());
    let store = MemoryStore::from_documents(docs);

    let mut results = Vec::new();

    for query in load_queries(&args.queries) {
        let predicate = parse_predicate(&query);

        // Holodex candidates
        let candidates = holodex.candidates(&predicate);
        let candidate_count = candidates.candidates.len();

        // True matches
        let true_matches = run_query(&store, &query);
        let true_count = true_matches.as_array().map(|a| a.len()).unwrap_or(0);

        // Verify no false negatives!
        assert!(candidate_count >= true_count,
                "FALSE NEGATIVE DETECTED: {} candidates < {} true matches",
                candidate_count, true_count);

        let fpr = if candidate_count > 0 {
            (candidate_count - true_count) as f64 / candidate_count as f64
        } else {
            0.0
        };

        results.push(FprResult {
            query: query.clone(),
            candidates: candidate_count,
            true_matches: true_count,
            fpr,
        });
    }

    // Output report
    print_fpr_report(&results);
}
```

---

## Baseline Measurements

Before Holodex implementation, establish baselines for current groqdb:

### Baseline 1: Full Scan Performance

| Dataset Size | Count Query | Type Filter | Compound Filter |
|--------------|-------------|-------------|-----------------|
| 1,000 docs | TBD | TBD | TBD |
| 10,000 docs | TBD | TBD | TBD |
| 100,000 docs | TBD | TBD | TBD |

### Baseline 2: Query Latency Distribution

For 10,000 doc dataset:

| Query Pattern | p50 | p95 | p99 |
|---------------|-----|-----|-----|
| `*[_type == "post"]` | TBD | TBD | TBD |
| `*[title match "x*"]` | TBD | TBD | TBD |
| `*[body[].children[].text match "x*"]` | TBD | TBD | TBD |

### Baseline 3: Memory Usage

| Dataset Size | Store Memory | Peak Query Memory |
|--------------|--------------|-------------------|
| 1,000 docs | TBD | TBD |
| 10,000 docs | TBD | TBD |
| 100,000 docs | TBD | TBD |

---

## Prototype Runner

Simple script to run initial Holodex prototype benchmarks:

```bash
#!/bin/bash
# scripts/run_holodex_bench.sh

set -e

# Generate test data
cargo run --release --bin generate_realistic_data -- \
    --docs 10000 \
    --output /tmp/holodex_bench_data.ndjson

# Run benchmarks
cargo bench --bench holodex_bench -- --save-baseline baseline

# Run FPR analysis
cargo run --release --bin fpr_analysis -- \
    --dataset /tmp/holodex_bench_data.ndjson \
    --queries benches/holodex_queries.txt \
    --output /tmp/fpr_report.json

echo "Results saved to target/criterion/ and /tmp/fpr_report.json"
```

---

## Query Corpus

File: `benches/holodex_queries.txt`

```groq
# Workload 1: Simple Equality
*[title == "Post post-100"]
*[slug.current == "article-50"]
*[metadata.featured == true]
*[author._ref == "author-10"]

# Workload 2: Text Matching
*[title match "Post*"]
*[name match "Author*"]
*[body[].children[].text match "Sample*"]
*[content[].children[].text match "text*"]

# Workload 3: Combined Predicates
*[_type == "post" && author._ref == "author-1"]
*[_type == "article" && title match "Article*"]
*[_type == "author" && name match "Author 1*"]

# Workload 4: Full Scan Control
*[views > 1000]
*[publishedAt > "2024-01-01"]
*[_type == "post" || _type == "article"]

# Workload 5: Stress Tests
*[body[].children[].text match "content*"]
*[comments[].text match "Comment*"]
*[events[].action == "click"]
```

---

## Success Criteria

| Metric | Target | Pass/Fail |
|--------|--------|-----------|
| False negatives | 0 | CRITICAL - any failure = bug |
| FPR (equality) | < 5% | Pass |
| FPR (text match) | < 10% | Pass |
| Speedup (qualifying queries) | > 10x | Pass |
| Index build time | < 10s / 100k docs | Pass |
| Index size | < 100 bytes / doc | Pass |
| Query overhead | < 1ms | Pass |

---

## Next Steps

1. **Run baseline benchmarks** on current groqdb (no Holodex)
2. **Build minimal Bloom prototype** with basic (path, value) hashing
3. **Run FPR analysis** to validate signature quality
4. **Iterate on fingerprinting** based on FPR results from Track 2
5. **Full benchmark suite** once Holodex integration complete

---

## Open Questions

1. **Warm vs cold cache**: How many warmup iterations? (Criterion default: 3)

2. **Statistical significance**: How many samples per benchmark? (Criterion default: 100)

3. **Real dataset access**: Can we get anonymized Sanity data for validation?

4. **Memory profiling**: Use `heaptrack` or `valgrind --tool=massif`?

5. **CI integration**: Run benchmarks on every PR? Nightly? Release only?

---

*Draft v1 - awaiting review*
