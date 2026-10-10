// The community server's public API (`server/README.md`), as the site reads it.

export interface Game {
  id: number
  name: string
  steamAppId: number | null
  modes: number
}

export interface Figures {
  players: number
  medianMinutes: number
  cameBack: number
  likes: number
  dislikes: number
  rating: number
  trend: number
}

export interface ModeSummary {
  id: string
  name: string
  description: string
  author: string
  downloads: number
  version: number
  api: number
  uses: Use[]
  updatedAt: string
  figures: Figures
}

// What a mode is made for, beyond the gamepad and the sound every mode reads (`SharedPackage.uses`).
export type Use = 'strokers' | 'screen' | 'programs'

export const USES: Record<Use, { label: string; text: string }> = {
  strokers: {
    label: 'Made for strokers',
    text: 'Written for strokers too: its own strokes, thrusts or funscripts. Vibrators play it as well.',
  },
  screen: {
    label: 'Reads the screen',
    text: "Reads the game's image: a health bar, a menu. On Linux, turn the in-game overlay on for this game.",
  },
  programs: {
    label: 'Needs another program',
    text: 'Reads values another program sends (a game mod, a script): its description says which.',
  },
}

export interface Version {
  number: number
  changelog: string
  api: number
  size: number
  createdAt: string
}

export interface ModeDetail {
  id: string
  name: string
  description: string
  game: Game
  author: string
  downloads: number
  versions: Version[]
  uses: Use[]
  createdAt: string
  updatedAt: string
  withdrawnAt: string | null
  withdrawnReason: string | null
  figures: Figures
}

export type Sort = 'trending' | 'rating' | 'played' | 'new' | 'downloads'

export class ApiError extends Error {
  constructor(message: string, readonly status: number) {
    super(message)
  }
}

async function get<T>(path: string): Promise<T> {
  const response = await fetch(path, { headers: { Accept: 'application/json' } })
  if (!response.ok) {
    const body = await response.json().catch(() => ({}))
    throw new ApiError(body.error ?? response.statusText, response.status)
  }
  return response.json()
}

export const api = {
  games: (search = '') => get<Game[]>(`/api/games?search=${encodeURIComponent(search)}`),
  game: (id: number) => get<Game>(`/api/games/${id}`),
  modes: (game: number, sort: Sort) => get<ModeSummary[]>(`/api/games/${game}/modes?sort=${sort}`),
  mode: (id: string) => get<ModeDetail>(`/api/modes/${encodeURIComponent(id)}`),
  shared: (code: string) => get<ModeDetail>(`/api/shared/${encodeURIComponent(code)}`),
}

/** Where a mode's package and image are, by its public id or its share code. */
export function modePaths(by: { id: string } | { code: string }) {
  const base = 'id' in by ? `/api/modes/${encodeURIComponent(by.id)}` : `/api/shared/${encodeURIComponent(by.code)}`
  return { package: `${base}/package`, image: `${base}/image` }
}

/** The game's banner on Steam, when GameViber knows its app id. */
export function steamHeader(game: Pick<Game, 'steamAppId'>): string | null {
  return game.steamAppId ? `https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/${game.steamAppId}/header.jpg` : null
}
