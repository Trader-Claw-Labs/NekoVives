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
    onMessage,
    onOpen,
    onClose,
    onError,
    autoReconnect = true,
    reconnectDelay = 3000,
  } = options

  const wsRef = useRef<WebSocket | null>(null)
  const [connected, setConnected] = useState(false)
  const reconnectTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const mountedRef = useRef(true)

  const connect = useCallback(() => {
    if (!mountedRef.current) return

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const host = window.location.host
    const token = getAuthToken()
    const query = token ? `?token=${encodeURIComponent(token)}` : ''
    const url = `${protocol}//${host}${path}${query}`

    const ws = new WebSocket(url)
    wsRef.current = ws

    ws.onopen = () => {
      if (!mountedRef.current) { ws.close(); return }
      setConnected(true)
      onOpen?.()
    }

    ws.onmessage = (ev) => {
      if (!mountedRef.current) return
      try {
        const msg = JSON.parse(ev.data) as WsMessage
        onMessage?.(msg)
      } catch {
        // ignore malformed frames
      }
    }

    ws.onclose = () => {
      setConnected(false)
      onClose?.()
      if (autoReconnect && mountedRef.current) {
        reconnectTimer.current = setTimeout(connect, reconnectDelay)
      }
    }

    ws.onerror = (err) => {
      onError?.(err)
    }
  }, [path, onMessage, onOpen, onClose, onError, autoReconnect, reconnectDelay])

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
