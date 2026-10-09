<script setup lang="ts">
import { computed } from 'vue'
import { steamHeader, type Game } from '@/api'
import { count } from '@/format'

const props = defineProps<{ game: Game }>()
const banner = computed(() => steamHeader(props.game))
</script>

<template>
  <RouterLink :to="`/games/${game.id}`" class="game card">
    <img v-if="banner" :src="banner" alt="" loading="lazy" width="460" height="215" />
    <div v-else class="placeholder" aria-hidden="true">{{ game.name.slice(0, 1) }}</div>
    <div class="body">
      <strong>{{ game.name }}</strong>
      <span class="muted">{{ count(game.modes, 'mode') }}</span>
    </div>
  </RouterLink>
</template>

<style scoped>
.game {
  padding: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

img,
.placeholder {
  width: 100%;
  aspect-ratio: 460 / 215;
  object-fit: cover;
  background: var(--raised);
}

.placeholder {
  display: grid;
  place-items: center;
  font-size: 40px;
  font-weight: 700;
  color: var(--muted);
}

.body {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  gap: 12px;
  padding: 12px 14px;
}

.body .muted {
  flex: none;
  font-size: 14px;
}
</style>
