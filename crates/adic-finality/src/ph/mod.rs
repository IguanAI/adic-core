pub mod adic_complex;
pub mod bottleneck;
pub mod f2_finality;
pub mod finality_method;
pub mod matrix;
pub mod persistence;
pub mod reduction;
pub mod simplex;
pub mod streaming;

pub use adic_complex::AdicComplexBuilder;
pub use bottleneck::bottleneck_distance;
pub use f2_finality::{F2Config, F2FinalityChecker, F2Result};
pub use finality_method::FinalityMethod;
pub use matrix::BoundaryMatrix;
pub use persistence::{PersistenceData, PersistenceDiagram, PersistenceInterval};
pub use reduction::{reduce_boundary_matrix, reduce_with_clearing, ReductionResult};
pub use simplex::{Simplex, SimplexId, SimplicialComplex, Weight};
