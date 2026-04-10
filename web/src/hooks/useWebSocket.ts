import { useEffect, useRef, useCallback, useState } from 'react'
import { getAuthToken } from './useApi'

export interface WsMessage {
  type: string
  content?: string
  session_id?: string
  [key: string]: unknown
}

export interface UseWebSocketOptions {
  onMessage?: (msg: WsMessage) => void
  onOpen?: () => void
  onClose?: () => void
  onError?: (err: Event) => void
  autoReconnect?: boolean
  reconnectDelay?: number
}

export function useWebSocket(path: string, options: UseWebSocketOptions = {}) {
  const {
    autoReconnect = true,
    reconnectDelay = 3000,
  } = options

  // Store callbacks in refs so they never invalidate the connect memoization.
  // Without this, an inline `onMessage` arrow in the parent component would
  // create a new function reference on every render, causing the useEffect
  // dep on `connect` to fire, tearing down and recreating the WS on each
  // incoming message — resetting session history to length 1 every time.
  const onMessageRef = useRef(options.onMessage)
  const onOpenRef    = useRef(options.onOpen)
  const onCloseRef   = useRef(options.onClose)
  const onErrorRef   = useRef(options.onError)

  // Keep refs current without triggering re-memoization of `connect`.
  useEffect(() => { onMessageRef.current = options.onMessage })
  useEffect(() => { onOpenRef.current    = options.onOpen    })
  useEffect(() => { onCloseRef.current   = options.onClose   })
  useEffect(() => { onErrorRef.current   = options.onError   })

  const wsRef           = useRef<WebSocket | null>(null)
  const [connected, setConnected] = useState(false)
  const reconnectTimer  = useRef<ReturnType<typeof setTimeout> | null>(null)
  const mountedRef      = useRef(true)

  // `connect` is now stable across renders because it only depends on
  // primitive values (path, autoReconnect, reconnectDelay), not callbacks.
  const connect = useCallback(() => {
    if (!mountedRef.current) return

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const host  = window.location.host
    const token = getAuthToken()
    const query = token ? `?token=${encodeURIComponent(token)}` : ''
    const url   = `${protocol}//${host}${path}${query}`

    const ws = new WebSocket(url)
    wsRef.current = ws

    ws.onopen = () => {
      if (!mountedRef.current) { ws.close(); return }
      setConnected(true)
      onOpenRef.current?.()
    }

    ws.onmessage = (ev) => {
      if (!mountedRef.current) return
      try {
        const msg = JSON.parse(ev.data) as WsMessage
        onMessageRef.current?.(msg)
      } catch {
        // ignore malformed frames
      }
    }

    ws.onclose = () => {
      setConnected(false)
      onCloseRef.current?.()
      if (autoReconnect && mountedRef.current) {
        reconnectTimer.current = setTimeout(connect, reconnectDelay)
      }
    }

    ws.onerror = (err) => {
      onErrorRef.current?.(err)
    }
  }, [path, autoReconnect, reconnectDelay]) // callbacks intentionally excluded

  useEffect(() => {
    mountedRef.current = true
    connect()
    return () => {
      mountedRef.current = false
      if (reconnectTimer.current) clearTimeout(reconnectTimer.current)
      wsRef.current?.close()
    }
  }, [connect])

  const send = useCallback((msg: WsMessage) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(msg))
    }
  }, [])

  return { connected, send }
}
