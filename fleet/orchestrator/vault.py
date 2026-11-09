"""
Vault de credenciales de la flota — cifrado local.

Guarda contraseñas de administrador/sudo de TUS máquinas en un archivo cifrado
(vault.enc). Se abre con una passphrase maestra que vos tenés; nunca se guarda
en texto plano. El orquestador pide la credencial al vault sólo en el momento
de usarla (p. ej. para pasar la become-password a Ansible/sudo).

Cifrado: Fernet (AES-128-CBC + HMAC) con clave derivada por scrypt de la
passphrase maestra. Sin dependencias externas salvo `cryptography`.

La passphrase maestra se toma de la variable de entorno FLEET_VAULT_PASSPHRASE
si está definida (para que el bot la use sin interacción); si no, se pregunta.
"""
from __future__ import annotations
import base64
import getpass
import json
import os
from pathlib import Path

from cryptography.fernet import Fernet, InvalidToken
from cryptography.hazmat.primitives.kdf.scrypt import Scrypt

VAULT_PATH = Path(__file__).resolve().parent.parent / "vault" / "vault.enc"
SALT_PATH = Path(__file__).resolve().parent.parent / "vault" / "vault.salt"


def _passphrase() -> str:
    pw = os.environ.get("FLEET_VAULT_PASSPHRASE")
    if pw:
        return pw
    return getpass.getpass("Passphrase maestra del vault: ")


def _key(passphrase: str) -> bytes:
    SALT_PATH.parent.mkdir(parents=True, exist_ok=True)
    if SALT_PATH.exists():
        salt = SALT_PATH.read_bytes()
    else:
        salt = os.urandom(16)
        SALT_PATH.write_bytes(salt)
    kdf = Scrypt(salt=salt, length=32, n=2**15, r=8, p=1)
    return base64.urlsafe_b64encode(kdf.derive(passphrase.encode()))


def _load(fernet: Fernet) -> dict:
    if not VAULT_PATH.exists():
        return {}
    try:
        return json.loads(fernet.decrypt(VAULT_PATH.read_bytes()).decode())
    except InvalidToken:
        raise SystemExit("Passphrase incorrecta o vault corrupto.")


def _save(fernet: Fernet, data: dict) -> None:
    VAULT_PATH.parent.mkdir(parents=True, exist_ok=True)
    VAULT_PATH.write_bytes(fernet.encrypt(json.dumps(data).encode()))


class Vault:
    """Almacén clave->valor cifrado. Las claves suelen ser 'host:tipo'."""

    def __init__(self, passphrase: str | None = None):
        self.fernet = Fernet(_key(passphrase or _passphrase()))
        self.data = _load(self.fernet)

    def set(self, key: str, value: str) -> None:
        self.data[key] = value
        _save(self.fernet, self.data)

    def get(self, key: str) -> str | None:
        return self.data.get(key)

    def delete(self, key: str) -> None:
        self.data.pop(key, None)
        _save(self.fernet, self.data)

    def keys(self) -> list[str]:
        return sorted(self.data.keys())

    # --- helpers específicos -------------------------------------------------
    def set_become(self, host: str, password: str) -> None:
        """Guarda la contraseña de admin/sudo de un host."""
        self.set(f"{host}:become", password)

    def get_become(self, host: str) -> str | None:
        return self.get(f"{host}:become")
