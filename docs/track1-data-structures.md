# Track 1: Probabilistic Data Structure Survey

**Author**: bloom
**Status**: Complete
**Last Updated**: 2024-12-22

---

## Executive Summary

For Holodex v1, I recommend **Binary Fuse filters** (via the `xorf` crate) as the primary probabilistic data structure. They offer the best space efficiency for our read-only, bulk-query use case while maintaining excellent lookup performance.

---

## Comparison Matrix

| Filter Type | Bits/Entry (1% FPR) | Lookup Cost | Deletion | Construction | Best Rust Crate |
|-------------|---------------------|-------------|----------|--------------|-----------------|
| **Bloom** | ~10 bits | k hash probes, variable | No* | O(n) | `fastbloom` |
| **Cuckoo** | ~7-8 bits (at <3% FPR) | 2 memory accesses | Yes | O(n), can fail | `cuckoofilter` |
| **XOR** | ~9 bits | 3 parallel accesses | No | O(n), probabilistic | `xorf` |
| **Binary Fuse** | ~9 bits | 3 parallel accesses | No | O(n), 2x faster than XOR | `xorf` (BinaryFuse8) |
| **Quotient** | ~10-12 bits | 1 random + sequential scan | Yes | O(n) | `qfilter` |

*Counting Bloom filters support deletion but require 4x space.

---

## Detailed Analysis

### Bloom Filters

**How they work**: Array of m bits with k hash functions. Insert: set k bit positions. Query: check if all k positions are set.

**Pros**:
- Simple, well-understood
- Flexible: can keep inserting at cost of higher FPR
- Many mature implementations

**Cons**:
- Variable lookup time (depends on whether element exists)
- ~44% space overhead vs theoretical minimum
- No deletion without counting variant

**Best Rust crate**: [`fastbloom`](https://github.com/tomtomwombat/fastbloom)
- 2-400x faster than competitors
- Full concurrency support
- Compatible with any hasher

### Cuckoo Filters

**How they work**: Hash table with buckets holding fingerprints. Uses cuckoo hashing for collision resolution.

**Pros**:
- Supports deletion
- Better space than Bloom at FPR <= 3%
- Consistent 2 memory accesses per lookup

**Cons**:
- Insertion can fail at high load factors
- Degraded FPR when not near full occupancy
- More complex implementation

**Best Rust crate**: [`cuckoofilter`](https://github.com/axiomhq/rust-cuckoofilter)
- C FFI bindings available
- 4-fingerprint buckets (optimal per paper)

### XOR Filters

**How they work**: 3-way XOR of fingerprints at hash-determined positions. Construction solves a system of linear equations.

**Pros**:
- More space-efficient than Bloom/Cuckoo
- Exactly 3 memory accesses (parallelizable)
- Consistent lookup time regardless of membership

**Cons**:
- Immutable after construction
- Construction slightly slower, can fail (retry needed)

**Reference**: [Xor Filters: Faster and Smaller Than Bloom and Cuckoo Filters](https://arxiv.org/pdf/1912.08258) (Lemire et al.)

### Binary Fuse Filters (Recommended)

**How they work**: Improved XOR filter variant with binary-partitioned fuse graph structure.

**Pros**:
- Same space efficiency as XOR (~9 bits/entry for 1% FPR)
- 2x faster construction than XOR
- Uses less memory during construction
- Higher construction success rate

**Cons**:
- Still immutable
- Relatively newer (less battle-tested)

**Best Rust crate**: [`xorf`](https://docs.rs/xorf/latest/xorf/) with `binary-fuse` feature
- BinaryFuse8 (8-bit fingerprints, ~0.4% FPR)
- BinaryFuse16 (16-bit fingerprints, ~0.0015% FPR)
- `no_std` compatible, serde support

**Reference**: [Binary Fuse Filters: Fast and Smaller Than Xor Filters](https://arxiv.org/abs/2201.01174) (Graf & Lemire)

### Quotient Filters

**How they work**: Compact hash table storing remainders. Quotient determines slot, remainder is stored with metadata bits for overflow handling.

**Pros**:
- Excellent cache locality (single random access + sequential scan)
- Supports merging and resizing
- Supports deletion
- Good for disk-based storage

**Cons**:
- 20% larger than Bloom filters
- Performance degrades at high load
- Vulnerable to adversarial queries that degrade lookup speed

**Best Rust crate**: [`qfilter`](https://crates.io/crates/qfilter)
- Based on Rank-Select Quotient Filter (RSQF)
- 62K+ monthly downloads
- Supports deletion, merging, resizing

---

## Recommendation for Holodex v1

### Primary Choice: Binary Fuse Filter (BinaryFuse8)

**Rationale**:

1. **Immutability is a feature, not a bug**: Holodex builds static indexes for read-only corpora. We rebuild on data change anyway.

2. **Space efficiency**: At ~9 bits/entry with <1% FPR, Binary Fuse is the most compact option. For a document with 100 (path, value) pairs, that's ~112 bytes per document signature.

3. **Consistent lookup performance**: 3 parallel memory accesses regardless of membership. Important for predictable query latency.

4. **Construction speed**: 2x faster than basic XOR filters, critical for index build time.

5. **Mature Rust implementation**: `xorf` is well-maintained, has serde support, and is `no_std` compatible.

### Alternative: Bloom Filter (for prototyping)

If we want maximum simplicity for initial prototypes, `fastbloom` is a solid choice. It's marginally less space-efficient but more forgiving (no construction failures, can query during building).

### Not Recommended for v1

- **Cuckoo**: Deletion support is wasted for our use case
- **Quotient**: Cache locality advantage not relevant for in-memory index; vulnerable to adversarial patterns

---

## Sizing Guidelines

For target FPR of 1%:

| Filter | Bits/Element | Doc with 100 pairs | Doc with 200 pairs |
|--------|--------------|--------------------|--------------------|
| Bloom | 9.6 | 120 bytes | 240 bytes |
| BinaryFuse8 | 9.0 | 112 bytes | 225 bytes |
| BinaryFuse16 | 18.0 | 225 bytes | 450 bytes |

**Recommendation**: Use BinaryFuse8 unless we need sub-0.1% FPR, then BinaryFuse16.

---

## Implementation Notes

### Using xorf BinaryFuse8

```rust
use xorf::{BinaryFuse8, Filter};

// Build from iterator of u64 hashes
let keys: Vec<u64> = vec![hash("_type:post"), hash("title:Hello"), ...];
let filter = BinaryFuse8::try_from(&keys).expect("construction failed");

// Query
if filter.contains(&hash("_type:post")) {
    // Might contain - need to verify
}

// Serialize with serde
let bytes = bincode::serialize(&filter)?;
```

### Hash Function

Use `xxhash3` (64-bit) for hashing (path, value) pairs:
- Fast, stable across platforms
- Already used by `qfilter` crate
- Available via `xxhash-rust` crate

---

## References

- [Xor Filters Paper](https://arxiv.org/pdf/1912.08258) - Lemire et al.
- [Binary Fuse Filters Paper](https://arxiv.org/abs/2201.01174) - Graf & Lemire
- [Cuckoo Filter Paper](https://www.cs.cmu.edu/~dga/papers/cuckoo-conext2014.pdf) - Fan et al.
- [Performance-Optimal Filtering](https://www.vldb.org/pvldb/vol12/p502-lang.pdf) - Bloom vs Cuckoo high-throughput
- [Quotient Filters](https://www.gakhov.com/articles/quotient-filters.html) - Gakhov tutorial
- [xorf crate docs](https://docs.rs/xorf/latest/xorf/)
- [fastbloom crate](https://github.com/tomtomwombat/fastbloom)
