# OpenClaw Fleet

Gestión de **tu flota de máquinas** (PCs/servidores propios, Linux y Windows) desde tu
OpenClaw bot: enrolás una máquina con **una línea**, aparece en el inventario, y
controlás todo por SSH, Ansible o escritorio gráfico (RustDesk) — sabiendo siempre
en qué máquina estás actuando.

No es un agente de control remoto hecho a mano: compone herramientas open-source
probadas (Headscale, Tailscale, OpenSSH, Ansible, RustDesk) y las orquesta. Es
administración remota de equipos de tu propiedad, como un TeamViewer/MDM self-hosted.

```
                        ┌───────────────────────────────┐
   OpenClaw bot ──────► │  orchestrator/fleet.py        │
   (corre comandos)     │  list · run · apply · vault   │
                        └───────────────┬───────────────┘
                                        │ SSH / Ansible / API
                     malla VPN cifrada  │  (Headscale + Tailscale)
            ┌───────────────┬───────────┴───────┬───────────────┐
         Linux #1        Windows #2          Linux #3        ...  (tu flota)
       (tailscale +    (tailscale +         (tailscale +
        ssh + rustdesk)  ssh + rustdesk)     ssh + rustdesk)
```

## Las 3 capas

1. **Conectividad** — `server/`: Headscale (tu control plane self-hosted) coordina una
   malla VPN. Cada máquina queda alcanzable por nombre/IP sin importar NAT ni firewall.
   El tráfico es P2P cifrado; el servidor sólo coordina.
2. **Control** — por la malla: SSH y Ansible (CLI/configs) y RustDesk (escritorio gráfico).
3. **Inventario + vault** — `orchestrator/`: `fleet.py` lee la lista de máquinas de la
   API de Headscale y guarda las contraseñas de admin cifradas en `vault/`.

---

## 1. Montar el servidor (una vez)

En tu VPS/servidor (con Docker):

```bash
cd server
cp .env.example .env            # completá FLEET_DOMAIN
# editá headscale/config/config.yaml -> server_url con tu dominio
docker compose up -d
bash bootstrap.sh               # crea usuario, API key y pre-auth key
```

`bootstrap.sh` imprime los 5 valores que necesitás: `HEADSCALE_URL`, `HEADSCALE_API_KEY`,
`FLEET_AUTHKEY`, `RUSTDESK_HOST`, `RUSTDESK_PUBKEY`.

> Poné un reverse proxy con TLS (Caddy/Nginx) delante del puerto 8080 y usá `https://`
> en `server_url`. RustDesk usa los puertos 21114-21119 (por eso `network_mode: host`).

Serví también los dos instaladores para el one-liner (cualquier hosting estático, o el
mismo servidor):

```bash
# ejemplo rápido con el propio server
cp install/install.sh install/install.ps1 /var/www/html/
```

## 2. Configurar el orquestador (una vez)

En tu equipo de control (donde corre el bot):

```bash
cd orchestrator
pip install -r requirements.txt
python fleet.py config set-server     # pegás los valores del paso 1 + tu clave SSH
```

Generá tu par de claves SSH si no tenés (la pública va a las máquinas, la privada se queda):

```bash
ssh-keygen -t ed25519 -f ~/.ssh/fleet -C "bot@control"
```

Cargá en `set-server` la **ruta a la privada** (`ssh_key`) y la **pública** (`ssh_pubkey`).

## 3. Enrolar una máquina (lo que repetís por cada PC)

Pedile al bot el comando, o:

```bash
python fleet.py enroll-cmd linux     # imprime el one-liner de Linux
python fleet.py enroll-cmd win       # imprime el one-liner de Windows
```

Copiás ese one-liner y lo corrés en la máquina nueva. Ejemplos:

**Linux** (como root):
```bash
curl -fsSL https://tu-servidor/install.sh | sudo bash -s -- \
    --server https://fleet.tudominio.com --authkey <FLEET_AUTHKEY> \
    --ssh-pubkey "ssh-ed25519 AAAA... bot@control" \
    --rustdesk-host fleet.tudominio.com --rustdesk-key <RUSTDESK_PUBKEY>
```

**Windows** (PowerShell como Administrador):
```powershell
irm https://tu-servidor/install.ps1 -OutFile install.ps1; .\install.ps1 `
  -Server https://fleet.tudominio.com -AuthKey <FLEET_AUTHKEY> `
  -SshPubKey "ssh-ed25519 AAAA... bot@control" `
  -RustdeskHost fleet.tudominio.com -RustdeskKey <RUSTDESK_PUBKEY>
```

El instalador enrola la máquina, deja el acceso SSH con tu clave y (si pasás `--rustdesk-*`)
el escritorio gráfico. Imprime su hostname, IP de malla y, si corresponde, el ID/clave de
RustDesk. A los pocos segundos aparece en el inventario.

## 4. Operar la flota

```bash
python fleet.py list                        # ver todas las máquinas y su estado
python fleet.py run web-01 "uptime"         # comando por SSH
python fleet.py run web-01 --sudo "apt upgrade -y"   # con admin (pass del vault)
python fleet.py apply web-01 ../playbooks/base-setup.yml --sudo   # config con Ansible
```

### Contraseñas de administrador (vault)

Lo mejor es **no** necesitarlas: el instalador deja `sudo` sin password para el usuario de
automatización en Linux, y el usuario es Administrador en Windows. Pero si alguna máquina
puntual pide una contraseña de admin, la cargás una vez y queda cifrada:

```bash
python fleet.py vault set-admin web-01      # la pide y la guarda cifrada
python fleet.py vault list                  # qué credenciales hay
```

El vault (`vault/vault.enc`) se cifra con una **passphrase maestra** tuya (scrypt+Fernet).
Para que el bot la use sin preguntar, exportá `FLEET_VAULT_PASSPHRASE` en su entorno.
Nunca se guarda en texto plano ni se versiona (ver `.gitignore`).

---

## Cómo lo usa el OpenClaw bot

El bot ya sabe ejecutar comandos con aprobación (`tool:build_command_run`). La integración
es simplemente que invoque el orquestador. Ejemplos de intención → comando:

| Le pedís al bot… | El bot corre |
|---|---|
| "¿qué máquinas tengo?" | `python fleet.py list --json` |
| "dame el comando para sumar una PC Linux" | `python fleet.py enroll-cmd linux` |
| "en web-01 reiniciá nginx" | `python fleet.py run web-01 --sudo "systemctl restart nginx"` |
| "aplicá la config base a web-01" | `python fleet.py apply web-01 ../playbooks/base-setup.yml --sudo` |

El `--json` de `list` está pensado para que el bot lo parsee y te diga en qué máquina está
operando. Ver `orchestrator/tool.json` para un manifiesto de las acciones.

## Seguridad (importante)

- Todo esto administra **máquinas de tu propiedad**, enroladas con **tu** pre-auth key y **tu**
  clave SSH. No instales esto en equipos que no administrás.
- La malla cifra el tráfico extremo a extremo; el servidor de control no ve el contenido.
- Protegé el servidor de Headscale con TLS y no expongas la API ni las métricas a internet.
- Rotá la pre-auth key y la API key periódicamente (`headscale preauthkeys` / `apikeys`).
- El vault y `config.json` tienen permisos 600 y están en `.gitignore`: no los subas a git.

## Notas / límites

- `apply` (Ansible) requiere un nodo de control Linux o WSL. En Windows, usá `run` por SSH
  (que funciona nativo con OpenSSH) o instalá WSL + ansible.
- Para 100% self-hosted sin relays de terceros, montá tu propio DERP y referencialo en
  `headscale/config/config.yaml`.
