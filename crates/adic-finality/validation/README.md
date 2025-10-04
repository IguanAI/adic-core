# ADIC Finality Validation Suite

This directory contains GUDHI-based validation infrastructure for verifying the correctness of the Rust F2 persistent homology implementation.

## Purpose

Generate reference persistence barcodes using GUDHI (a gold-standard TDA library) and compare against our Rust implementation to ensure mathematical correctness per the 0.2.0 design document (Appendix D).

## Setup

### Install Dependencies with Poetry

```bash
cd crates/adic-finality/validation
poetry install
```

### Alternative: Manual Installation

```bash
pip install gudhi numpy
```

## Generate Reference Data

```bash
cd crates/adic-finality/validation
poetry run python generate_references.py
```

This generates JSON reference files in `references/` for:
- **simple_triangle**: Minimal test (3 vertices, 1 loop)
- **tetrahedron**: Filled 3-simplex (contractible, all β=0 except β₀=1)
- **sphere**: S² with 500 points (β₀=1, β₂=1)
- **torus**: 2D surface with 1000 points (β₀=1, β₁=2, β₂=1)
- **klein_bottle**: Non-orientable surface with 800 points (β₀=1, β₁=2, β₂=1)

## Run Validation Tests

```bash
cd ../../../  # Back to workspace root
cargo test --package adic-finality --test validation_against_gudhi
```

## Validation Criteria (Design Doc D.4)

Our Rust implementation must satisfy:

1. **Barcode Accuracy**: Birth/death times match GUDHI within ε = 10⁻⁶
2. **Betti Number Exactness**: Integer Betti numbers match exactly
3. **Bottleneck Distance**: Paired diagram distances match within ε = 10⁻⁴
4. **Performance Bounds**:
   - Torus (1000 points): < 100ms
   - Sphere (500 points): < 50ms
   - Tetrahedron: < 10ms

## File Structure

```
validation/
├── README.md                    # This file
├── pyproject.toml              # Poetry dependencies
├── generate_references.py      # GUDHI reference generator
└── references/                 # Generated JSON references
    ├── simple_triangle_reference.json
    ├── tetrahedron_reference.json
    ├── sphere_reference.json
    ├── torus_reference.json
    ├── klein_bottle_reference.json
    └── _summary_reference.json
```

## Reference JSON Format

Each reference file contains:

```json
{
  "test_case": "torus",
  "description": "Torus (2D surface) - 1000 random points",
  "num_points": 1000,
  "expected_betti": {"beta_0": 1, "beta_1": 2, "beta_2": 1},
  "betti_numbers": {"beta_0": 1, "beta_1": 2, "beta_2": 1},
  "barcodes": {
    "H0": [{"birth": 0.0, "death": 0.15}, ...],
    "H1": [{"birth": 0.3, "death": null}, ...],
    "H2": [{"birth": 0.5, "death": 0.75}]
  }
}
```

Note: `"death": null` represents infinite persistence (essential features).

## CI Integration

The validation suite runs automatically in CI:

```bash
# In .github/workflows/rust.yml
- name: Generate GUDHI references
  run: |
    cd crates/adic-finality/validation
    poetry install
    poetry run python generate_references.py

- name: Run validation tests
  run: cargo test --test validation_against_gudhi
```

## Troubleshooting

### GUDHI Installation Issues

If `poetry install` fails:

1. **macOS**: `brew install cmake eigen boost` (GUDHI needs these)
2. **Ubuntu/Debian**: `apt-get install libboost-all-dev libeigen3-dev cmake`
3. **Conda alternative**: `conda install -c conda-forge gudhi`

### Reference Regeneration

If you modify test parameters or need fresh references:

```bash
rm -rf references/*.json
poetry run python generate_references.py
```

## Design Document Reference

See `/0.2.0.design.md` Appendix D for full validation specification.
