const BASE = ''

let authToken: string | null = localStorage.getItem('auth_token')

// Called by App when a 401 is received — shows the pairing modal
let onAuthError: (() => void) | null = null
export function setAuthErrorCallback(cb: () => void) {
  onAuthError = cb
}

export function setAuthToken(token: string) {
  authToken = token
  localStorage.setItem('auth_token', token)
}

export function clearAuthToken() {
  authToken = null
  localStorage.removeItem('auth_token')
}

export function getAuthToken(): string | null {
  return authToken
}

function buildHeaders(extra?: HeadersInit): Headers {
  const h = new Headers(extra)
  h.set('Content-Type', 'application/json')
  if (authToken) {
    h.set('Authorization', `Bearer ${authToken}`)
  }
  return h
}

export async function apiFetch<T = unknown>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${BASE}${path}`, {
    ...init,
    headers: buildHeaders(init?.headers),
  })
  if (!res.ok) {
    // Token rejected by server — clear it and trigger re-pairing
    if (res.status === 401) {
      clearAuthToken()
      onAuthError?.()
    }
    const text = await res.text().catch(() => res.statusText)
    throw new Error(`${res.status}: ${text}`)
  }
  return res.json() as Promise<T>
}

export async function apiPost<T>(path: string, body: unknown): Promise<T> {
  return apiFetch<T>(path, {
    method: 'POST',
    body: JSON.stringify(body),
  })
}

export async function apiPut<T>(path: string, body: unknown): Promise<T> {
  return apiFetch<T>(path, {
    method: 'PUT',
    body: JSON.stringify(body),
  })
}

export async function apiDelete<T>(path: string): Promise<T> {
  return apiFetch<T>(path, { method: 'DELETE' })
}

export async function apiPatch<T>(path: string, body: unknown): Promise<T> {
  return apiFetch<T>(path, {
    method: 'PATCH',
    body: JSON.stringify(body),
  })
}
