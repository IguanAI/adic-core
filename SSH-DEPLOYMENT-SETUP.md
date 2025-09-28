# SSH Key Setup and Fast Deployment Guide for ADIC-Core

## Overview
This guide helps you set up passwordless SSH authentication and fast deployment for ADIC-Core by building on your local machine and deploying only the compiled binary to your server.

## Part 1: SSH Key Setup (One-time Configuration)

### Step 1: Generate SSH Key Pair
On your **local development machine**, generate a dedicated SSH key for ADIC deployments:

```bash
# Generate ED25519 key (recommended for security and performance)
ssh-keygen -t ed25519 -C "adic-deployment" -f ~/.ssh/adic_deploy_key

# Or if ED25519 is not supported, use RSA
ssh-keygen -t rsa -b 4096 -C "adic-deployment" -f ~/.ssh/adic_deploy_key
```

**Important:**
- When prompted for a passphrase, press Enter twice for no passphrase (enables automation)
- This creates two files:
  - `~/.ssh/adic_deploy_key` (private key - keep this secret!)
  - `~/.ssh/adic_deploy_key.pub` (public key - safe to share)

### Step 2: Copy Public Key to Server

#### Option A: Using ssh-copy-id (Easiest)
```bash
ssh-copy-id -i ~/.ssh/adic_deploy_key.pub username@your-server-ip
```

#### Option B: Manual Method
```bash
# Read your public key
cat ~/.ssh/adic_deploy_key.pub

# SSH into your server
ssh username@your-server-ip

# On the server, add the key to authorized_keys
mkdir -p ~/.ssh
chmod 700 ~/.ssh
echo "YOUR_PUBLIC_KEY_CONTENT" >> ~/.ssh/authorized_keys
chmod 600 ~/.ssh/authorized_keys
```

#### Option C: One-liner
```bash
cat ~/.ssh/adic_deploy_key.pub | ssh username@server-ip "mkdir -p ~/.ssh && cat >> ~/.ssh/authorized_keys && chmod 700 ~/.ssh && chmod 600 ~/.ssh/authorized_keys"
```

### Step 3: Configure SSH Client
Create or edit `~/.ssh/config` on your local machine:

```bash
# Edit SSH config
nano ~/.ssh/config
```

Add the following configuration:

```
Host adic-server
    HostName YOUR_SERVER_IP
    User YOUR_USERNAME
    Port 22
    IdentityFile ~/.ssh/adic_deploy_key
    IdentitiesOnly yes
    StrictHostKeyChecking no
    ServerAliveInterval 60
    ServerAliveCountMax 3
```

Replace:
- `YOUR_SERVER_IP` with your server's IP address or domain
- `YOUR_USERNAME` with your server username

### Step 4: Test SSH Connection
```bash
# Test the connection
ssh adic-server "echo 'SSH key authentication successful!'"

# You should see the success message without entering a password
```

### Troubleshooting SSH Setup

If SSH key authentication isn't working:

1. **Check permissions on server:**
```bash
ssh username@server-ip
chmod 700 ~/.ssh
chmod 600 ~/.ssh/authorized_keys
```

2. **Check SSH daemon configuration on server:**
```bash
sudo nano /etc/ssh/sshd_config
```
Ensure these are set:
```
PubkeyAuthentication yes
AuthorizedKeysFile .ssh/authorized_keys
```

3. **Restart SSH service on server:**
```bash
sudo systemctl restart sshd
# or
sudo service ssh restart
```

4. **Test with verbose output:**
```bash
ssh -vvv adic-server
```

## Part 2: Deployment Configuration

### Server Requirements
- Ubuntu 20.04+ or similar Linux distribution
- `/usr/local/bin` directory exists (or create custom location)
- Sufficient disk space for the binary (~50-100MB)
- Ports 9000, 9001, and 8080 available

### Directory Structure
On your server, create the following structure:
```bash
ssh adic-server
sudo mkdir -p /opt/adic/{bin,data,config,logs}
sudo chown -R $USER:$USER /opt/adic
```

## Part 3: Using the Deployment Scripts

### Quick Deployment
Once SSH is set up, deployment is simple:

```bash
# On your local machine
./deploy.sh
```

This will:
1. Build the release binary locally (2-5 minutes)
2. Transfer it to your server (~10 seconds)
3. Install and restart the service

### Manual Deployment
```bash
# Build locally
cargo build --release --bin adic

# Copy to server
scp target/release/adic adic-server:/opt/adic/bin/adic-new

# SSH to server and replace binary
ssh adic-server
cd /opt/adic/bin
mv adic adic-old
mv adic-new adic
chmod +x adic

# Restart service
./start-adic.sh
```

## Security Best Practices

1. **Protect your private key:**
```bash
chmod 600 ~/.ssh/adic_deploy_key
```

2. **Use a dedicated deployment user on the server:**
```bash
# On server
sudo useradd -m -s /bin/bash adic-deploy
sudo usermod -aG docker adic-deploy  # if using Docker
```

3. **Limit SSH access:**
   - Use firewall rules to restrict SSH access
   - Consider using fail2ban to prevent brute force attacks
   - Disable password authentication once keys are working

4. **Rotate keys periodically:**
   - Generate new keys every 6-12 months
   - Remove old public keys from server

## Backup Your Keys

**Important:** Backup your SSH keys securely:

```bash
# Create encrypted backup
tar czf - ~/.ssh/adic_deploy_key* | openssl enc -aes-256-cbc -out adic-keys-backup.tar.gz.enc

# Restore from backup
openssl enc -d -aes-256-cbc -in adic-keys-backup.tar.gz.enc | tar xzf -
```

Store the encrypted backup in a secure location (not on the same machine).

## Next Steps

1. Run `./deploy.sh` to test the deployment
2. Monitor the service with `ssh adic-server "journalctl -f -u adic"`
3. Check websocket connectivity with `./test-websocket.py`

## Support

For issues with:
- SSH setup: Check your server's SSH logs (`/var/log/auth.log`)
- Deployment: Check the deployment script output
- ADIC service: Check logs in `/opt/adic/logs/`