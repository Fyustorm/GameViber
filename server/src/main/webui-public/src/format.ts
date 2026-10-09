const relative = new Intl.RelativeTimeFormat('en', { numeric: 'auto' })

/** "3 days ago", "last month". */
export function ago(iso: string): string {
  const seconds = (new Date(iso).getTime() - Date.now()) / 1000
  const units: [Intl.RelativeTimeFormatUnit, number][] = [
    ['year', 31536000],
    ['month', 2592000],
    ['week', 604800],
    ['day', 86400],
    ['hour', 3600],
    ['minute', 60],
  ]
  for (const [unit, size] of units) {
    if (Math.abs(seconds) >= size) return relative.format(Math.round(seconds / size), unit)
  }
  return 'just now'
}

export function date(iso: string): string {
  return new Date(iso).toLocaleDateString('en', { year: 'numeric', month: 'short', day: 'numeric' })
}

export function count(n: number, one: string, many = one + 's'): string {
  return `${n.toLocaleString('en')} ${n === 1 ? one : many}`
}

export function size(bytes: number): string {
  return bytes < 1 << 20 ? `${Math.max(1, Math.round(bytes / 1024))} kB` : `${(bytes / (1 << 20)).toFixed(1)} MB`
}
