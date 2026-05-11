# Minion Vision

Minion is the smallest reliable path from "I have a Dockerized app" to "it is running on infrastructure I understand, with HTTPS, persistence, and operational basics handled."

The long-term idea is not to become a cheaper clone of Vercel. Minion should become the managed VPS platform for Docker apps: simple enough for solo builders, transparent enough for teams, and flexible enough to run any workload that can live inside a container.

## Core Belief

Modern deployment platforms are excellent when an application fits their preferred shape. Many apps do not. They need long-running processes, background workers, custom binaries, non-JavaScript runtimes, local disk, private services, or just a normal Linux environment.

Minion should make that world feel approachable again.

The promise:

> Run any Docker app on managed server capacity, with the ergonomics of a modern app platform and the escape hatch of a real machine.

## Positioning

Minion sits between raw VPS hosting and fully abstracted platforms.

Raw VPS providers give users control, but leave them responsible for provisioning, TLS, deploys, backups, monitoring, and security maintenance.

Platforms like Vercel and Cloudflare provide polished managed workflows, but they often optimize around specific runtimes, request lifecycles, framework conventions, and usage-based billing meters.

Minion should occupy a different lane:

> Heroku/Vercel-like deployment ergonomics for people who want Docker portability and server ownership.

## Product Principles

- Run normal containers.
- Prefer boring infrastructure over platform magic.
- Price around reserved capacity, not framework-specific events.
- Make the default path safe, but keep the machine understandable.
- Keep deployments portable: Docker, Compose, Traefik, SSH, and ordinary files should remain recognizable.
- Support both hobby projects and production apps without changing the mental model.

## Target Customer

Minion is for builders who want a simpler path to production without giving up control.

Good early customers:

- Indie hackers deploying full-stack apps.
- Agencies hosting client projects.
- Small teams that have outgrown shared hosting but do not want Kubernetes.
- Developers with apps that do not fit serverless platforms cleanly.
- Open-source maintainers who want self-hostable infrastructure with an optional managed layer.
- AI/tooling builders who need arbitrary runtimes, background jobs, queues, browsers, or custom binaries inside containers.

## Managed Hosting Model

The managed product should sell managed ownership, not opaque platform abstraction.

Two commercial modes can coexist:

### Bring Your Own VPS

Users connect a server from providers like DigitalOcean, Hetzner, Linode, Vultr, or another VPS host. Minion provisions it, deploys apps, manages Traefik, handles operational workflows, and provides a dashboard.

This is the lowest-risk starting point. Users keep the cloud account and infrastructure ownership. Minion charges for management, automation, visibility, and support.

### Minion Hosted Nodes

Minion provides the server and sends the customer one bill.

This is a cleaner user experience, but it means Minion owns more operational responsibility: uptime, abuse handling, provider failures, noisy workloads, migrations, backups, and support.

The hosted path should come after the operational playbook is proven through the BYO VPS model.

## Pricing Philosophy

Minion should bill for managed capacity.

The main billing unit is a node: a managed VPS with fixed CPU, memory, disk, and bandwidth. A customer can run one app or many apps on that node as long as they fit within its capacity.

This keeps pricing legible:

> Pick a server size. Deploy Docker apps. Upgrade when you need more capacity.

Avoid making the app count the primary billing lever. Charging per app too early makes the product feel like a tax on architecture. A web app, worker, cron process, and private service should be treated as natural parts of one system.

Possible plan shape:

| Plan | Use Case | Pricing Direction |
| --- | --- | --- |
| Starter Node | One small app, bot, API, or side project | Low monthly price |
| Small Node | A few apps, workers, or a small database | Moderate monthly price |
| Pro Node | Production app with more memory and traffic | Higher monthly price |
| Business Node | Teams, stronger support, compliance posture, and higher limits | Premium monthly price |

Add-ons should be clear and infrastructure-shaped:

- Extra persistent storage.
- Backups and snapshot retention.
- Managed Postgres.
- Managed Redis or Valkey.
- Log retention.
- Metrics retention.
- Uptime checks and alerting.
- Team features.
- Priority support.

Bandwidth should be generous by default, then metered beyond the included amount. Predictable pricing is part of the product value.

## Go To Market

The clean market message:

> Minion is managed hosting for any Docker app. No runtime lock-in, no serverless billing puzzles, no Kubernetes required.

The initial audience should be developers who already feel the pain:

- They tried Vercel or Cloudflare, but their workload did not fit.
- They have a VPS, but deployments and maintenance are annoying.
- They want predictable monthly hosting costs.
- They need background workers, custom runtimes, or persistent services.
- They are comfortable with Docker, but not interested in becoming DevOps staff.

### Wedge

Start with the most obvious job:

> Deploy a Dockerized app to a VPS with HTTPS in minutes.

This is already the core of the CLI. The managed layer should first make that flow safer and more repeatable:

- Server provisioning.
- App deploys.
- Domains and TLS.
- Environment variables and secrets.
- Logs.
- Status checks.
- Rollbacks.
- Backups.

### Differentiation

Against Vercel:

- Vercel is optimized for frontend and framework-native web deployment.
- Minion is optimized for arbitrary Docker workloads on managed server capacity.

Against Cloudflare:

- Cloudflare is a global edge platform.
- Minion is a simple managed server platform for ordinary containerized apps.

Against raw VPS hosting:

- VPS hosting gives a machine.
- Minion gives the machine plus the operational workflow.

Against Kubernetes:

- Kubernetes is powerful infrastructure orchestration.
- Minion is the pragmatic path for small teams that need production basics without cluster complexity.

### Launch Motion

Start developer-first:

- Keep the CLI open source.
- Publish simple deploy examples for common app types.
- Show real cost comparisons for common workloads.
- Emphasize Docker portability and predictable node pricing.
- Build trust through transparency: generated Compose files, visible logs, clear server state, and documented escape hatches.

The first paid product should likely be hosted management for BYO VPS:

- Connect a server.
- Let Minion configure it.
- Deploy from CLI or dashboard.
- Charge per managed server or per node tier.

Once the operational model is stable, add Minion Hosted Nodes for users who want the same experience without bringing their own provider account.

## Product Roadmap Themes

The CLI should remain useful at every step. New managed-platform features should first appear as practical CLI capabilities: deploy from CI, inspect a server, read logs, restart an app, validate configuration, and recover from mistakes.

This keeps Minion from becoming speculative platform code before it is a better daily tool.

Near-term:

- Configuration validation.
- Better error messages.
- `minion doctor`.
- Environment-first configuration for CI.
- Environment variable support.
- Secret management.
- Logs and status commands.
- Safer deploys with health checks.
- Rollbacks.

Mid-term:

- Managed dashboard.
- Team support.
- Backup and restore workflows.
- Metrics and alerting.
- Multi-app server overview.
- CI-friendly deploys.
- Provider integrations for VPS provisioning.

Long-term:

- Hosted nodes.
- Managed databases.
- Marketplace-style app templates.
- Multi-node support where it stays simple.
- Migration tools from raw VPS, Docker Compose, and platform providers.

## Immediate Roadmap

The next implementation phase should focus on two foundations: CI deployability and operational control.

### CI Deployability

Minion needs a fully non-interactive path so it can run inside GitHub Actions or another CI system.

Configuration should resolve in this order:

> CLI flags > environment variables > `.minion` file > interactive prompts

In CI mode, missing required values should fail with clear errors. Minion should never silently deploy with empty configuration.

Important environment variables:

- `MINION_VPS_HOST`
- `MINION_APP_NAME`
- `MINION_APP_URL`
- `MINION_APP_PORT`
- `MINION_APP_VOLUMES`
- `MINION_SSH_USER`
- `MINION_SSH_KEY_PATH`
- `MINION_SSH_PRIVATE_KEY`
- `MINION_SSH_PASSWORD`
- `MINION_SSH_PASSPHRASE`
- `MINION_DOCKER_PLATFORM`

The goal:

> A GitHub Action can check out a repo, load secrets, and run `minion deploy --ci` without prompts.

### Server Control

Once an app is deployed, Minion should make the server understandable from the same CLI.

The first control surface can be SSH-based and Compose-backed. It does not need a daemon or hosted control plane yet.

Initial commands:

- `minion status`
- `minion logs`
- `minion ps`
- `minion restart`
- `minion stop`
- `minion start`
- `minion doctor`

These commands should inspect the existing `/opt/minion/<app>` deployment directory and run ordinary Docker Compose commands remotely.

This creates a direct bridge to the later managed dashboard. The hosted product can eventually show the same information, but the CLI should prove the workflows first.

## Strategic North Star

Minion wins if a developer can say:

> I can deploy anything that fits in a container, pay a predictable price for the server it runs on, and still understand where my app lives.
