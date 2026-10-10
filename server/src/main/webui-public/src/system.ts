// The visitor's system, told by their browser: its download is offered first.

export type System = 'windows' | 'linux'

export function visitorSystem(): System | null {
  const agent = navigator.userAgent
  if (agent.includes('Windows')) return 'windows'
  if (/Linux|X11/.test(agent) && !agent.includes('Android')) return 'linux'
  return null
}
