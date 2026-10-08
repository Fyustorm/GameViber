#!/usr/bin/env bash
#
# Prepares a Debian or Ubuntu VPS for the community server, once, as root:
# a system user running it, its data, its secrets, a systemd service and a
# "deploy" user that `../deploy.sh` signs in as. The reverse proxy (HTTPS) is
# the server's own, set up apart. Running it again keeps the secrets and the
# data, and refreshes the rest.
#
# From this PC, with the settings of server/deploy.env: `./deploy.sh --setup`.
# By hand:
#   ssh root@<server> 'bash -s' -- "$(cat <key>.pub)" [<deploy dir>] [<port>] [<address>] [<proxy>] < setup.sh
#
# <deploy dir> holds the releases (/opt/gameviber by default); <port> is the
# one the server listens on (8080 by default), at <address>: by default the
# Docker bridge's (docker0) when Docker runs, for a reverse proxy in a
# container, else 127.0.0.1. <proxy> is the reverse proxy's address (or
# CIDR range): only its forwarded client address is believed. Set it when
# the server listens on an address others reach (a proxy on another machine);
# never listen on a public one.

set -euo pipefail

DEPLOY_KEY="${1:?usage: setup.sh <deploy public key> [<deploy dir>] [<port>] [<address>] [<proxy>]}"
APP_DIR="${2:-/opt/gameviber}"
HTTP_PORT="${3:-8080}"
HTTP_HOST="${4:-}"
PROXY="${5:-}"
if [[ -z "$HTTP_HOST" ]]; then
	HTTP_HOST=$(ip -4 -o addr show docker0 2>/dev/null | awk '{ sub(/\/.*/, "", $4); print $4; exit }')
	HTTP_HOST="${HTTP_HOST:-127.0.0.1}"
fi

APP_USER="gameviber"
DEPLOY_USER="deploy"
DATA_DIR="/var/lib/gameviber"
CONFIG_DIR="/etc/gameviber"

[[ $EUID -eq 0 ]] || { echo "Run as root." >&2; exit 1; }
[[ "$APP_DIR" == /* && "$APP_DIR" != *[[:space:]]* ]] \
	|| { echo "The deploy directory must be an absolute path without spaces: $APP_DIR" >&2; exit 1; }
[[ "$HTTP_PORT" =~ ^[0-9]+$ && "$HTTP_PORT" -ge 1 && "$HTTP_PORT" -le 65535 ]] \
	|| { echo "Not a port: $HTTP_PORT" >&2; exit 1; }
[[ "$HTTP_HOST" =~ ^[0-9.]+$ ]] || { echo "Not an IPv4 address: $HTTP_HOST" >&2; exit 1; }
[[ -z "$PROXY" || "$PROXY" =~ ^[0-9.]+(/[0-9]+)?$ ]] || { echo "Not an IPv4 address or range: $PROXY" >&2; exit 1; }

echo "==> Packages"
apt-get update -q
apt-get install -y -q curl sqlite3

echo "==> Users"
id "$APP_USER" >/dev/null 2>&1 \
	|| useradd --system --home-dir "$DATA_DIR" --shell /usr/sbin/nologin "$APP_USER"
id "$DEPLOY_USER" >/dev/null 2>&1 || useradd --create-home --shell /bin/bash "$DEPLOY_USER"
install -d -m 700 -o "$DEPLOY_USER" -g "$DEPLOY_USER" "/home/$DEPLOY_USER/.ssh"
grep -qxF "$DEPLOY_KEY" "/home/$DEPLOY_USER/.ssh/authorized_keys" 2>/dev/null \
	|| echo "$DEPLOY_KEY" >> "/home/$DEPLOY_USER/.ssh/authorized_keys"
chown "$DEPLOY_USER:$DEPLOY_USER" "/home/$DEPLOY_USER/.ssh/authorized_keys"
chmod 600 "/home/$DEPLOY_USER/.ssh/authorized_keys"
# The deploy user may run the swap script as root, nothing else.
echo "$DEPLOY_USER ALL=(root) NOPASSWD: $APP_DIR/deploy.sh" > /etc/sudoers.d/gameviber-deploy
chmod 440 /etc/sudoers.d/gameviber-deploy
visudo -cf /etc/sudoers.d/gameviber-deploy >/dev/null

echo "==> Directories"
# "latest" is where uploads land; everything the root script runs stays root's.
install -d -m 755 -o root -g root "$APP_DIR" "$APP_DIR/releases"
install -d -m 755 -o "$DEPLOY_USER" -g "$DEPLOY_USER" "$APP_DIR/latest"
install -d -m 750 -o "$APP_USER" -g "$APP_USER" "$DATA_DIR"
install -d -m 750 -o root -g "$APP_USER" "$CONFIG_DIR"
# A deploy directory in a home: both users must get through its parents.
runuser -u "$APP_USER" -- test -x "$APP_DIR/releases" \
	&& runuser -u "$DEPLOY_USER" -- test -w "$APP_DIR/latest" \
	|| { echo "The users $APP_USER and $DEPLOY_USER cannot reach $APP_DIR: choose a directory" \
		"outside the homes (/opt, /srv), or let others through its parents (chmod o+x)." >&2; exit 1; }

echo "==> Secrets"
if [[ ! -f "$CONFIG_DIR/jwt.jwk" ]]; then
	key=$(head -c 32 /dev/urandom | base64 | tr '+/' '-_' | tr -d '=')
	printf '{"kty": "oct", "kid": "prod-key", "alg": "HS256", "k": "%s"}\n' "$key" > "$CONFIG_DIR/jwt.jwk"
fi
if [[ ! -f "$CONFIG_DIR/gameviber.env" ]]; then
	cat > "$CONFIG_DIR/gameviber.env" <<EOF
JWT_KEY_FILE=$CONFIG_DIR/jwt.jwk
STATS_SALT=$(head -c 24 /dev/urandom | base64 | tr -d '/+=')
ADMIN_TOKEN=$(head -c 24 /dev/urandom | base64 | tr -d '/+=')
EOF
fi
chown root:"$APP_USER" "$CONFIG_DIR/jwt.jwk" "$CONFIG_DIR/gameviber.env"
chmod 640 "$CONFIG_DIR/jwt.jwk" "$CONFIG_DIR/gameviber.env"

echo "==> Swap script"
{
check_host="$HTTP_HOST"
[[ "$check_host" == 0.0.0.0 ]] && check_host=127.0.0.1
printf '#!/usr/bin/env bash\nAPP_DIR=%q\nHTTP_HOST=%q\nHTTP_PORT=%q\n' "$APP_DIR" "$check_host" "$HTTP_PORT"
cat <<'EOF'
#
# Swaps in the binary uploaded to latest/, restarts the service, and puts the
# previous one back if the new one does not answer.
#   deploy.sh            the uploaded binary
#   deploy.sh rollback   the previous one

set -euo pipefail

KEEP=5

healthy() {
	for _ in $(seq 30); do
		curl -fsS "http://$HTTP_HOST:$HTTP_PORT/q/health/ready" >/dev/null 2>&1 && return 0
		sleep 1
	done
	return 1
}

switch_to() {
	ln -sfn "$1" "$APP_DIR/current.new"
	mv -T "$APP_DIR/current.new" "$APP_DIR/current"
	systemctl restart gameviber-server
}

previous=$(readlink -f "$APP_DIR/current" 2>/dev/null || true)

if [[ "${1:-}" == "rollback" ]]; then
	target=$(ls -1d "$APP_DIR"/releases/* | grep -vxF "$previous" | tail -n 1)
	[[ -n "$target" ]] || { echo "No other release." >&2; exit 1; }
	switch_to "$target"
	healthy && echo "Rolled back to $(basename "$target")" || { echo "Not healthy" >&2; exit 1; }
	exit 0
fi

upload="$APP_DIR/latest/gameviber-server-runner"
[[ -f "$upload" ]] || { echo "Nothing uploaded: $upload" >&2; exit 1; }

release="$APP_DIR/releases/$(date +%Y%m%d-%H%M%S)"
install -d -m 755 "$release"
install -m 755 -o root -g root "$upload" "$release/gameviber-server-runner"
rm -f "$upload"

switch_to "$release"
if healthy; then
	echo "Running $(basename "$release")"
	ls -1d "$APP_DIR"/releases/* | head -n -"$KEEP" | xargs -r rm -rf
else
	echo "The new release does not answer; back to the previous one." >&2
	journalctl -u gameviber-server -n 30 --no-pager >&2 || true
	if [[ -n "$previous" ]]; then
		switch_to "$previous"
		rm -rf "$release"
	fi
	exit 1
fi
EOF
} > "$APP_DIR/deploy.sh"
chown root:root "$APP_DIR/deploy.sh"
chmod 755 "$APP_DIR/deploy.sh"

echo "==> Service"
# The service sees no home, but the one holding its releases, read-only.
protect_home="yes"
[[ "$APP_DIR" == /home/* || "$APP_DIR" == /root/* ]] && protect_home="read-only"
cat > /etc/systemd/system/gameviber-server.service <<EOF
[Unit]
Description=GameViber community server
# docker0's address exists once Docker runs.
After=network-online.target docker.service
Wants=network-online.target
ConditionPathExists=$APP_DIR/current/gameviber-server-runner

[Service]
User=$APP_USER
Group=$APP_USER
EnvironmentFile=$CONFIG_DIR/gameviber.env
Environment=DATA_DIR=$DATA_DIR
Environment=QUARKUS_HTTP_HOST=$HTTP_HOST
Environment=QUARKUS_HTTP_PORT=$HTTP_PORT
${PROXY:+Environment=QUARKUS_HTTP_PROXY_TRUSTED_PROXIES=$PROXY}
WorkingDirectory=$DATA_DIR
ExecStart=$APP_DIR/current/gameviber-server-runner -Xmx256m
Restart=on-failure
RestartSec=5
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=$protect_home
PrivateTmp=yes
ReadWritePaths=$DATA_DIR

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable gameviber-server >/dev/null
# A port or a deploy directory changed: the running release moves to them.
if [[ -e "$APP_DIR/current" ]]; then
	systemctl restart gameviber-server
fi

echo
echo "Ready. The reverse proxy forwards to http://$HTTP_HOST:$HTTP_PORT"
if [[ -z "$PROXY" && "$HTTP_HOST" != 127.* ]]; then
	echo "Warning: no proxy address given, whoever reaches $HTTP_HOST:$HTTP_PORT can forge" \
		"the client address the limits count. Give it (PROXY_ADDRESS), or firewall the port." >&2
fi
echo "Back-office token (also in $CONFIG_DIR/gameviber.env):"
grep '^ADMIN_TOKEN=' "$CONFIG_DIR/gameviber.env" | cut -d= -f2
echo "Deploy from the PC with server/deploy.sh (SSH_USER=$DEPLOY_USER)."
