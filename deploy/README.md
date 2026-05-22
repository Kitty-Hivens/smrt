# Deployment

First-time setup on a fresh Ubuntu / Debian VPS, then ongoing
deployments via `deploy.sh`.

## Prerequisites on the VPS

- nginx installed and running
- certbot installed (`apt install certbot python3-certbot-nginx`)
- A-record `smrt.hivens.dev` pointing at the VPS IP, propagated

## One-time VPS bootstrap

Run as root on the VPS:

```bash
# 1. service user + storage dir
useradd -r -s /usr/sbin/nologin -d /var/lib/smrt -M smrt
mkdir -p /var/lib/smrt/{packs,servers,cache}
chown -R smrt:smrt /var/lib/smrt

# 2. config dir + env file (admin token; pick a long random value)
mkdir -p /etc/smrt
cat > /etc/smrt/env <<EOF
SMRT_BIND_ADDR=127.0.0.1:9000
SMRT_STORAGE_DIR=/var/lib/smrt
SMRT_ADMIN_TOKEN=$(openssl rand -base64 32)
RUST_LOG=smrt=info,tower_http=info
EOF
chmod 640 /etc/smrt/env
chown root:smrt /etc/smrt/env

# 3. systemd unit (replace path with where you cloned this repo)
cp deploy/smrt.service /etc/systemd/system/smrt.service
systemctl daemon-reload

# 4. nginx site (HTTP-only initially; certbot fills HTTPS)
cp deploy/smrt.nginx.conf /etc/nginx/sites-available/smrt.conf
ln -s /etc/nginx/sites-available/smrt.conf /etc/nginx/sites-enabled/smrt.conf
nginx -t && systemctl reload nginx

# 5. issue cert (certbot edits the 443 block in-place)
certbot --nginx -d smrt.hivens.dev --non-interactive --agree-tos --email admin@hivens.dev

# 6. push the binary (from your dev machine, see Ongoing deployment below)

# 7. enable + start once the binary is in place
systemctl enable --now smrt
systemctl status smrt
```

## Ongoing deployment

From your dev machine:

```bash
./deploy/deploy.sh
```

The script builds `--release`, scp's the binary to
`/usr/local/bin/smrt.new`, atomically swaps it into place, and
restarts the systemd unit. Override host or key via env:

```bash
HOST=root@hivens.dev KEY=~/.ssh/other_key ./deploy/deploy.sh
```

## Verification

```bash
curl -s https://smrt.hivens.dev/v1/health | jq
```

Expected:

```json
{"schema_version":1,"status":"ok","version":"0.1.0"}
```

## Logs

```bash
ssh root@hivens.dev journalctl -u smrt -n 100 -f
```

## Rotating the admin token

```bash
# on VPS
sed -i "s|^SMRT_ADMIN_TOKEN=.*|SMRT_ADMIN_TOKEN=$(openssl rand -base64 32)|" /etc/smrt/env
systemctl restart smrt
```
