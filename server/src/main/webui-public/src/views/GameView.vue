<script setup lang="ts">
import { computed, ref, watchEffect } from 'vue'
import { api, steamHeader, ApiError, type Sort } from '@/api'
import { useLoad } from '@/load'
import { setTitle } from '@/router'
import ModeCard from '@/components/ModeCard.vue'

const props = defineProps<{ id: number }>()

const sorts: { value: Sort; label: string }[] = [
  { value: 'trending', label: 'Trending' },
  { value: 'rating', label: 'Top rated' },
  { value: 'played', label: 'Most played' },
  { value: 'new', label: 'New' },
  { value: 'downloads', label: 'Downloads' },
]
const sort = ref<Sort>('trending')

const { data: game } = useLoad(() => api.game(props.id), () => props.id)
const modes = useLoad(() => api.modes(props.id, sort.value), () => [props.id, sort.value])
const banner = computed(() => (game.value ? steamHeader(game.value) : null))
const missing = computed(() => modes.error.value instanceof ApiError && modes.error.value.status === 404)

watchEffect(() => setTitle(game.value ? `${game.value.name} modes` : undefined))
</script>

<template>
  <div class="wrap page">
    <RouterLink to="/games" class="back">‹ All games</RouterLink>
    <header class="head">
      <img v-if="banner" :src="banner" alt="" width="460" height="215" class="banner" />
      <div>
        <p class="eyebrow">Community modes</p>
        <h1>{{ game?.name ?? (missing ? 'Unknown game' : ' ') }}</h1>
      </div>
    </header>

    <div class="chips" role="group" aria-label="Sort">
      <button v-for="s in sorts" :key="s.value" type="button" :aria-pressed="sort === s.value" @click="sort = s.value">
        {{ s.label }}
      </button>
    </div>

    <div v-if="modes.loading.value && !modes.data.value" class="grid list">
      <div v-for="n in 4" :key="n" class="skeleton tile" />
    </div>
    <p v-else-if="missing" class="state">This game has no public mode. <RouterLink to="/games">See the others</RouterLink></p>
    <p v-else-if="modes.error.value" class="state error">
      The modes cannot be reached right now. <button class="button small" @click="modes.reload()">Try again</button>
    </p>
    <p v-else-if="!modes.data.value?.length" class="state">No public mode for this game yet.</p>
    <div v-else class="grid list" :class="{ dim: modes.loading.value }">
      <ModeCard v-for="mode in modes.data.value" :key="mode.id" :mode="mode" />
    </div>
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

.head {
  display: grid;
  gap: 16px;
  margin: 8px 0 20px;
}

.head h1 {
  margin: 0;
}

.banner {
  border-radius: var(--radius);
  width: 100%;
  max-width: 460px;
}

.list {
  margin-top: 20px;
}

.dim {
  opacity: 0.6;
  transition: opacity 0.2s;
}

.tile {
  height: 160px;
}

@media (min-width: 640px) {
  .head {
    grid-template-columns: 240px 1fr;
    align-items: center;
  }
}
</style>
