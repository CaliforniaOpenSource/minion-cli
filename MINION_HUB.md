# Minion Hub

`minion-hub` is a companion executable that runs on a private hub VPS. It sets up a
WireGuard network, CoreDNS for private names, and a small HTTP API for managing
machine peers. It is separate from normal app deployment; `minion deploy` does not
use it yet.

## Defaults

| Setting | Value |
| --- | --- |
| WireGuard interface | `minion0` |
| Private network | `10.42.42.0/24` |
| Hub VPN IP | `10.42.42.1` |
| WireGuard listen port | `51820/udp` |
| CoreDNS listen address | `10.42.42.1:53` |
| API listen address | `10.42.42.1:4242` |

The API only binds to the WireGuard address. It should not be reachable on the hub's
public IP.

## Install And Initialize The Hub

On a fresh Ubuntu/Debian VPS, install the latest release binary and run `init`:

```bash
curl -fsSL https://raw.githubusercontent.com/CaliforniaOpenSource/minion-cli/main/install-minion-hub.sh | sh
sudo minion-hub init
```

The installer detects `amd64` or `arm64`, downloads the matching GitHub Release
asset, verifies it with `minion-hub-checksums.txt`, and installs the binary to
`/usr/local/bin/minion-hub`.

To install a specific release, set `MINION_HUB_VERSION` to the tag:

```bash
curl -fsSL https://raw.githubusercontent.com/CaliforniaOpenSource/minion-cli/main/install-minion-hub.sh \
  | MINION_HUB_VERSION=v0.0.7 sh
```

For development builds, you can still build the binary locally, copy it to the VPS,
then run `init` as root:

```bash
cargo build --release --bin minion-hub
scp target/release/minion-hub root@<hub-public-ip>:/root/minion-hub
ssh root@<hub-public-ip>
chmod +x /root/minion-hub
/root/minion-hub init
```

## Release Assets

The release workflow builds and uploads these files when a GitHub Release is
published:

```text
minion-hub-linux-amd64.tar.gz
minion-hub-linux-arm64.tar.gz
minion-hub-checksums.txt
```

It can also be run manually with a release tag through GitHub Actions.

`minion-hub init` is idempotent. It installs WireGuard and CoreDNS, creates the hub
WireGuard key if needed, writes the config files, installs the current executable to
`/usr/local/bin/minion-hub`, and starts these services:

```bash
systemctl status wg-quick@minion0
systemctl status coredns
systemctl status minion-hub
```

Important files:

```text
/etc/wireguard/minion0.conf
/etc/wireguard/minion0.pub
/etc/coredns/Corefile
/etc/coredns/minion.hosts
/etc/systemd/system/minion-hub.service
```

Use `cat /etc/wireguard/minion0.pub` on the hub when you need the hub public key
for client configs.

On Ubuntu releases where CoreDNS is not available through apt, `init` downloads the
official CoreDNS release to `/usr/local/bin/coredns` and installs its own systemd
unit.

## Add A Client

Install WireGuard tools on the client and generate a client keypair:

```bash
apt-get update
apt-get install -y wireguard-tools
umask 077
wg genkey | tee /etc/wireguard/minion0.key | wg pubkey > /etc/wireguard/minion0.pub
cat /etc/wireguard/minion0.pub
```

Register the client from the hub or from any machine that can already reach the hub
over WireGuard:

```bash
curl -X POST http://10.42.42.1:4242/machines \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "client",
    "vpn_ip": "10.42.42.2",
    "public_key": "<client-public-key>"
  }'
```

Then create `/etc/wireguard/minion0.conf` on the client:

```ini
[Interface]
Address = 10.42.42.2/24
PrivateKey = <client-private-key>

[Peer]
PublicKey = <hub-public-key-from-/etc/wireguard/minion0.pub>
Endpoint = <hub-public-ip>:51820
AllowedIPs = 10.42.42.0/24
PersistentKeepalive = 25
```

Start the tunnel:

```bash
systemctl enable --now wg-quick@minion0
```

Quick checks:

```bash
ping 10.42.42.1
curl http://10.42.42.1:4242/machines
dig @10.42.42.1 client
dig @10.42.42.1 example.com A
```

## API Reference

The API stores no separate database. Machine membership comes from the WireGuard
config, and names come from the CoreDNS hosts file.

All request bodies are JSON objects with string values.

```bash
# List machines
curl http://10.42.42.1:4242/machines

# Add a machine
curl -X POST http://10.42.42.1:4242/machines \
  -H 'Content-Type: application/json' \
  -d '{"name":"web-01","vpn_ip":"10.42.42.2","public_key":"<wireguard-public-key>"}'

# Read a machine
curl http://10.42.42.1:4242/machines/web-01

# Rename, move IPs, or rotate the peer public key
curl -X PATCH http://10.42.42.1:4242/machines/web-01 \
  -H 'Content-Type: application/json' \
  -d '{"name":"web-02","vpn_ip":"10.42.42.3","public_key":"<new-wireguard-public-key>"}'

# Delete a machine
curl -X DELETE http://10.42.42.1:4242/machines/web-02
```

Machine names may contain ASCII letters, numbers, dashes, underscores, and dots.
VPN IPs must be inside `10.42.42.0/24`; `.0`, `.1`, and `.255` are reserved. The hub
only stores peer public keys, never client private keys.

## Troubleshooting

```bash
wg show minion0
ip addr show minion0
ss -ltnup | grep -E '10\.42\.42\.1:(4242|53)|:51820'
journalctl -u minion-hub -u coredns -u wg-quick@minion0 --no-pager
```

If you use a firewall, allow public inbound UDP `51820` to the hub. Do not open TCP
`4242` publicly; the API is intended to be private to the WireGuard network.
