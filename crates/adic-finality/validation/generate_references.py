#!/usr/bin/env python3
"""
GUDHI Reference Data Generation for ADIC Persistent Homology Validation

This script generates reference persistence barcodes and Betti numbers using GUDHI
for validating the Rust F2 persistent homology implementation.

Usage:
    python3 generate_references.py

Requirements:
    pip install gudhi numpy

Output:
    JSON files in ./references/ directory containing:
    - Barcodes for each dimension
    - Betti numbers
    - Test case metadata
"""

import json
import numpy as np
from pathlib import Path

try:
    import gudhi
except ImportError:
    print("ERROR: GUDHI not installed. Install with: pip install gudhi")
    print("       or: conda install -c conda-forge gudhi")
    exit(1)


def save_reference(name: str, data: dict):
    """Save reference data to JSON file"""
    output_dir = Path(__file__).parent / "references"
    output_dir.mkdir(exist_ok=True)

    output_path = output_dir / f"{name}_reference.json"
    with open(output_path, 'w') as f:
        json.dump(data, f, indent=2)
    print(f"✓ Generated {output_path}")


def generate_torus_test():
    """
    Generate torus test case (2D surface embedded in R³)
    Expected: β₀ = 1, β₁ = 2, β₂ = 1
    """
    print("\n=== Generating Torus Test Case ===")

    # Sample points from torus
    n_samples = 1000
    np.random.seed(42)  # For reproducibility
    theta = np.random.uniform(0, 2*np.pi, n_samples)
    phi = np.random.uniform(0, 2*np.pi, n_samples)
    R, r = 2.0, 1.0

    x = (R + r*np.cos(theta)) * np.cos(phi)
    y = (R + r*np.cos(theta)) * np.sin(phi)
    z = r * np.sin(theta)
    points = np.column_stack([x, y, z])

    # Build Rips complex
    rips = gudhi.RipsComplex(points=points, max_edge_length=0.8)
    simplex_tree = rips.create_simplex_tree(max_dimension=3)

    # Compute persistence
    simplex_tree.compute_persistence()

    # Extract barcodes per dimension
    barcodes = {}
    for dim in range(4):
        intervals = simplex_tree.persistence_intervals_in_dimension(dim)
        # Convert numpy arrays to lists, handle infinity
        intervals_list = []
        for interval in intervals:
            birth, death = float(interval[0]), float(interval[1])
            if np.isinf(death):
                death = None  # Represent infinity as null in JSON
            intervals_list.append({"birth": birth, "death": death})
        barcodes[f"H{dim}"] = intervals_list

    # Compute Betti numbers
    betti = simplex_tree.betti_numbers()
    betti_dict = {f"beta_{dim}": int(betti[dim]) if dim < len(betti) else 0
                  for dim in range(4)}

    result = {
        "test_case": "torus",
        "description": "Torus (2D surface) - 1000 random points",
        "num_points": n_samples,
        "max_edge_length": 0.8,
        "max_dimension": 3,
        "expected_betti": {"beta_0": 1, "beta_1": 2, "beta_2": 1, "beta_3": 0},
        "betti_numbers": betti_dict,
        "barcodes": barcodes,
        "num_simplices": simplex_tree.num_simplices()
    }

    save_reference("torus", result)
    print(f"  Points: {n_samples}, Simplices: {simplex_tree.num_simplices()}")
    print(f"  Betti numbers: {betti_dict}")
    return result


def generate_sphere_test():
    """
    Generate sphere S² test case (unit sphere in R³)
    Expected: β₀ = 1, β₁ = 0, β₂ = 1
    """
    print("\n=== Generating Sphere Test Case ===")

    # Sample points from unit sphere
    n_samples = 500
    np.random.seed(43)

    # Use spherical coordinates
    theta = np.random.uniform(0, 2*np.pi, n_samples)
    phi = np.arccos(2*np.random.uniform(0, 1, n_samples) - 1)

    x = np.sin(phi) * np.cos(theta)
    y = np.sin(phi) * np.sin(theta)
    z = np.cos(phi)
    points = np.column_stack([x, y, z])

    # Build Rips complex
    rips = gudhi.RipsComplex(points=points, max_edge_length=0.6)
    simplex_tree = rips.create_simplex_tree(max_dimension=3)

    # Compute persistence
    simplex_tree.compute_persistence()

    # Extract barcodes
    barcodes = {}
    for dim in range(4):
        intervals = simplex_tree.persistence_intervals_in_dimension(dim)
        intervals_list = []
        for interval in intervals:
            birth, death = float(interval[0]), float(interval[1])
            if np.isinf(death):
                death = None
            intervals_list.append({"birth": birth, "death": death})
        barcodes[f"H{dim}"] = intervals_list

    # Betti numbers
    betti = simplex_tree.betti_numbers()
    betti_dict = {f"beta_{dim}": int(betti[dim]) if dim < len(betti) else 0
                  for dim in range(4)}

    result = {
        "test_case": "sphere",
        "description": "Sphere S² - 500 random points on unit sphere",
        "num_points": n_samples,
        "max_edge_length": 0.6,
        "max_dimension": 3,
        "expected_betti": {"beta_0": 1, "beta_1": 0, "beta_2": 1, "beta_3": 0},
        "betti_numbers": betti_dict,
        "barcodes": barcodes,
        "num_simplices": simplex_tree.num_simplices()
    }

    save_reference("sphere", result)
    print(f"  Points: {n_samples}, Simplices: {simplex_tree.num_simplices()}")
    print(f"  Betti numbers: {betti_dict}")
    return result


def generate_klein_bottle_test():
    """
    Generate Klein bottle test case (non-orientable surface in R⁴)
    Expected (over F₂): β₀ = 1, β₁ = 2, β₂ = 1
    """
    print("\n=== Generating Klein Bottle Test Case ===")

    # Sample points from Klein bottle in R⁴
    n_samples = 800
    np.random.seed(44)

    u = np.random.uniform(0, 2*np.pi, n_samples)
    v = np.random.uniform(0, 2*np.pi, n_samples)

    r = 2.0
    x = (r + np.cos(u/2)*np.sin(v) - np.sin(u/2)*np.sin(2*v)) * np.cos(u)
    y = (r + np.cos(u/2)*np.sin(v) - np.sin(u/2)*np.sin(2*v)) * np.sin(u)
    z = np.sin(u/2)*np.sin(v) + np.cos(u/2)*np.sin(2*v)
    w = np.cos(v)  # 4th dimension

    points = np.column_stack([x, y, z, w])

    # Build Rips complex
    rips = gudhi.RipsComplex(points=points, max_edge_length=1.2)
    simplex_tree = rips.create_simplex_tree(max_dimension=3)

    # Compute persistence
    simplex_tree.compute_persistence()

    # Extract barcodes
    barcodes = {}
    for dim in range(4):
        intervals = simplex_tree.persistence_intervals_in_dimension(dim)
        intervals_list = []
        for interval in intervals:
            birth, death = float(interval[0]), float(interval[1])
            if np.isinf(death):
                death = None
            intervals_list.append({"birth": birth, "death": death})
        barcodes[f"H{dim}"] = intervals_list

    # Betti numbers
    betti = simplex_tree.betti_numbers()
    betti_dict = {f"beta_{dim}": int(betti[dim]) if dim < len(betti) else 0
                  for dim in range(4)}

    result = {
        "test_case": "klein_bottle",
        "description": "Klein bottle - 800 points (non-orientable surface in R⁴)",
        "num_points": n_samples,
        "max_edge_length": 1.2,
        "max_dimension": 3,
        "expected_betti": {"beta_0": 1, "beta_1": 2, "beta_2": 1, "beta_3": 0},
        "betti_numbers": betti_dict,
        "barcodes": barcodes,
        "num_simplices": simplex_tree.num_simplices()
    }

    save_reference("klein_bottle", result)
    print(f"  Points: {n_samples}, Simplices: {simplex_tree.num_simplices()}")
    print(f"  Betti numbers: {betti_dict}")
    return result


def generate_simple_triangle_test():
    """
    Generate simple triangle test case for basic validation
    Expected: β₀ = 1, β₁ = 1, β₂ = 0
    """
    print("\n=== Generating Simple Triangle Test Case ===")

    # Create simplex tree directly
    simplex_tree = gudhi.SimplexTree()

    # Add vertices
    simplex_tree.insert([0])
    simplex_tree.insert([1])
    simplex_tree.insert([2])

    # Add edges with weights
    simplex_tree.insert([0, 1], filtration=1.0)
    simplex_tree.insert([1, 2], filtration=1.0)
    simplex_tree.insert([0, 2], filtration=1.0)

    # Compute persistence
    simplex_tree.compute_persistence()

    # Extract barcodes
    barcodes = {}
    for dim in range(3):
        intervals = simplex_tree.persistence_intervals_in_dimension(dim)
        intervals_list = []
        for interval in intervals:
            birth, death = float(interval[0]), float(interval[1])
            if np.isinf(death):
                death = None
            intervals_list.append({"birth": birth, "death": death})
        barcodes[f"H{dim}"] = intervals_list

    # Betti numbers
    betti = simplex_tree.betti_numbers()
    betti_dict = {f"beta_{dim}": int(betti[dim]) if dim < len(betti) else 0
                  for dim in range(3)}

    result = {
        "test_case": "simple_triangle",
        "description": "Simple triangle (3 vertices, 3 edges) - minimal test",
        "num_points": 3,
        "max_dimension": 1,
        "expected_betti": {"beta_0": 1, "beta_1": 1, "beta_2": 0},
        "betti_numbers": betti_dict,
        "barcodes": barcodes,
        "num_simplices": simplex_tree.num_simplices()
    }

    save_reference("simple_triangle", result)
    print(f"  Simplices: {simplex_tree.num_simplices()}")
    print(f"  Betti numbers: {betti_dict}")
    return result


def generate_tetrahedron_test():
    """
    Generate filled tetrahedron (3-simplex) test case
    Expected: β₀ = 1, β₁ = 0, β₂ = 0, β₃ = 0 (contractible)
    """
    print("\n=== Generating Tetrahedron Test Case ===")

    simplex_tree = gudhi.SimplexTree()

    # Add all faces of a tetrahedron
    simplex_tree.insert([0], filtration=0.0)
    simplex_tree.insert([1], filtration=0.0)
    simplex_tree.insert([2], filtration=0.0)
    simplex_tree.insert([3], filtration=0.0)

    # Edges
    simplex_tree.insert([0, 1], filtration=1.0)
    simplex_tree.insert([0, 2], filtration=1.0)
    simplex_tree.insert([0, 3], filtration=1.0)
    simplex_tree.insert([1, 2], filtration=1.0)
    simplex_tree.insert([1, 3], filtration=1.0)
    simplex_tree.insert([2, 3], filtration=1.0)

    # Triangular faces
    simplex_tree.insert([0, 1, 2], filtration=2.0)
    simplex_tree.insert([0, 1, 3], filtration=2.0)
    simplex_tree.insert([0, 2, 3], filtration=2.0)
    simplex_tree.insert([1, 2, 3], filtration=2.0)

    # Full tetrahedron
    simplex_tree.insert([0, 1, 2, 3], filtration=3.0)

    # Compute persistence
    simplex_tree.compute_persistence()

    # Extract barcodes
    barcodes = {}
    for dim in range(4):
        intervals = simplex_tree.persistence_intervals_in_dimension(dim)
        intervals_list = []
        for interval in intervals:
            birth, death = float(interval[0]), float(interval[1])
            if np.isinf(death):
                death = None
            intervals_list.append({"birth": birth, "death": death})
        barcodes[f"H{dim}"] = intervals_list

    # Betti numbers
    betti = simplex_tree.betti_numbers()
    betti_dict = {f"beta_{dim}": int(betti[dim]) if dim < len(betti) else 0
                  for dim in range(4)}

    result = {
        "test_case": "tetrahedron",
        "description": "Filled tetrahedron (3-ball) - contractible space",
        "num_points": 4,
        "max_dimension": 3,
        "expected_betti": {"beta_0": 1, "beta_1": 0, "beta_2": 0, "beta_3": 0},
        "betti_numbers": betti_dict,
        "barcodes": barcodes,
        "num_simplices": simplex_tree.num_simplices()
    }

    save_reference("tetrahedron", result)
    print(f"  Simplices: {simplex_tree.num_simplices()}")
    print(f"  Betti numbers: {betti_dict}")
    return result


def main():
    """Generate all reference test cases"""
    print("=" * 60)
    print("ADIC Persistent Homology Validation - Reference Generator")
    print("Using GUDHI version:", gudhi.__version__)
    print("=" * 60)

    # Generate all test cases
    results = []
    results.append(generate_simple_triangle_test())
    results.append(generate_tetrahedron_test())
    results.append(generate_sphere_test())
    results.append(generate_torus_test())
    results.append(generate_klein_bottle_test())

    # Generate summary
    summary = {
        "generator_version": "1.0.0",
        "gudhi_version": gudhi.__version__,
        "test_cases": [r["test_case"] for r in results],
        "total_test_cases": len(results)
    }

    save_reference("_summary", summary)

    print("\n" + "=" * 60)
    print(f"✓ Successfully generated {len(results)} reference test cases")
    print("=" * 60)
    print("\nNext steps:")
    print("  1. Run: cargo test --test validation_against_gudhi")
    print("  2. Verify all assertions pass with ε=10⁻⁶ tolerance")
    print("=" * 60)


if __name__ == "__main__":
    main()
