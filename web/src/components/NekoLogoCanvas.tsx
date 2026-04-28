import { useEffect, useRef } from 'react'

interface Props {
  size?: number
}

export default function NekoLogoCanvas({ size = 280 }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const cv = canvasRef.current
    if (!cv) return
    const ctx = cv.getContext('2d')
    if (!ctx) return

    const C = {
      body: '#f0efe8', outline: '#111927', red: '#ff2244',
      gold: '#ffd700', goldDk: '#b8860b', cyan: '#00ccff',
      green: '#00ff88', pink: '#f9a8b8', chartBg: '#08081a',
    }

    function px(x: number, y: number, w: number, h: number, fill: string) {
      ctx!.fillStyle = fill
      ctx!.fillRect(x, y, w, h)
    }
    function rr(x: number, y: number, w: number, h: number, r: number, fill: string, stroke: string | null, sw = 3) {
      ctx!.beginPath();(ctx as any).roundRect(x, y, w, h, r)
      ctx!.fillStyle = fill; ctx!.fill()
      if (stroke) {
        ctx!.beginPath();(ctx as any).roundRect(x + sw / 2, y + sw / 2, w - sw, h - sw, r)
        ctx!.strokeStyle = stroke; ctx!.lineWidth = sw; ctx!.stroke()
      }
    }
    function circ(x: number, y: number, r: number, fill: string, stroke: string | null, sw = 3) {
      ctx!.beginPath(); ctx!.arc(x, y, r, 0, Math.PI * 2)
      ctx!.fillStyle = fill; ctx!.fill()
      if (stroke) { ctx!.strokeStyle = stroke; ctx!.lineWidth = sw; ctx!.stroke() }
    }
    function ln(x1: number, y1: number, x2: number, y2: number, col: string, w = 2) {
      ctx!.beginPath(); ctx!.moveTo(x1, y1); ctx!.lineTo(x2, y2)
      ctx!.strokeStyle = col; ctx!.lineWidth = w; ctx!.lineCap = 'round'; ctx!.stroke()
    }
    function poly(pts: number[][], fill: string, stroke: string | null, sw = 3) {
      ctx!.beginPath(); ctx!.moveTo(pts[0][0], pts[0][1])
      for (let i = 1; i < pts.length; i++) ctx!.lineTo(pts[i][0], pts[i][1])
      ctx!.closePath(); ctx!.fillStyle = fill; ctx!.fill()
      if (stroke) { ctx!.strokeStyle = stroke; ctx!.lineWidth = sw; ctx!.stroke() }
    }
    function ell(x: number, y: number, rx: number, ry: number, fill: string, stroke: string | null, sw = 2) {
      ctx!.beginPath(); ctx!.ellipse(x, y, rx, ry, 0, 0, Math.PI * 2)
      ctx!.fillStyle = fill; ctx!.fill()
      if (stroke) { ctx!.strokeStyle = stroke; ctx!.lineWidth = sw; ctx!.stroke() }
    }

    let pawAngle = 0, bellAngle = 0, chartProgress = 0, t = 0
    let eyePhase = 0, eyeTimer = 0
    const EYE_DUR = [220, 60]

    const BOLTS = Array.from({ length: 14 }, (_, i) => ({
      angle: (i / 14) * Math.PI * 2 + Math.random() * 0.4,
      len: 14 + Math.random() * 28,
      life: Math.random(),
      spd: 0.013 + Math.random() * 0.022,
    }))

    function drawBolts() {
      BOLTS.forEach(b => {
        b.life += b.spd
        if (b.life > 1) { b.life = 0; b.angle = Math.random() * Math.PI * 2; b.len = 14 + Math.random() * 28 }
        const a = Math.sin(b.life * Math.PI)
        if (a < 0.06) return
        const sx = 148 + Math.cos(b.angle) * 128, sy = 172 + Math.sin(b.angle) * 128
        const ex = sx + Math.cos(b.angle) * b.len, ey = sy + Math.sin(b.angle) * b.len
        const mx_ = sx + (ex - sx) * 0.45 + (Math.random() - 0.5) * 7
        const my_ = sy + (ey - sy) * 0.45 + (Math.random() - 0.5) * 7
        ctx!.beginPath(); ctx!.moveTo(sx, sy); ctx!.lineTo(mx_, my_); ctx!.lineTo(ex, ey)
        ctx!.strokeStyle = `rgba(0,255,136,${a * 0.85})`; ctx!.lineWidth = 1.5
        ctx!.lineJoin = 'round'; ctx!.lineCap = 'round'; ctx!.stroke()
        ctx!.beginPath(); ctx!.arc(ex, ey, 2, 0, Math.PI * 2)
        ctx!.fillStyle = `rgba(0,255,136,${a * 0.65})`; ctx!.fill()
      })
    }

    let rafId: number

    function draw() {
      ctx!.clearRect(0, 0, 280, 300)

      eyeTimer++
      if (eyeTimer >= EYE_DUR[eyePhase]) { eyeTimer = 0; eyePhase = (eyePhase + 1) % 2 }
      const isWink = eyePhase === 1

      const ringR = 128 + Math.sin(t * 0.02) * 6
      ctx!.beginPath(); ctx!.arc(148, 172, ringR, 0, Math.PI * 2)
      ctx!.strokeStyle = `rgba(0,255,136,${0.1 + Math.sin(t * 0.02) * 0.08})`
      ctx!.lineWidth = 1.5; ctx!.stroke()

      drawBolts()

      rr(44, 195, 28, 54, 12, C.body, C.outline, 3)
      circ(58, 193, 15, C.body, C.outline, 3)
      rr(2, 130, 68, 64, 5, C.chartBg, C.green, 2.5)
      rr(6, 134, 60, 56, 3, '#030310', null)

      const candles = [
        { x: 12, yH: 162, h: 14, bull: false }, { x: 24, yH: 150, h: 14, bull: true },
        { x: 36, yH: 146, h: 12, bull: false }, { x: 48, yH: 136, h: 14, bull: true },
        { x: 60, yH: 126, h: 13, bull: true },
      ]
      candles.forEach(c => {
        const col = c.bull ? C.green : C.red
        ln(c.x + 3, c.yH - 4, c.x + 3, c.yH, col, 1.5)
        ln(c.x + 3, c.yH + c.h, c.x + 3, c.yH + c.h + 4, col, 1.5)
        px(c.x, c.yH, 7, c.h, col)
      })

      if (chartProgress < 1) chartProgress += 0.008
      const pts = [[12, 176], [24, 164], [36, 160], [48, 147], [60, 136], [70, 124]]
      ctx!.beginPath(); ctx!.moveTo(pts[0][0], pts[0][1])
      const drawTo = Math.floor(chartProgress * (pts.length - 1))
      for (let i = 1; i <= drawTo; i++) ctx!.lineTo(pts[i][0], pts[i][1])
      if (drawTo < pts.length - 1) {
        const frac = chartProgress * (pts.length - 1) - drawTo
        ctx!.lineTo(pts[drawTo][0] + (pts[drawTo + 1][0] - pts[drawTo][0]) * frac,
          pts[drawTo][1] + (pts[drawTo + 1][1] - pts[drawTo][1]) * frac)
      }
      ctx!.strokeStyle = C.cyan; ctx!.lineWidth = 2; ctx!.lineJoin = 'round'; ctx!.lineCap = 'round'; ctx!.stroke()

      poly([[60, 121], [66, 121], [63, 117]], C.cyan, null)

      ctx!.fillStyle = C.green; ctx!.font = '6px monospace'
      ctx!.fillText('NKVO', 6, 188); ctx!.fillText('+8.3%', 40, 188)

      rr(74, 190, 152, 106, 20, C.body, C.outline, 3.5)

      ctx!.strokeStyle = `rgba(0,204,255,${0.3 + Math.sin(t * 0.04) * 0.2})`
      ctx!.lineWidth = 1.5
      ctx!.beginPath();(ctx as any).roundRect(108, 228, 64, 44, 4); ctx!.stroke()
      ln(140, 228, 140, 272, `rgba(0,204,255,.3)`, 1)
      ln(108, 250, 172, 250, `rgba(0,204,255,.3)`, 1)
      circ(140, 250, 5, `rgba(0,204,255,${0.4 + Math.sin(t * 0.06 + 1) * 0.3})`, null)
      circ(120, 238, 3, `rgba(0,204,255,${0.4 + Math.sin(t * 0.06 + 2) * 0.3})`, null)
      circ(160, 262, 3, `rgba(0,204,255,${0.4 + Math.sin(t * 0.06 + 3) * 0.3})`, null)

      ctx!.save(); ctx!.beginPath(); ctx!.ellipse(148, 152, 70, 66, 0, 0, Math.PI * 2)
      ctx!.fillStyle = C.body; ctx!.fill()
      ctx!.strokeStyle = C.outline; ctx!.lineWidth = 3.5; ctx!.stroke(); ctx!.restore()

      poly([[92, 104], [72, 62], [118, 94]], C.body, C.outline, 3)
      poly([[90, 100], [79, 70], [112, 92]], C.red, null)
      poly([[208, 104], [228, 62], [182, 94]], C.body, C.outline, 3)
      poly([[210, 100], [221, 70], [188, 92]], C.red, null)

      ;([[112, 136, false], [168, 136, true]] as [number, number, boolean][]).forEach(([ex, ey, isRight]) => {
        if (isRight && isWink) {
          ctx!.beginPath(); ctx!.moveTo(ex + 2, ey + 14)
          ctx!.bezierCurveTo(ex + 6, ey + 6, ex + 18, ey + 6, ex + 22, ey + 14)
          ctx!.strokeStyle = C.outline; ctx!.lineWidth = 4; ctx!.lineCap = 'round'; ctx!.stroke()
          ;[6, 10, 14, 18].forEach(lx => {
            const ly = ey + 14 - Math.sin((lx - 2) / 22 * Math.PI) * 8
            ln(ex + lx, ly, ex + lx, ly - 7, C.outline, 2.5)
          })
          return
        }
        rr(ex, ey, 24, 24, 4, '#0a1020', C.cyan, 2)
        ln(ex + 4, ey + 6, ex + 20, ey + 6, C.cyan, 1.5)
        ln(ex + 4, ey + 18, ex + 20, ey + 18, C.cyan, 1.5)
        ln(ex + 12, ey + 2, ex + 12, ey + 22, C.cyan, 1.5)
        ln(ex + 4, ey + 2, ex + 4, ey + 6, C.cyan, 1.5)
        ln(ex + 20, ey + 2, ex + 20, ey + 6, C.cyan, 1.5)
        ln(ex + 4, ey + 18, ex + 4, ey + 22, C.cyan, 1.5)
        ln(ex + 20, ey + 18, ex + 20, ey + 22, C.cyan, 1.5)
        const g = 0.5 + Math.sin(t * 0.05) * 0.45
        ctx!.globalAlpha = g; circ(ex + 12, ey + 12, 5, C.cyan, null)
        ctx!.globalAlpha = 0.15 + g * 0.2; circ(ex + 12, ey + 12, 9, C.cyan, null)
        ctx!.globalAlpha = 1
      })

      poly([[148, 166], [142, 174], [154, 174]], C.red, null)

      ctx!.beginPath(); ctx!.moveTo(138, 178)
      ctx!.quadraticCurveTo(148, 190, 158, 178)
      ctx!.strokeStyle = C.outline; ctx!.lineWidth = 2.5; ctx!.lineCap = 'round'; ctx!.stroke()

      ctx!.globalAlpha = 0.5
      ell(118, 177, 13, 8, C.pink, null)
      ell(178, 177, 13, 8, C.pink, null)
      ctx!.globalAlpha = 1

      ctx!.globalAlpha = 0.65
      ln(100, 170, 136, 172, C.outline, 1.5)
      ln(100, 179, 136, 177, C.outline, 1.5)
      ln(160, 172, 196, 170, C.outline, 1.5)
      ln(160, 177, 196, 179, C.outline, 1.5)
      ctx!.globalAlpha = 1

      pawAngle = 0.35 + Math.sin(t * 0.045) * 0.3
      ctx!.save(); ctx!.translate(206, 200); ctx!.rotate(pawAngle)
      rr(-13, -72, 26, 72, 13, C.body, C.outline, 3)
      circ(0, -78, 20, C.body, C.outline, 3)
      ln(-10, -97, -10, -86, C.outline, 2.5)
      ln(0, -100, 0, -88, C.outline, 2.5)
      ln(10, -97, 10, -86, C.outline, 2.5)
      ell(0, -76, 7, 5, C.pink, null)
      circ(-10, -82, 4, C.pink, null)
      circ(10, -82, 4, C.pink, null)
      ctx!.restore()

      rr(88, 206, 124, 24, 12, C.red, C.outline, 3)
      ;[102, 116, 172, 186].forEach(sx => { px(sx, 213, 7, 7, '#ff6680') })

      bellAngle = Math.sin(t * 0.025) * 0.15
      ctx!.save(); ctx!.translate(150, 240); ctx!.rotate(bellAngle); ctx!.translate(-150, -240)
      rr(148, 222, 4, 8, 2, C.goldDk, null)
      circ(150, 244, 17, C.gold, C.outline, 2.5)
      ctx!.globalAlpha = 0.5; ell(144, 238, 5, 3, '#fff9aa', null); ctx!.globalAlpha = 1
      ctx!.fillStyle = C.goldDk; ctx!.font = 'bold 22px serif'; ctx!.textAlign = 'center'
      ctx!.fillText('₿', 150, 252); ctx!.textAlign = 'left'; ctx!.restore()

      ell(108, 288, 26, 12, C.body, C.outline, 2.5)
      ell(192, 288, 26, 12, C.body, C.outline, 2.5)
      ;[[96, 108, 120], [180, 192, 204]].forEach(xs =>
        xs.forEach(x => ln(x, 284, x, 292, C.outline, 2))
      )

      ctx!.globalAlpha = 0.5; ctx!.fillStyle = C.green
      ;[[0, 0], [274, 0], [0, 294], [274, 294]].forEach(([cx, cy]) => {
        ctx!.fillRect(cx, cy, 6, 6)
        ctx!.globalAlpha = 0.3
        ctx!.fillRect(cx + (cx > 0 ? -10 : 10), cy, 4, 4)
        ctx!.fillRect(cx, cy + (cy > 0 ? -8 : 8), 4, 4)
        ctx!.globalAlpha = 0.5
      })
      ctx!.globalAlpha = 1

      t++
      rafId = requestAnimationFrame(draw)
    }

    draw()
    return () => cancelAnimationFrame(rafId)
  }, [])

  const scale = size / 280
  return (
    <canvas
      ref={canvasRef}
      width={280}
      height={300}
      style={{
        width: size,
        height: size * (300 / 280),
        imageRendering: 'pixelated',
        filter: 'drop-shadow(0 0 14px rgba(0,255,136,.6)) drop-shadow(0 0 40px rgba(0,255,136,.25))',
        display: 'block',
      }}
    />
  )
}
