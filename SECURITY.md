# Security Policy

## Supported Versions

We release patches for security vulnerabilities. Currently supported versions:

| Version | Supported          |
| ------- | ------------------ |
| 1.0.x   | :white_check_mark: |
| < 1.0   | :x:                |

## Reporting a Vulnerability

The ADIC Core team takes security bugs seriously. We appreciate your efforts to responsibly disclose your findings, and will make every effort to acknowledge your contributions.

### How to Report

To report a security vulnerability, please use ONE of the following methods:

1. **Email**: Send details to ADICL1@proton.me with subject line "SECURITY: [brief description]"
2. **GitHub Security Advisories**: Use GitHub's private vulnerability reporting feature
3. **Encrypted Communication**: PGP key available upon request via email

### What to Include

Please include the following information in your report:

- Type of issue (e.g., buffer overflow, SQL injection, cross-site scripting, etc.)
- Full paths of source file(s) related to the manifestation of the issue
- The location of the affected source code (tag/branch/commit or direct URL)
- Any special configuration required to reproduce the issue
- Step-by-step instructions to reproduce the issue
- Proof-of-concept or exploit code (if possible)
- Impact of the issue, including how an attacker might exploit it

### Response Timeline

- **Initial Response**: Within 48 hours
- **Status Update**: Within 7 days
- **Resolution Target**: 
  - Critical: 7 days
  - High: 14 days
  - Medium: 30 days
  - Low: 90 days

### Disclosure Policy

- Security issues will be disclosed publicly after patches are available
- We will credit reporters who wish to be acknowledged
- We request a 90-day disclosure embargo for critical vulnerabilities

## Security Best Practices

When deploying ADIC Core:

### Node Operation
- Always use strong, unique keypairs
- Store private keys securely (use hardware security modules in production)
- Enable TLS for all API endpoints
- Use firewall rules to restrict network access
- Monitor logs for suspicious activity

### Docker Deployment
- Never use default passwords
- Set strong passwords via environment variables or secrets management
- Use Docker secrets or Kubernetes secrets for sensitive data
- Run containers with minimal privileges
- Keep base images updated

### Configuration
- Review all configuration before deployment
- Use environment-specific configs (dev/staging/prod)
- Enable rate limiting on public endpoints
- Configure appropriate resource limits
- Implement proper access controls

## Security Features

ADIC Core includes several security features:

- **Cryptographic Security**: Ed25519 signatures for message authentication
- **Deposit System**: Anti-spam mechanism with slashing for malicious behavior
- **Reputation System**: Tracks node behavior and penalizes bad actors
- **Message Validation**: Comprehensive validation of all messages
- **Rate Limiting**: Built-in protection against DoS attacks

## Dependencies

We regularly audit our dependencies for known vulnerabilities using:
- `cargo audit` for Rust dependencies
- GitHub Dependabot alerts
- Manual security reviews

## Contact

For security concerns, contact: ADICL1@proton.me

For general issues: https://github.com/IguanAI/adic-core/issues

## Update System Security (v0.1.7+)

### Update Verification
The P2P update system implements multiple layers of security:

1. **Cryptographic Verification**:
   - Ed25519 signature verification for all binaries
   - SHA256 hash verification for each 1MB chunk
   - DNS TXT record validation with optional DNSSEC

2. **Attack Mitigation**:
   - Sybil resistance through multiple peer verification
   - Version pinning to prevent forced updates
   - Rollback protection against downgrade attacks
   - Chunk poisoning prevention via hash verification

3. **Secure Distribution**:
   - P2P distribution reduces single points of failure
   - Reputation-based peer selection
   - Rate limiting to prevent resource exhaustion
   - Copyover technique preserves active connections

### Verifying Updates
```bash
# Verify binary signature
openssl dgst -sha256 -verify release.pub -signature update.sig adic-binary

# Check DNS record
dig TXT _version.adic.network.adicl1.com +short

# Verify via API
curl http://localhost:8080/update/verify
```

## Acknowledgments

We thank the following researchers for responsibly disclosing security issues:

<!-- Security researchers will be added here as issues are reported and fixed -->

---

*This security policy is subject to change. Last updated: December 2024*