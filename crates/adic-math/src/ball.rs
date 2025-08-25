use adic_types::features::QpDigits;
use std::collections::HashSet;

pub fn ball_id(x: &QpDigits, radius: usize) -> Vec<u8> {
    x.ball_id(radius)
}

pub fn balls_are_distinct(points: &[QpDigits], radius: usize, min_distinct: usize) -> bool {
    let mut unique_balls = HashSet::new();
    
    for point in points {
        let id = ball_id(point, radius);
        unique_balls.insert(id);
        
        if unique_balls.len() >= min_distinct {
            return true;
        }
    }
    
    unique_balls.len() >= min_distinct
}

pub fn count_distinct_balls(points: &[QpDigits], radius: usize) -> usize {
    let mut unique_balls = HashSet::new();
    
    for point in points {
        let id = ball_id(point, radius);
        unique_balls.insert(id);
    }
    
    unique_balls.len()
}

pub fn is_in_ball(x: &QpDigits, center: &QpDigits, radius: usize) -> bool {
    assert_eq!(x.p, center.p, "Cannot compare balls with different primes");
    
    for i in 0..radius {
        let x_digit = x.digits.get(i).copied().unwrap_or(0);
        let c_digit = center.digits.get(i).copied().unwrap_or(0);
        
        if x_digit != c_digit {
            return false;
        }
    }
    
    true
}

pub fn find_ball_representative(points: &[QpDigits], radius: usize) -> Option<Vec<u8>> {
    if points.is_empty() {
        return None;
    }
    
    let first_id = ball_id(&points[0], radius);
    
    for point in &points[1..] {
        if ball_id(point, radius) != first_id {
            return None;
        }
    }
    
    Some(first_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ball_id_basic() {
        let x = QpDigits::from_u64(42, 3, 5);
        let id = ball_id(&x, 2);
        assert_eq!(id.len(), 2);
        assert_eq!(id, x.digits[..2].to_vec());
    }

    #[test]
    fn test_balls_are_distinct() {
        let points = vec![
            QpDigits::from_u64(10, 3, 5),
            QpDigits::from_u64(11, 3, 5),
            QpDigits::from_u64(12, 3, 5),
        ];
        
        assert!(balls_are_distinct(&points, 1, 3));
        
        let same_ball = vec![
            QpDigits::from_u64(9, 3, 5),
            QpDigits::from_u64(12, 3, 5),
            QpDigits::from_u64(15, 3, 5),
        ];
        
        assert!(!balls_are_distinct(&same_ball, 2, 3));
    }

    #[test]
    fn test_is_in_ball() {
        let center = QpDigits::from_u64(9, 3, 5);
        let x = QpDigits::from_u64(12, 3, 5);
        let y = QpDigits::from_u64(10, 3, 5);
        
        assert!(is_in_ball(&x, &center, 1));
        assert!(!is_in_ball(&y, &center, 1));
    }

    #[test]
    fn test_count_distinct_balls() {
        let points = vec![
            QpDigits::from_u64(10, 3, 5),
            QpDigits::from_u64(11, 3, 5),
            QpDigits::from_u64(12, 3, 5),
            QpDigits::from_u64(13, 3, 5),
        ];
        
        let count = count_distinct_balls(&points, 1);
        assert_eq!(count, 4);
        
        let count_r2 = count_distinct_balls(&points, 2);
        assert!(count_r2 <= count);
    }
}