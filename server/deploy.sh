#!/usr/bin/env bash
#
# Deploys the native binary onto the server: upload it into the "latest" slot,
# then let the server-side script swap it in and restart the service.
#
# Usage:
#   ./deploy.sh              # asks whether to build, then deploys
#   ./deploy.sh --build      # builds without asking
#   ./deploy.sh --no-build   # deploys whatever is already in target/
#
# From the environment: SSH_HOST (required), SSH_USER, SSH_PORT.

set -euo pipefail

cd "$(dirname "$0")"

SSH_HOST="${SSH_HOST:?set SSH_HOST to the server}"
SSH_USER="${SSH_USER:-root}"
SSH_PORT="${SSH_PORT:-22}"

LOCAL_BINARY="target/gameviber-server-0.1.0-SNAPSHOT-runner"
REMOTE_BINARY="/opt/gameviber/latest/gameviber-server-runner"
REMOTE_DEPLOY_SCRIPT="/opt/gameviber/deploy.sh"

build=""
case "${1:-}" in
	--build) build="yes" ;;
	--no-build) build="no" ;;
	"") ;;
	*) echo "Unknown option: $1" >&2; exit 2 ;;
esac

if [[ -z "$build" ]]; then
	read -r -p "Build the native binary before deploying? (y/n) " answer
	[[ "$answer" == "y" || "$answer" == "o" ]] && build="yes" || build="no"
fi

if [[ "$build" == "yes" ]]; then
	echo "==> Building the native binary (in a container)"
	./mvnw package -Dnative
fi

if [[ ! -f "$LOCAL_BINARY" ]]; then
	echo "Binary missing: $LOCAL_BINARY (build first: ./deploy.sh --build)." >&2
	exit 1
fi

echo "==> Uploading to ${SSH_USER}@${SSH_HOST}:${REMOTE_BINARY}"
scp -P "$SSH_PORT" "$LOCAL_BINARY" "${SSH_USER}@${SSH_HOST}:${REMOTE_BINARY}"

echo "==> Swapping on the server"
ssh -p "$SSH_PORT" "${SSH_USER}@${SSH_HOST}" "sudo ${REMOTE_DEPLOY_SCRIPT}"

echo "Deployed"
