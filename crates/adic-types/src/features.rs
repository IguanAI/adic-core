use num_bigint::BigUint;
use num_traits::Zero;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AxisId(pub u32);

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QpDigits {
    pub digits: Vec<u8>,
    pub p: u32,
}

impl QpDigits {
    pub fn new(value: &BigUint, p: u32, precision: usize) -> Self {
        let mut digits = Vec::with_capacity(precision);
        let mut v = value.clone();
        let p_big = BigUint::from(p);

        for _ in 0..precision {
            let digit = (&v % &p_big).to_u32_digits().first().copied().unwrap_or(0) as u8;
            digits.push(digit);
            v /= &p_big;
            if v.is_zero() {
                break;
            }
        }

        while digits.len() < precision {
            digits.push(0);
        }

        Self { digits, p }
    }

    pub fn from_u64(value: u64, p: u32, precision: usize) -> Self {
        Self::new(&BigUint::from(value), p, precision)
    }

    pub fn ball_id(&self, radius: usize) -> Vec<u8> {
        self.digits.iter().take(radius).copied().collect()
    }

    pub fn is_zero(&self) -> bool {
        self.digits.iter().all(|&d| d == 0)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.digits.clone()
    }
}

impl fmt::Debug for QpDigits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "QpDigits(p={}, digits={:?})", self.p, self.digits)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxisPhi {
    pub axis: AxisId,
    pub qp_digits: QpDigits,
}

impl AxisPhi {
    pub fn new(axis: u32, digits: QpDigits) -> Self {
        Self {
            axis: AxisId(axis),
            qp_digits: digits,
        }
    }
}

impl fmt::Debug for AxisPhi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AxisPhi(axis={}, {:?})", self.axis.0, self.qp_digits)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdicFeatures {
    pub phi: Vec<AxisPhi>,
}

impl AdicFeatures {
    pub fn new(phi: Vec<AxisPhi>) -> Self {
        Self { phi }
    }

    pub fn get_axis(&self, axis: AxisId) -> Option<&AxisPhi> {
        self.phi.iter().find(|p| p.axis == axis)
    }

    pub fn dimension(&self) -> usize {
        self.phi.len()
    }
}

impl fmt::Debug for AdicFeatures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdicFeatures")
            .field("dimension", &self.dimension())
            .field("phi", &self.phi)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_qp_digits() {
        let digits = QpDigits::from_u64(42, 3, 5);
        assert_eq!(digits.p, 3);
        assert_eq!(digits.digits.len(), 5);

        let ball_id = digits.ball_id(2);
        assert_eq!(ball_id.len(), 2);
    }

    #[test]
    fn test_adic_features() {
        let phi1 = AxisPhi::new(0, QpDigits::from_u64(10, 3, 4));
        let phi2 = AxisPhi::new(1, QpDigits::from_u64(20, 3, 4));
        let features = AdicFeatures::new(vec![phi1, phi2]);

        assert_eq!(features.dimension(), 2);
        assert!(features.get_axis(AxisId(0)).is_some());
        assert!(features.get_axis(AxisId(2)).is_none());
    }
}
