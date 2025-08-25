use adic_types::features::QpDigits;
use crate::padic::vp_diff;

pub fn padic_distance(x: &QpDigits, y: &QpDigits) -> f64 {
    assert_eq!(x.p, y.p, "Cannot compute distance between different primes");

    // Handle exact equality for robustness
    if x.digits == y.digits {
        return 0.0;
    }
    
    let valuation = vp_diff(x, y);
    
    let p = x.p as f64;
    p.powi(-(valuation as i32))
}

pub fn proximity_score(x: &QpDigits, y: &QpDigits, radius: u32) -> f64 {
    let valuation = vp_diff(x, y);
    let clamped = valuation.min(radius);
    clamped as f64 / radius as f64
}

pub fn proximity_weight(x: &QpDigits, y: &QpDigits, radius: u32, lambda: f64) -> f64 {
    let prox = proximity_score(x, y, radius);
    (lambda * prox).exp()
}

pub fn are_close(x: &QpDigits, y: &QpDigits, min_valuation: u32) -> bool {
    vp_diff(x, y) >= min_valuation
}

pub fn find_closest(reference: &QpDigits, candidates: &[QpDigits]) -> Option<usize> {
    if candidates.is_empty() {
        return None;
    }
    
    let mut best_idx = 0;
    let mut best_valuation = vp_diff(reference, &candidates[0]);
    
    for (i, candidate) in candidates.iter().enumerate().skip(1) {
        let val = vp_diff(reference, candidate);
        if val > best_valuation {
            best_valuation = val;
            best_idx = i;
        }
    }
    
    Some(best_idx)
}

pub fn find_furthest(reference: &QpDigits, candidates: &[QpDigits]) -> Option<usize> {
    if candidates.is_empty() {
        return None;
    }
    
    let mut worst_idx = 0;
    let mut worst_valuation = vp_diff(reference, &candidates[0]);
    
    for (i, candidate) in candidates.iter().enumerate().skip(1) {
        let val = vp_diff(reference, candidate);
        if val < worst_valuation {
            worst_valuation = val;
            worst_idx = i;
        }
    }
    
    Some(worst_idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_padic_distance() {
        let x = QpDigits::from_u64(10, 3, 5);
        let y = QpDigits::from_u64(11, 3, 5);
        let dist = padic_distance(&x, &y);
        assert_eq!(dist, 1.0);
        
        let x2 = QpDigits::from_u64(9, 3, 5);
        let y2 = QpDigits::from_u64(12, 3, 5);
        let dist2 = padic_distance(&x2, &y2);
        assert!(dist2 < 1.0);
    }

    #[test]
    fn test_proximity_score() {
        let x = QpDigits::from_u64(10, 3, 5);
        let y = QpDigits::from_u64(11, 3, 5);
        let score = proximity_score(&x, &y, 2);
        assert_eq!(score, 0.0);
        
        let x2 = QpDigits::from_u64(9, 3, 5);
        let y2 = QpDigits::from_u64(12, 3, 5);
        let score2 = proximity_score(&x2, &y2, 2);
        assert_eq!(score2, 0.5);
    }

    #[test]
    fn test_proximity_weight() {
        let x = QpDigits::from_u64(10, 3, 5);
        let y = QpDigits::from_u64(11, 3, 5);
        let weight = proximity_weight(&x, &y, 2, 1.0);
        assert_eq!(weight, 1.0);
        
        let x2 = QpDigits::from_u64(9, 3, 5);
        let y2 = QpDigits::from_u64(12, 3, 5);
        let weight2 = proximity_weight(&x2, &y2, 2, 1.0);
        assert!(weight2 > 1.0);
    }

    #[test]
    fn test_find_closest() {
        let reference = QpDigits::from_u64(10, 3, 5);
        let candidates = vec![
            QpDigits::from_u64(11, 3, 5),
            QpDigits::from_u64(13, 3, 5),
            QpDigits::from_u64(19, 3, 5),
        ];
        
        let closest = find_closest(&reference, &candidates);
        assert_eq!(closest, Some(1));
    }
}