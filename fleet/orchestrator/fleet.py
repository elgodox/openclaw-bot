#!/usr/bin/env python3
"""
OpenClaw Fleet — orquestador.

Lo que tu OpenClaw bot invoca para gestionar la flota de máquinas tuyas:

  python fleet.py config set-server      # guardar URL/keys del servidor
  python fleet.py list                    # inventario (lee la API de Headscale)
  python fleet.py enroll-cmd [linux|win]  # imprime el one-liner de instalación
  python fleet.py run <host> <cmd...>     # ejecuta un comando por SSH
  python fleet.py run <host> --sudo <cmd> # idem, con admin/sudo (pass del vault)
  python fleet.py apply <host> <play.yml> # aplica un playbook de Ansible
  python fleet.py vault set-admin <host>  # guarda la contraseña admin de un host
  python fleet.py vault list              # lista qué credenciales hay guardadas

Diseñado para que la salida sea clara y fácil de parsear por el bot.
Pensado para administrar máquinas de tu propiedad: cada una se enroló con TU
pre-auth key y TU clave SSH.
"""
from __future__ import annotations
import argparse
import getpass
import json
import os
import ssl
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONFIG_PATH = ROOT / "orchestrator" / "config.json"

try:
    from vault import Vault  # noqa: E402
except ModuleNotFoundError as e:  # p. ej. falta 'cryptography'
    sys.exit(f"Falta la dependencia de Python '{e.name}'. "
             f"Instalá con: python -m pip install -r requirements.txt")


# ----------------------------------------------------------------------------- config
def load_config() -> dict:
    if CONFIG_PATH.exists():
        return json.loads(CONFIG_PATH.read_text())
    return {}


def save_config(cfg: dict) -> None:
    CONFIG_PATH.write_text(json.dumps(cfg, indent=2))
    try:
        os.chmod(CONFIG_PATH, 0o600)
    except OSError:
        pass


def cmd_config_set_server(_: argparse.Namespace) -> None:
    cfg = load_config()
    print("Dejá vacío para conservar el valor actual.\n")
    def ask(label, key, secret=False):
        cur = cfg.get(key, "")
        shown = ("******" if secret and cur else cur)
        val = (getpass.getpass if secret else input)(f"{label} [{shown}]: ").strip()
        if val:
            cfg[key] = val
    ask("Headscale URL (https://fleet.tudominio.com)", "headscale_url")
    ask("Headscale API key", "headscale_api_key", secret=True)
    ask("Pre-auth key (FLEET_AUTHKEY)", "fleet_authkey", secret=True)
    ask("Ruta a tu clave SSH privada (ej C:\\Users\\vos\\.ssh\\id_ed25519)", "ssh_key")
    ask("Tu clave SSH pública (ssh-ed25519 AAAA... bot@control)", "ssh_pubkey")
    ask("Usuario de automatización", "auto_user")
    ask("RustDesk host", "rustdesk_host")
    ask("RustDesk pubkey", "rustdesk_key")
    ask("Base de instaladores (URL donde servís install.sh/ps1)", "install_base")
    cfg.setdefault("auto_user", "fleet")
    save_config(cfg)
    print("\nGuardado en", CONFIG_PATH)


def cmd_config_show(_: argparse.Namespace) -> None:
    cfg = load_config()
    redacted = {k: ("******" if "key" in k or "authkey" in k else v) for k, v in cfg.items()}
    print(json.dumps(redacted, indent=2, ensure_ascii=False))


# ----------------------------------------------------------------------------- inventario
def _headscale(path: str) -> dict:
    cfg = load_config()
    base = cfg.get("headscale_url", "").strip().rstrip("/")
    if not base or not cfg.get("headscale_api_key"):
        sys.exit("Falta configurar el servidor de la flota (URL y API key de Headscale). "
                 "Corré en una terminal: python fleet.py config set-server")
    url = base + path
    req = urllib.request.Request(url, headers={"Authorization": f"Bearer {cfg.get('headscale_api_key','')}"})
    ctx = ssl.create_default_context()
    try:
        with urllib.request.urlopen(req, context=ctx, timeout=15) as r:
            return json.loads(r.read().decode())
    except urllib.error.HTTPError as e:
        hint = " (¿API key vencida o incorrecta?)" if e.code in (401, 403) else ""
        sys.exit(f"Headscale respondió {e.code}{hint}.")
    except (urllib.error.URLError, OSError, ValueError) as e:
        sys.exit(f"No pude conectar con Headscale en {base}: {getattr(e, 'reason', e)}")


def inventory() -> list[dict]:
    """Devuelve la lista de máquinas de la flota desde Headscale."""
    data = _headscale("/api/v1/node")
    out = []
    for n in data.get("nodes", []):
        out.append({
            "hostname": n.get("givenName") or n.get("name"),
            "mesh_ip": (n.get("ipAddresses") or ["?"])[0],
            "os": (n.get("hostinfo") or {}).get("os", "?"),
            "online": n.get("online", False),
            "last_seen": n.get("lastSeen", ""),
        })
    return out


def cmd_list(args: argparse.Namespace) -> None:
    machines = inventory()
    if args.json:
        print(json.dumps(machines, indent=2, ensure_ascii=False))
        return
    if not machines:
        print("(sin máquinas enroladas todavía)")
        return
    print(f"{'HOSTNAME':24} {'IP MALLA':16} {'SO':10} ESTADO")
    print("-" * 64)
    for m in machines:
        estado = "● online" if m["online"] else "○ offline"
        print(f"{m['hostname']:24} {m['mesh_ip']:16} {m['os']:10} {estado}")


def resolve_host(name: str) -> str:
    """Traduce un hostname de la flota a su IP de malla."""
    for m in inventory():
        if m["hostname"] == name or m["mesh_ip"] == name:
            return m["mesh_ip"]
    # Si no lo encuentra en el inventario, lo usa tal cual (IP/host directo).
    return name


# ----------------------------------------------------------------------------- enroll-cmd
def cmd_enroll_cmd(args: argparse.Namespace) -> None:
    cfg = load_config()
    base = cfg.get("install_base", "https://tu-servidor").rstrip("/")
    server = cfg.get("headscale_url", "https://fleet.tudominio.com")
    authkey = cfg.get("fleet_authkey", "<FLEET_AUTHKEY>")
    pubkey = cfg.get("ssh_pubkey", "<TU_SSH_PUBKEY>")
    rd_host = cfg.get("rustdesk_host", "")
    rd_key = cfg.get("rustdesk_key", "")
    rd_linux = f' \\\n      --rustdesk-host {rd_host} --rustdesk-key {rd_key}' if rd_host else ""
    rd_win = f' `\n     -RustdeskHost {rd_host} -RustdeskKey {rd_key}' if rd_host else ""

    if args.platform == "win":
        print(f"""# Windows — PowerShell COMO ADMINISTRADOR:
irm {base}/install.ps1 -OutFile install.ps1; .\\install.ps1 `
     -Server {server} `
     -AuthKey {authkey} `
     -SshPubKey "{pubkey}"{rd_win}""")
    else:
        print(f"""# Linux — como root:
curl -fsSL {base}/install.sh | sudo bash -s -- \\
      --server {server} \\
      --authkey {authkey} \\
      --ssh-pubkey "{pubkey}"{rd_linux}""")


# ----------------------------------------------------------------------------- run (SSH)
def cmd_run(args: argparse.Namespace) -> None:
    cfg = load_config()
    ip = resolve_host(args.host)
    user = cfg.get("auto_user", "fleet")
    key = cfg.get("ssh_key", "")
    # argparse.REMAINDER se queda con todo lo que sigue al host, incluido un
    # "--sudo" escrito después (como en la ayuda y en tool.json).
    command = list(args.command)
    while command and command[0] in ("--sudo", "--"):
        if command.pop(0) == "--sudo":
            args.sudo = True
    if not command:
        sys.exit("Falta el comando a ejecutar.")
    remote = " ".join(command)

    ssh = ["ssh", "-o", "StrictHostKeyChecking=accept-new", "-o", "BatchMode=yes"]
    if key:
        ssh += ["-i", key]
    ssh += [f"{user}@{ip}"]

    stdin_data = None
    if args.sudo:
        pw = Vault().get_become(args.host)
        if not pw:
            sys.exit(f"No hay contraseña admin guardada para '{args.host}'. "
                     f"Cargala con: python fleet.py vault set-admin {args.host}")
        # sudo -S lee la pass de stdin; -p '' sin prompt visible.
        remote = f"sudo -S -p '' bash -c {json.dumps(remote)}"
        stdin_data = (pw + "\n").encode()

    ssh += [remote]
    proc = subprocess.run(ssh, input=stdin_data)
    sys.exit(proc.returncode)


# ----------------------------------------------------------------------------- apply (Ansible)
def cmd_apply(args: argparse.Namespace) -> None:
    cfg = load_config()
    ip = resolve_host(args.host)
    user = cfg.get("auto_user", "fleet")
    key = cfg.get("ssh_key", "")

    if not _have("ansible-playbook"):
        sys.exit("Ansible no está instalado o el nodo de control es Windows.\n"
                 "El control de Ansible corre en Linux/WSL. En Windows usá 'run' por SSH,\n"
                 "o instalá WSL + ansible para usar 'apply'.")

    inv = f"{ip},"  # inventario de un solo host (la coma es intencional)
    env = os.environ.copy()
    cmd = ["ansible-playbook", "-i", inv,
           "-e", f"ansible_user={user}",
           "-u", user]
    if key:
        cmd += ["--private-key", key]
    if args.sudo:
        pw = Vault().get_become(args.host)
        if not pw:
            sys.exit(f"No hay contraseña admin para '{args.host}'. "
                     f"Cargala con: python fleet.py vault set-admin {args.host}")
        env["ANSIBLE_BECOME_PASS"] = pw
        cmd += ["--become"]
    cmd += [args.playbook]
    sys.exit(subprocess.run(cmd, env=env).returncode)


def _have(binary: str) -> bool:
    from shutil import which
    return which(binary) is not None


# ----------------------------------------------------------------------------- vault
def cmd_vault_set_admin(args: argparse.Namespace) -> None:
    # Permite pasarla por entorno (para la GUI) o preguntarla por teclado.
    pw = os.environ.get("FLEET_ADMIN_PASSWORD") or getpass.getpass(f"Contraseña admin/sudo de '{args.host}': ")
    Vault().set_become(args.host, pw)
    print(f"Guardada (cifrada) para {args.host}.")


def cmd_vault_list(_: argparse.Namespace) -> None:
    for k in Vault().keys():
        print(k)


def cmd_vault_delete(args: argparse.Namespace) -> None:
    Vault().delete(f"{args.host}:become")
    print(f"Borrada la credencial de {args.host}.")


# ----------------------------------------------------------------------------- CLI
def main() -> None:
    p = argparse.ArgumentParser(prog="fleet", description="Orquestador de la flota OpenClaw")
    sub = p.add_subparsers(dest="cmd", required=True)

    pc = sub.add_parser("config", help="configuración del servidor").add_subparsers(dest="sub", required=True)
    pc.add_parser("set-server", help="cargar URL/keys del servidor").set_defaults(func=cmd_config_set_server)
    pc.add_parser("show", help="ver configuración (secretos ocultos)").set_defaults(func=cmd_config_show)

    pl = sub.add_parser("list", help="inventario de máquinas")
    pl.add_argument("--json", action="store_true")
    pl.set_defaults(func=cmd_list)

    pe = sub.add_parser("enroll-cmd", help="imprime el one-liner de instalación")
    pe.add_argument("platform", nargs="?", default="linux", choices=["linux", "win"])
    pe.set_defaults(func=cmd_enroll_cmd)

    pr = sub.add_parser("run", help="ejecutar un comando por SSH")
    pr.add_argument("host")
    pr.add_argument("--sudo", action="store_true", help="correr con admin (pass del vault)")
    pr.add_argument("command", nargs=argparse.REMAINDER)
    pr.set_defaults(func=cmd_run)

    pa = sub.add_parser("apply", help="aplicar un playbook de Ansible")
    pa.add_argument("host")
    pa.add_argument("playbook")
    pa.add_argument("--sudo", action="store_true")
    pa.set_defaults(func=cmd_apply)

    pv = sub.add_parser("vault", help="credenciales admin cifradas").add_subparsers(dest="sub", required=True)
    sa = pv.add_parser("set-admin"); sa.add_argument("host"); sa.set_defaults(func=cmd_vault_set_admin)
    pv.add_parser("list").set_defaults(func=cmd_vault_list)
    vd = pv.add_parser("delete"); vd.add_argument("host"); vd.set_defaults(func=cmd_vault_delete)

    args = p.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
