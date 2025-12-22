//! Holodex Prototype Implementation
//!
//! A minimal prototype for validating the Holodex approach:
//! - BinaryFuse8 filters per document (from xorf crate)
//! - XxHash64 for (path, value) hashing
//! - Path normalization: array indexes → [*]
//!
//! This is a research prototype, not production code.
//!
//! Usage:
//!   cargo run --bin holodex_prototype -- --input data.ndjson --query "title == \"Hello\""

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use xxhash_rust::xxh64::Xxh64;

// Note: In real implementation, use xorf::BinaryFuse8
// For prototype, we use a simple Bloom filter approximation
use bitvec::prelude::*;

/// Type tag bytes for hashing different value types
const TYPE_TAG_STRING: u8 = 0x01;
const TYPE_TAG_NUMBER: u8 = 0x02;
const TYPE_TAG_BOOL: u8 = 0x03;
const TYPE_TAG_NULL: u8 = 0x04;
const TYPE_TAG_PATH_ONLY: u8 = 0x05; // For defined() queries

/// Simple Bloom filter for prototype
/// In production, replace with BinaryFuse8 from xorf crate
pub struct BloomFilter {
    bits: BitVec,
    num_hashes: usize,
    size: usize,
}

impl BloomFilter {
    /// Create a new Bloom filter
    /// - num_elements: expected number of elements
    /// - fpr: target false positive rate (e.g., 0.01 for 1%)
    pub fn new(num_elements: usize, fpr: f64) -> Self {
        // Calculate optimal size and hash count
        // m = -n * ln(p) / (ln(2)^2)
        // k = (m/n) * ln(2)
        let m = (-(num_elements as f64) * fpr.ln() / (2.0_f64.ln().powi(2))).ceil() as usize;
        let k = ((m as f64 / num_elements as f64) * 2.0_f64.ln()).ceil() as usize;

        let size = m.max(64); // Minimum 64 bits
        let num_hashes = k.max(1).min(10); // 1-10 hash functions

        BloomFilter {
            bits: bitvec![0; size],
            num_hashes,
            size,
        }
    }

    /// Insert a hash into the filter
    pub fn insert(&mut self, hash: u64) {
        for i in 0..self.num_hashes {
            let idx = self.get_index(hash, i);
            self.bits.set(idx, true);
        }
    }

    /// Check if a hash might be in the filter
    pub fn contains(&self, hash: u64) -> bool {
        for i in 0..self.num_hashes {
            let idx = self.get_index(hash, i);
            if !self.bits[idx] {
                return false;
            }
        }
        true
    }

    fn get_index(&self, hash: u64, i: usize) -> usize {
        // Double hashing: h(i) = h1 + i*h2
        let h1 = hash as usize;
        let h2 = (hash >> 32) as usize;
        (h1.wrapping_add(i.wrapping_mul(h2))) % self.size
    }

    /// Get size in bytes
    pub fn size_bytes(&self) -> usize {
        self.bits.len() / 8
    }
}

/// Represents a JSON value for fingerprinting
#[derive(Debug, Clone)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    /// Parse from serde_json::Value
    pub fn from_serde(v: &serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => JsonValue::Null,
            serde_json::Value::Bool(b) => JsonValue::Bool(*b),
            serde_json::Value::Number(n) => JsonValue::Number(n.as_f64().unwrap_or(0.0)),
            serde_json::Value::String(s) => JsonValue::String(s.clone()),
            serde_json::Value::Array(arr) => {
                JsonValue::Array(arr.iter().map(JsonValue::from_serde).collect())
            }
            serde_json::Value::Object(obj) => {
                JsonValue::Object(obj.iter().map(|(k, v)| (k.clone(), JsonValue::from_serde(v))).collect())
            }
        }
    }
}

/// Hash a (path, value) pair using XxHash64
pub fn hash_pair(path: &str, value: &JsonValue) -> u64 {
    let mut hasher = Xxh64::new(0);

    // Hash the normalized path
    hasher.write(path.as_bytes());

    // Hash type tag + value
    match value {
        JsonValue::String(s) => {
            hasher.write_u8(TYPE_TAG_STRING);
            hasher.write(s.as_bytes());
        }
        JsonValue::Number(n) => {
            hasher.write_u8(TYPE_TAG_NUMBER);
            hasher.write(&n.to_le_bytes());
        }
        JsonValue::Bool(b) => {
            hasher.write_u8(TYPE_TAG_BOOL);
            hasher.write_u8(if *b { 1 } else { 0 });
        }
        JsonValue::Null => {
            hasher.write_u8(TYPE_TAG_NULL);
        }
        _ => {
            // Objects and arrays are not directly hashed as values
            // They contribute through their nested paths
        }
    }

    hasher.finish()
}

/// Hash just a path (for defined() queries)
pub fn hash_path_only(path: &str) -> u64 {
    let mut hasher = Xxh64::new(0);
    hasher.write(path.as_bytes());
    hasher.write_u8(TYPE_TAG_PATH_ONLY);
    hasher.finish()
}

/// Normalize a path segment
/// - Array indexes become [*]
/// - Preserves field names
fn normalize_path_segment(segment: &str) -> String {
    // Check if this is an array index like [0], [123], etc.
    if segment.starts_with('[') && segment.ends_with(']') {
        let inner = &segment[1..segment.len()-1];
        if inner.chars().all(|c| c.is_ascii_digit()) {
            return "[*]".to_string();
        }
    }
    segment.to_string()
}

/// Normalize a full query path for Holodex lookup
/// Converts: body[0].children[1].text → body[*].children[*].text
/// Converts: items[].name → items[*].name
pub fn normalize_query_path(path: &str) -> String {
    let mut result = String::new();
    let mut current_segment = String::new();
    let mut in_bracket = false;

    for ch in path.chars() {
        match ch {
            '[' => {
                if !current_segment.is_empty() {
                    if !result.is_empty() {
                        result.push('.');
                    }
                    result.push_str(&current_segment);
                    current_segment.clear();
                }
                in_bracket = true;
                current_segment.push(ch);
            }
            ']' => {
                current_segment.push(ch);
                in_bracket = false;
                let normalized = normalize_path_segment(&current_segment);
                result.push_str(&normalized);
                current_segment.clear();
            }
            '.' if !in_bracket => {
                if !current_segment.is_empty() {
                    if !result.is_empty() {
                        result.push('.');
                    }
                    result.push_str(&current_segment);
                    current_segment.clear();
                }
            }
            _ => {
                current_segment.push(ch);
            }
        }
    }

    // Handle remaining segment
    if !current_segment.is_empty() {
        if !result.is_empty() {
            result.push('.');
        }
        result.push_str(&current_segment);
    }

    result
}

/// Hash a query predicate with path normalization
pub fn hash_predicate(path: &str, value: &JsonValue) -> u64 {
    let normalized_path = normalize_query_path(path);
    hash_pair(&normalized_path, value)
}

/// Extract all (path, value) pairs from a document
pub fn extract_pairs(doc: &JsonValue) -> Vec<(String, JsonValue)> {
    let mut pairs = Vec::new();
    extract_pairs_recursive(doc, String::new(), &mut pairs);
    pairs
}

fn extract_pairs_recursive(value: &JsonValue, current_path: String, pairs: &mut Vec<(String, JsonValue)>) {
    match value {
        JsonValue::Object(obj) => {
            for (key, val) in obj {
                let new_path = if current_path.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", current_path, key)
                };

                // Add the path for this object key (for defined() queries)
                pairs.push((new_path.clone(), val.clone()));

                // Recurse into nested structures
                extract_pairs_recursive(val, new_path, pairs);
            }
        }
        JsonValue::Array(arr) => {
            for (i, val) in arr.iter().enumerate() {
                // Normalize array index to [*]
                let new_path = format!("{}[*]", current_path);

                // Add array element
                pairs.push((new_path.clone(), val.clone()));

                // Recurse
                extract_pairs_recursive(val, new_path, pairs);
            }
        }
        // Primitive values are already added by parent
        _ => {}
    }
}

/// Build a Bloom filter signature for a document
pub fn fingerprint(doc: &JsonValue) -> BloomFilter {
    let pairs = extract_pairs(doc);

    // Estimate element count (paths + values)
    let num_elements = pairs.len().max(10);

    // Target 1% FPR
    let mut filter = BloomFilter::new(num_elements, 0.01);

    // Insert all (path, value) hashes
    for (path, value) in &pairs {
        // Only hash primitive values
        match value {
            JsonValue::String(_) | JsonValue::Number(_) | JsonValue::Bool(_) | JsonValue::Null => {
                let hash = hash_pair(path, value);
                filter.insert(hash);
            }
            _ => {}
        }

        // Also insert path-only hash for defined() queries
        let path_hash = hash_path_only(path);
        filter.insert(path_hash);
    }

    filter
}

/// The main Holodex index structure
pub struct Holodex {
    /// Per-document Bloom filters
    signatures: Vec<BloomFilter>,
    /// Document IDs (parallel to signatures)
    doc_ids: Vec<String>,
}

impl Holodex {
    /// Build Holodex from a collection of documents
    pub fn build(docs: &[(String, JsonValue)]) -> Self {
        let mut signatures = Vec::with_capacity(docs.len());
        let mut doc_ids = Vec::with_capacity(docs.len());

        for (id, doc) in docs {
            let filter = fingerprint(doc);
            signatures.push(filter);
            doc_ids.push(id.clone());
        }

        Holodex { signatures, doc_ids }
    }

    /// Find candidate documents that might match a (path, value) predicate
    /// Path is automatically normalized (e.g., body[0].text → body[*].text)
    pub fn candidates_eq(&self, path: &str, value: &JsonValue) -> Vec<usize> {
        let hash = hash_predicate(path, value);

        self.signatures
            .iter()
            .enumerate()
            .filter(|(_, sig)| sig.contains(hash))
            .map(|(i, _)| i)
            .collect()
    }

    /// Find candidate documents that have a path defined
    pub fn candidates_defined(&self, path: &str) -> Vec<usize> {
        let hash = hash_path_only(path);

        self.signatures
            .iter()
            .enumerate()
            .filter(|(_, sig)| sig.contains(hash))
            .map(|(i, _)| i)
            .collect()
    }

    /// Get document ID by index
    pub fn doc_id(&self, idx: usize) -> &str {
        &self.doc_ids[idx]
    }

    /// Get total number of documents
    pub fn len(&self) -> usize {
        self.signatures.len()
    }

    /// Get total index size in bytes
    pub fn size_bytes(&self) -> usize {
        self.signatures.iter().map(|s| s.size_bytes()).sum()
    }
}

/// Metrics for evaluating Holodex effectiveness
#[derive(Debug, Default)]
pub struct HolodexMetrics {
    pub total_docs: usize,
    pub candidates: usize,
    pub true_matches: usize,
    pub false_positives: usize,
    pub false_positive_rate: f64,
    pub reduction_ratio: f64,
}

impl HolodexMetrics {
    pub fn calculate(total_docs: usize, candidates: usize, true_matches: usize) -> Self {
        let false_positives = candidates.saturating_sub(true_matches);
        let fpr = if candidates > 0 {
            false_positives as f64 / candidates as f64
        } else {
            0.0
        };
        let reduction = 1.0 - (candidates as f64 / total_docs as f64);

        HolodexMetrics {
            total_docs,
            candidates,
            true_matches,
            false_positives,
            false_positive_rate: fpr,
            reduction_ratio: reduction,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_doc(id: &str, title: &str, author_ref: &str) -> (String, JsonValue) {
        let doc = JsonValue::Object(vec![
            ("_id".to_string(), JsonValue::String(id.to_string())),
            ("_type".to_string(), JsonValue::String("post".to_string())),
            ("title".to_string(), JsonValue::String(title.to_string())),
            ("author".to_string(), JsonValue::Object(vec![
                ("_ref".to_string(), JsonValue::String(author_ref.to_string())),
            ])),
        ]);
        (id.to_string(), doc)
    }

    #[test]
    fn test_path_normalization() {
        assert_eq!(normalize_path_segment("[0]"), "[*]");
        assert_eq!(normalize_path_segment("[123]"), "[*]");
        assert_eq!(normalize_path_segment("title"), "title");
        assert_eq!(normalize_path_segment("[key]"), "[key]"); // Not a number, keep as-is
    }

    #[test]
    fn test_query_path_normalization() {
        // Simple paths unchanged
        assert_eq!(normalize_query_path("title"), "title");
        assert_eq!(normalize_query_path("metadata.featured"), "metadata.featured");

        // Array indexes normalized
        assert_eq!(normalize_query_path("body[0]"), "body[*]");
        assert_eq!(normalize_query_path("body[0].text"), "body[*].text");
        assert_eq!(normalize_query_path("body[0].children[1].text"), "body[*].children[*].text");

        // Empty array access [] also normalized
        assert_eq!(normalize_query_path("items[]"), "items[]");

        // Mixed paths
        assert_eq!(normalize_query_path("author._ref"), "author._ref");
        assert_eq!(normalize_query_path("categories[0]._ref"), "categories[*]._ref");
    }

    #[test]
    fn test_extract_pairs() {
        let doc = JsonValue::Object(vec![
            ("title".to_string(), JsonValue::String("Hello".to_string())),
            ("nested".to_string(), JsonValue::Object(vec![
                ("field".to_string(), JsonValue::Number(42.0)),
            ])),
        ]);

        let pairs = extract_pairs(&doc);

        // Should have: title, nested, nested.field
        assert!(pairs.iter().any(|(p, _)| p == "title"));
        assert!(pairs.iter().any(|(p, _)| p == "nested.field"));
    }

    #[test]
    fn test_holodex_basic() {
        let docs = vec![
            make_doc("doc-1", "Hello World", "author-1"),
            make_doc("doc-2", "Goodbye World", "author-2"),
            make_doc("doc-3", "Hello Again", "author-1"),
        ];

        let holodex = Holodex::build(&docs);

        // Query for title == "Hello World"
        let candidates = holodex.candidates_eq("title", &JsonValue::String("Hello World".to_string()));

        // Should find doc-1, might have false positives
        assert!(candidates.contains(&0), "Should find doc-1");

        // Verify no false negatives
        assert!(!candidates.is_empty());
    }

    #[test]
    fn test_holodex_nested_path() {
        let docs = vec![
            make_doc("doc-1", "Post 1", "author-1"),
            make_doc("doc-2", "Post 2", "author-2"),
            make_doc("doc-3", "Post 3", "author-1"),
        ];

        let holodex = Holodex::build(&docs);

        // Query for author._ref == "author-1"
        let candidates = holodex.candidates_eq("author._ref", &JsonValue::String("author-1".to_string()));

        // Should find doc-1 and doc-3
        assert!(candidates.contains(&0), "Should find doc-1");
        assert!(candidates.contains(&2), "Should find doc-3");
    }

    #[test]
    fn test_holodex_metrics() {
        let metrics = HolodexMetrics::calculate(1000, 50, 45);

        assert_eq!(metrics.false_positives, 5);
        assert!((metrics.false_positive_rate - 0.1).abs() < 0.001);
        assert!((metrics.reduction_ratio - 0.95).abs() < 0.001);
    }

    #[test]
    fn test_no_false_negatives() {
        // This is the critical invariant: we must never miss a matching document

        let docs: Vec<_> = (0..100)
            .map(|i| make_doc(&format!("doc-{}", i), &format!("Title {}", i), "author-1"))
            .collect();

        let holodex = Holodex::build(&docs);

        // Query for each title and verify it's in candidates
        for i in 0..100 {
            let title = format!("Title {}", i);
            let candidates = holodex.candidates_eq("title", &JsonValue::String(title));

            assert!(candidates.contains(&i),
                    "FALSE NEGATIVE: doc-{} not in candidates for its own title", i);
        }
    }

    #[test]
    fn test_array_path_normalization() {
        let doc = JsonValue::Object(vec![
            ("_id".to_string(), JsonValue::String("doc-1".to_string())),
            ("body".to_string(), JsonValue::Array(vec![
                JsonValue::Object(vec![
                    ("children".to_string(), JsonValue::Array(vec![
                        JsonValue::Object(vec![
                            ("text".to_string(), JsonValue::String("Hello".to_string())),
                        ]),
                    ])),
                ]),
            ])),
        ]);

        let pairs = extract_pairs(&doc);

        // Check that array paths are normalized
        let has_normalized_path = pairs.iter()
            .any(|(p, _)| p.contains("[*]"));

        assert!(has_normalized_path, "Array paths should be normalized to [*]");
    }
}

// ============================================================
// Benchmark Runner (when compiled as binary)
// ============================================================

#[cfg(feature = "bench")]
fn main() {
    use std::fs::File;
    use std::io::{BufRead, BufReader};
    use std::time::Instant;

    let args: Vec<String> = std::env::args().collect();
    let input_file = args.get(1).expect("Usage: holodex_prototype <input.ndjson>");

    println!("Loading documents from {}...", input_file);

    let file = File::open(input_file).expect("Failed to open file");
    let reader = BufReader::new(file);

    let mut docs = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line.expect("Failed to read line");
        if line.trim().is_empty() {
            continue;
        }
        let json: serde_json::Value = serde_json::from_str(&line)
            .expect(&format!("Failed to parse line {}", i + 1));

        let id = json.get("_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("doc-{}", i));

        docs.push((id, JsonValue::from_serde(&json)));
    }

    println!("Loaded {} documents", docs.len());

    // Build index
    println!("\nBuilding Holodex index...");
    let start = Instant::now();
    let holodex = Holodex::build(&docs);
    let build_time = start.elapsed();

    println!("Build time: {:?}", build_time);
    println!("Index size: {} bytes ({:.1} bytes/doc)",
             holodex.size_bytes(),
             holodex.size_bytes() as f64 / docs.len() as f64);

    // Run sample queries
    println!("\n--- Sample Queries ---");

    let queries = [
        ("_type", JsonValue::String("post".to_string())),
        ("_type", JsonValue::String("author".to_string())),
        ("title", JsonValue::String("Hello World".to_string())),
    ];

    for (path, value) in &queries {
        let start = Instant::now();
        let candidates = holodex.candidates_eq(path, value);
        let query_time = start.elapsed();

        println!("{} == {:?}: {} candidates in {:?}",
                 path, value, candidates.len(), query_time);
    }
}
