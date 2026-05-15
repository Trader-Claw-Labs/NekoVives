#!/usr/bin/env python3
"""
Script standalone para diagnosticar autenticación con Polymarket CLOB API.
Prueba L2 (api_key/secret/passphrase) y opcionalmente L1 (private_key → EIP-712).
"""

import base64
import hashlib
import hmac
import json
import time
from datetime import datetime, timezone
from urllib.parse import urlencode

import requests

# ── Configuración ───────────────────────────────────────────────────────────
# Puedes editar estos valores directamente o dejarlos vacíos para que el
# script te pregunte.

API_KEY = ""
SECRET = ""
PASSPHRASE = ""
WALLET_ADDRESS = ""
PRIVATE_KEY = ""  # Opcional — solo para probar L1 auth

CLOB_BASE = "https://clob.polymarket.com"

# ── Helpers ─────────────────────────────────────────────────────────────────

def b64urlsafe(b: bytes) -> str:
    return base64.b64encode(b).decode().replace("+", "-").replace("/", "_")

def l2_headers(api_key: str, secret: str, passphrase: str, wallet: str, method: str, path: str, body: str = "", decode_strategy: str = "base64") -> dict:
    """Genera headers L2 exactamente como lo hace el crate Rust."""
    ts = str(int(time.time()))
    message = f"{ts}{method}{path}{body}"

    if decode_strategy == "base64":
        # Normalizar base64url → standard base64
        normalized = secret.replace("-", "+").replace("_", "/")
        try:
            secret_bytes = base64.b64decode(normalized)
            if len(secret_bytes) >= 16:
                pass  # OK
            else:
                secret_bytes = secret.encode()
        except Exception:
            secret_bytes = secret.encode()
    elif decode_strategy == "hex":
        s = secret[2:] if secret.startswith("0x") else secret
        try:
            secret_bytes = bytes.fromhex(s)
            if len(secret_bytes) >= 16:
                pass
            else:
                secret_bytes = secret.encode()
        except Exception:
            secret_bytes = secret.encode()
    else:  # raw
        secret_bytes = secret.encode()

    sig = hmac.new(secret_bytes, message.encode(), hashlib.sha256).digest()
    signature = b64urlsafe(sig)

    return {
        "POLY_ADDRESS": wallet,
        "POLY_API_KEY": api_key,
        "POLY_PASSPHRASE": passphrase,
        "POLY_SIGNATURE": signature,
        "POLY_TIMESTAMP": ts,
    }

def probe(path: str, headers: dict, method: str = "GET", body: str = None) -> tuple:
    """Hace un request y retorna (status, body_text)."""
    url = f"{CLOB_BASE}{path}"
    try:
        if method == "GET":
            r = requests.get(url, headers=headers, timeout=15)
        else:
            r = requests.post(url, headers={**headers, "Content-Type": "application/json"}, data=body, timeout=15)
        return r.status_code, r.text
    except Exception as e:
        return 0, f"network error: {e}"

def test_l2(api_key: str, secret: str, passphrase: str, wallet: str):
    """Prueba los 3 encodings de secret."""
    print(f"\n{'='*60}")
    print("L2 AUTH TEST")
    print(f"{'='*60}")
    print(f"api_key : {api_key[:8]}…{api_key[-4:]} (len {len(api_key)})")
    print(f"secret  : {secret[:8]}…{secret[-4:]} (len {len(secret)})")
    print(f"pass    : {passphrase[:8]}…{passphrase[-4:]} (len {len(passphrase)})")
    print(f"wallet  : {wallet}")
    print()

    strategies = [("Base64", "base64"), ("Raw", "raw"), ("Hex", "hex")]
    any_ok = False

    for name, strat in strategies:
        headers = l2_headers(api_key, secret, passphrase, wallet, "GET", "/auth/api-keys", "", strat)
        status, body = probe("/auth/api-keys", headers)

        print(f"  [{name}] GET /auth/api-keys → HTTP {status}")

        if status == 401 or status == 403:
            # Fallback: POST /order con dummy body
            dummy_body = '{"order":{"tokenID":"0","price":"0.5","size":"1","side":"BUY","type":"GTC"},"owner":""}'
            headers_post = l2_headers(api_key, secret, passphrase, wallet, "POST", "/order", dummy_body, strat)
            status2, body2 = probe("/order", headers_post, "POST", dummy_body)
            print(f"         POST /order (dummy) → HTTP {status2}")
            if status2 != 401 and status2 != 403 and status2 != 0:
                print(f"         ✓ Auth PASSED (order rejected for other reason)")
                print(f"         Response: {body2[:120]}")
                any_ok = True
                continue

        if 200 <= status < 300:
            print(f"         ✓ Auth PASSED")
            print(f"         Response: {body[:120]}")
            any_ok = True
        elif status == 0:
            print(f"         ✗ Network error")
        else:
            print(f"         ✗ Auth FAILED")
            print(f"         Response: {body[:120]}")

    if not any_ok:
        print("\n  ⚠ Todos los encodings fallaron con 401/403.")
        print("  Eso significa que el trío api_key/secret/passphrase NO pertenece a la wallet indicada.")
        print("  Posibles causas:")
        print("    1. La wallet_address no coincide con la wallet que generó estas credenciales.")
        print("    2. Las credenciales fueron revocadas/regeneradas en Polymarket.")
        print("    3. El 'secret' o 'passphrase' se copiaron incorrectamente.")

    return any_ok


def test_l1(private_key: str):
    """Prueba L1 auth (EIP-712) para obtener credenciales frescas."""
    print(f"\n{'='*60}")
    print("L1 AUTH TEST (EIP-712)")
    print(f"{'='*60}")

    try:
        from eth_account import Account
        from eth_account.messages import encode_defunct
    except ImportError:
        print("  ⚠ Instala eth-account:  pip3 install eth-account")
        return None

    try:
        account = Account.from_key(private_key)
        address = account.address
        print(f"Private key derives address: {address}")
    except Exception as e:
        print(f"  ✗ Invalid private key: {e}")
        return None

    # Construir EIP-712 digest exactamente como el crate Rust
    # Este es un cálculo simplificado — en producción el crate hace el digest completo.
    # Para este test, usamos directamente el endpoint /auth/api-key de Polymarket
    # con los headers L1 que espera.

    timestamp = str(int(time.time()))
    nonce = "0"

    # El mensaje que firma Polymarket L1 es EIP-712 struct ClobAuth
    # Lo más práctico es usar eth_signTypedData o firmar el digest directamente.
    # Como fallback, probaremos el endpoint directamente con una firma estándar.

    # NOTA: La firma L1 real de Polymarket usa EIP-712 con un domain específico.
    # Para este script diagnóstico, intentaremos primero con una firma personal
    # y luego mostraremos el resultado.

    # Mensaje canonical para Polymarket L1 (versión simplificada que algunas
    # implementaciones aceptan como fallback):
    msg = f"This message attests that I control the given wallet\nTimestamp: {timestamp}\nNonce: {nonce}"
    signable = encode_defunct(text=msg)
    signed = account.sign_message(signable)
    signature = signed.signature.hex()

    headers = {
        "POLY_ADDRESS": address,
        "POLY_SIGNATURE": signature,
        "POLY_TIMESTAMP": timestamp,
        "POLY_NONCE": nonce,
    }

    url = f"{CLOB_BASE}/auth/api-key"
    print(f"\n  POST {url}")
    print(f"  Headers: {json.dumps(headers, indent=4)}")

    try:
        r = requests.post(url, headers=headers, timeout=15)
        print(f"\n  → HTTP {r.status_code}")
        print(f"  → Body: {r.text[:300]}")

        if r.status_code == 200:
            data = r.json()
            print(f"\n  ✓ L1 auth SUCCESS!")
            print(f"  api_key     : {data.get('apiKey', 'N/A')}")
            print(f"  secret      : {data.get('secret', 'N/A')}")
            print(f"  passphrase  : {data.get('passphrase', 'N/A')}")
            print(f"\n  Copia estos valores a tu config y prueba de nuevo.")
            return data
        else:
            print(f"\n  ✗ L1 auth FAILED — la firma no es válida o la wallet no está registrada.")
            return None
    except Exception as e:
        print(f"  ✗ Request failed: {e}")
        return None


# ── Main ────────────────────────────────────────────────────────────────────

def main():
    global API_KEY, SECRET, PASSPHRASE, WALLET_ADDRESS, PRIVATE_KEY

    print("Polymarket CLOB Auth Diagnostic Tool")
    print("=" * 60)

    if not API_KEY:
        API_KEY = input("API Key: ").strip()
    if not SECRET:
        SECRET = input("Secret: ").strip()
    if not PASSPHRASE:
        PASSPHRASE = input("Passphrase: ").strip()
    if not WALLET_ADDRESS:
        WALLET_ADDRESS = input("Wallet Address (0x...): ").strip()

    if not all([API_KEY, SECRET, PASSPHRASE, WALLET_ADDRESS]):
        print("\nError: todos los campos son obligatorios.")
        return

    # 1. Probar conectividad pública
    print(f"\n{'─'*60}")
    print("Paso 1: Conectividad pública (sin auth)")
    print(f"{'─'*60}")
    try:
        r = requests.get(f"{CLOB_BASE}/markets?limit=1", timeout=10)
        print(f"GET /markets?limit=1 → HTTP {r.status_code}")
        if r.status_code != 200:
            print("⚠ El endpoint público no responde — puede haber un problema de red.")
    except Exception as e:
        print(f"✗ Error de conectividad: {e}")
        return

    # 2. Probar L2 auth
    l2_ok = test_l2(API_KEY, SECRET, PASSPHRASE, WALLET_ADDRESS)

    # 3. Opcional: probar L1 auth
    if not l2_ok:
        print(f"\n{'─'*60}")
        print("Paso 3: Probar L1 auth (regenerar credenciales)")
        print(f"{'─'*60}")
        pk_input = PRIVATE_KEY or input("\nPrivate key (0x...) para probar L1 auth [Enter para omitir]: ").strip()
        if pk_input:
            test_l1(pk_input)


if __name__ == "__main__":
    main()
