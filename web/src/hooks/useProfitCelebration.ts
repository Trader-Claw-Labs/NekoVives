import { useState, useCallback, useEffect } from 'react'
import confetti from 'canvas-confetti'

const CELEBRATION_SETTINGS_KEY = 'trader-claw:celebration-settings'

export interface CelebrationSettings {
  enabled: boolean
  sound: boolean
}

export function useProfitCelebration() {
  const [settings, setSettingsState] = useState<CelebrationSettings>(() => {
    try {
      const stored = localStorage.getItem(CELEBRATION_SETTINGS_KEY)
      if (stored) {
        return JSON.parse(stored) as CelebrationSettings
      }
    } catch (e) {
      // Ignore parse errors
    }
    return { enabled: true, sound: true }
  })

  // Keep localStorage in sync
  const setSettings = useCallback((newSettings: CelebrationSettings | ((prev: CelebrationSettings) => CelebrationSettings)) => {
    setSettingsState(prev => {
      const updated = typeof newSettings === 'function' ? newSettings(prev) : newSettings
      localStorage.setItem(CELEBRATION_SETTINGS_KEY, JSON.stringify(updated))
      return updated
    })
  }, [])

  const celebrate = useCallback(() => {
    if (!settings.enabled) return

    // Trigger visual confetti
    confetti({
      particleCount: 100,
      spread: 70,
      origin: { y: 0.6 },
      colors: ['#4ade80', '#10b981', '#fbbf24'] // Greenish & gold
    })

    // Trigger audio if enabled
    if (settings.sound) {
      try {
        const audio = new Audio('/profit-sound.mp3')
        audio.volume = 0.3
        audio.play().catch(e => {
          console.warn('Playback prevented by browser policy until user interaction:', e)
        })
      } catch (e) {
        console.error('Failed to play profit sound', e)
      }
    }
  }, [settings])

  return { celebrate, settings, setSettings }
}