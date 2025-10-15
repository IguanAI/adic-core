# Canonical JSON Fuzz Testing

This directory contains fuzz testing targets for the ADIC canonical JSON implementation. Fuzzing helps discover edge cases, security vulnerabilities, and unexpected behaviors by feeding random inputs to the code.

## Prerequisites

Fuzzing requires **Rust nightly** toolchain:

```bash
rustup toolchain install nightly
```

## Fuzz Targets

### 1. `fuzz_canonical_json_serialization`

Tests the canonical JSON serialization with arbitrary data structures.

**What it tests:**
- Serialization doesn't panic with random inputs
- Determinism: same data → same JSON output
- Valid UTF-8 output

**Run:**
```bash
cargo +nightly fuzz run fuzz_canonical_json_serialization
```

### 2. `fuzz_canonical_hash`

Tests canonical hash computation with various message structures.

**What it tests:**
- Hash computation doesn't panic
- Determinism: identical data → identical hash
- `verify_canonical_match` correctness
- Hash is always 32 bytes

**Run:**
```bash
cargo +nightly fuzz run fuzz_canonical_hash
```

### 3. `fuzz_key_ordering`

**CRITICAL SECURITY TEST** - Validates that different key orderings produce the same hash.

**What it tests:**
- Struct field declaration order doesn't affect hash
- HashMap insertion order doesn't affect hash
- Nested structures with different key orders hash identically

**Run:**
```bash
cargo +nightly fuzz run fuzz_key_ordering
```

## Running Fuzz Tests

### Quick Test (10 seconds)
```bash
cargo +nightly fuzz run fuzz_canonical_json_serialization -- -max_total_time=10
```

### Continuous Fuzzing (recommended: 1 hour+)
```bash
cargo +nightly fuzz run fuzz_canonical_json_serialization -- -max_total_time=3600
```

### Run All Targets
```bash
for target in fuzz_canonical_json_serialization fuzz_canonical_hash fuzz_key_ordering; do
    echo "Fuzzing $target..."
    cargo +nightly fuzz run $target -- -max_total_time=300
done
```

## Understanding Results

### Success
```
#1	INITED cov: 245 ft: 245 corp: 1/1b exec/s: 0 rss: 30Mb
#2	NEW    cov: 250 ft: 250 corp: 2/3b exec/s: 0 rss: 30Mb
...
```

### Crash Found
```
==12345==ERROR: AddressSanitizer: heap-buffer-overflow
```
If a crash is found, the input is saved in `fuzz/artifacts/<target>/crash-...`

## Coverage

Check code coverage:
```bash
cargo +nightly fuzz coverage fuzz_canonical_json_serialization
```

Generate coverage report:
```bash
cargo +nightly fuzz coverage fuzz_canonical_json_serialization --html
```

## Corpus

Fuzz corpus (interesting test cases) are saved in `fuzz/corpus/<target>/`

### Minimize corpus
```bash
cargo +nightly fuzz cmin fuzz_canonical_json_serialization
```

### Add custom corpus entries
```bash
echo '{"test": "data"}' > fuzz/corpus/fuzz_canonical_json_serialization/custom1
```

## CI Integration

### GitHub Actions Example
```yaml
- name: Install nightly Rust
  run: rustup toolchain install nightly

- name: Fuzz test canonical JSON (5 min)
  run: |
    cd crates/adic-types
    cargo +nightly fuzz run fuzz_canonical_json_serialization -- -max_total_time=300
```

## Security Considerations

The `fuzz_key_ordering` target is **critical for security**:

- **Attack vector**: Malicious nodes could attempt to create different hashes for same proposal by manipulating key order
- **Defense**: Canonical JSON sorts keys alphabetically
- **Validation**: This fuzz test ensures sorting works correctly for all data structures

If this fuzz test fails, it indicates a **critical security vulnerability** where:
1. Consensus could fail due to hash mismatches
2. Malicious nodes could create conflicting proposal IDs
3. Governance votes could be invalidated

## Troubleshooting

### Error: "requires nightly compiler"
```bash
rustup toolchain install nightly
```

### Error: "workspace issues"
The fuzz directory has its own workspace (`[workspace]` in Cargo.toml) to avoid conflicts.

### Slow execution
Add `-jobs=N` to parallelize:
```bash
cargo +nightly fuzz run <target> -- -jobs=8
```

## Resources

- [cargo-fuzz book](https://rust-fuzz.github.io/book/)
- [libFuzzer options](https://llvm.org/docs/LibFuzzer.html#options)
- ADIC Canonical JSON spec: `crates/adic-types/src/canonical_json.rs`
