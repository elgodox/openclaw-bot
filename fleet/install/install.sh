#!/usr/bin/env bash
# =============================================================================
# OpenClaw Fleet — enrolador de máquinas LINUX
#
# Enrola ESTA máquina (de tu propiedad/administración) en tu flota:
#   1) Instala Tailscale y la une a tu Headscale.
#   2) Crea un usuario de automatización con tu clave SSH (sin password).
#   3) (Opcional) Instala RustDesk apuntando a tu servidor, para control gráfico.
#   4) Reporta hostname + IP de malla para que el bot la vea en el inventario.
#
# Uso típico (one-liner), como root o con sudo:
#   curl -fsSL https://tu-servidor/install.sh | sudo bash -s -- \
#       --server https://fleet.tudominio.com \
#       --authkey <FLEET_AUTHKEY> \
#       --ssh-pubkey "ssh-ed25519 AAAA... bot@control" \
#       --rustdesk-host fleet.tudominio.com \
#       --rustdesk-key <RUSTDESK_PUBKEY>
#
# Omití --rustdesk-* si no querés control gráfico en esa máquina.
# =============================================================================
set -euo pipefail

SERVER="" AUTHKEY="" SSH_PUBKEY="" RUSTDESK_HOST="" RUSTDESK_KEY=""
AUTO_USER="fleet"

while [ $# -gt 0 ]; do
  case "$1" in
    --server)        SERVER="$2"; shift 2;;
    --authkey)       AUTHKEY="$2"; shift 2;;
    --ssh-pubkey)    SSH_PUBKEY="$2"; shift 2;;
    --rustdesk-host) RUSTDESK_HOST="$2"; shift 2;;
    --rustdesk-key)  RUSTDESK_KEY="$2"; shift 2;;
    --user)          AUTO_USER="$2"; shift 2;;
    *) echo "Opción desconocida: $1" >&2; exit 1;;
  esac
done

[ "$(id -u)" -eq 0 ] || { echo "Ejecutá como root (sudo)." >&2; exit 1; }
[ -n "$SERVER" ] && [ -n "$AUTHKEY" ] || { echo "Faltan --server y --authkey." >&2; exit 1; }

log(){ echo -e "\033[1;36m>>\033[0m $*"; }

# --- 1) Tailscale + unión a Headscale -------------------------------------
if ! command -v tailscale >/dev/null 2>&1; then
  log "Instalando Tailscale..."
  curl -fsSL https://tailscale.com/install.sh | sh
fi
systemctl enable --now tailscaled >/dev/null 2>&1 || true

log "Uniendo la máquina a la malla ($SERVER)..."
tailscale up \
  --login-server="$SERVER" \
  --authkey="$AUTHKEY" \
  --ssh \
  --accept-routes \
  --hostname="$(hostname)" \
  --reset

# --- 2) Usuario de automatización + clave SSH -----------------------------
if [ -n "$SSH_PUBKEY" ]; then
  log "Configurando usuario de automatización '$AUTO_USER'..."
  id "$AUTO_USER" >/dev/null 2>&1 || useradd -m -s /bin/bash "$AUTO_USER"

  # sudo sin password SOLO para este usuario de automatización.
  echo "$AUTO_USER ALL=(ALL) NOPASSWD:ALL" > "/etc/sudoers.d/90-$AUTO_USER"
  chmod 440 "/etc/sudoers.d/90-$AUTO_USER"

  HOME_DIR="$(getent passwd "$AUTO_USER" | cut -d: -f6)"
  install -d -m 700 -o "$AUTO_USER" -g "$AUTO_USER" "$HOME_DIR/.ssh"
  touch "$HOME_DIR/.ssh/authorized_keys"
  grep -qxF "$SSH_PUBKEY" "$HOME_DIR/.ssh/authorized_keys" || echo "$SSH_PUBKEY" >> "$HOME_DIR/.ssh/authorized_keys"
  chmod 600 "$HOME_DIR/.ssh/authorized_keys"
  chown "$AUTO_USER:$AUTO_USER" "$HOME_DIR/.ssh/authorized_keys"
fi

# --- 3) RustDesk (control gráfico, opcional) ------------------------------
RUSTDESK_PW=""
if [ -n "$RUSTDESK_HOST" ] && [ -n "$RUSTDESK_KEY" ]; then
  log "Instalando RustDesk y apuntándolo a tu servidor..."
  if ! command -v rustdesk >/dev/null 2>&1; then
    ARCH="$(dpkg --print-architecture 2>/dev/null || echo amd64)"
    TMP="$(mktemp -d)"
    curl -fsSL -o "$TMP/rustdesk.deb" \
      "https://github.com/rustdesk/rustdesk/releases/latest/download/rustdesk-1.4.2-${ARCH}.deb" || true
    apt-get update -y >/dev/null 2>&1 || true
    apt-get install -y "$TMP/rustdesk.deb" >/dev/null 2>&1 || dpkg -i "$TMP/rustdesk.deb" || true
    rm -rf "$TMP"
  fi
  # Configura servidor propio y contraseña de acceso no atendido (aleatoria).
  RUSTDESK_PW="$(head -c 18 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 16)"
  rustdesk --config "host=$RUSTDESK_HOST,key=$RUSTDESK_KEY" >/dev/null 2>&1 || true
  rustdesk --password "$RUSTDESK_PW" >/dev/null 2>&1 || true
  systemctl enable --now rustdesk >/dev/null 2>&1 || true
fi

# --- 4) Reporte final ------------------------------------------------------
sleep 2
MESH_IP="$(tailscale ip -4 2>/dev/null | head -n1 || echo '?')"
RUSTDESK_ID="$(rustdesk --get-id 2>/dev/null || echo '-')"

cat <<EOF

========================= MÁQUINA ENROLADA =========================
 hostname      : $(hostname)
 IP de malla   : $MESH_IP
 usuario auto  : ${SSH_PUBKEY:+$AUTO_USER (SSH con clave, sudo sin pass)}${SSH_PUBKEY:+}
 rustdesk id   : $RUSTDESK_ID
 rustdesk pass : ${RUSTDESK_PW:-'(no instalado)'}
===================================================================
 Ya aparece en el inventario del bot:  python fleet.py list
EOF
