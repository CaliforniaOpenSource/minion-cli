# Minion CLI

A CLI tool that simplifies deploying any application to VPS servers.

## Overview

Minion CLI simplifies the process of deploying containerized applications to VPS servers. It handles Docker image building, transfer, and deployment with automatic SSL certificate management via Traefik.

## Prerequisites

Before using Minion CLI, ensure you have the following installed on your local machine:

### Required

- **Rust** (1.70 or later) - For building and installing the CLI
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

- **Docker** - For building container images locally
  - [Docker Desktop for Mac](https://docs.docker.com/desktop/install/mac-install/)
  - [Docker Desktop for Windows](https://docs.docker.com/desktop/install/windows-install/)
  - [Docker Engine for Linux](https://docs.docker.com/engine/install/)
  
  Make sure Docker is running before attempting to deploy.

- **SSH Access** - You need SSH key-based authentication to your VPS
  - First, ensure your public key is added to the VPS server's `~/.ssh/authorized_keys`
  - Then add your private key to the SSH agent on your local machine:
    ```bash
    ssh-add ~/.ssh/id_ed25519
    # or
    ssh-add ~/.ssh/id_rsa
    ```

### VPS Requirements

- Ubuntu/Debian-based VPS (tested on Ubuntu 25.10)
- Root or sudo access
- SSH access configured

## Installation

Install Minion CLI globally on your system:

```bash
# Clone the repository
git clone https://github.com/CaliforniaOpenSource/minion-cli.git
cd minion-cli

# Install the binary
cargo install --path .
```

This will install the `minion` command in your PATH.

To verify installation:
```bash
minion --version
```

## Usage

Minion CLI has three main commands:

### 1. Setup - Configure your VPS

Run this **once** to set up your VPS with Docker and Traefik:

```bash
minion setup
```

This command will:
- Create a `minion` user on the VPS
- Install Docker
- Set up Traefik as a reverse proxy
- Configure SSL certificate management with Let's Encrypt

You'll be prompted for:
- VPS hostname or IP address
- Email address for SSL certificates

### 2. Init - Initialize a project

Run this in your application directory to configure deployment settings:

```bash
cd /path/to/your/app
minion init
```

You'll be prompted for:
- VPS hostname or IP address
- Application name
- Domain/URL (e.g., `app.example.com`)
- Port your app listens on (e.g., `3000`)

This creates a `.minion` configuration file in your project.

### 3. Deploy - Deploy your application

Deploy your application to the VPS:

```bash
minion deploy
```

This command will:
1. Build a Docker image locally
2. Save and transfer the image to your VPS
3. Create a docker-compose configuration
4. Start your application with automatic SSL


Your app will be available at `https://app.example.com` after deployment completes.

## Configuration File

The `.minion` file stores your deployment configuration:

```
APP_NAME=my-app
APP_PORT=3000
APP_URL=app.example.com
APP_VOLUMES=
VPS_HOST=167.99.231.125
```

This file is automatically created by `minion init` and updated by `minion deploy`.

## Volume Mappings (experimental)
To persist data, you can specify volume mappings during deployment:

```bash
minion deploy
# When prompted for volumes:
# Enter: data:/app/data,uploads:/app/uploads
```

This maps:
- `/opt/minion/your-app/volumes/data` on VPS → `/app/data` in container
- `/opt/minion/your-app/volumes/uploads` on VPS → `/app/uploads` in container

## Troubleshooting

### SSH Key Issues

If you see `Error: no identities found in the ssh agent`:

```bash
# Start the SSH agent
eval "$(ssh-agent -s)"

# Add your SSH key
ssh-add ~/.ssh/id_ed25519
```

### Docker Not Running

If you see `docker: command not found` or connection errors:
- Ensure Docker Desktop is running
- Verify with: `docker ps`

### Build Failures

If Docker build fails, you can test locally:

```bash
docker build -t test-build . --platform=linux/amd64
```

This will show the full error output.

