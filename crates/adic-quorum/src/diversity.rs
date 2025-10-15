use crate::{QuorumError, QuorumMember, Result};
use std::collections::HashMap;
use tracing::debug;

/// Diversity cap configuration
#[derive(Debug, Clone)]
pub struct DiversityCaps {
    pub max_per_asn: usize,
    pub max_per_region: usize,
}

impl DiversityCaps {
    pub fn new(max_per_asn: usize, max_per_region: usize) -> Self {
        Self {
            max_per_asn,
            max_per_region,
        }
    }
}

/// Enforce diversity caps on quorum members
/// Returns filtered list respecting ASN and region caps
pub fn enforce_diversity_caps(
    members: Vec<QuorumMember>,
    caps: &DiversityCaps,
) -> Result<Vec<QuorumMember>> {
    let mut asn_counts: HashMap<u32, usize> = HashMap::new();
    let mut region_counts: HashMap<String, usize> = HashMap::new();
    let mut result = Vec::new();

    // Sort by VRF score (lowest first) to prioritize selection order
    let mut sorted_members = members;
    sorted_members.sort_by_key(|m| m.vrf_score);

    for member in sorted_members {
        let mut can_add = true;

        // Check ASN cap
        if let Some(asn) = member.asn {
            let count = asn_counts.get(&asn).copied().unwrap_or(0);
            if count >= caps.max_per_asn {
                debug!(
                    asn,
                    count,
                    max = caps.max_per_asn,
                    "Skipping member due to ASN cap"
                );
                can_add = false;
            }
        }

        // Check region cap
        if let Some(ref region) = member.region {
            let count = region_counts.get(region).copied().unwrap_or(0);
            if count >= caps.max_per_region {
                debug!(
                    region,
                    count,
                    max = caps.max_per_region,
                    "Skipping member due to region cap"
                );
                can_add = false;
            }
        }

        if can_add {
            // Update counts
            if let Some(asn) = member.asn {
                *asn_counts.entry(asn).or_insert(0) += 1;
            }
            if let Some(ref region) = member.region {
                *region_counts.entry(region.clone()).or_insert(0) += 1;
            }

            result.push(member);
        }
    }

    Ok(result)
}

/// Check if adding a member would violate diversity caps
pub fn check_diversity_caps(
    member: &QuorumMember,
    current_members: &[QuorumMember],
    caps: &DiversityCaps,
) -> Result<()> {
    // Count current ASNs
    if let Some(asn) = member.asn {
        let asn_count = current_members
            .iter()
            .filter(|m| m.asn == Some(asn))
            .count();

        if asn_count >= caps.max_per_asn {
            return Err(QuorumError::DiversityCapExceeded {
                cap_type: format!("ASN {}", asn),
                limit: caps.max_per_asn,
                actual: asn_count + 1,
            });
        }
    }

    // Count current regions
    if let Some(ref region) = member.region {
        let region_count = current_members
            .iter()
            .filter(|m| m.region.as_ref() == Some(region))
            .count();

        if region_count >= caps.max_per_region {
            return Err(QuorumError::DiversityCapExceeded {
                cap_type: format!("Region {}", region),
                limit: caps.max_per_region,
                actual: region_count + 1,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use adic_types::PublicKey;

    fn make_member(asn: u32, region: &str, score: u8) -> QuorumMember {
        QuorumMember {
            public_key: PublicKey::from_bytes([score; 32]),
            reputation: 50.0,
            vrf_score: [score; 32],
            axis_id: 0,
            asn: Some(asn),
            region: Some(region.to_string()),
        }
    }

    #[test]
    fn test_asn_cap_enforcement() {
        let members = vec![
            make_member(100, "us-east", 1),
            make_member(100, "us-west", 2),
            make_member(100, "eu-west", 3),
            make_member(200, "ap-east", 4),
        ];

        let caps = DiversityCaps::new(2, 10);
        let result = enforce_diversity_caps(members, &caps).unwrap();

        // Should only have 2 members from ASN 100
        let asn_100_count = result.iter().filter(|m| m.asn == Some(100)).count();
        assert_eq!(asn_100_count, 2);
    }

    #[test]
    fn test_region_cap_enforcement() {
        let members = vec![
            make_member(100, "us-east", 1),
            make_member(200, "us-east", 2),
            make_member(300, "us-east", 3),
            make_member(400, "us-east", 4),
        ];

        let caps = DiversityCaps::new(10, 3);
        let result = enforce_diversity_caps(members, &caps).unwrap();

        // Should only have 3 members from us-east
        assert_eq!(result.len(), 3);
    }
}
