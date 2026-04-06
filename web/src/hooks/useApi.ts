const BASE = ''

let authToken: string | null = localStorage.getItem('auth_token')

export function setAuthToken(token: string) {
  authToken = token
  localStorage.setItem('auth_token', token)
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
