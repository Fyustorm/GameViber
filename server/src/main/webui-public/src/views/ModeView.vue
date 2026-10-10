<script setup lang="ts">
import { computed, ref, watch, watchEffect } from 'vue'
import { api, modePaths, steamHeader, ApiError, USES } from '@/api'
import { useLoad } from '@/load'
import { setTitle } from '@/router'
import { ago, date, size } from '@/format'
import FigureList from '@/components/FigureList.vue'

// A public mode by its id, or any mode by the code its author shares with testers.
const props = defineProps<{ id?: string; code?: string }>()

const by = computed(() => (props.code ? { code: props.code } : { id: props.id ?? '' }))
const mode = useLoad(() => (props.code ? api.shared(props.code) : api.mode(props.id ?? '')), by)
const paths = computed(() => modePaths(by.value))
// Handled by GameViber's desktop entry (`gameviber/src/links`).
const appLink = computed(() => (props.code ? `gameviber://shared/${props.code}` : `gameviber://mode/${props.id}`))
const versions = computed(() => [...(mode.data.value?.versions ?? [])].sort((a, b) => b.number - a.number))
const latest = computed(() => versions.value[0])
const missing = computed(() => mode.error.value instanceof ApiError && mode.error.value.status === 404)

// The first capture of the package, else the game's banner, else nothing.
const failedImages = ref(0)
watch(by, () => (failedImages.value = 0))
const image = computed(() => {
  const game = mode.data.value?.game
  return [paths.value.image, game && steamHeader(game)].filter(Boolean)[failedImages.value] ?? null
})

const copied = ref(false)
async function copyCode() {
  if (!props.code) return
  await navigator.clipboard.writeText(props.code)
  copied.value = true
  setTimeout(() => (copied.value = false), 2000)
}

watchEffect(() => {
  const m = mode.data.value
  setTitle(m ? `${m.name} for ${m.game.name}` : missing.value ? 'Mode not found' : undefined)
})
</script>

<template>
  <div class="wrap page">
    <div v-if="mode.loading.value" class="skeleton" style="height: 320px" />

    <div v-else-if="missing" class="state">
      <h1>No such mode</h1>
      <p>{{ code ? 'This code matches no mode: it may have been replaced by a new one.' : 'It was removed, or is not public.' }}</p>
      <RouterLink to="/games" class="button">Browse the community modes</RouterLink>
    </div>

    <p v-else-if="mode.error.value" class="state error">
      {{ mode.error.value.message }} <button class="button small" @click="mode.reload()">Try again</button>
    </p>

    <article v-else-if="mode.data.value" class="mode">
      <RouterLink :to="`/games/${mode.data.value.game.id}`" class="back">‹ {{ mode.data.value.game.name }}</RouterLink>

      <div class="layout">
        <div class="main">
          <img
            v-if="image"
            :src="image"
            alt=""
            class="shot cover"
            @error="failedImages++"
          />
          <header>
            <p class="eyebrow">
              Mode for {{ mode.data.value.game.name }}
              <span v-if="code" class="tag accent">Shared with you</span>
            </p>
            <h1>{{ mode.data.value.name }}</h1>
            <p class="muted">
              by <strong>{{ mode.data.value.author }}</strong> · updated {{ ago(mode.data.value.updatedAt) }}
            </p>
          </header>

          <FigureList :figures="mode.data.value.figures" :downloads="mode.data.value.downloads" class="figures" />

          <ul v-if="mode.data.value.uses?.length" class="uses">
            <li v-for="use in mode.data.value.uses" :key="use">
              <span class="tag accent">{{ USES[use].label }}</span>
              <span class="muted">{{ USES[use].text }}</span>
            </li>
          </ul>

          <p v-if="mode.data.value.description" class="description">{{ mode.data.value.description }}</p>
          <p v-else class="muted">No description.</p>
        </div>

        <aside class="install card">
          <template v-if="mode.data.value.withdrawnAt">
            <h2>Withdrawn</h2>
            <p class="muted">
              This mode was withdrawn {{ ago(mode.data.value.withdrawnAt) }}<template v-if="mode.data.value.withdrawnReason">: {{ mode.data.value.withdrawnReason }}</template>.
            </p>
          </template>
          <template v-else>
            <h2>Install it</h2>
            <a :href="appLink" class="button primary wide">Open in GameViber</a>
            <p class="muted small hint">
              Opens its page in GameViber on this computer, to install it in a click.
              No GameViber yet? <RouterLink to="/download">Download it</RouterLink>
            </p>
            <details class="other-ways" :open="!!code">
              <summary>Other ways</summary>
              <ol>
                <li v-if="code">
                  In GameViber, open <strong>Community</strong> and enter the code
                  <button type="button" class="code" :title="copied ? 'Copied' : 'Copy the code'" @click="copyCode">
                    <span class="mono">{{ code }}</span>
                    <span class="muted">{{ copied ? 'copied' : 'copy' }}</span>
                  </button>
                </li>
                <li v-else>
                  In GameViber, open <strong>Community</strong> and look for
                  <strong>{{ mode.data.value.game.name }}</strong>.
                </li>
                <li>Or download the file, then <strong>Library › Import a file</strong>.</li>
              </ol>
              <a :href="paths.package" class="button wide" download>
                Download .gameviber
                <span v-if="latest" class="weight">v{{ latest.number }} · {{ size(latest.size) }}</span>
              </a>
            </details>
          </template>
        </aside>
      </div>

      <section class="versions">
        <h2>Versions</h2>
        <ol>
          <li v-for="v in versions" :key="v.number">
            <div class="version-head">
              <strong>Version {{ v.number }}</strong>
              <span class="muted">{{ date(v.createdAt) }}</span>
            </div>
            <p v-if="v.changelog" class="changelog">{{ v.changelog }}</p>
          </li>
        </ol>
      </section>
    </article>
  </div>
</template>

<style scoped>
.page {
  padding-top: 24px;
}

.back {
  display: inline-block;
  padding: 10px 0;
  color: var(--muted);
}

.layout {
  display: grid;
  gap: 24px;
  margin-top: 8px;
}

.cover {
  width: 100%;
  aspect-ratio: 16 / 9;
  object-fit: cover;
  margin-bottom: 20px;
}

.eyebrow .tag {
  margin-left: 8px;
  letter-spacing: 0;
  text-transform: none;
}

h1 {
  word-break: break-word;
}

.figures {
  margin: 16px 0 20px;
  padding: 14px 0;
  border-block: 1px solid var(--line);
}

.uses {
  list-style: none;
  padding: 0;
  margin: 0 0 20px;
  display: grid;
  gap: 8px;
}

.uses .tag {
  margin-right: 8px;
}

.description,
.changelog {
  white-space: pre-line;
  overflow-wrap: anywhere;
}

.install h2 {
  font-size: 20px;
}

.install ol {
  padding-left: 20px;
  margin: 0 0 16px;
}

.install li {
  margin-bottom: 10px;
}

.code {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  width: 100%;
  min-height: 44px;
  margin-top: 8px;
  padding: 0 14px;
  border: 1px dashed var(--accent);
  border-radius: 10px;
  background: var(--accent-soft);
  color: var(--text);
  font: inherit;
  font-size: 17px;
  cursor: pointer;
}

.code .muted {
  font-size: 13px;
}

.wide {
  width: 100%;
  flex-wrap: wrap;
  padding-block: 8px;
}

.hint {
  margin: 10px 0 0;
}

.other-ways {
  margin-top: 16px;
  border-top: 1px solid var(--line);
  padding-top: 12px;
}

.other-ways summary {
  cursor: pointer;
  min-height: 32px;
  color: var(--muted);
  font-weight: 600;
}

.other-ways ol {
  margin-top: 10px;
}

.weight {
  font-weight: 400;
  font-size: 13px;
  opacity: 0.8;
}

.small {
  font-size: 14px;
}

.versions {
  margin-top: 40px;
}

.versions ol {
  list-style: none;
  padding: 0;
  margin: 0;
}

.versions li {
  padding: 14px 0;
  border-top: 1px solid var(--line);
}

.version-head {
  display: flex;
  justify-content: space-between;
  gap: 12px;
}

.changelog {
  margin: 6px 0 0;
  color: var(--muted);
}

@media (min-width: 900px) {
  .layout {
    grid-template-columns: 1fr 340px;
    align-items: start;
  }

  .install {
    position: sticky;
    top: 80px;
  }
}
</style>
