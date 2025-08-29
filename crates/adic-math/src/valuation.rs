use num_bigint::BigUint;
use num_traits::Zero;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadicValuation {
    pub value: u32,
    pub is_infinite: bool,
}

impl PadicValuation {
    pub fn new(value: u32) -> Self {
        Self {
            value,
            is_infinite: false,
        }
    }

    pub fn infinite() -> Self {
        Self {
            value: u32::MAX,
            is_infinite: true,
        }
    }

    pub fn is_zero(&self) -> bool {
        !self.is_infinite && self.value == 0
    }

    pub fn finite_value(&self) -> Option<u32> {
        if self.is_infinite {
            None
        } else {
            Some(self.value)
        }
    }

    pub fn min(&self, other: &Self) -> Self {
        match (self.is_infinite, other.is_infinite) {
            (true, true) => Self::infinite(),
            (true, false) => other.clone(),
            (false, true) => self.clone(),
            (false, false) => Self::new(self.value.min(other.value)),
        }
    }

    pub fn max(&self, other: &Self) -> Self {
        match (self.is_infinite, other.is_infinite) {
            (true, _) | (_, true) => Self::infinite(),
            (false, false) => Self::new(self.value.max(other.value)),
        }
    }
}

pub fn compute_valuation(n: &BigUint, p: u32) -> PadicValuation {
    if n.is_zero() {
        return PadicValuation::infinite();
    }

    let mut v = 0u32;
    let mut m = n.clone();
    let p_big = BigUint::from(p);

    while (&m % &p_big).is_zero() {
        v += 1;
        m /= &p_big;
    }

    PadicValuation::new(v)
}

pub fn valuation_of_sum(val_a: &PadicValuation, val_b: &PadicValuation) -> PadicValuation {
    val_a.min(val_b)
}

pub fn valuation_of_product(val_a: &PadicValuation, val_b: &PadicValuation) -> PadicValuation {
    match (val_a.is_infinite, val_b.is_infinite) {
        (true, _) | (_, true) => PadicValuation::infinite(),
        (false, false) => PadicValuation::new(val_a.value + val_b.value),
    }
}

pub fn valuation_of_quotient(
    val_a: &PadicValuation,
    val_b: &PadicValuation,
) -> Option<PadicValuation> {
    if val_b.is_infinite {
        return None;
    }

    if val_a.is_infinite {
        return Some(PadicValuation::infinite());
    }

    Some(PadicValuation::new(val_a.value.saturating_sub(val_b.value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_padic_valuation_basic() {
        let val = PadicValuation::new(3);
        assert_eq!(val.value, 3);
        assert!(!val.is_infinite);
        assert_eq!(val.finite_value(), Some(3));
    }

    #[test]
    fn test_padic_valuation_infinite() {
        let val = PadicValuation::infinite();
        assert!(val.is_infinite);
        assert_eq!(val.finite_value(), None);
        assert_eq!(val.value, u32::MAX);
    }

    #[test]
    fn test_padic_valuation_is_zero() {
        let val_zero = PadicValuation::new(0);
        assert!(val_zero.is_zero());

        let val_non_zero = PadicValuation::new(5);
        assert!(!val_non_zero.is_zero());

        let val_inf = PadicValuation::infinite();
        assert!(!val_inf.is_zero());
    }

    #[test]
    fn test_compute_valuation() {
        let n = BigUint::from(27u32);
        let val = compute_valuation(&n, 3);
        assert_eq!(val.value, 3);
        assert!(!val.is_infinite);

        let zero = BigUint::zero();
        let val_zero = compute_valuation(&zero, 3);
        assert!(val_zero.is_infinite);
    }

    #[test]
    fn test_compute_valuation_various_cases() {
        // 8 = 2^3
        let n = BigUint::from(8u32);
        let val = compute_valuation(&n, 2);
        assert_eq!(val.value, 3);

        // 10 = 2 * 5
        let n2 = BigUint::from(10u32);
        let val2 = compute_valuation(&n2, 2);
        assert_eq!(val2.value, 1);

        // 10 = 2 * 5
        let val3 = compute_valuation(&n2, 5);
        assert_eq!(val3.value, 1);

        // 7 is not divisible by 2
        let n3 = BigUint::from(7u32);
        let val4 = compute_valuation(&n3, 2);
        assert_eq!(val4.value, 0);

        // Large number: 243 = 3^5
        let n4 = BigUint::from(243u32);
        let val5 = compute_valuation(&n4, 3);
        assert_eq!(val5.value, 5);
    }

    #[test]
    fn test_valuation_operations() {
        let val_a = PadicValuation::new(2);
        let val_b = PadicValuation::new(3);

        let sum_val = valuation_of_sum(&val_a, &val_b);
        assert_eq!(sum_val.value, 2);

        let prod_val = valuation_of_product(&val_a, &val_b);
        assert_eq!(prod_val.value, 5);

        let quot_val = valuation_of_quotient(&val_a, &val_b).unwrap();
        assert_eq!(quot_val.value, 0);
    }

    #[test]
    fn test_valuation_operations_with_infinite() {
        let val_finite = PadicValuation::new(5);
        let val_inf = PadicValuation::infinite();

        // Sum with infinite
        let sum_val = valuation_of_sum(&val_finite, &val_inf);
        assert_eq!(sum_val, val_finite);

        // Product with infinite
        let prod_val = valuation_of_product(&val_finite, &val_inf);
        assert!(prod_val.is_infinite);

        // Quotient with infinite divisor
        let quot_val = valuation_of_quotient(&val_finite, &val_inf);
        assert!(quot_val.is_none());

        // Quotient with infinite dividend
        let quot_val2 = valuation_of_quotient(&val_inf, &val_finite);
        assert!(quot_val2.unwrap().is_infinite);
    }

    #[test]
    fn test_valuation_quotient_saturating() {
        let val_a = PadicValuation::new(2);
        let val_b = PadicValuation::new(5);

        // 2 - 5 should saturate at 0
        let quot_val = valuation_of_quotient(&val_a, &val_b).unwrap();
        assert_eq!(quot_val.value, 0);

        // 5 - 2 = 3
        let quot_val2 = valuation_of_quotient(&val_b, &val_a).unwrap();
        assert_eq!(quot_val2.value, 3);
    }

    #[test]
    fn test_min_max() {
        let val_a = PadicValuation::new(2);
        let val_b = PadicValuation::new(3);
        let val_inf = PadicValuation::infinite();

        assert_eq!(val_a.min(&val_b).value, 2);
        assert_eq!(val_a.max(&val_b).value, 3);
        assert_eq!(val_a.min(&val_inf), val_a);
        assert!(val_a.max(&val_inf).is_infinite);
    }

    #[test]
    fn test_min_max_with_same_values() {
        let val_a = PadicValuation::new(5);
        let val_b = PadicValuation::new(5);

        assert_eq!(val_a.min(&val_b).value, 5);
        assert_eq!(val_a.max(&val_b).value, 5);
    }

    #[test]
    fn test_min_max_both_infinite() {
        let val_inf1 = PadicValuation::infinite();
        let val_inf2 = PadicValuation::infinite();

        assert!(val_inf1.min(&val_inf2).is_infinite);
        assert!(val_inf1.max(&val_inf2).is_infinite);
    }

    #[test]
    fn test_equality() {
        let val_a = PadicValuation::new(5);
        let val_b = PadicValuation::new(5);
        let val_c = PadicValuation::new(6);

        assert_eq!(val_a, val_b);
        assert_ne!(val_a, val_c);

        let val_inf1 = PadicValuation::infinite();
        let val_inf2 = PadicValuation::infinite();
        assert_eq!(val_inf1, val_inf2);
        assert_ne!(val_a, val_inf1);
    }

    #[test]
    fn test_clone() {
        let val = PadicValuation::new(10);
        let cloned = val.clone();
        assert_eq!(val, cloned);

        let val_inf = PadicValuation::infinite();
        let cloned_inf = val_inf.clone();
        assert_eq!(val_inf, cloned_inf);
    }
}
