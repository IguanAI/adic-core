# Quick Deployment Guide for ADIC-Core

## Prerequisites
- Local machine with Rust installed (for building)
- Target server running Ubuntu/Debian
- SSH access to the server

## Step 1: Initial Setup (One-time)

### On Your Local Machine:
```bash
# 1. Generate SSH key for deployment
ssh-keygen -t ed25519 -f ~/.ssh/adic_deploy_key -N ""

# 2. Copy the public key to your server
ssh-copy-id -i ~/.ssh/adic_deploy_key.pub username@your-server-ip

# 3. Configure SSH (edit ~/.ssh/config)
echo "Host adic-server
    HostName YOUR_SERVER_IP
    User YOUR_USERNAME
    IdentityFile ~/.ssh/adic_deploy_key" >> ~/.ssh/config

# 4. Test connection
ssh adic-server echo "Connected!"
```

### On Your Server:
```bash
# 1. Copy and run the installation script
scp scripts/install-on-server.sh adic-server:/tmp/
ssh adic-server
sudo bash /tmp/install-on-server.sh
```

## Step 2: Deploy ADIC

### Edit the deployment script:
```bash
# Edit deploy.sh and update the SERVER_HOST if needed
nano deploy.sh
```

### Run deployment:
```bash
# This builds locally and deploys to server (takes 2-5 minutes)
./deploy.sh
```

## Step 3: Verify Deployment

### Check service status:
```bash
ssh adic-server "sudo systemctl status adic"
```

### Monitor logs:
```bash
ssh adic-server "tail -f /opt/adic/logs/adic.log"
```

### Test API:
```bash
ssh adic-server "curl http://localhost:8080/health"
```

## Common Commands

### Start/Stop Service:
```bash
# Start
ssh adic-server "sudo systemctl start adic"

# Stop
ssh adic-server "sudo systemctl stop adic"

# Restart
ssh adic-server "sudo systemctl restart adic"
```

### Update and Redeploy:
```bash
# Make your code changes locally, then:
./deploy.sh
```

### Skip Build (use existing binary):
```bash
./deploy.sh --skip-build
```

### Deploy without restarting:
```bash
./deploy.sh --no-restart
```

## WebSocket Testing

Test WebSocket connectivity to bootstrap node:
```bash
poetry run python test-websocket.py
```

## Troubleshooting

### SSH Connection Issues:
```bash
# Test with verbose output
ssh -vvv adic-server

# Check SSH key permissions
chmod 600 ~/.ssh/adic_deploy_key
```

### Service Won't Start:
```bash
# Check logs
ssh adic-server "journalctl -u adic -n 50"

# Check binary permissions
ssh adic-server "ls -la /opt/adic/bin/"

# Test binary manually
ssh adic-server "/opt/adic/bin/adic --version"
```

### Port Already in Use:
```bash
# Find what's using the port
ssh adic-server "sudo lsof -i :9000"
ssh adic-server "sudo lsof -i :8080"
```

## Performance Tips

1. **Build on powerful machine**: Use a machine with many CPU cores for faster builds
2. **Use cargo cache**: Subsequent builds are much faster
3. **Network speed**: Use compression with scp (already enabled in deploy.sh)
4. **Parallel deployment**: Deploy to multiple servers simultaneously

## Security Notes

- Keep your SSH private key secure
- Use firewall rules on the server
- Regularly update the ADIC binary
- Monitor logs for suspicious activity
- Consider using a dedicated deployment user

## Full Deployment Time Breakdown

- SSH key setup: 2 minutes (one-time)
- Server installation: 3 minutes (one-time)
- Local build: 2-5 minutes
- Binary transfer: 10 seconds
- Service restart: 5 seconds
- **Total per deployment: ~5 minutes**

Compare to building on server: **1+ hour**

## Need Help?

Check the detailed guides:
- `SSH-DEPLOYMENT-SETUP.md` - Complete SSH setup
- `deploy.sh --help` - Deployment script options
- Server logs: `/opt/adic/logs/`