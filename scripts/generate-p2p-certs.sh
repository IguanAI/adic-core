#!/bin/bash

# Generate P2P certificates for ADIC nodes
# This creates a CA and node certificates for secure P2P connections

set -e

CERT_DIR="${1:-./certs}"
DOMAIN="${2:-adicl1.com}"
NODE_NAME="${3:-bootstrap1}"

echo "🔐 Generating P2P certificates..."
echo "   Certificate directory: $CERT_DIR"
echo "   Domain: $DOMAIN"
echo "   Node name: $NODE_NAME"

mkdir -p "$CERT_DIR"

# Generate CA private key
echo "📝 Generating CA private key..."
openssl genrsa -out "$CERT_DIR/ca-key.pem" 4096

# Generate CA certificate
echo "📝 Generating CA certificate..."
cat > "$CERT_DIR/ca.conf" <<EOF
[req]
distinguished_name = req_distinguished_name
x509_extensions = v3_ca
prompt = no

[req_distinguished_name]
C = US
ST = CA
L = San Francisco
O = ADIC Network
OU = Certificate Authority
CN = ADIC Network CA

[v3_ca]
basicConstraints = critical,CA:TRUE
keyUsage = critical,digitalSignature,keyCertSign,cRLSign
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:always,issuer:always
EOF

openssl req -new -x509 -days 3650 -key "$CERT_DIR/ca-key.pem" \
    -out "$CERT_DIR/ca-cert.pem" -config "$CERT_DIR/ca.conf"

# Generate node private key
echo "📝 Generating node private key..."
openssl genrsa -out "$CERT_DIR/node-key.pem" 4096

# Generate node certificate request
echo "📝 Generating node certificate request..."
cat > "$CERT_DIR/node.conf" <<EOF
[req]
distinguished_name = req_distinguished_name
req_extensions = v3_req
prompt = no

[req_distinguished_name]
C = US
ST = CA
L = San Francisco
O = ADIC Network
OU = P2P Node
CN = $NODE_NAME.$DOMAIN

[v3_req]
basicConstraints = CA:FALSE
keyUsage = nonRepudiation, digitalSignature, keyEncipherment
subjectAltName = @alt_names

[alt_names]
DNS.1 = $NODE_NAME.$DOMAIN
DNS.2 = *.$DOMAIN
DNS.3 = localhost
IP.1 = 172.232.181.239
IP.2 = 127.0.0.1
IP.3 = ::1
EOF

openssl req -new -key "$CERT_DIR/node-key.pem" \
    -out "$CERT_DIR/node.csr" -config "$CERT_DIR/node.conf"

# Sign node certificate with CA
echo "📝 Signing node certificate with CA..."
cat > "$CERT_DIR/node-sign.conf" <<EOF
basicConstraints = CA:FALSE
keyUsage = nonRepudiation, digitalSignature, keyEncipherment
subjectAltName = @alt_names
extendedKeyUsage = serverAuth, clientAuth

[alt_names]
DNS.1 = $NODE_NAME.$DOMAIN
DNS.2 = *.$DOMAIN
DNS.3 = localhost
IP.1 = 172.232.181.239
IP.2 = 127.0.0.1
IP.3 = ::1
EOF

openssl x509 -req -in "$CERT_DIR/node.csr" \
    -CA "$CERT_DIR/ca-cert.pem" -CAkey "$CERT_DIR/ca-key.pem" \
    -CAcreateserial -out "$CERT_DIR/node-cert.pem" \
    -days 365 -extfile "$CERT_DIR/node-sign.conf"

# Create combined certificate chain
echo "📝 Creating certificate chain..."
cat "$CERT_DIR/node-cert.pem" "$CERT_DIR/ca-cert.pem" > "$CERT_DIR/node-chain.pem"

# Convert to DER format for Rust
echo "📝 Converting certificates to DER format..."
openssl x509 -in "$CERT_DIR/node-cert.pem" -outform DER -out "$CERT_DIR/node-cert.der"
openssl x509 -in "$CERT_DIR/ca-cert.pem" -outform DER -out "$CERT_DIR/ca-cert.der"
openssl rsa -in "$CERT_DIR/node-key.pem" -outform DER -out "$CERT_DIR/node-key.der"

# Display certificate info
echo ""
echo "✅ Certificates generated successfully!"
echo ""
echo "📋 Certificate details:"
openssl x509 -in "$CERT_DIR/node-cert.pem" -text -noout | grep -E "Subject:|DNS:|IP:"
echo ""
echo "📁 Files created:"
ls -la "$CERT_DIR"/*.pem "$CERT_DIR"/*.der 2>/dev/null | awk '{print "   " $9}'
echo ""
echo "🔧 To use these certificates:"
echo "   1. Copy ca-cert.pem to all nodes"
echo "   2. Copy node-cert.pem and node-key.pem to the specific node"
echo "   3. Update node config with ca_cert_path pointing to ca-cert.pem"
echo ""
echo "🚀 For production deployment:"
echo "   - Use Let's Encrypt for the API endpoint (HTTPS)"
echo "   - Use these certificates for P2P connections (QUIC)"