# Track 2: Document Fingerprinting Strategies

**Agent**: shard
**Status**: In Progress

---

## Executive Summary

This document defines how Holodex converts JSON documents into compact, queryable signatures. The fingerprinting algorithm is the core of Holodex - it determines what queries we can accelerate and at what false positive rate.

---

## Design Goals

1. **No false negatives**: If a document matches a predicate, its signature MUST indicate potential match
2. **Low false positive rate**: Target ~1-5% FPR for typical predicates
3. **Compact signatures**: Sub-linear growth with document complexity
4. **Fast construction**: Linear time in document size
5. **Query-friendly**: Support efficient predicate checking

---

## Core Algorithm

### Step 1: Path-Value Extraction

Walk the JSON document and extract all (path, value) pairs:

```rust
struct PathValue {
    path: NormalizedPath,  // e.g., "body[*].children[*].text"
    value: HashableValue,  // e.g., String("Hello")
}

fn extract_path_values(doc: &JsonValue) -> Vec<PathValue> {
    let mut pairs = Vec::new();
    walk(doc, Path::root(), &mut pairs);
    pairs
}

fn walk(value: &JsonValue, path: Path, pairs: &mut Vec<PathValue>) {
    match value {
        JsonValue::Object(map) => {
            for (key, val) in map {
                walk(val, path.field(key), pairs);
            }
        }
        JsonValue::Array(arr) => {
            for (i, elem) in arr.iter().enumerate() {
                // Store BOTH concrete index AND wildcard
                walk(elem, path.index(i), pairs);
                walk(elem, path.wildcard(), pairs);
            }
        }
        JsonValue::String(s) => pairs.push(PathValue { path: path.normalize(), value: HashableValue::String(s.clone()) }),
        JsonValue::Number(n) => pairs.push(PathValue { path: path.normalize(), value: HashableValue::Number(*n) }),
        JsonValue::Bool(b) => pairs.push(PathValue { path: path.normalize(), value: HashableValue::Bool(*b) }),
        JsonValue::Null => pairs.push(PathValue { path: path.normalize(), value: HashableValue::Null }),
    }
}
```

### Step 2: Path Normalization

GROQ queries use multiple array access patterns. We normalize paths to enable matching:

| Query Pattern | Document Path | Normalized Form | Match? |
|--------------|---------------|-----------------|--------|
| `body[0].text` | `body[0].text` | `body[*].text` | Yes |
| `body[].text` | `body[0].text` | `body[*].text` | Yes |
| `body[].text` | `body[5].text` | `body[*].text` | Yes |
| `body[*].text` | `body[0].text` | `body[*].text` | Yes |

**Normalization rules**:
1. All array indexes (`[0]`, `[1]`, `[-1]`) become `[*]`
2. Array traversal `[]` in queries maps to `[*]` in signatures
3. Multiple consecutive arrays: `body[*].children[*]`

```rust
impl Path {
    fn normalize(&self) -> NormalizedPath {
        self.segments.iter().map(|seg| {
            match seg {
                Segment::Field(name) => Segment::Field(name.clone()),
                Segment::Index(_) => Segment::Wildcard,
                Segment::Wildcard => Segment::Wildcard,
            }
        }).collect()
    }
}
```

**Trade-off**: We lose the ability to distinguish `body[0]` from `body[1]`. A query for `body[0].text == "x"` will match any document with `body[*].text == "x"`, even if it's at index 5. This is an acceptable false positive.

### Step 3: Hash Computation

Each (path, value) pair is hashed into the signature:

```rust
fn hash_pair(path: &NormalizedPath, value: &HashableValue) -> u64 {
    let mut hasher = XxHash64::new();

    // Hash path components
    for segment in path.segments() {
        match segment {
            Segment::Field(name) => {
                hasher.write_u8(0x01);  // Field marker
                hasher.write(name.as_bytes());
            }
            Segment::Wildcard => {
                hasher.write_u8(0x02);  // Wildcard marker
            }
        }
    }

    // Hash value with type tag
    match value {
        HashableValue::String(s) => {
            hasher.write_u8(0x10);
            hasher.write(s.as_bytes());
        }
        HashableValue::Number(n) => {
            hasher.write_u8(0x20);
            hasher.write(&n.to_le_bytes());
        }
        HashableValue::Bool(b) => {
            hasher.write_u8(0x30);
            hasher.write_u8(if *b { 1 } else { 0 });
        }
        HashableValue::Null => {
            hasher.write_u8(0x40);
        }
    }

    hasher.finish()
}
```

**Why XxHash64?**: Fast, high-quality non-cryptographic hash. We need speed, not security.

**Why type tags?**: Distinguishes `"true"` (string) from `true` (boolean). GROQ is type-aware.

### Step 4: Signature Construction

Insert all hashes into a probabilistic filter:

```rust
struct DocumentSignature {
    filter: BloomFilter,  // Or XorFilter, depending on Track 1 findings
}

impl DocumentSignature {
    fn from_document(doc: &JsonValue) -> Self {
        let pairs = extract_path_values(doc);
        let hashes: Vec<u64> = pairs.iter()
            .map(|pv| hash_pair(&pv.path, &pv.value))
            .collect();

        Self {
            filter: BloomFilter::from_hashes(&hashes),
        }
    }

    fn might_match(&self, path: &NormalizedPath, value: &HashableValue) -> bool {
        let hash = hash_pair(path, value);
        self.filter.contains(hash)
    }
}
```

---

## Signature Sizing

The filter size determines false positive rate. Key parameters:

### Bloom Filter Sizing

For a Bloom filter with `m` bits, `k` hash functions, and `n` elements:

```
FPR ≈ (1 - e^(-kn/m))^k
```

Optimal `k = (m/n) * ln(2)`.

| Elements (n) | Bits (m) | Bits/Element | FPR |
|-------------|----------|--------------|-----|
| 100 | 960 | 9.6 | 1% |
| 100 | 720 | 7.2 | 2% |
| 100 | 480 | 4.8 | 5% |
| 1000 | 9600 | 9.6 | 1% |
| 1000 | 4800 | 4.8 | 5% |

**Recommendation**: ~10 bits per element for 1% FPR.

### Estimating Element Count

A typical Sanity document might have:
- 10-20 root-level fields
- 5-10 nested objects (each with 3-5 fields)
- 0-50 array elements (Portable Text blocks with children)

**Estimate**: 50-200 path-value pairs per document.

**Signature size**: 500-2000 bits (64-256 bytes) per document at 1% FPR.

### Adaptive Sizing Strategy

Option A: **Fixed size per document**
- Simple implementation
- Larger docs have higher FPR
- Predictable memory layout

Option B: **Size based on element count**
- Count pairs first, then size filter
- Consistent FPR across documents
- Variable memory per document

**Recommendation**: Start with Option A (fixed size), profile FPR distribution, consider Option B if variance is too high.

---

## Path-Only Signatures

For `defined(path)` queries, we need to know if a path exists regardless of value:

```rust
fn hash_path_only(path: &NormalizedPath) -> u64 {
    let mut hasher = XxHash64::new();
    hasher.write_u8(0xFF);  // Path-only marker (distinct from value types)
    for segment in path.segments() {
        // ... same as hash_pair
    }
    hasher.finish()
}
```

Add path-only hashes during extraction:

```rust
fn walk(...) {
    // After adding (path, value):
    pairs.push(PathValue { path: path.normalize(), value: HashableValue::PathExists });
}
```

This supports queries like `*[defined(metadata.seo)]` efficiently.

---

## Edge Cases

### Empty Arrays

```json
{ "tags": [] }
```

No path-value pairs for `tags[*]`. But we should still record that `tags` exists:

```rust
JsonValue::Array(arr) if arr.is_empty() => {
    // Record path existence but no elements
    pairs.push(PathValue { path: path.normalize(), value: HashableValue::EmptyArray });
}
```

### Null Values

```json
{ "name": null }
```

Explicitly hash `(name, null)`. This distinguishes:
- `name` is null: `hash(name, null)` present
- `name` doesn't exist: no hash for `name`

### Deep Nesting

```json
{ "a": { "b": { "c": { "d": { "e": "deep" } } } } }
```

Path: `a.b.c.d.e`

No special handling needed - just more bytes in the path hash. Performance is O(path_length) which is bounded by document depth.

### Very Long Strings

```json
{ "content": "... 10MB of text ..." }
```

Options:
1. Hash entire string (slow but accurate)
2. Hash first N bytes (fast but may miss variations)
3. Hash multiple chunks (compromise)

**Recommendation**: Hash entire string. Construction is a one-time cost, and we need accuracy.

### Numbers

JSON numbers can be integers or floats. Normalize to f64 for hashing:

```rust
HashableValue::Number(n) => {
    let f = n.as_f64();
    hasher.write(&f.to_bits().to_le_bytes());
}
```

**Caveat**: Large integers may lose precision. If this matters, could hash both integer and float representations.

---

## Query Predicate Translation

To check if a document might match a predicate:

### Equality: `field == value`

```rust
fn check_equality(sig: &Signature, path: &str, value: &JsonValue) -> bool {
    let normalized = normalize_query_path(path);
    let hashable = to_hashable(value);
    sig.might_match(&normalized, &hashable)
}
```

### Defined: `defined(field)`

```rust
fn check_defined(sig: &Signature, path: &str) -> bool {
    let normalized = normalize_query_path(path);
    sig.might_have_path(&normalized)
}
```

### Boolean: `field == true` / `field == false`

Same as equality - booleans are hashable values.

### Compound: `a == 1 && b == 2`

```rust
fn check_and(sig: &Signature, predicates: &[Predicate]) -> bool {
    predicates.iter().all(|p| check_predicate(sig, p))
}
```

For AND, all must pass (might_match). If any fails, document is pruned.

### Disjunction: `a == 1 || b == 2`

```rust
fn check_or(sig: &Signature, predicates: &[Predicate]) -> bool {
    predicates.iter().any(|p| check_predicate(sig, p))
}
```

For OR, any passing is enough. Less selective but still useful.

---

## What We Cannot Fingerprint (Limitations)

### Range Queries

`*[views > 1000]` - Bloom filters don't support ordering. See Track 4 for range strategies.

**Fallback**: Return all documents (no filtering).

### Negation

`*[status != "draft"]` - Would need to enumerate all non-draft values.

**Fallback**: Return all documents. Could potentially use path-exists check if available.

### Match/Text Search

`*[title match "hello*"]` - Prefix matching incompatible with exact hashing.

**Potential extension**: Add n-gram hashes. E.g., hash trigrams: `hel`, `ell`, `llo`. Query `match "hello*"` checks for trigrams `hel`, `ell`, `llo`. High FPR but better than full scan.

**Defer to**: Track 4 or future phase.

### Dereferences

`*[author->name == "Alice"]` - Requires fetching referenced document.

**Options**:
1. Ignore dereferences (no filtering, full scan for refs)
2. Pre-compute joined signatures (expensive, complex updates)
3. Two-phase: filter by `author._ref` presence, then filter referenced docs

**Recommendation**: Start with Option 1 (no deref support), consider Option 3 for common patterns.

---

## Open Questions

1. **Should we include parent path prefixes?**
   - E.g., for path `a.b.c`, also hash `a.b` and `a`?
   - Enables `defined(a.b)` to work when we only saw `a.b.c`
   - Trade-off: more hashes per document

2. **How to handle `_type` specially?**
   - Almost every query filters by `_type`
   - Could use exact index for `_type` alongside Bloom signature
   - Or ensure `_type` is always first hash (hot in cache)

3. **Filter choice depends on Track 1 findings**
   - Bloom: classic, well-understood
   - XOR: more space-efficient for static data
   - Cuckoo: supports deletion (if needed later)

---

## Next Steps

1. Wait for Track 1 (bloom) to recommend filter type
2. Prototype implementation on sample documents
3. Measure actual FPR on GROQ test suite predicates (coordinate with Track 3/hash)
4. Profile construction performance

---

*Draft v1 - shard*
