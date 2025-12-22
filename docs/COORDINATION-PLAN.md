# Omni-Index Research Coordination Plan

## Overview

This document defines the research tracks for exploring the omni-index problem. Each track is designed to be tackled by a separate agent, producing independent findings that will be synthesized into a final design.

**Coordinator**: balder
**Status**: Draft - awaiting approval

---

## Research Tracks

### Track 1: Probabilistic Data Structure Survey
**Focus**: Survey and compare probabilistic membership data structures

**Questions to answer**:
- What are the space/time/FPR trade-offs for Bloom, Cuckoo, XOR, and Quotient filters?
- Which structures support deletion? (relevant if we later support mutations)
- Which are most cache-friendly for bulk membership testing?
- What are the practical limits (element count, FPR floors)?

**Deliverables**:
- Comparison matrix with quantitative data
- Recommendation for our use case (read-only, bulk queries, ~1% target FPR)
- Links to best Rust implementations

**Why separate track**: Deep literature review requiring focused context on filter internals, not GROQ semantics.

---

### Track 2: Document Fingerprinting Strategies
**Focus**: How to convert a JSON document into a compact, queryable signature

**Questions to answer**:
- How should we hash (path, value) pairs? Separate hashes or combined?
- How to handle different value types (string, number, boolean, null)?
- How to normalize array paths (`body[0]` vs `body[*]` vs `body[]`)?
- Should we include path-only signatures (for `defined()` queries)?
- What's the right signature size per document?

**Deliverables**:
- Fingerprinting algorithm specification
- Analysis of signature size vs false positive rate
- Handling of edge cases (empty arrays, null values, deep nesting)

**Why separate track**: Core algorithm design requiring experimentation with different hashing approaches.

---

### Track 3: GROQ Predicate Analysis
**Focus**: Analyze real-world GROQ queries to understand what predicates we need to optimize

**Questions to answer**:
- What percentage of predicates are simple equality (`field == value`)?
- How common are nested paths vs root-level fields?
- How common are array traversals (`[]`) in filter predicates?
- What's the distribution of predicate complexity (single vs compound)?
- Which predicates cause the most full scans today?

**Deliverables**:
- Quantitative analysis of GROQ test suite predicates
- Categorization of predicate patterns by frequency
- Priority ranking: which patterns to optimize first

**Why separate track**: Empirical analysis requiring deep dive into GROQ test suite and real query patterns.

---

### Track 4: Range Query Strategies
**Focus**: How to handle non-equality predicates (>, <, >=, <=, match)

**Questions to answer**:
- Can Bloom-style filters help with range queries at all?
- Should we use separate structures for numeric ranges (interval trees, etc.)?
- How to handle `match` predicates (text matching with wildcards)?
- Are learned indexes viable for range estimation?
- What's the cost/benefit of range support vs equality-only?

**Deliverables**:
- Survey of range query acceleration techniques
- Recommendation: support ranges or defer to later stage?
- If supporting: proposed data structure design

**Why separate track**: Fundamentally different problem from membership testing; may conclude ranges are out of scope for v1.

---

### Track 5: Integration Architecture
**Focus**: How the omni-index integrates with groqdb's query pipeline

**Questions to answer**:
- Where in the pipeline does the omni-index sit? (before/after type index?)
- How does the query planner decide when to use it?
- What's the API between planner and omni-index?
- How do we handle predicates the omni-index can't help with?
- How do we combine omni-index results with existing indexes?

**Deliverables**:
- Integration design document
- API specification (Rust traits)
- Query planner modification proposal

**Why separate track**: Systems design requiring understanding of groqdb internals, separate from algorithm research.

---

### Track 6: Benchmarking Framework
**Focus**: Build infrastructure to measure omni-index effectiveness

**Questions to answer**:
- What datasets should we benchmark against?
- What queries represent realistic workloads?
- How do we measure false positive rate in practice?
- How do we measure speedup vs full scan?
- What's the baseline we're comparing against?

**Deliverables**:
- Benchmark suite (datasets + queries)
- Measurement methodology document
- Baseline measurements for current groqdb

**Why separate track**: Infrastructure work that enables evaluation of other tracks' outputs.

---

## Track Dependencies

```
Track 1 (Data Structures) ──┐
                            ├──> Track 2 (Fingerprinting) ──┐
Track 3 (Predicate Analysis)┘                               │
                                                            ├──> Synthesis
Track 4 (Range Queries) ────────────────────────────────────┤
                                                            │
Track 5 (Integration) ──────────────────────────────────────┤
                                                            │
Track 6 (Benchmarking) ─────────────────────────────────────┘
```

- Tracks 1, 3, 4, 5, 6 can run in parallel
- Track 2 benefits from Track 1 (data structure choice) and Track 3 (what to fingerprint)
- Final synthesis requires all tracks

---

## Proposed Agent Assignments

| Track | Agent Role | Key Skills Needed |
|-------|------------|-------------------|
| 1 | Literature Researcher | Academic paper analysis, data structure theory |
| 2 | Algorithm Designer | Hashing, probabilistic algorithms, JSON handling |
| 3 | Data Analyst | GROQ expertise, quantitative analysis |
| 4 | Algorithm Designer | Range queries, text search, ML/learned indexes |
| 5 | Systems Architect | Rust, groqdb internals, query planning |
| 6 | Infrastructure Engineer | Benchmarking, performance measurement |

**Note**: Some tracks could be combined if we have fewer agents (e.g., 1+4 both algorithm-heavy, 5+6 both systems-focused).

---

## Coordination Protocol

1. **Kickoff**: balder briefs each agent on their track, shares relevant context
2. **Checkpoints**: Agents post progress updates to #omni-index channel
3. **Questions**: Agents ask balder or each other for clarification
4. **Deliverables**: Each agent commits their findings to the research repo
5. **Synthesis**: balder collates findings and drafts unified design proposal

---

## Success Criteria

Research phase is complete when we have:
- [ ] Chosen a probabilistic data structure (Track 1)
- [ ] Defined fingerprinting algorithm (Track 2)
- [ ] Prioritized predicates to support (Track 3)
- [ ] Decision on range query support (Track 4)
- [ ] Integration design approved (Track 5)
- [ ] Benchmark suite ready (Track 6)

---

## Open Questions for simen

1. **Scope**: Should we include Track 4 (range queries) or defer to a future phase?
2. **Agents**: How many agents will we have? Should we combine any tracks?
3. **Timeline**: Any deadline pressure or is this exploratory?
4. **Real data**: Do we have access to real Sanity datasets for benchmarking, or just the test suite?

---

*Draft v1 - awaiting review*
