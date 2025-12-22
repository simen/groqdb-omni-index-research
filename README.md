# GROQDB Omni-Index Research

**A research project exploring probabilistic indexing strategies for GROQ query engines.**

## Mission Statement

GROQ is designed for non-expert developers who shouldn't need to think about indexes. Yet GROQ's expressiveness means users can filter and project over arbitrary field combinations, leading to full scans when specific indexes don't exist.

**Our goal**: Design an "omni-index" - a probabilistic data structure that can drastically reduce scan scope for *any* query predicate, without requiring per-field index configuration.

**Key constraint**: False positives are acceptable (we scan a few extra documents), but false negatives are not (we must never miss a matching document).

## The Problem

Consider this query:
```groq
count(*[_type=="post" && title match "mario"])
```

Currently, groqdb can use a `_type` index to narrow to posts, but `title match "mario"` requires scanning all posts. With 100k posts, this is expensive.

**What if we could probabilistically eliminate 95% of documents that *definitely* don't match, leaving only ~5k candidates for precise evaluation?**

## Research Goals

1. **Explore probabilistic pre-filter structures** that can answer "might this document match predicate P?" with controlled false positive rates

2. **Design a unified "omni-index"** that works across arbitrary field paths and predicate types (equality, match, range, etc.)

3. **Prototype and benchmark** against real GROQ workloads to validate the approach

4. **Integrate with groqdb** query planner as a pre-filter stage

## Repository Structure

```
/
├── README.md                    # This file
├── docs/
│   ├── RESEARCH-PLAN.md         # Detailed research roadmap
│   ├── LITERATURE-REVIEW.md     # Survey of related work
│   └── proposals/               # Design proposals
├── prototypes/                  # Experimental implementations
└── benchmarks/                  # Performance evaluation
```

## Related Work

This research draws inspiration from:

- **Bloom filters** - Classic probabilistic membership testing
- **Cuckoo filters** - Deletion-supporting alternative with better space efficiency
- **Roaring bitmaps** - Compressed bitmap indexes for set operations
- **Signature files** - Information retrieval technique using bit signatures
- **Learned indexes** - ML models approximating index structures
- **Column sketches** - Probabilistic data structures for approximate query processing

## Team

This is a collaborative research project with AI agents coordinated through the #omni-index channel.

---

*Research project for [groqdb](https://github.com/simen/groqdb-proto)*
