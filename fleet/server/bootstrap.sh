#!/usr/bin/env bash
# Prepara el servidor de control después de "docker compose up -d".
# Crea el usuario de la flota, una API key (para el orquestador/bot) y una
# pre-auth key reutilizable (para los instaladores). Imprime todo al final.
set -euo pipefail

USER_NS="${1:-fleet}"
HS="docker exec fleet-headscale headscale"

echo ">> Esperando a que Headscale levante..."
for i in $(seq 1 30); do
  if $HS users list >/dev/null 2>&1; then break; fi
  sleep 1
done

echo ">> Creando usuario '$USER_NS' (si no existe)..."
$HS users create "$USER_NS" 2>/dev/null || true

echo ">> Generando API key (para el orquestador / OpenClaw bot)..."
API_KEY="$($HS apikeys create --expiration 9000d | tail -n1)"

echo ">> Generando pre-auth key reutilizable (para los instaladores)..."
AUTH_KEY="$($HS preauthkeys create --user "$USER_NS" --reusable --expiration 9000d | tail -n1)"

RUSTDESK_PUB=""
if [ -f ./rustdesk/data/id_ed25519.pub ]; then
  RUSTDESK_PUB="$(cat ./rustdesk/data/id_ed25519.pub)"
fi

cat <<EOF

=====================================================================
 LISTO. Guardá estos valores (el orquestador los necesita):
=====================================================================
 HEADSCALE_URL      = https://<tu-FLEET_DOMAIN>
 HEADSCALE_API_KEY  = $API_KEY
 FLEET_AUTHKEY      = $AUTH_KEY
 RUSTDESK_HOST      = <tu-FLEET_DOMAIN>
 RUSTDESK_PUBKEY    = ${RUSTDESK_PUB:-'(aún no generada; reiniciá hbbs y volvé a correr)'}
=====================================================================
 Cargalos en el orquestador con:
   python fleet.py config set-server
=====================================================================
EOF
