#!/usr/bin/env bash
#
# Deploys the native binary onto the server: upload it into the "latest" slot,
# then let the server-side script swap it in and restart the service.
#
# Usage:
#   ./deploy.sh              # asks whether to build, then deploys
#   ./deploy.sh --build      # builds without asking
#   ./deploy.sh --no-build   # deploys whatever is already in target/
#   ./deploy.sh --rollback   # puts the previous release back
#   ./deploy.sh --setup      # prepares the server, as SETUP_USER (deploy/setup.sh)
#
# Settings come from the environment or from deploy.env beside this script
# (not committed, see deploy.env.example): SSH_HOST (required), SSH_USER,
# SSH_PORT, SSH_KEY, DEPLOY_DIR, SERVER_PORT, SERVER_ADDRESS, PROXY_ADDRESS,
# and for --setup SETUP_USER.

set -euo pipefail

cd "$(dirname "$0")"

[[ -f deploy.env ]] && source deploy.env
SSH_HOST="${SSH_HOST:?set SSH_HOST to the server (in deploy.env)}"
SSH_USER="${SSH_USER:-deploy}"
SSH_PORT="${SSH_PORT:-22}"
DEPLOY_DIR="${DEPLOY_DIR:-/opt/gameviber}"
SERVER_PORT="${SERVER_PORT:-8080}"
SETUP_USER="${SETUP_USER:-root}"
ssh_options=(-o "Port=$SSH_PORT")
[[ -n "${SSH_KEY:-}" ]] && ssh_options+=(-i "$SSH_KEY" -o IdentitiesOnly=yes)

LOCAL_BINARY="target/gameviber-server-0.1.0-SNAPSHOT-runner"
REMOTE_BINARY="${DEPLOY_DIR}/latest/gameviber-server-runner"
REMOTE_DEPLOY_SCRIPT="${DEPLOY_DIR}/deploy.sh"

build=""
case "${1:-}" in
	--build) build="yes" ;;
	--no-build) build="no" ;;
	--rollback)
		ssh "${ssh_options[@]}" "${SSH_USER}@${SSH_HOST}" "sudo ${REMOTE_DEPLOY_SCRIPT} rollback"
		exit ;;
	--setup)
		[[ -f "${SSH_KEY:-}.pub" ]] || { echo "Set SSH_KEY: its public key lets the deploy user in." >&2; exit 1; }
		sudo=""
		[[ "$SETUP_USER" == root ]] || sudo="sudo"
		# Single-quoted for the remote shell, whichever it is.
		args=""
		for arg in "$(cat "${SSH_KEY}.pub")" "$DEPLOY_DIR" "$SERVER_PORT" "${SERVER_ADDRESS:-}" "${PROXY_ADDRESS:-}"; do
			args+="'${arg//\'/\'\\\'\'}' "
		done
		# The script goes in the command, so that sudo has the terminal to ask its password on.
		script=$(base64 -w0 < deploy/setup.sh)
		ssh -t "${ssh_options[@]}" "${SETUP_USER}@${SSH_HOST}" \
			"f=\$(mktemp) && echo $script | base64 -d > \"\$f\" && $sudo bash \"\$f\" $args; s=\$?; rm -f \"\$f\"; exit \$s"
		exit ;;
	"") ;;
	*) echo "Unknown option: $1" >&2; exit 2 ;;
esac

if [[ -z "$build" ]]; then
	read -r -p "Build the native binary before deploying? (y/n) " answer
	[[ "$answer" == "y" || "$answer" == "o" ]] && build="yes" || build="no"
fi

if [[ "$build" == "yes" ]]; then
	echo "==> Building the native binary (in a container)"
	./mvnw clean package -Dnative
fi

if [[ ! -f "$LOCAL_BINARY" ]]; then
	echo "Binary missing: $LOCAL_BINARY (build first: ./deploy.sh --build)." >&2
	exit 1
fi

echo "==> Uploading to ${SSH_USER}@${SSH_HOST}:${REMOTE_BINARY}"
scp "${ssh_options[@]}" "$LOCAL_BINARY" "${SSH_USER}@${SSH_HOST}:${REMOTE_BINARY}"

echo "==> Swapping on the server"
ssh "${ssh_options[@]}" "${SSH_USER}@${SSH_HOST}" "sudo ${REMOTE_DEPLOY_SCRIPT}"

echo "Deployed"
