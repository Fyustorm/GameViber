<script setup lang="ts">
import { computed, ref } from 'vue'
import type { Capture, ModeSetup } from '@/api'
import { count, words } from '@/format'
import Lightbox from '@/components/Lightbox.vue'

// What a mode's package sets up: its phases first (their captures seen one
// phase at a time), the gauges it reads drawn over the game, its other
// indicators, variants and funscripts.
const props = defineProps<{ setup: ModeSetup; capture: (file: string) => string }>()

// The captures seen full size: a phase's, or the one a gauge is drawn on.
const viewed = ref<Capture[]>([])
const shown = ref<number | null>(null)
const images = computed(() =>
  viewed.value.map((c) => ({ src: props.capture(c.file), caption: c.phase ? words(c.phase) : 'Not sorted' })),
)

function view(captures: Capture[]) {
  viewed.value = captures
  shown.value = captures.length ? 0 : null
}

function viewPhase(phase: string) {
  view(props.setup.captures.filter((c) => c.phase === phase))
}

function viewFile(file: string) {
  view(props.setup.captures.filter((c) => c.file === file))
}

const phases = computed(() =>
  props.setup.phases.map((phase) => {
    const own = props.setup.captures.filter((c) => c.phase === phase.name)
    return { ...phase, cover: own[0]?.file ?? null, captures: own.length }
  }),
)

// The gauges drawn over the capture they were drawn on, one figure per capture.
const gauges = computed(() => {
  const byCapture = new Map<string, { name: string; rect: number[] }[]>()
  for (const indicator of props.setup.indicators.filter((i) => i.kind === 'gauge')) {
    for (const zone of indicator.zones) {
      if (!zone.capture) continue
      const boxes = byCapture.get(zone.capture) ?? []
      // An indicator in several places of one capture is drawn once there, where it was drawn first.
      if (!boxes.some((b) => b.name === indicator.name)) boxes.push({ name: indicator.name, rect: zone.rect })
      byCapture.set(zone.capture, boxes)
    }
  }
  return [...byCapture.entries()].map(([file, boxes]) => ({ file, boxes }))
})
const gaugeNames = computed(() => props.setup.indicators.filter((i) => i.kind === 'gauge').map((i) => i.name))
const shownNames = computed(() => props.setup.indicators.filter((i) => i.kind === 'visibility').map((i) => i.name))

function box(rect: number[]) {
  const [left, top, width, height] = rect
  return { left: `${left * 100}%`, top: `${top * 100}%`, width: `${width * 100}%`, height: `${height * 100}%` }
}
</script>

<template>
  <section v-if="phases.length" class="block">
    <h2>Phases</h2>
    <p class="muted">
      The parts of the game this mode tells apart, from the sound and the image, so that each one feels different.
    </p>
    <ul class="phases">
      <li v-for="phase in phases" :key="phase.name" class="card phase">
        <button v-if="phase.cover" type="button" class="thumb" @click="viewPhase(phase.name)">
          <img :src="capture(phase.cover)" alt="" loading="lazy" />
        </button>
        <div v-else class="thumb empty" aria-hidden="true">{{ words(phase.name).slice(0, 1) }}</div>
        <div class="phase-body">
          <h3>{{ words(phase.name) }}</h3>
          <p v-if="phase.screen" class="muted small">Looks like: {{ phase.screen }}</p>
          <p v-if="phase.sound" class="muted small">Sounds like: {{ phase.sound }}</p>
          <p v-if="phase.indicators.length" class="muted small">
            Recognized when {{ phase.indicators.map(words).join(' and ') }} {{ phase.indicators.length > 1 ? 'show' : 'shows' }}
          </p>
          <p v-if="phase.captures" class="muted small">{{ count(phase.captures, 'capture') }}</p>
        </div>
      </li>
    </ul>
  </section>

  <section v-if="setup.indicators.length" class="block">
    <h2>What it reads on screen</h2>
    <div v-if="gauges.length" class="gauges">
      <button v-for="figure in gauges" :key="figure.file" type="button" class="drawn" @click="viewFile(figure.file)">
        <img :src="capture(figure.file)" alt="" loading="lazy" />
        <span v-for="b in figure.boxes" :key="b.name" class="box" :style="box(b.rect)">
          <span class="label" :class="{ below: b.rect[1] < 0.1 }">{{ words(b.name) }}</span>
        </span>
      </button>
    </div>
    <dl class="indicators">
      <template v-if="gaugeNames.length">
        <dt>Gauges <span class="muted">how full a bar is</span></dt>
        <dd>
          <span v-for="name in gaugeNames" :key="name" class="tag accent">{{ words(name) }}</span>
        </dd>
      </template>
      <template v-if="shownNames.length">
        <dt>On screen or not <span class="muted">a menu, a boss bar, a part of the interface</span></dt>
        <dd>
          <span v-for="name in shownNames" :key="name" class="tag">{{ words(name) }}</span>
        </dd>
      </template>
    </dl>
  </section>

  <section v-if="setup.variants.length || setup.funscripts.length" class="block">
    <h2>Also in it</h2>
    <dl class="indicators">
      <template v-if="setup.variants.length">
        <dt>Variants <span class="muted">other scripts of the mode, to pick in the app</span></dt>
        <dd>
          <span v-for="name in setup.variants" :key="name" class="tag">{{ words(name) }}</span>
        </dd>
      </template>
      <template v-if="setup.funscripts.length">
        <dt>Funscripts <span class="muted">motions for strokers</span></dt>
        <dd>
          <span v-for="name in setup.funscripts" :key="name" class="tag">{{ words(name) }}</span>
        </dd>
      </template>
    </dl>
  </section>

  <Lightbox v-model="shown" :images="images" />
</template>

<style scoped>
.block {
  margin-top: 32px;
}

.small {
  font-size: 14px;
  margin: 0 0 4px;
}

.phases {
  list-style: none;
  padding: 0;
  margin: 12px 0 0;
  display: grid;
  gap: 12px;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
}

.phase {
  padding: 0;
  overflow: hidden;
}

.phase-body {
  padding: 12px 14px 10px;
}

.phase h3 {
  margin-bottom: 6px;
  text-transform: capitalize;
}

.thumb {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 9;
  padding: 0;
  border: 0;
  background: var(--raised);
  cursor: zoom-in;
}

.thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.thumb.empty {
  display: grid;
  place-items: center;
  font-size: 40px;
  font-weight: 700;
  color: var(--muted);
  text-transform: uppercase;
  cursor: default;
}

.gauges {
  margin: 12px 0 16px;
  display: grid;
  gap: 12px;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
}

.drawn {
  position: relative;
  display: block;
  width: 100%;
  padding: 0;
  border: 1px solid var(--line);
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--panel);
  cursor: zoom-in;
}

.drawn img {
  width: 100%;
  filter: brightness(0.7);
}

.box {
  position: absolute;
  min-width: 6px;
  min-height: 6px;
  border: 2px solid var(--accent);
  border-radius: 3px;
  box-shadow: 0 0 12px rgb(240 109 148 / 0.6);
}

.label {
  position: absolute;
  bottom: calc(100% + 4px);
  left: -2px;
  padding: 1px 6px;
  border-radius: 4px;
  background: var(--accent);
  color: var(--on-accent);
  font-size: 11px;
  font-weight: 700;
  white-space: nowrap;
}

/* A bar at the top of the screen: its name under it. */
.label.below {
  bottom: auto;
  top: calc(100% + 4px);
}

.indicators {
  margin: 0;
}

.indicators dt {
  font-weight: 600;
  margin-top: 12px;
}

.indicators dt .muted {
  font-weight: 400;
  font-size: 14px;
  margin-left: 6px;
}

.indicators dd {
  margin: 6px 0 0;
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
</style>
