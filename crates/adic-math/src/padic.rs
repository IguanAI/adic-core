use adic_types::features::QpDigits;
use num_bigint::BigUint;
use num_traits::Zero;

pub fn vp(n: u64, p: u32) -> u32 {
    if n == 0 {
        return u32::MAX;
    }

    let mut v = 0;
    let mut m = n;
    let p64 = p as u64;

    while m % p64 == 0 {
        v += 1;
        m /= p64;
    }

    v
}

pub fn vp_bigint(n: &BigUint, p: u32) -> u32 {
    if n.is_zero() {
        return u32::MAX;
    }

    let mut v = 0;
    let mut m = n.clone();
    let p_big = BigUint::from(p);

    while (&m % &p_big).is_zero() {
        v += 1;
        m /= &p_big;
    }

    v
}

pub fn vp_diff(x: &QpDigits, y: &QpDigits) -> u32 {
    assert_eq!(x.p, y.p, "Cannot compute vp_diff for different primes");

    let max_len = x.digits.len().max(y.digits.len());

    for i in 0..max_len {
        let x_digit = x.digits.get(i).copied().unwrap_or(0);
        let y_digit = y.digits.get(i).copied().unwrap_or(0);

        if x_digit != y_digit {
            return i as u32;
        }
    }

    max_len as u32
}

pub fn compute_unit(n: &BigUint, p: u32, valuation: u32) -> BigUint {
    if n.is_zero() {
        return BigUint::zero();
    }

    let p_big = BigUint::from(p);
    let p_power = p_big.pow(valuation);
    n / p_power
}

pub fn expand_padic(n: &BigUint, p: u32, precision: usize) -> Vec<u8> {
    let mut digits = Vec::with_capacity(precision);
    let mut value = n.clone();
    let p_big = BigUint::from(p);

    for _ in 0..precision {
        if value.is_zero() {
            digits.push(0);
        } else {
            let digit = (&value % &p_big)
                .to_u32_digits()
                .first()
                .copied()
                .unwrap_or(0) as u8;
            digits.push(digit);
            value /= &p_big;
        }
    }

    digits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vp_zero() {
        assert_eq!(vp(0, 3), u32::MAX);
    }

    #[test]
    fn test_vp_powers() {
        assert_eq!(vp(1, 3), 0);
        assert_eq!(vp(3, 3), 1);
        assert_eq!(vp(9, 3), 2);
        assert_eq!(vp(27, 3), 3);
        assert_eq!(vp(81, 3), 4);
    }

    #[test]
    fn test_vp_mixed() {
        assert_eq!(vp(6, 3), 1);
        assert_eq!(vp(12, 3), 1);
        assert_eq!(vp(18, 3), 2);
        assert_eq!(vp(54, 3), 3);
    }

    #[test]
    fn test_vp_diff_same() {
        let x = QpDigits::from_u64(42, 3, 5);
        let y = QpDigits::from_u64(42, 3, 5);
        assert_eq!(vp_diff(&x, &y), 5);
    }

    #[test]
    fn test_vp_diff_different() {
        let x = QpDigits::from_u64(10, 3, 5);
        let y = QpDigits::from_u64(11, 3, 5);
        let diff = vp_diff(&x, &y);
        assert_eq!(diff, 0);
    }

    #[test]
    fn test_expand_padic() {
        let n = BigUint::from(42u32);
        let digits = expand_padic(&n, 3, 5);
        assert_eq!(digits.len(), 5);

        let reconstructed = QpDigits {
            digits: digits.clone(),
            p: 3,
        };
        assert_eq!(reconstructed.digits[0], 0);
        assert_eq!(reconstructed.digits[1], 2);
        assert_eq!(reconstructed.digits[2], 1);
        assert_eq!(reconstructed.digits[3], 1);
    }
}
