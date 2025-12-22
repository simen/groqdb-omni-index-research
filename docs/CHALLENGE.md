# The Omni-Index Challenge

## Executive Summary

GROQ allows developers to write expressive queries without understanding database internals. But this expressiveness creates an indexing nightmare: **the space of possible predicates is essentially infinite**.

This document explores the full complexity of what we're trying to index, why traditional approaches fail, and what constraints any solution must satisfy.

---

## The Core Problem

Consider a simple Sanity content model:

```javascript
// A blog with posts, authors, and comments
{
  _id: "post-1",
  _type: "post",
  title: "Hello World",
  slug: { current: "hello-world" },
  author: { _ref: "author-1" },
  body: [
    { _type: "block", children: [{ text: "Welcome..." }] },
    { _type: "image", asset: { _ref: "image-1" } }
  ],
  categories: [{ _ref: "cat-1" }, { _ref: "cat-2" }],
  metadata: {
    featured: true,
    publishedAt: "2024-01-15",
    stats: { views: 1234, likes: 56 }
  }
}
```

A developer might write ANY of these queries:

```groq
// Simple - we can index this
*[_type == "post"]

// Also simple
*[_type == "post" && metadata.featured == true]

// Getting harder - nested path
*[slug.current == "hello-world"]

// Array access
*[body[0]._type == "image"]

// Filtered array + deep path
*[body[_type == "image"][0].asset._ref == "image-1"]

// Cross-document reference + path
*[author->name == "Alice"]

// Text matching on nested content
*[body[].children[].text match "welcome*"]

// Existence checks
*[defined(metadata.stats.engagement)]

// Comparisons on deep paths
*[metadata.stats.views > 1000]

// Multiple array traversals
*[categories[]->articles[]->author->name == "Bob"]
```

**The question: How do we make ALL of these fast without requiring explicit index definitions?**

---

## Taxonomy of GROQ Path Expressions

### 1. Simple Attribute Access

```groq
title                    // Direct field
metadata.featured        // Nested path
author._ref              // Reference field
```

These produce a **single, deterministic path** per document.

### 2. Array Index Access

```groq
body[0]                  // First element
body[0].children[0]      // Chained indexes
items[-1]                // Last element (negative index)
```

These still produce a **single path**, but the index may vary or be out of bounds.

### 3. Array Traversal (Flattening)

```groq
body[]                   // All elements
body[].children[]        // Nested flatten
categories[]->.name      // Flatten + deref
```

This is where complexity explodes. `body[]` means "for each element in body". A single document can produce **multiple paths** - one per array element.

Example: If a document has `body` with 10 blocks, each with 5 children:
- `body[].children[].text` expands to **50 paths** for that single document

### 4. Filtered Array Access

```groq
body[_type == "image"]           // Filter by predicate
body[_type == "image"][0]        // Filter then index
content[_key == "abc123"]        // Filter by key
```

This combines array traversal with predicate evaluation. The filter predicate itself can contain arbitrary expressions.

### 5. Dereference Chains

```groq
author->name                     // Single deref
author->organization->ceo->name  // Multi-hop
categories[]->parent->           // Array + multi-deref
```

Dereferencing crosses document boundaries. The path `author->name` requires:
1. Read `author._ref` from current document
2. Fetch the referenced document
3. Read `name` from that document

### 6. Parent Scope References

```groq
*[_id == ^.author._ref]          // Reference parent context
*[^.^.category in categories]    // Grandparent reference
```

Parent scope (`^`) refers to enclosing evaluation contexts. This creates **runtime-dependent paths** that can't be pre-indexed.

### 7. Computed Paths

```groq
*[content[@._type == $type]]     // Parameter-dependent
*[data[string::lower(key) == "test"]]  // Function in path
```

Paths that depend on runtime values or function evaluation.

---

## Why Traditional Indexing Fails

### Problem 1: Path Cardinality Explosion

Traditional databases index specific columns: `CREATE INDEX ON posts(title)`.

In GROQ, the number of possible paths is **unbounded**:

```
title
metadata.featured
metadata.stats.views
metadata.stats.likes
metadata.stats.shares
metadata.seo.title
metadata.seo.description
metadata.seo.keywords[]
body[].children[].text
body[].children[].marks[]
body[].asset._ref
...
```

A real Sanity dataset might have **thousands of unique paths**. Creating an index for each is impractical.

### Problem 2: Array Traversal Multiplies Entries

For a traditional index on `body[].children[].text`:

```javascript
// Single document with 3 blocks, each with 2 children
{
  body: [
    { children: [{ text: "a" }, { text: "b" }] },
    { children: [{ text: "c" }, { text: "d" }] },
    { children: [{ text: "e" }, { text: "f" }] }
  ]
}
```

This document would need **6 index entries** for this one path. With 100k documents averaging 10 entries each, that's 1M index entries for ONE path pattern.

### Problem 3: Dereference Chains Cross Documents

```groq
*[author->organization->industry == "tech"]
```

To index this, we'd need to:
1. For each post, resolve author reference
2. For each author, resolve organization reference
3. Index (post._id, organization.industry) pairs

This creates **join indexes** that must be maintained when ANY document in the chain changes. Combinatorial explosion.

### Problem 4: Predicates Are Arbitrary Expressions

```groq
*[string::lower(title) match "hello*" && metadata.stats.views > 1000]
```

The predicate isn't just "field equals value" - it's an arbitrary expression tree combining:
- Function calls (`string::lower`)
- Multiple operators (`match`, `>`, `&&`)
- Multiple paths (`title`, `metadata.stats.views`)

Traditional B-tree indexes can't help with function transformations or complex boolean combinations.

### Problem 5: Schema-less Documents

Unlike SQL databases, Sanity documents are schema-less. Different documents of the same `_type` can have completely different shapes:

```javascript
// Both are valid "post" documents
{ _type: "post", title: "A", content: "..." }
{ _type: "post", headline: "B", body: [...], featured: true }
```

We can't know all paths in advance.

---

## Concrete Examples That Break Naive Approaches

### Example 1: Portable Text Search

Sanity's Portable Text format stores rich text as nested arrays:

```javascript
{
  body: [
    {
      _type: "block",
      style: "h1",
      children: [
        { _type: "span", text: "Welcome to ", marks: [] },
        { _type: "span", text: "Mario", marks: ["strong"] },
        { _type: "span", text: "'s World", marks: [] }
      ]
    },
    {
      _type: "block",
      style: "normal",
      children: [
        { _type: "span", text: "A story about...", marks: [] }
      ]
    }
  ]
}
```

Query: `*[body[].children[].text match "mario*"]`

To answer this:
- Traverse all blocks in body
- For each block, traverse all children
- For each child, check if text matches "mario*"
- If ANY match, include the document

A naive per-path index would need entries for every text span in every document.

### Example 2: Reference Graph Traversal

```javascript
// Customer document
{
  _id: "customer-1",
  name: "Acme Corp",
  projects: [
    { _ref: "project-1" },
    { _ref: "project-2" }
  ]
}

// Project document
{
  _id: "project-1",
  name: "Website Redesign",
  team: [
    { _ref: "person-1" },
    { _ref: "person-2" }
  ]
}

// Person document
{
  _id: "person-1",
  name: "Alice",
  skills: ["react", "typescript"]
}
```

Query: `*[_type == "customer" && projects[]->team[]->skills[] match "react"]`

This requires:
1. Find all customers
2. For each customer, resolve all project references
3. For each project, resolve all team member references
4. For each person, check if "react" is in skills
5. If ANY person on ANY project has "react", include the customer

**Depth: 3 dereferences, 3 array traversals**

### Example 3: Conditional Paths

```groq
*[
  (_type == "article" && content[].text match "breaking") ||
  (_type == "video" && transcript[].segments[].text match "breaking")
]
```

Different document types have different paths to searchable text. The "correct" path depends on the document type.

### Example 4: Dynamic Array Filtering

```groq
*[
  content[_type == "callout" && style == "warning"][0].message match "deprecated"
]
```

The array filter (`[_type == "callout" && style == "warning"]`) is evaluated at query time. We can't pre-compute which elements will match arbitrary predicates.

---

## Constraints on Any Solution

### Hard Requirements

1. **No False Negatives**: The index MUST NOT exclude documents that match the query. Missing results is unacceptable.

2. **Works for Arbitrary Paths**: Users should not need to declare which paths they'll query.

3. **Handles Array Traversal**: Must work with `[]` flattening operations.

4. **Handles Dereferencing**: Should accelerate `->` operations where possible (or at least not break).

5. **Reasonable Space**: Index size should be sub-linear or at worst linear in document count, not multiplicative with path count.

6. **Fast Filtering**: The whole point is to avoid full scans. Should eliminate most non-matching documents quickly.

### Acceptable Trade-offs

1. **False Positives OK**: The index can return candidates that don't actually match. We'll filter them with full predicate evaluation.

2. **Some Full Scans OK**: For pathological queries (e.g., `*[true]`), full scan is unavoidable.

3. **Build Time**: Index construction can be slower if it enables fast queries.

4. **Read-Only**: For groqdb's use case, the index is built once for static corpora.

---

## The Shape of a Solution

Given these constraints, a solution likely needs:

### 1. Content-Addressable Signatures

Each document gets a compact "signature" encoding what values appear at what paths:

```
Document: { title: "Hello", metadata: { featured: true } }
Signature: hash(["title", "Hello"]) | hash(["metadata.featured", true])
```

The signature is a probabilistic summary - it might produce false positives but never false negatives.

### 2. Path Normalization

Array indexes are normalized away:
- `body[0].text` → `body[*].text`
- `body[].children[].text` → `body[*].children[*].text`

This trades precision for generality - we can't distinguish `body[0]` from `body[1]`, but we can at least prune documents that don't have `body[*].text` at all.

### 3. Hierarchical Pruning

Signatures organized for fast bulk elimination:
- Level 0: Individual document signatures
- Level 1: Block signatures (OR of 64 doc signatures)
- Level 2: Super-block signatures (OR of 64 block signatures)

Query evaluation starts at the top and prunes entire branches.

### 4. Selective Exact Indexes

For extremely common patterns (`_type`, `_id`, common field names), maintain exact indexes alongside probabilistic ones.

---

## Open Questions for Research

1. **How to handle `match` predicates?**
   Text matching needs n-gram or prefix signatures, not just exact value hashes.

2. **How to handle range queries (`>`, `<`, `>=`, `<=`)?**
   Bloom filters work for membership testing, not ordering. Need different structure for numeric ranges.

3. **How to handle negation (`!=`, `!defined()`)?**
   Proving something is NOT in a document is fundamentally harder than proving it IS.

4. **How to handle dereference chains?**
   The referenced document might change independently. Do we pre-compute joined signatures?

5. **What's the right signature size?**
   Larger = fewer false positives but more space. What's the sweet spot?

6. **How to handle very deep paths?**
   `a.b.c.d.e.f.g.h` - does path depth affect signature design?

---

## Success Criteria

We succeed if we can demonstrate:

| Query Type | Without Omni-Index | With Omni-Index | Target |
|------------|-------------------|-----------------|--------|
| `*[_type == "post" && title match "mario"]` | Scan all posts | Scan ~5% of posts | 20x speedup |
| `*[body[].children[].text match "hello"]` | Scan all docs | Scan ~10% of docs | 10x speedup |
| `*[metadata.stats.views > 1000]` | Full scan | Scan ~20% of docs | 5x speedup |
| `*[deep.nested.path == "value"]` | Full scan | Scan ~5% of docs | 20x speedup |

---

## Next Steps

1. **Prototype document signatures** - Test different hashing schemes and signature sizes
2. **Benchmark false positive rates** - Measure actual FPR on real Sanity datasets
3. **Design path normalization** - Handle array traversal and filtering
4. **Explore range query support** - Investigate interval trees or learned indexes

---

*This document captures the challenge space. See RESEARCH-PLAN.md for the proposed research roadmap.*
