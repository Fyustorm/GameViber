<script setup lang="ts">
import { computed, ref } from 'vue'
import { api } from '@/api'
import { useLoad } from '@/load'
import { setTitle } from '@/router'
import GameCard from '@/components/GameCard.vue'

setTitle('Community modes')

// Every game is loaded once and filtered here: there are few, and typing stays instant.
const games = useLoad(() => api.games())
const search = ref('')
const slug = (s: string) => s.toLowerCase().replace(/[^\p{L}\p{N}]/gu, '')
const shown = computed(() => (games.data.value ?? []).filter((g) => slug(g.name).includes(slug(search.value))))
</script>

<template>
  <div class="wrap page">
    <p class="eyebrow">Community</p>
    <h1>Modes by game</h1>
    <p class="lead">
      Modes other players made and shared. Install them from the app's <strong>Community</strong> page, or download
      one here.
    </p>
    <label class="visually-hidden" for="search">Search a game</label>
    <input id="search" v-model="search" class="field" type="search" placeholder="Search a game" autocomplete="off" />

    <div v-if="games.loading.value" class="grid three list">
      <div v-for="n in 6" :key="n" class="skeleton tile" />
    </div>
    <p v-else-if="games.error.value" class="state error">
      The community modes cannot be reached right now. <button class="button small" @click="games.reload()">Try again</button>
    </p>
    <p v-else-if="!shown.length && search" class="state">No mode for “{{ search }}” yet. Make one in the app, then share it.</p>
    <p v-else-if="!shown.length" class="state">No mode published yet.</p>
    <div v-else class="grid three list">
      <GameCard v-for="game in shown" :key="game.id" :game="game" />
    </div>
  </div>
</template>

<style scoped>
.page {
  padding-top: 32px;
}

.list {
  margin-top: 20px;
}

.tile {
  aspect-ratio: 460 / 260;
}
</style>
